package main

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sync"
	"sync/atomic"
	"time"

	"password/internal/crypto"
	"password/internal/export"
	"password/internal/model"
	"password/internal/settings"
	"password/internal/store"

	"github.com/wailsapp/wails/v2/pkg/runtime"
)

type App struct {
	ctx         context.Context
	store       *store.Store
	key         []byte
	mu          sync.RWMutex
	settingsDir string
	dbPath      string
	settings    *settings.Settings
	// quitting 标记用户已明确要求退出（托盘菜单「退出」）。
	// OnBeforeClose 依赖它区分「关闭窗口（只隐藏）」与「真正退出」。
	quitting atomic.Bool
	// autoLockStop 用于通知自动锁定监控协程退出。
	autoLockStop chan struct{}
	// autoLockOnce 保证监控协程只启动一次。
	autoLockOnce sync.Once
}

func NewApp() *App {
	return &App{}
}

// withKey 在已解锁状态下获取密钥副本并执行 fn，执行结束后清零副本，
// 避免调用方持有与 a.key 共享底层数组的切片头、
// 在 RUnlock 后被并发 Lock() 清零导致的 TOCTOU。
func (a *App) withKey(fn func(key []byte) error) error {
	a.mu.RLock()
	src := a.key
	a.mu.RUnlock()
	if src == nil {
		return errors.New("未解锁")
	}
	// 拷贝独立副本，与 a.key 底层数组解耦
	key := make([]byte, len(src))
	copy(key, src)
	defer func() {
		for i := range key {
			key[i] = 0
		}
	}()
	return fn(key)
}

func validateCredential(c *model.Credential) error {
	if c.Name == "" {
		return errors.New("名称不能为空")
	}
	if len(c.Name) > 256 {
		return errors.New("名称过长（最多 256 字符）")
	}
	if len(c.Secret) > 65536 {
		return errors.New("密钥/密码过长（最多 64KB）")
	}
	if c.Type < model.TypeBrowserPassword || c.Type > model.TypeBankCard {
		return errors.New("无效的凭证类型")
	}
	return nil
}

func (a *App) startup(ctx context.Context) {
	a.ctx = ctx
	startupLog("--- startup begin ---")

	// 便携版：数据优先落在 exe 同级的 data/ 子目录，
	// 目录不可写时自动回退到 %APPDATA%（见 resolveDataDir）。
	appDataDir, usedDir, err := resolveDataDir()
	if err != nil {
		startupLog("resolveDataDir FAILED: %v", err)
		runtime.MessageDialog(ctx, runtime.MessageDialogOptions{
			Type:    runtime.ErrorDialog,
			Title:   "启动失败",
			Message: "无法确定数据目录: " + err.Error(),
		})
		runtime.Quit(ctx)
		return
	}
	startupLog("portable=%s used=%s", appDataDir, usedDir)

	if err := os.MkdirAll(appDataDir, 0700); err != nil {
		startupLog("MkdirAll FAILED: %v", err)
		runtime.MessageDialog(ctx, runtime.MessageDialogOptions{
			Type:    runtime.ErrorDialog,
			Title:   "启动失败",
			Message: "无法创建数据目录: " + err.Error(),
		})
		runtime.Quit(ctx)
		return
	}

	a.settingsDir = appDataDir
	s, err := settings.Load(appDataDir)
	if err != nil {
		startupLog("settings.Load FAILED: %v", err)
		runtime.MessageDialog(ctx, runtime.MessageDialogOptions{
			Type:    runtime.ErrorDialog,
			Title:   "启动失败",
			Message: "无法读取配置: " + err.Error(),
		})
		runtime.Quit(ctx)
		return
	}
	a.settings = s

	dbPath := s.DbPath
	if dbPath == "" {
		dbPath = filepath.Join(appDataDir, "data.db")
	}
	a.dbPath = dbPath
	startupLog("dbPath=%s", dbPath)

	store, err := store.New(dbPath)
	if err != nil {
		startupLog("store.New FAILED: %v", err)
		runtime.MessageDialog(ctx, runtime.MessageDialogOptions{
			Type:    runtime.ErrorDialog,
			Title:   "启动失败",
			Message: "数据库打开失败: " + err.Error(),
		})
		runtime.Quit(ctx)
		return
	}
	a.store = store

	// 托盘放在最后初始化：托盘菜单的「显示主窗口」「立即锁定」都依赖
	// a.ctx 与 a.store 已经就绪，且需要 Wails 主消息循环已经起来。
	a.initTray()
	startupLog("initTray returned")

	// 闲置自动锁定监控：独立协程轮询系统级空闲时长。
	// 放在托盘之后启动，保证锁定广播事件时前端与窗口都已就绪。
	a.startAutoLockMonitor()
	startupLog("--- startup done ---")
}

func (a *App) shutdown(ctx context.Context) {
	// 先停自动锁定监控，避免退出过程中它还在访问已关闭的资源。
	a.stopAutoLockMonitor()

	a.mu.Lock()
	if a.key != nil {
		for i := range a.key {
			a.key[i] = 0
		}
		a.key = nil
	}
	a.mu.Unlock()

	if a.store != nil {
		a.store.Close()
	}

	// 兜底收起托盘图标：正常退出路径已由 quitApp 处理，
	// 这里覆盖异常退出（如系统注销）的情况，避免通知区残留图标。
	stopTray()
}

// GetDbPath 返回当前数据库路径
func (a *App) GetDbPath() string {
	return a.dbPath
}

// GetDataDir 返回配置与默认数据库所在的数据目录。
// 便携版为 exe 同级的 data/，回退时为 %APPDATA%\PasswordManager。
func (a *App) GetDataDir() string {
	return a.settingsDir
}

// IsPortable 返回当前是否运行在便携模式（数据目录与 exe 同级）。
// 前端据此展示「便携版 / 系统版」标识。
func (a *App) IsPortable() bool {
	if a.settingsDir == "" {
		return false
	}
	exe, err := os.Executable()
	if err != nil {
		return false
	}
	if resolved, evalErr := filepath.EvalSymlinks(exe); evalErr == nil {
		exe = resolved
	}
	return filepath.Dir(a.settingsDir) == filepath.Dir(exe)
}

// GetAutoLockMinutes 返回当前自动锁定时长（分钟）。0 表示永不自动锁定。
func (a *App) GetAutoLockMinutes() int {
	return a.autoLockMinutes()
}

// SetAutoLockMinutes 设置自动锁定时长（分钟）并持久化到配置文件。
// 传入 0 表示关闭自动锁定。仅接受预设的白名单值，避免前端传入异常数值
// 导致配置文件被写坏。
func (a *App) SetAutoLockMinutes(minutes int) error {
	valid := map[int]bool{0: true, 1: true, 5: true, 10: true, 15: true, 30: true}
	if !valid[minutes] {
		return fmt.Errorf("不支持的锁定时长: %d 分钟", minutes)
	}

	a.mu.Lock()
	if a.settings == nil {
		a.mu.Unlock()
		return errors.New("配置未初始化")
	}
	a.settings.AutoLockMinutes = &minutes
	s := a.settings
	dir := a.settingsDir
	a.mu.Unlock()

	if err := s.Save(dir); err != nil {
		return fmt.Errorf("保存配置失败: %w", err)
	}
	return nil
}

// ChangeDbPath 更改数据库存储位置
func (a *App) ChangeDbPath() (string, error) {
	if a.store == nil {
		return "", errors.New("数据库未初始化")
	}

	filePath, err := runtime.SaveFileDialog(a.ctx, runtime.SaveDialogOptions{
		Title:           "选择数据库存储位置",
		DefaultFilename: "data.db",
		Filters: []runtime.FileFilter{
			{DisplayName: "数据库文件 (*.db)", Pattern: "*.db"},
		},
	})
	if err != nil || filePath == "" {
		return "", errors.New("已取消")
	}

	// 如果选的就是当前路径，直接返回
	if filePath == a.dbPath {
		return a.dbPath, nil
	}

	// 先关闭当前连接，触发 WAL checkpoint，确保 -wal 文件中的事务落盘，
	// 避免只复制 data.db 而丢失尚未 checkpoint 的写入
	a.store.Close()

	// 复制数据库及配套的 -wal / -shm 文件
	if err := copyDbFiles(a.dbPath, filePath); err != nil {
		// 复制失败：回滚——重新打开原路径
		if oldStore, openErr := store.New(a.dbPath); openErr == nil {
			a.store = oldStore
		} else {
			// 回滚也失败：明确报错而非吞掉，避免 a.store 变成 nil 后续 panic
			return "", fmt.Errorf("复制数据库失败且回滚打开原库也失败: 复制错误=%v, 回滚错误=%v", err, openErr)
		}
		return "", fmt.Errorf("无法复制数据库: %w", err)
	}

	// 打开新位置的数据库
	newStore, err := store.New(filePath)
	if err != nil {
		// 回滚：重新打开原路径
		if oldStore, openErr := store.New(a.dbPath); openErr == nil {
			a.store = oldStore
		} else {
			return "", fmt.Errorf("新数据库打开失败且回滚打开原库也失败: 打开错误=%v, 回滚错误=%v", err, openErr)
		}
		return "", fmt.Errorf("新数据库打开失败: %w", err)
	}
	a.store = newStore
	a.dbPath = filePath

	// 保存设置
	a.settings.DbPath = filePath
	if err := a.settings.Save(a.settingsDir); err != nil {
		return filePath, fmt.Errorf("路径已更改但设置保存失败: %w", err)
	}
	return filePath, nil
}

// ResetDbPath 恢复默认数据库路径
func (a *App) ResetDbPath() error {
	if a.store == nil {
		return errors.New("数据库未初始化")
	}

	defaultPath := filepath.Join(a.settingsDir, "data.db")
	if a.dbPath == defaultPath {
		return errors.New("已经是默认路径")
	}

	// 先关闭当前连接，触发 WAL checkpoint
	a.store.Close()

	// 如果默认位置没有 db 文件，从当前位置复制过去
	if _, err := os.Stat(defaultPath); os.IsNotExist(err) {
		if copyErr := copyDbFiles(a.dbPath, defaultPath); copyErr != nil {
			// 复制失败：回滚——重新打开原路径
			if oldStore, openErr := store.New(a.dbPath); openErr == nil {
				a.store = oldStore
			} else {
				return fmt.Errorf("复制到默认路径失败且回滚打开原库也失败: 复制错误=%v, 回滚错误=%v", copyErr, openErr)
			}
			return fmt.Errorf("无法复制数据库到默认路径: %w", copyErr)
		}
	}

	// 打开默认路径
	newStore, err := store.New(defaultPath)
	if err != nil {
		// 回滚：重新打开原路径
		if oldStore, openErr := store.New(a.dbPath); openErr == nil {
			a.store = oldStore
		} else {
			return fmt.Errorf("默认数据库打开失败且回滚打开原库也失败: 打开错误=%v, 回滚错误=%v", err, openErr)
		}
		return fmt.Errorf("默认数据库打开失败: %w", err)
	}
	a.store = newStore
	a.dbPath = defaultPath

	// 清空自定义路径设置
	a.settings.DbPath = ""
	if err := a.settings.Save(a.settingsDir); err != nil {
		return fmt.Errorf("设置保存失败: %w", err)
	}
	return nil
}

// copyDbFiles 复制 SQLite 数据库主文件及配套的 -wal / -shm 文件，
// 并将副本权限设为 0600（仅所有者可读写），避免密码库副本被同机其他用户读取。
func copyDbFiles(srcPath, dstPath string) error {
	src, err := os.Open(srcPath)
	if err != nil {
		return fmt.Errorf("无法读取当前数据库: %w", err)
	}
	defer src.Close()

	// 用 0600 创建目标文件，而非默认的 0666
	dst, err := os.OpenFile(dstPath, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0600)
	if err != nil {
		return fmt.Errorf("无法创建目标文件: %w", err)
	}
	defer dst.Close()

	if _, err := io.Copy(dst, src); err != nil {
		return fmt.Errorf("复制数据库失败: %w", err)
	}
	if err := dst.Close(); err != nil {
		return fmt.Errorf("关闭目标文件失败: %w", err)
	}

	// 复制 -wal / -shm（存在时）；这些文件携带尚未 checkpoint 的事务
	for _, suffix := range []string{"-wal", "-shm"} {
		// walPath 是检查源文件是否存在
		walSrc := srcPath + suffix
		if _, err := os.Stat(walSrc); err != nil {
			continue // 文件不存在，跳过
		}
		if err := copyFile0600(walSrc, dstPath+suffix); err != nil {
			return err
		}
	}

	return nil
}

// copyFile0600 复制单个文件并以 0600 权限落盘
func copyFile0600(srcPath, dstPath string) error {
	src, err := os.Open(srcPath)
	if err != nil {
		return err
	}
	defer src.Close()

	dst, err := os.OpenFile(dstPath, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0600)
	if err != nil {
		return err
	}
	defer dst.Close()

	if _, err := io.Copy(dst, src); err != nil {
		return err
	}
	return dst.Close()
}

// IsMasterSet 检查是否已设置主密钥
func (a *App) IsMasterSet() bool {
	if a.store == nil {
		return false
	}
	_, err := a.store.GetMasterConfig()
	return err == nil
}

// SetupMaster 首次设置主密钥
func (a *App) SetupMaster(password string) error {
	if a.store == nil {
		return errors.New("数据库未初始化")
	}

	salt, err := crypto.GenerateSalt()
	if err != nil {
		return err
	}

	key := crypto.DeriveKey(password, salt)

	verifyToken, err := crypto.GenerateVerifyToken()
	if err != nil {
		return err
	}

	encryptedVerify, err := crypto.Encrypt(key, verifyToken)
	if err != nil {
		return err
	}

	if err := a.store.SetMasterConfig(salt, encryptedVerify); err != nil {
		return err
	}

	a.mu.Lock()
	a.key = key
	a.mu.Unlock()
	return nil
}

// Unlock 解锁
func (a *App) Unlock(password string) error {
	if a.store == nil {
		return errors.New("数据库未初始化")
	}

	// 服务端暴力破解防护（状态持久化到数据库，重启不丢失）
	st, err := a.store.GetUnlockState()
	if err != nil {
		return fmt.Errorf("读取锁定状态失败: %w", err)
	}
	if !st.LockedUntil.IsZero() && time.Now().Before(st.LockedUntil) {
		remaining := int(st.LockedUntil.Sub(time.Now()).Seconds())
		return fmt.Errorf("已锁定，请 %d 秒后再试", remaining)
	}

	mc, err := a.store.GetMasterConfig()
	if err != nil {
		return errors.New("未设置主密钥")
	}

	key := crypto.DeriveKey(password, mc.Salt)

	_, err = crypto.Decrypt(key, mc.EncryptedVerify)
	if err != nil {
		// 解锁失败：累加失败计数并持久化
		st.FailCount++
		if st.FailCount >= 5 {
			st.LockedUntil = time.Now().Add(30 * time.Second)
			st.FailCount = 0
			_ = a.store.SetUnlockState(st)
			return errors.New("连续错误 5 次，已锁定 30 秒")
		}
		_ = a.store.SetUnlockState(st)
		return errors.New("主密钥错误")
	}

	// 成功：清零失败状态并持久化
	st.FailCount = 0
	st.LockedUntil = time.Time{}
	_ = a.store.SetUnlockState(st)

	a.mu.Lock()
	a.key = key
	a.mu.Unlock()
	return nil
}

// Lock 锁定
func (a *App) Lock() {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.key != nil {
		for i := range a.key {
			a.key[i] = 0
		}
		a.key = nil
	}
}

// IsUnlocked 检查是否已解锁
func (a *App) IsUnlocked() bool {
	a.mu.RLock()
	defer a.mu.RUnlock()
	return a.key != nil
}

// GetCredentials 获取凭证列表
func (a *App) GetCredentials(credType int, keyword string) []model.CredentialView {
	if a.store == nil {
		return nil
	}

	var ct *model.CredentialType
	if credType >= 0 {
		t := model.CredentialType(credType)
		ct = &t
	}

	credentials, err := a.store.ListCredentials(ct, keyword)
	if err != nil {
		return nil
	}

	views := make([]model.CredentialView, len(credentials))
	for i, c := range credentials {
		views[i] = c.ToView()
	}
	return views
}

// GetCredentialViewById 获取单条凭证视图（不含密文）
func (a *App) GetCredentialViewById(id int64) (*model.CredentialView, error) {
	if a.store == nil {
		return nil, errors.New("数据库未初始化")
	}
	c, err := a.store.GetCredential(id)
	if err != nil {
		return nil, err
	}
	view := c.ToView()
	return &view, nil
}

// AddCredential 添加凭证
func (a *App) AddCredential(c model.Credential) error {
	if err := validateCredential(&c); err != nil {
		return err
	}

	return a.withKey(func(key []byte) error {
		encrypted, err := crypto.Encrypt(key, []byte(c.Secret))
		if err != nil {
			return err
		}
		c.Secret = string(encrypted)
		return a.store.CreateCredential(&c)
	})
}

// UpdateCredential 更新凭证。
//
// 两个字段需要「缺省即保留」，因为前端编辑表单并不提供它们的值：
//
//   - Secret：编辑表单的密码框恒为空值，占位符写明「留空则不修改」。
//     若照旧把空字符串加密后写回，就等于用「空密码」覆盖用户已保存的密码
//     （查看时看不到、复制为空、导出也是空的）。
//   - IsFavorite：编辑表单没有收藏开关，收藏状态只能由 ToggleFavorite 改变。
//     表单固定提交 is_favorite=0，直接写回会让「编辑一次就掉收藏」。
//
// 因此这里先读回库中现有记录：密码留空时沿用原密文，收藏状态始终保留。
// 显式传入非空密码时照常重新加密，改密码不受影响。
func (a *App) UpdateCredential(c model.Credential) error {
	if err := validateCredential(&c); err != nil {
		return err
	}

	return a.withKey(func(key []byte) error {
		existing, err := a.store.GetCredential(c.ID)
		if err != nil {
			if errors.Is(err, sql.ErrNoRows) {
				return errors.New("凭证不存在")
			}
			return err
		}

		c.IsFavorite = existing.IsFavorite

		if c.Secret == "" {
			c.Secret = existing.Secret
		} else {
			encrypted, err := crypto.Encrypt(key, []byte(c.Secret))
			if err != nil {
				return err
			}
			c.Secret = string(encrypted)
		}

		return a.store.UpdateCredential(&c)
	})
}

// DeleteCredential 删除凭证
func (a *App) DeleteCredential(id int64) error {
	a.mu.RLock()
	unlocked := a.key != nil
	a.mu.RUnlock()
	if !unlocked {
		return errors.New("未解锁")
	}
	return a.store.DeleteCredential(id)
}

// DeleteCredentials 批量删除凭证
func (a *App) DeleteCredentials(ids []int64) error {
	a.mu.RLock()
	unlocked := a.key != nil
	a.mu.RUnlock()
	if !unlocked {
		return errors.New("未解锁")
	}
	return a.store.DeleteCredentials(ids)
}

// ToggleFavorite 切换收藏
func (a *App) ToggleFavorite(id int64) error {
	a.mu.RLock()
	unlocked := a.key != nil
	a.mu.RUnlock()
	if !unlocked {
		return errors.New("未解锁")
	}
	return a.store.ToggleFavorite(id)
}

// GetDecryptedSecretDirect 直接解密密码（已解锁状态下调用，无需二次验证）
func (a *App) GetDecryptedSecretDirect(id int64) (string, error) {
	if a.store == nil {
		return "", errors.New("数据库未初始化")
	}

	var result string
	err := a.withKey(func(key []byte) error {
		c, err := a.store.GetCredential(id)
		if err != nil {
			return err
		}
		plaintext, err := crypto.Decrypt(key, []byte(c.Secret))
		if err != nil {
			return err
		}
		result = string(plaintext)
		return nil
	})
	return result, err
}

// GetDecryptedSecret 解密密码（需二次验证）
func (a *App) GetDecryptedSecret(id int64, verifyPassword string) (string, error) {
	if a.store == nil {
		return "", errors.New("数据库未初始化")
	}

	mc, err := a.store.GetMasterConfig()
	if err != nil {
		return "", err
	}

	verifyKey := crypto.DeriveKey(verifyPassword, mc.Salt)
	_, err = crypto.Decrypt(verifyKey, mc.EncryptedVerify)
	if err != nil {
		return "", errors.New("主密钥验证失败")
	}

	var result string
	err = a.withKey(func(key []byte) error {
		c, err := a.store.GetCredential(id)
		if err != nil {
			return err
		}
		plaintext, err := crypto.Decrypt(key, []byte(c.Secret))
		if err != nil {
			return err
		}
		result = string(plaintext)
		return nil
	})
	return result, err
}

// GetFavorites 获取收藏列表
func (a *App) GetFavorites(keyword string) []model.CredentialView {
	if a.store == nil {
		return nil
	}
	credentials, err := a.store.ListFavorites(keyword)
	if err != nil {
		return nil
	}

	views := make([]model.CredentialView, len(credentials))
	for i, c := range credentials {
		views[i] = c.ToView()
	}
	return views
}

// GetCredentialCounts 返回各类型凭证数量，key -1=全部, -2=收藏
func (a *App) GetCredentialCounts() map[int]int {
	if a.store == nil {
		return nil
	}
	counts, err := a.store.CountByType()
	if err != nil {
		return nil
	}
	return counts
}

// GeneratePassword 生成随机密码
func (a *App) GeneratePassword(length int) (string, error) {
	return crypto.GenerateRandomPassword(length, true, true, true)
}

// CheckPasswordStrength 检查密码强度
func (a *App) CheckPasswordStrength(password string) int {
	return int(model.CheckPasswordStrength(password))
}

// ChangeMasterPassword 修改主密钥
func (a *App) ChangeMasterPassword(oldPassword, newPassword string) error {
	if a.store == nil {
		return errors.New("数据库未初始化")
	}

	// 验证旧密码
	mc, err := a.store.GetMasterConfig()
	if err != nil {
		return errors.New("未设置主密钥")
	}

	oldKey := crypto.DeriveKey(oldPassword, mc.Salt)
	_, err = crypto.Decrypt(oldKey, mc.EncryptedVerify)
	if err != nil {
		for i := range oldKey {
			oldKey[i] = 0
		}
		return errors.New("旧密码错误")
	}
	defer func() {
		for i := range oldKey {
			oldKey[i] = 0
		}
	}()

	// 生成新盐和新密钥
	newSalt, err := crypto.GenerateSalt()
	if err != nil {
		return err
	}
	newKey := crypto.DeriveKey(newPassword, newSalt)

	// committed 标记事务是否成功；失败时必须清零 newKey，避免 32 字节密钥泄露
	committed := false
	defer func() {
		if !committed {
			for i := range newKey {
				newKey[i] = 0
			}
		}
	}()

	// 在事务中重新加密所有凭证并更新主配置
	err = a.store.WithTransaction(func(tx *sql.Tx) error {
		// 获取所有凭证（使用事务）
		rows, err := tx.Query(`SELECT id, secret FROM credentials`)
		if err != nil {
			return err
		}
		defer rows.Close()

		type credSecret struct {
			id     int64
			secret string
		}
		var creds []credSecret
		for rows.Next() {
			var cs credSecret
			if err := rows.Scan(&cs.id, &cs.secret); err != nil {
				return err
			}
			creds = append(creds, cs)
		}
		if err := rows.Err(); err != nil {
			return err
		}

		// 重新加密每条凭证
		for _, cs := range creds {
			plaintext, err := crypto.Decrypt(oldKey, []byte(cs.secret))
			if err != nil {
				return fmt.Errorf("解密凭证 ID=%d 失败: %w", cs.id, err)
			}
			newEncrypted, err := crypto.Encrypt(newKey, plaintext)
			if err != nil {
				return fmt.Errorf("重新加密凭证 ID=%d 失败: %w", cs.id, err)
			}
			// 重新加密后立即清零明文
			for i := range plaintext {
				plaintext[i] = 0
			}
			_, err = tx.Exec(
				"UPDATE credentials SET secret=?, updated_at=datetime('now','localtime') WHERE id=?",
				newEncrypted, cs.id,
			)
			if err != nil {
				return fmt.Errorf("更新凭证 ID=%d 失败: %w", cs.id, err)
			}
		}

		// 更新主密钥配置
		verifyToken, err := crypto.GenerateVerifyToken()
		if err != nil {
			return err
		}
		newEncryptedVerify, err := crypto.Encrypt(newKey, verifyToken)
		if err != nil {
			return err
		}
		_, err = tx.Exec(
			"INSERT OR REPLACE INTO master_config (id, salt, encrypted_verify) VALUES (1, ?, ?)",
			newSalt, newEncryptedVerify,
		)
		return err
	})

	if err != nil {
		return err
	}

	// 事务成功提交：newKey 转交 a.key，不再清零
	committed = true

	// 更新内存中的密钥（先清零旧 key，再赋新值）
	a.mu.Lock()
	if a.key != nil {
		for i := range a.key {
			a.key[i] = 0
		}
	}
	a.key = newKey
	a.mu.Unlock()
	return nil
}

// ExportData 导出数据 (format: "json" or "excel")
func (a *App) ExportData(format string) (string, error) {
	if a.store == nil {
		return "", errors.New("数据库未初始化")
	}

	var ext, filterName, filterPattern string
	if format == "excel" {
		ext = ".xlsx"
		filterName = "Excel 文件 (*.xlsx)"
		filterPattern = "*.xlsx"
	} else {
		ext = ".json"
		filterName = "JSON 文件 (*.json)"
		filterPattern = "*.json"
	}

	filePath, err := runtime.SaveFileDialog(a.ctx, runtime.SaveDialogOptions{
		Title:           "导出凭证",
		DefaultFilename: fmt.Sprintf("passwords_%s%s", time.Now().Format("20060102"), ext),
		Filters: []runtime.FileFilter{
			{DisplayName: filterName, Pattern: filterPattern},
		},
	})
	if err != nil || filePath == "" {
		return "", errors.New("已取消")
	}

	credentials, err := a.store.GetAllCredentials()
	if err != nil {
		return "", err
	}

	var exportCreds []export.ExportCredential
	var failCount int
	exportErr := a.withKey(func(key []byte) error {
		for _, c := range credentials {
			plaintext, err := crypto.Decrypt(key, []byte(c.Secret))
			if err != nil {
				failCount++
				continue
			}
			exportCreds = append(exportCreds, export.ExportCredentialToExport(c, string(plaintext)))
		}
		return nil
	})
	if exportErr != nil {
		return "", exportErr
	}

	if format == "excel" {
		err = export.ExportExcel(exportCreds, filePath)
	} else {
		err = export.ExportJSON(exportCreds, filePath)
	}

	if err != nil {
		return "", err
	}

	if failCount > 0 {
		return filePath, fmt.Errorf("导出完成，但有 %d 条凭证解密失败被跳过", failCount)
	}
	return filePath, nil
}

// ImportData 导入数据
func (a *App) ImportData() (int, error) {
	if a.store == nil {
		return 0, errors.New("数据库未初始化")
	}
	if !a.IsUnlocked() {
		return 0, errors.New("未解锁")
	}

	filePath, err := runtime.OpenFileDialog(a.ctx, runtime.OpenDialogOptions{
		Title: "导入凭证",
		Filters: []runtime.FileFilter{
			{DisplayName: "支持的文件 (*.json;*.xlsx)", Pattern: "*.json;*.xlsx"},
			{DisplayName: "JSON 文件 (*.json)", Pattern: "*.json"},
			{DisplayName: "Excel 文件 (*.xlsx)", Pattern: "*.xlsx"},
		},
	})
	if err != nil || filePath == "" {
		return 0, errors.New("已取消")
	}

	var exportCreds []export.ExportCredential
	if export.GuessFormat(filePath) == "excel" {
		exportCreds, err = export.ImportExcel(filePath)
	} else {
		exportCreds, err = export.ImportJSON(filePath)
	}
	if err != nil {
		return 0, fmt.Errorf("解析文件失败: %w", err)
	}

	count := 0
	var failCount int
	importErr := a.withKey(func(key []byte) error {
		for _, ec := range exportCreds {
			if ec.Name == "" || ec.Secret == "" {
				continue
			}
			c := export.ExportCredentialToModel(ec)
			encrypted, err := crypto.Encrypt(key, []byte(c.Secret))
			if err != nil {
				failCount++
				continue
			}
			c.Secret = string(encrypted)
			if err := a.store.CreateCredential(&c); err != nil {
				failCount++
				continue
			}
			count++
		}
		return nil
	})
	if importErr != nil {
		return 0, importErr
	}

	if failCount > 0 {
		return count, fmt.Errorf("成功导入 %d 条，%d 条失败", count, failCount)
	}
	return count, nil
}

// ExportSingle 导出单条凭证 (format: "json" or "excel")
func (a *App) ExportSingle(id int64, format string) (string, error) {
	if a.store == nil {
		return "", errors.New("数据库未初始化")
	}

	c, err := a.store.GetCredential(id)
	if err != nil {
		return "", err
	}

	var plaintext string
	err = a.withKey(func(key []byte) error {
		pt, err := crypto.Decrypt(key, []byte(c.Secret))
		if err != nil {
			return err
		}
		plaintext = string(pt)
		return nil
	})
	if err != nil {
		return "", err
	}

	var ext, filterName, filterPattern string
	if format == "excel" {
		ext = ".xlsx"
		filterName = "Excel 文件 (*.xlsx)"
		filterPattern = "*.xlsx"
	} else {
		ext = ".json"
		filterName = "JSON 文件 (*.json)"
		filterPattern = "*.json"
	}

	filePath, err := runtime.SaveFileDialog(a.ctx, runtime.SaveDialogOptions{
		Title:           "导出凭证",
		DefaultFilename: c.Name + ext,
		Filters: []runtime.FileFilter{
			{DisplayName: filterName, Pattern: filterPattern},
		},
	})
	if err != nil || filePath == "" {
		return "", errors.New("已取消")
	}

	ec := export.ExportCredentialToExport(*c, plaintext)

	if format == "excel" {
		err = export.ExportExcel([]export.ExportCredential{ec}, filePath)
	} else {
		err = export.ExportJSON([]export.ExportCredential{ec}, filePath)
	}

	if err != nil {
		return "", err
	}
	return filePath, nil
}

// DownloadJSONTemplate 下载JSON模板
func (a *App) DownloadJSONTemplate() (string, error) {
	filePath, err := runtime.SaveFileDialog(a.ctx, runtime.SaveDialogOptions{
		Title:           "保存JSON模板",
		DefaultFilename: "password_template.json",
		Filters: []runtime.FileFilter{
			{DisplayName: "JSON 文件 (*.json)", Pattern: "*.json"},
		},
	})
	if err != nil || filePath == "" {
		return "", errors.New("已取消")
	}

	data, err := export.JSONTemplate()
	if err != nil {
		return "", err
	}
	if err := os.WriteFile(filePath, data, 0600); err != nil {
		return "", err
	}
	return filePath, nil
}

// DownloadExcelTemplate 下载Excel模板
func (a *App) DownloadExcelTemplate() (string, error) {
	filePath, err := runtime.SaveFileDialog(a.ctx, runtime.SaveDialogOptions{
		Title:           "保存Excel模板",
		DefaultFilename: "password_template.xlsx",
		Filters: []runtime.FileFilter{
			{DisplayName: "Excel 文件 (*.xlsx)", Pattern: "*.xlsx"},
		},
	})
	if err != nil || filePath == "" {
		return "", errors.New("已取消")
	}

	if err := export.ExcelTemplate(filePath); err != nil {
		return "", err
	}
	return filePath, nil
}

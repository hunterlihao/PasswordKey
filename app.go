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
	"time"

	"password/internal/crypto"
	"password/internal/export"
	"password/internal/model"
	"password/internal/settings"
	"password/internal/store"

	"github.com/wailsapp/wails/v2/pkg/runtime"
)

type App struct {
	ctx              context.Context
	store            *store.Store
	key              []byte
	mu               sync.RWMutex
	unlockFailCount  int
	unlockLockedUntil time.Time
	settingsDir      string
	dbPath           string
	settings         *settings.Settings
}

func NewApp() *App {
	return &App{}
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

	appDataDir, err := getAppDataDir()
	if err != nil {
		runtime.MessageDialog(ctx, runtime.MessageDialogOptions{
			Type:    runtime.ErrorDialog,
			Title:   "启动失败",
			Message: "无法获取应用数据目录: " + err.Error(),
		})
		runtime.Quit(ctx)
		return
	}

	if err := os.MkdirAll(appDataDir, 0700); err != nil {
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

	store, err := store.New(dbPath)
	if err != nil {
		runtime.MessageDialog(ctx, runtime.MessageDialogOptions{
			Type:    runtime.ErrorDialog,
			Title:   "启动失败",
			Message: "数据库打开失败: " + err.Error(),
		})
		runtime.Quit(ctx)
		return
	}
	a.store = store
}

func (a *App) shutdown(ctx context.Context) {
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
}

// GetDbPath 返回当前数据库路径
func (a *App) GetDbPath() string {
	return a.dbPath
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

	// 复制当前数据库到新路径
	src, err := os.Open(a.dbPath)
	if err != nil {
		return "", fmt.Errorf("无法读取当前数据库: %w", err)
	}
	defer src.Close()

	dst, err := os.Create(filePath)
	if err != nil {
		return "", fmt.Errorf("无法创建目标文件: %w", err)
	}
	defer dst.Close()

	if _, err := io.Copy(dst, src); err != nil {
		return "", fmt.Errorf("复制数据库失败: %w", err)
	}
	dst.Close()
	src.Close()

	// 关闭当前连接，打开新位置
	a.store.Close()
	newStore, err := store.New(filePath)
	if err != nil {
		// 回滚：重新打开原路径
		oldStore, _ := store.New(a.dbPath)
		a.store = oldStore
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

	// 如果默认位置没有 db 文件，从当前位置复制过去
	if _, err := os.Stat(defaultPath); os.IsNotExist(err) {
		src, err := os.Open(a.dbPath)
		if err != nil {
			return fmt.Errorf("无法读取当前数据库: %w", err)
		}
		defer src.Close()

		dst, err := os.Create(defaultPath)
		if err != nil {
			return fmt.Errorf("无法创建默认路径文件: %w", err)
		}
		if _, err := io.Copy(dst, src); err != nil {
			dst.Close()
			return fmt.Errorf("复制数据库失败: %w", err)
		}
		dst.Close()
	}

	// 关闭当前连接，打开默认路径
	a.store.Close()
	newStore, err := store.New(defaultPath)
	if err != nil {
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

func getAppDataDir() (string, error) {
	appData := os.Getenv("APPDATA")
	if appData == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			return "", err
		}
		appData = filepath.Join(home, "AppData", "Roaming")
	}
	return filepath.Join(appData, "PasswordManager"), nil
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

	// 服务端暴力破解防护
	a.mu.RLock()
	locked := time.Now().Before(a.unlockLockedUntil)
	remaining := int(a.unlockLockedUntil.Sub(time.Now()).Seconds())
	a.mu.RUnlock()
	if locked {
		return fmt.Errorf("已锁定，请 %d 秒后再试", remaining)
	}

	mc, err := a.store.GetMasterConfig()
	if err != nil {
		return errors.New("未设置主密钥")
	}

	key := crypto.DeriveKey(password, mc.Salt)

	_, err = crypto.Decrypt(key, mc.EncryptedVerify)
	if err != nil {
		a.mu.Lock()
		a.unlockFailCount++
		if a.unlockFailCount >= 5 {
			a.unlockLockedUntil = time.Now().Add(30 * time.Second)
			a.unlockFailCount = 0
			a.mu.Unlock()
			return errors.New("连续错误 5 次，已锁定 30 秒")
		}
		a.mu.Unlock()
		return errors.New("主密钥错误")
	}

	a.mu.Lock()
	a.key = key
	a.unlockFailCount = 0
	a.unlockLockedUntil = time.Time{}
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

	a.mu.RLock()
	key := a.key
	a.mu.RUnlock()
	if key == nil {
		return errors.New("未解锁")
	}

	encrypted, err := crypto.Encrypt(key, []byte(c.Secret))
	if err != nil {
		return err
	}
	c.Secret = string(encrypted)

	return a.store.CreateCredential(&c)
}

// UpdateCredential 更新凭证
func (a *App) UpdateCredential(c model.Credential) error {
	if err := validateCredential(&c); err != nil {
		return err
	}

	a.mu.RLock()
	key := a.key
	a.mu.RUnlock()
	if key == nil {
		return errors.New("未解锁")
	}

	encrypted, err := crypto.Encrypt(key, []byte(c.Secret))
	if err != nil {
		return err
	}
	c.Secret = string(encrypted)

	return a.store.UpdateCredential(&c)
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

	a.mu.RLock()
	key := a.key
	a.mu.RUnlock()
	if key == nil {
		return "", errors.New("未解锁")
	}

	c, err := a.store.GetCredential(id)
	if err != nil {
		return "", err
	}

	plaintext, err := crypto.Decrypt(key, []byte(c.Secret))
	if err != nil {
		return "", err
	}

	return string(plaintext), nil
}

// GetDecryptedSecret 解密密码（需二次验证）
func (a *App) GetDecryptedSecret(id int64, verifyPassword string) (string, error) {
	if a.store == nil {
		return "", errors.New("数据库未初始化")
	}

	a.mu.RLock()
	key := a.key
	a.mu.RUnlock()
	if key == nil {
		return "", errors.New("未解锁")
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

	c, err := a.store.GetCredential(id)
	if err != nil {
		return "", err
	}

	plaintext, err := crypto.Decrypt(key, []byte(c.Secret))
	if err != nil {
		return "", err
	}

	return string(plaintext), nil
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

	// 更新内存中的密钥
	a.mu.Lock()
	a.key = newKey
	a.mu.Unlock()
	return nil
}

// ExportData 导出数据 (format: "json" or "excel")
func (a *App) ExportData(format string) (string, error) {
	a.mu.RLock()
	key := a.key
	a.mu.RUnlock()
	if key == nil {
		return "", errors.New("未解锁")
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
	for _, c := range credentials {
		plaintext, err := crypto.Decrypt(key, []byte(c.Secret))
		if err != nil {
			failCount++
			continue
		}
		exportCreds = append(exportCreds, export.ExportCredentialToExport(c, string(plaintext)))
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
	a.mu.RLock()
	key := a.key
	a.mu.RUnlock()
	if key == nil {
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

	if failCount > 0 {
		return count, fmt.Errorf("成功导入 %d 条，%d 条失败", count, failCount)
	}
	return count, nil
}

// ExportSingle 导出单条凭证 (format: "json" or "excel")
func (a *App) ExportSingle(id int64, format string) (string, error) {
	a.mu.RLock()
	key := a.key
	a.mu.RUnlock()
	if key == nil {
		return "", errors.New("未解锁")
	}

	c, err := a.store.GetCredential(id)
	if err != nil {
		return "", err
	}

	plaintext, err := crypto.Decrypt(key, []byte(c.Secret))
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

	ec := export.ExportCredentialToExport(*c, string(plaintext))

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

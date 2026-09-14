package main

import (
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
)

// exeDirName 便携版数据目录名。放在 exe 同级，整个目录可整体拷到 U 盘运行。
const exeDirName = "data"

// legacyAppDirName 旧版的 %APPDATA% 子目录名，仅用于一次性迁移。
const legacyAppDirName = "PasswordManager"

// dataDirFallback 带回退的系统数据目录，仅用于记录了「降级为系统版」。
// 一旦标记存在就不再尝试便携目录，避免在只读目录场景下反复弹 UAC 或告警。
const dataDirFallback = "system_dir"

// resolveDataDir 解析应用数据目录，并保证可写。
//
// 便携版优先：exe 同级的 data/ 子目录。若该位置不可写（例如程序装在
// C:\Program Files、U 盘被写保护），则回退到 %APPDATA%\PasswordManager，
// 并在便携目录留下一个标记文件，让后续启动直接走回退路径。
//
// 首次启用便携模式时会从旧位置一次性迁移配置与数据库。
func resolveDataDir() (portableDir string, appDir string, err error) {
	exe, err := os.Executable()
	if err != nil {
		return "", "", fmt.Errorf("无法定位程序路径: %w", err)
	}
	// 解析符号链接，避免从快捷方式/链接启动时把 data/ 建到链接所在目录
	if resolved, evalErr := filepath.EvalSymlinks(exe); evalErr == nil {
		exe = resolved
	}
	exeDir := filepath.Dir(exe)

	fallback, err := fallbackDataDir()
	if err != nil {
		return "", "", err
	}

	portableDir = filepath.Join(exeDir, exeDirName)

	// 已有降级标记：不再打扰便携目录
	if _, statErr := os.Stat(filepath.Join(portableDir, dataDirFallback)); statErr == nil {
		return portableDir, fallback, nil
	}

	if err := ensureWritableDir(portableDir); err != nil {
		// 便携目录不可用：留标记（能留则留）并回退
		_ = os.MkdirAll(portableDir, 0700)
		_ = os.WriteFile(filepath.Join(portableDir, dataDirFallback), []byte(err.Error()+"\n"), 0600)
		return portableDir, fallback, nil
	}

	// 便携目录可用：首次启用时从旧位置迁移一次
	if err := migrateLegacyData(portableDir, fallback); err != nil {
		// 迁移失败不阻断启动，仅提示；用户仍可手动拷贝
		return portableDir, portableDir, nil
	}

	return portableDir, portableDir, nil
}

// ensureWritableDir 创建目录（若不存在）并实测可写。
// 不能只靠权限位判断：Windows 上 UAC 只读目录的权限位仍显示可写。
// 同时校验「创建目录」与「创建文件」，覆盖 exe 位于只读介质的情况。
func ensureWritableDir(dir string) error {
	if err := os.MkdirAll(dir, 0700); err != nil {
		return fmt.Errorf("无法创建目录: %w", err)
	}

	probe := filepath.Join(dir, ".write-probe")
	f, err := os.OpenFile(probe, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0600)
	if err != nil {
		return fmt.Errorf("目录不可写: %w", err)
	}
	_ = f.Close()
	_ = os.Remove(probe)
	return nil
}

// fallbackDataDir 返回旧版使用的 %APPDATA%\PasswordManager 路径。
func fallbackDataDir() (string, error) {
	appData := os.Getenv("APPDATA")
	if appData == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			return "", err
		}
		appData = filepath.Join(home, "AppData", "Roaming")
	}
	return filepath.Join(appData, legacyAppDirName), nil
}

// migrateLegacyData 首次启用便携版时，把旧位置的数据搬到便携目录。
//
// 触发条件：便携目录尚无数据（没有 settings.json 也没有 data.db），
// 且旧位置确实存在数据。两个条件缺一不可 —— 否则会把便携目录里
// 已经用起来的数据覆盖掉。
func migrateLegacyData(portableDir, legacyDir string) error {
	if portableDir == legacyDir {
		return nil
	}

	// 便携目录已有数据：不动
	if fileExists(filepath.Join(portableDir, "settings.json")) || fileExists(filepath.Join(portableDir, "data.db")) {
		return nil
	}
	// 旧位置没有数据：没什么可迁的
	if !dirExists(legacyDir) {
		return nil
	}

	// 先迁 settings.json，用户的自定义 dbPath 才能一起带过来
	if err := copyFileIfExists(
		filepath.Join(legacyDir, "settings.json"),
		filepath.Join(portableDir, "settings.json"),
	); err != nil {
		return err
	}

	// 数据库主文件 + WAL 配套文件。
	// 只搬主文件会丢未 checkpoint 的写入，所以 -wal/-shm 必须一起走。
	if fileExists(filepath.Join(legacyDir, "data.db")) {
		if err := copyDbFiles(filepath.Join(legacyDir, "data.db"), filepath.Join(portableDir, "data.db")); err != nil {
			return err
		}
	}
	return nil
}

func fileExists(path string) bool {
	info, err := os.Stat(path)
	return err == nil && !info.IsDir()
}

func dirExists(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.IsDir()
}

// copyFileIfExists 源文件存在时复制到目标；源不存在视为成功（无需复制）。
func copyFileIfExists(src, dst string) error {
	if !fileExists(src) {
		return nil
	}
	if err := copyFile0600(src, dst); err != nil && !errors.Is(err, os.ErrNotExist) {
		return err
	}
	return nil
}

// copyFile0600 复制单个文件并以 0600 权限落盘。
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

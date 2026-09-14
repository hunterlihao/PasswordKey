package main

import (
	"fmt"
	"os"
	"path/filepath"
)

// exeDirName 便携版数据目录名。放在 exe 同级，整个目录可整体拷到 U 盘运行。
const exeDirName = "data"

// dataDirFallback 回退标记文件名。便携目录不可写时会留下它，
// 后续启动直接走系统目录，不再反复尝试（避免每次弹 UAC 或告警）。
const dataDirFallback = "system_dir"

// systemAppDirName 回退时使用的 %APPDATA% 子目录名。
const systemAppDirName = "PasswordManager"

// resolveDataDir 解析应用数据目录，并保证可写。
//
// 便携版优先：exe 同级的 data/ 子目录。若该位置不可写（例如程序装在
// C:\Program Files、U 盘被写保护），则回退到 %APPDATA%\PasswordManager，
// 并在便携目录留下一个标记文件，让后续启动直接走回退路径。
//
// 不读取也不迁移任何历史数据：目录为空时由 store 直接建新库。
// 返回 (便携目录, 实际使用的数据目录)。
func resolveDataDir() (portableDir string, appDir string, err error) {
	exe, err := os.Executable()
	if err != nil {
		return "", "", fmt.Errorf("无法定位程序路径: %w", err)
	}
	// 解析符号链接，避免从快捷方式/链接启动时把 data/ 建到链接所在目录
	if resolved, evalErr := filepath.EvalSymlinks(exe); evalErr == nil {
		exe = resolved
	}

	portableDir = filepath.Join(filepath.Dir(exe), exeDirName)

	// 已有降级标记：不再打扰便携目录
	if fileExists(filepath.Join(portableDir, dataDirFallback)) {
		return portableDir, systemDataDir(), nil
	}

	if err := ensureWritableDir(portableDir); err != nil {
		// 便携目录不可用：尽量留下标记再回退
		_ = os.MkdirAll(portableDir, 0700)
		_ = os.WriteFile(filepath.Join(portableDir, dataDirFallback), []byte(err.Error()+"\n"), 0600)
		return portableDir, systemDataDir(), nil
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

// systemDataDir 返回回退用的 %APPDATA%\PasswordManager 路径。
func systemDataDir() string {
	appData := os.Getenv("APPDATA")
	if appData == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			return filepath.Join(".", systemAppDirName)
		}
		appData = filepath.Join(home, "AppData", "Roaming")
	}
	return filepath.Join(appData, systemAppDirName)
}

func fileExists(path string) bool {
	info, err := os.Stat(path)
	return err == nil && !info.IsDir()
}

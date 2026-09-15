package main

import (
	"fmt"
	"os"
	"path/filepath"
	"time"
)

// exeDirName 便携版数据目录名。放在 exe 同级，整个目录可整体拷到 U 盘运行。
const exeDirName = "data"

// dataDirFallback 回退标记文件名。便携目录不可写时会留下它，
// 后续启动直接走系统目录，不再反复尝试（避免每次弹 UAC 或告警）。
const dataDirFallback = "system_dir"

// systemAppDirName 回退时使用的 %APPDATA% 子目录名。
const systemAppDirName = "PasswordManager"

// startupLog 把启动阶段的关键节点追加到 exe 同级 data/startup.log。
//
// 保留理由：托盘点击无反应、单实例抢锁这类问题的成因在运行期无法从外部观测
// （沙箱里 WebView2 起不来，startup 根本跑不到；真机上又不好挂调试器）。
// 靠这份日志能一眼看出 startup 走到哪一步、用的是哪个数据目录。
//
// 纯诊断辅助：任何写入失败都直接忽略，绝不影响启动。
func startupLog(format string, args ...any) {
	dir := portableDataDir()
	if dir == "" {
		return
	}
	if err := os.MkdirAll(dir, 0700); err != nil {
		return
	}
	f, err := os.OpenFile(filepath.Join(dir, "startup.log"), os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0600)
	if err != nil {
		return
	}
	defer f.Close()

	fmt.Fprintf(f, "%s  ", time.Now().Format("2006-01-02 15:04:05.000"))
	fmt.Fprintf(f, format+"\n", args...)
}

// portableDataDir 解析便携目录路径（exe 同级 data/），不创建目录、不判可写。
func portableDataDir() string {
	exe, err := os.Executable()
	if err != nil {
		return ""
	}
	// 解析符号链接，避免从快捷方式/链接启动时把 data/ 建到链接所在目录
	if resolved, evalErr := filepath.EvalSymlinks(exe); evalErr == nil {
		exe = resolved
	}
	return filepath.Join(filepath.Dir(exe), exeDirName)
}

// resolveDataDir 解析应用数据目录，并保证可写。
//
// 便携版优先：exe 同级的 data/ 子目录。若该位置不可写（例如程序装在
// C:\Program Files、U 盘被写保护），则回退到 %APPDATA%\PasswordManager，
// 并在便携目录留下一个标记文件，让后续启动直接走回退路径。
//
// 不读取也不迁移任何历史数据：目录为空时由 store 直接建新库。
// 返回 (便携目录, 实际使用的数据目录)。
func resolveDataDir() (portableDir string, appDir string, err error) {
	portableDir = portableDataDir()
	if portableDir == "" {
		return "", "", fmt.Errorf("无法定位程序路径")
	}

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

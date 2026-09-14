//go:build windows

package main

import (
	"syscall"
	"unsafe"
)

// lastInputInfo 对应 Win32 的 LASTINPUTINFO 结构体，用于 GetLastInputInfo。
type lastInputInfo struct {
	cbSize uint32
	dwTime uint32
}

var (
	kernel32             = syscall.NewLazyDLL("kernel32.dll")
	user32               = syscall.NewLazyDLL("user32.dll")
	procGetLastInputInfo = user32.NewProc("GetLastInputInfo")
	procGetTickCount64   = kernel32.NewProc("GetTickCount64")
)

// systemIdleMillis 返回系统级空闲毫秒数（距最后一次键鼠输入的时间）。
//
// 用 GetLastInputInfo 而非前端事件监听的原因：它是**系统全局**的输入时间戳，
// 鼠标移出应用窗口、用户全屏看视频、在别的程序里操作，都能被正确计入；
// 前端监听只能在窗口内生效，且页面卡死时计时失效。
//
// GetLastInputInfo 返回的 dwTime 是自系统启动以来的毫秒数（32 位，约 49.7 天回绕），
// 因此与 64 位的 GetTickCount64 相减时需先做回绕处理，避免算出巨大负数。
// 任何一步失败都返回 0（视为"刚刚有输入"），宁可漏锁也不误锁。
func systemIdleMillis() uint64 {
	var lii lastInputInfo
	lii.cbSize = uint32(unsafe.Sizeof(lii))

	ret, _, _ := procGetLastInputInfo.Call(uintptr(unsafe.Pointer(&lii)))
	if ret == 0 {
		return 0
	}

	tick, _, _ := procGetTickCount64.Call()
	if tick == 0 {
		return 0
	}

	now := uint64(tick)
	last := uint64(lii.dwTime)

	// dwTime 是 32 位值，扩展到与 GetTickCount64 同量纲：
	// 用 now 的高 32 位拼上 last 的低 32 位，得到最近的等价时间戳。
	last64 := (now & 0xFFFFFFFF00000000) | last
	if last64 > now {
		// 拼出的值比 now 大，说明发生了 32 位回绕，退到上一个周期
		last64 -= 1 << 32
	}
	if now <= last64 {
		return 0
	}
	return now - last64
}

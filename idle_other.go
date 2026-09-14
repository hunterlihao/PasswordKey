//go:build !windows

package main

// systemIdleMillis 非 Windows 平台恒返回 0（视为"持续有输入"），
// 因此自动锁定在这些平台上不生效。本项目主要面向 Windows 打包。
func systemIdleMillis() uint64 {
	return 0
}

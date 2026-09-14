//go:build !windows

package main

// 非 Windows 平台的空实现，保证项目在其它平台上仍然可以编译通过。
//
// 本项目当前只面向 Windows 发行（winres/ 资源、NSIS 安装包、
// %APPDATA% 数据目录），托盘能力由 tray.go 提供。
// 这里保留同名的空函数，只是为了不让 build tag 让整个工程编译失败，
// 同时也是对「托盘是平台相关能力」这一事实的显式说明。

// initTray 非 Windows 平台无托盘，空实现。
func (a *App) initTray() {}

// showWindow 非 Windows 平台没有托盘唤起的需求，空实现。
func (a *App) showWindow() {}

// stopTray 非 Windows 平台无托盘可收起，空实现。
func stopTray() {}

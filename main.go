package main

import (
	"context"
	"embed"
	"log"

	"github.com/wailsapp/wails/v2"
	"github.com/wailsapp/wails/v2/pkg/options"
	"github.com/wailsapp/wails/v2/pkg/options/assetserver"
)

//go:embed all:frontend/dist
var assets embed.FS

// singleInstanceID 单实例锁的唯一标识。
// 同一台机器上重复启动时，只有第一个进程会真正运行，
// 后续进程会通过 wails 的实例间通信把参数送给首实例然后自行退出。
const singleInstanceID = "PasswordManager-SingleInstance-8f2c4a17-3d5e-4b96-a1c7-2e9f6b0d8a35"

func main() {
	// Create an instance of the app structure
	app := NewApp()

	// Create application with options
	err := wails.Run(&options.App{
		Title:  "密码管理器",
		Width:  900,
		Height: 700,
		// 关闭窗口时只隐藏而不退出进程，配合系统托盘实现后台运行。
		// 注意：这个开关会让窗口的 OnClose 直接走 WindowHide()，
		// 不会经过 OnBeforeClose —— 所以真正的退出必须由托盘菜单显式触发。
		HideWindowOnClose: true,
		AssetServer: &assetserver.Options{
			Assets: assets,
		},
		BackgroundColour: &options.RGBA{R: 245, G: 240, B: 232, A: 1},
		OnStartup:        app.startup,
		OnShutdown:       app.shutdown,
		// OnBeforeClose 只在 Quit() 路径被调用（HideWindowOnClose 的隐藏路径不经过它）。
		// 返回 true 表示阻止退出：只有托盘菜单点「退出」置位了 quitting 才放行，
		// 避免任何意外的退出调用把用户数据"关掉"。
		OnBeforeClose: func(ctx context.Context) bool {
			return !app.quitting.Load()
		},
		// 单实例锁：避免重复启动产生多个进程同时操作同一个数据库。
		SingleInstanceLock: &options.SingleInstanceLock{
			UniqueId: singleInstanceID,
			OnSecondInstanceLaunch: func(data options.SecondInstanceData) {
				// 用户又双击了一次图标：把已有窗口拉到前台，
				// 而不是开第二个窗口/第二个进程。
				app.showWindow()
			},
		},
		Bind: []interface{}{
			app,
		},
	})

	if err != nil {
		log.Fatal(err)
	}
}

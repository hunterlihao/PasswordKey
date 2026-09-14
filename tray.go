//go:build windows

package main

import (
	_ "embed"
	"sync"

	"github.com/energye/systray"
	"github.com/wailsapp/wails/v2/pkg/runtime"
)

//go:embed build/windows/icon.ico
var trayIcon []byte

// trayStartOnce 保证托盘消息泵只启动一次（initTray 理论上只会调用一次，
// 这里防御性处理，避免重复启动多个 GetMessage 泵）。
var trayStartOnce sync.Once

// initTray 初始化系统托盘。
//
// 必须在 Wails 的 OnStartup 之后调用：托盘菜单的「显示主窗口」「立即锁定」
// 都依赖 a.ctx 已经就绪。
//
// 关键：这里使用 RunWithExternalLoop 而不是 Run，也不是 Register。
//
//	systray.Run()                 → nativeLoop() 阻塞当前线程跑 GetMessage 消息泵，
//	                                会和 Wails 的主消息循环抢消息，导致托盘异常/应用卡死。
//	systray.Register()            → 只创建托盘窗口、注册图标，但【不启动消息泵】。
//	                                图标能看到，但右键 WM_RBUTTONUP 消息永远没人处理，
//	                                结果是右键弹不出菜单（症状：有图标、无菜单）。
//	RunWithExternalLoop()         → 返回 start/end 两个函数，start 会在【独立 goroutine】
//	                                里跑消息泵，既不占用 Wails 主线程，又保证托盘消息被处理。
//	                                这正是库文档里为 "webview" 场景推荐的用法。
func (a *App) initTray() {
	start, _ := systray.RunWithExternalLoop(func() {
		systray.SetIcon(trayIcon)
		systray.SetTooltip("密码管理器")

		showItem := systray.AddMenuItem("显示主窗口", "打开密码管理器")
		lockItem := systray.AddMenuItem("立即锁定", "清空内存中的主密钥")
		systray.AddSeparator()
		quitItem := systray.AddMenuItem("退出", "完全退出应用")

		// 左键单击托盘图标直接唤起窗口。
		// 注意：设置了 onRClick 才会改变右键行为（弹菜单 vs 回调），
		// 这里只设 onClick，右键仍走默认的 ShowMenu()，即正常弹出菜单。
		systray.SetOnClick(func(systray.IMenu) { a.showWindow() })

		showItem.Click(func() { a.showWindow() })
		lockItem.Click(func() { a.lockFromTray() })
		quitItem.Click(func() { a.quitApp() })
	}, nil)

	// 启动托盘消息泵（独立 goroutine，不阻塞 Wails）
	trayStartOnce.Do(start)
}

// lockFromTray 处理托盘菜单的「立即锁定」。
//
// 与闲置自动锁定共用 lockAndNotify（清密钥 + 广播事件，前端切锁定页）。
// 额外差异：这里先把窗口唤到前台 —— 用户在托盘主动点了"锁定"，
// 理应立刻看到锁定页；窗口若是隐藏的，点完毫无反馈会让人以为没生效。
func (a *App) lockFromTray() {
	if a.ctx != nil {
		runtime.WindowShow(a.ctx)
		runtime.WindowUnminimise(a.ctx)
	}
	a.lockAndNotify()
}

// showWindow 从托盘恢复窗口：显示并取消最小化。
//
// 第二次启动实例时也会走到这里（见 main.go 的 OnSecondInstanceLaunch），
// 此时首实例可能正处于隐藏或最小化状态，两步都做才能保证窗口真的可见。
//
// 线程安全：runtime.WindowShow 内部会 marshal 回 UI 线程
// （wails 的 Frontend.ShowWindow 里包了一层 mainWindow.Invoke），
// 所以从托盘消息泵线程调用是安全的。
func (a *App) showWindow() {
	if a.ctx == nil {
		return
	}
	runtime.WindowShow(a.ctx)
	runtime.WindowUnminimise(a.ctx)
}

// quitApp 走真正的退出路径。
//
// 因为配置了 HideWindowOnClose，关闭窗口只会隐藏窗口而不会退出进程，
// 所以退出必须显式声明意图：先置 quitting 标志让 OnBeforeClose 放行，
// 再收起托盘图标（否则通知区会残留一个"幽灵图标"，
// 要等鼠标划过才消失），最后交给 Wails 退出。
func (a *App) quitApp() {
	a.quitting.Store(true)
	systray.Quit()
	if a.ctx != nil {
		runtime.Quit(a.ctx)
	}
}

// stopTray 收起托盘图标，供 shutdown 兜底调用。
func stopTray() {
	systray.Quit()
}

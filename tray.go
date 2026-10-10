//go:build windows

package main

import (
	_ "embed"
	"runtime"

	"github.com/energye/systray"
	wailsruntime "github.com/wailsapp/wails/v2/pkg/runtime"
)

//go:embed build/windows/icon.ico
var trayIcon []byte

// initTray 初始化系统托盘。
//
// 必须在 Wails 的 OnStartup 之后调用：托盘菜单的「显示主窗口」「立即锁定」
// 都依赖 a.ctx 已经就绪。
//
// ⚠️ 这里【不能】用 systray.RunWithExternalLoop —— 这是踩了很久的坑，务必看完。
//
// 先说各个 API 的行为：
//
//	systray.Run(onReady, onExit)
//	    → setInternalLoop(true) + Register + nativeLoop()
//	      nativeLoop 是【阻塞式】for { GetMessage } 循环，跑在【调用者所在 goroutine】。
//	systray.Register(onReady, onExit)
//	    → 只创建托盘窗口和图标，【不启动消息泵】。
//	      症状：图标能看到，但右键弹不出菜单（WM_RBUTTONUP 永远没人处理）。
//	RunWithExternalLoop(onReady, onExit)
//	    → 返回 start/end。但 start 内部实现是：
//	          func nativeStart() { go func() { for doNativeTick() {} }() }
//	      **它自己新起一个 goroutine**，且【没有 LockOSThread】。
//
// 问题出在 GetMessage 是【线程绑定】的 Win32 API：它只取当前线程的消息队列。
// nativeStart 里那个消息泵 goroutine 会被 Go 调度器在任意 OS 线程间迁移，
// 一旦飘到别的线程，GetMessage 立刻返回 -1，
//
//	doNativeTick() 里 case -1: return false
//
// 消息泵就【静默退出】了。此时托盘窗口早就建好、图标也画上了，所以表现为：
// **图标在，但左键、右键点击全部没反应。**
// 触发与否取决于调度时机 —— 时好时坏，极难复现，只能从库源码推出来。
//
// 正确做法：自己起一个 goroutine，先 LockOSThread 把它钉死在当前 OS 线程，
// 再调【阻塞式】的 systray.Run()。这样消息泵始终在同一个线程上跑，
// GetMessage 能稳定收到该线程的消息。
// （systray 包自己的 init() 里有 runtime.LockOSThread()，但那锁的是 main goroutine
// 所在的线程，对 nativeStart 新起的 goroutine 没有任何约束力。）
func (a *App) initTray() {
	// Wails 的 OnStartup 线程不能让给阻塞的 GetMessage 泵，否则应用卡死；
	// 所以托盘泵必须另起 goroutine。
	go func() {
		// 把本 goroutine 钉死在当前 OS 线程上，保证 GetMessage 始终面对
		// 同一个线程消息队列。这是整个托盘能否响应点击的关键。
		runtime.LockOSThread()
		// 故意不 Unlock：消息泵是常驻循环，线程要一直归它专用。
		// 若中途解锁，调度器再次迁移线程就会重现「图标在但点不动」。

		defer func() {
			if r := recover(); r != nil {
				startupLog("tray PANIC: %v", r)
			}
		}()

		startupLog("tray: pump goroutine locked, calling Run")

		// Run 会阻塞在这里直到 systray.Quit()。
		systray.Run(func() {
			startupLog("tray onReady: enter")
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

			startupLog("tray onReady: menu items + click handlers registered")
		}, func() {
			startupLog("tray onExit")
		})

		startupLog("tray: Run returned (pump exited)")
	}()
}

// lockFromTray 处理托盘菜单的「立即锁定」。
//
// 与闲置自动锁定共用 lockAndNotify（清密钥 + 广播事件，前端切锁定页）。
// 额外差异：这里先把窗口唤到前台 —— 用户在托盘主动点了"锁定"，
// 理应立刻看到锁定页；窗口若是隐藏的，点完毫无反馈会让人以为没生效。
func (a *App) lockFromTray() {
	if a.ctx != nil {
		wailsruntime.WindowShow(a.ctx)
		wailsruntime.WindowUnminimise(a.ctx)
	}
	a.lockAndNotify()
}

// showWindow 从托盘恢复窗口：显示并取消最小化。
//
// 第二次启动实例时也会走到这里（见 main.go 的 OnSecondInstanceLaunch），
// 此时首实例可能正处于隐藏或最小化状态，两步都做才能保证窗口真的可见。
//
// 线程安全：wailsruntime.WindowShow 内部会 marshal 回 UI 线程
// （wails 的 Frontend.ShowWindow 里包了一层 mainWindow.Invoke），
// 所以从托盘消息泵线程调用是安全的。
func (a *App) showWindow() {
	if a.ctx == nil {
		return
	}
	wailsruntime.WindowShow(a.ctx)
	wailsruntime.WindowUnminimise(a.ctx)
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
		wailsruntime.Quit(a.ctx)
	}
}

// stopTray 收起托盘图标，供 shutdown 兜底调用。
func stopTray() {
	systray.Quit()
}

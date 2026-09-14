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
// 与窗口内那个锁定按钮不同：窗口内按钮点完会由前端自己调 showLoginView()
// 切回登录页，而托盘锁定发生时前端完全不知情 —— 内存里的主密钥已经清掉了，
// 但页面还停在凭证列表上，窗口一恢复就能看到明文列表（越权展示）。
//
// 这里的解决办法是让页面整体重载：
//   - WindowReload() 执行的是 window.location.reload()（见 wails 的
//     runtime_prod_desktop.js：`function oe(){window.location.reload()}`），
//   - 重载后前端会重新跑 init()，此时 IsMasterSet() 为真而密钥已清空，
//     于是走 showLoginView()，正好落到登录页。
//
// 为什么不用 window.runtime.EventsOn 发事件？
// 前端 index.html 目前完全没有引入 wails 的 runtime（它要与 Rust/wry 宿主共用），
// 一旦引入就会破坏"同一份前端兼容两个宿主"的约定。所以选择用重载，
// 做到既修好问题、又零前端改动。
//
// 顺带的好处：重载会重新显示 #splash（它在 index.html 里是静态元素），
// 锁定后看到一次"重新初始化"动画，语义上也比直接切页面更自然。
//
// 最后把窗口显示出来，让用户直接看到"已锁定"的结果 ——
// 否则窗口若是隐藏的，用户点完锁定毫无反馈，会怀疑没生效。
func (a *App) lockFromTray() {
	a.Lock()
	if a.ctx == nil {
		return
	}
	// 先唤起窗口，再重载：保证用户看到的是重载后的登录页，
	// 而不是一闪而过的旧列表。
	runtime.WindowShow(a.ctx)
	runtime.WindowUnminimise(a.ctx)
	runtime.WindowReload(a.ctx)
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

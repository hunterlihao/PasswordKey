package main

import "github.com/wailsapp/wails/v2/pkg/runtime"

// eventVaultLocked 是「已锁定」事件名。
//
// Go 侧（托盘「立即锁定」、闲置自动锁定）清空主密钥后广播该事件，
// 前端监听后切换到专用锁定页。用事件而非 window.location.reload()
// 的原因：重载会重新播放 #splash 启动动画，观感上像"重启"而不是"锁定"，
// 且会把整个 SPA 状态推倒重建。发事件只切视图，干净利落。
//
// 事件名带 vault 前缀，避免与将来其它事件重名。
const eventVaultLocked = "vault:locked"

// lockAndNotify 清空主密钥并通知前端切换到锁定页。
//
// 这个收尾动作是托盘「立即锁定」与闲置自动锁定共用的：
// 两者都发生在 Go 侧，前端完全不知情 —— 如果只清密钥不通知，
// 页面会停在凭证列表上，窗口一恢复就能看到明文列表（越权展示）。
//
// 线程安全：EventsEmit 内部会 marshal 回 UI 线程，因此从托盘消息泵线程
// 或自动锁定协程调用都是安全的。
func (a *App) lockAndNotify() {
	a.Lock()

	if a.ctx == nil {
		return
	}
	runtime.EventsEmit(a.ctx, eventVaultLocked)
}

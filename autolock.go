package main

import (
	"time"

	"github.com/wailsapp/wails/v2/pkg/runtime"
)

// autoLockCheckInterval 空闲检测的轮询间隔。
// 1 秒足够精确（最短可配 1 分钟），开销可忽略。
const autoLockCheckInterval = time.Second

// startAutoLockMonitor 启动自动锁定监控协程。
//
// 用 sync.Once 保护，重复调用是安全的（幂等）。
func (a *App) startAutoLockMonitor() {
	a.autoLockOnce.Do(func() {
		stop := make(chan struct{})
		a.autoLockStop = stop
		go a.autoLockLoop(stop)
	})
}

// stopAutoLockMonitor 通知监控协程退出。
func (a *App) stopAutoLockMonitor() {
	if a.autoLockStop != nil {
		close(a.autoLockStop)
		a.autoLockStop = nil
	}
}

func (a *App) autoLockLoop(stop chan struct{}) {
	ticker := time.NewTicker(autoLockCheckInterval)
	defer ticker.Stop()

	for {
		select {
		case <-stop:
			return
		case <-ticker.C:
			a.checkAutoLock()
		}
	}
}

// checkAutoLock 读一次空闲时长，必要时执行锁定。
func (a *App) checkAutoLock() {
	// 未解锁时无需处理：内存里本来就没有密钥，没有可泄露的内容。
	// 这也避免了「锁屏界面挂机 → 反复重载」的无谓开销。
	if !a.IsUnlocked() {
		return
	}

	minutes := a.autoLockMinutes()
	if minutes <= 0 {
		return // 配置为「永不自动锁定」
	}

	idle := systemIdleMillis()
	if idle < uint64(minutes)*60*1000 {
		return
	}

	a.lockByInactivity()
}

// autoLockMinutes 返回当前生效的自动锁定时长（分钟）。
func (a *App) autoLockMinutes() int {
	a.mu.RLock()
	s := a.settings
	a.mu.RUnlock()
	if s == nil {
		return 0
	}
	return s.EffectiveAutoLockMinutes()
}

// lockByInactivity 因闲置而锁定：清密钥，并把前端切回登录页。
//
// 与托盘「立即锁定」共用同一套收尾逻辑（见 lockFromTray）：
// 光清 Go 侧密钥是不够的，前端仍停在凭证列表上，必须重载页面
// 让 init() 重跑，才会落到 showLoginView()。
//
// 这里不区分窗口是否可见，一律重载 —— 原因是：
//   - Wails 不向 Go 侧暴露窗口可见性（options 只有 Startup/DomReady/
//     Shutdown/BeforeClose 四个回调，HideWindowOnClose 的隐藏路径连
//     OnBeforeClose 都不经过），自己维护标志位极易失真；
//   - 重载是幂等的：窗口若正隐藏着，重载不会打断任何人
//     （用户下次唤出窗口时正好看到登录页，符合预期）。
//
// 注意不清空 fail_count：自动锁定是正常操作，不应影响防爆破计数。
func (a *App) lockByInactivity() {
	a.Lock()
	if a.ctx != nil {
		runtime.WindowReload(a.ctx)
	}
}

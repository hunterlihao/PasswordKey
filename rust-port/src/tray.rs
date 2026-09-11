//! 系统托盘：点击关闭窗口后隐藏到托盘后台运行。
//!
//! 设计：
//! - 托盘图标始终存在（应用启动即创建），让用户始终知道程序在跑
//! - 右键托盘弹出菜单：「显示主窗口」「退出」
//! - 左键单击托盘 = 直接显示主窗口
//! - 关闭按钮不退出，只把窗口隐藏；真正退出走托盘菜单的「退出」
//!
//! 与事件循环的集成：tray-icon / muda 各自有 `set_event_handler`，
//! 这里只负责把 TrayIcon / 菜单构造出来；事件转发由 [`crate::main`] 通过
//! `set_event_handler` 接到 tao 的 `EventLoopProxy` 上。

use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// 菜单项 ID：「显示主窗口」
pub const MENU_ID_SHOW: &str = "show";
/// 菜单项 ID：「退出」
pub const MENU_ID_QUIT: &str = "quit";

/// 主窗口对托盘的请求。事件循环把它们通过 `UserEvent::Tray` 转发回来，
/// 让 main loop 决定怎么处理窗口可见性 / 退出。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    /// 显示并聚焦主窗口（来自左键单击托盘 / 「显示主窗口」菜单项）
    ShowWindow,
    /// 真正退出应用（来自「退出」菜单项）
    Quit,
}

/// 构造系统托盘：图标 + 右键菜单。
///
/// 失败时返回 `Err`，调用方自行决定是否降级（不显示托盘也能用，
/// 只是关闭按钮就会真的退出）。
pub fn build(icon_bytes: &[u8], tooltip: &str) -> Result<TrayIcon, String> {
    let icon = tray_icon_from_png(icon_bytes, 64).map_err(String::from)?;

    // 右键菜单：显示主窗口 / 分隔线 / 退出
    let menu = Menu::new();
    let show_item = MenuItem::with_id(MENU_ID_SHOW, "显示主窗口(&S)", true, None);
    menu.append(&show_item)
        .map_err(|e| format!("追加菜单项失败: {e}"))?;
    menu.append(&PredefinedMenuItem::separator())
        .map_err(|e| format!("追加分隔线失败: {e}"))?;
    let quit_item = MenuItem::with_id(MENU_ID_QUIT, "退出(&X)", true, None);
    menu.append(&quit_item)
        .map_err(|e| format!("追加菜单项失败: {e}"))?;

    // 左键单击托盘 = 显示窗口；右键才出菜单
    let tray = TrayIconBuilder::new()
        .with_tooltip(tooltip)
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false) // 左键不出菜单，留给 ShowWindow
        .build()
        .map_err(|e| format!("创建托盘图标失败: {e}"))?;

    Ok(tray)
}

/// 把内置 PNG 解码 + 降采样到指定边长（系统托盘图标 16/32/64 都常见，
/// 64 在高 DPI 下更清晰，系统会再缩到实际需要的尺寸）
fn tray_icon_from_png(bytes: &[u8], target: u32) -> Result<Icon, String> {
    let (width, height, rgba) =
        crate::decode_png(bytes).ok_or_else(|| "内置 PNG 图标解码失败".to_string())?;

    // 整数倍降采样到 target 以内；非整数倍时按 box filter 取最大的整除倍率
    let scale = if width > target && height > target {
        let s_x = width.div_ceil(target).max(1);
        let s_y = height.div_ceil(target).max(1);
        s_x.max(s_y)
    } else {
        1
    };

    let (w, h, data) = if scale > 1 {
        crate::downscale_rgba(&rgba, width, height, scale)
    } else {
        (width, height, rgba)
    };

    Icon::from_rgba(data, w, h).map_err(|e| format!("构造托盘图标失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 内置 PNG 应能解码并降采样到 64x64 以内，且能构造托盘图标
    #[test]
    fn decodes_embedded_icon_to_tray_size() {
        let (w, h, data) = crate::decode_png(crate::APP_ICON_PNG)
            .expect("内置 PNG 应可解码");
        assert!(w >= 64 && h >= 64);
        assert_eq!(data.len(), (w * h * 4) as usize);

        // 大图必须触发降采样到 64
        let icon = tray_icon_from_png(crate::APP_ICON_PNG, 64);
        let icon = icon.expect("构造托盘图标应成功");
        let _ = icon; // 仅验证可构造；Icon 本身没有公开 size getter
    }

    #[test]
    fn menu_ids_are_stable_strings() {
        // 这些 ID 是 main loop 用来匹配菜单事件的关键字，必须保持稳定
        assert_eq!(MENU_ID_SHOW, "show");
        assert_eq!(MENU_ID_QUIT, "quit");
    }
}

//! 密码管理器 —— Rust 便携版
//!
//! 架构：
//! - 窗口与 WebView：`tao` + `wry`（WebView2，Win10/11 自带运行时）
//! - 前端：Vite 构建后内联成单文件 HTML，编译期嵌入 exe（`assets/app.html`）
//! - 通信：WebView2 原生 IPC（`window.ipc.postMessage`），
//!   注入 `window.go.main.App.*` 后前端代码无需任何改动
//! - 数据：便携目录下的 SQLite（`data/data.db`）+ `settings.json`

// release 构建用 windows 子系统，避免双击 exe 时弹出控制台窗口；
// debug 构建保留控制台，方便看 println! / eprintln! 输出。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod crypto;
mod dialogs;
mod export;
mod model;
mod paths;
mod rpc;
mod settings;
mod store;
mod timefmt;
mod tray;

use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::Value;
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{Icon, WindowBuilder};
use wry::WebViewBuilder;

use crate::app::App;
use crate::dialogs::WindowHost;

/// 内联后的前端单文件页面（由 scripts/build-frontend.mjs 生成）
const APP_HTML: &str = include_str!("../assets/app.html");
/// 应用图标（PNG，运行时转成 RGBA 交给 tao）
const APP_ICON_PNG: &[u8] = include_bytes!("../assets/appicon.png");
/// JS 桥模板
const BRIDGE_TEMPLATE: &str = include_str!("bridge.js");

/// 主线程与工作线程之间的事件
enum UserEvent {
    /// 前端发起一次 RPC
    Rpc {
        id: u64,
        method: String,
        args: Vec<Value>,
    },
    /// 工作线程返回结果
    Reply { id: u64, ok: bool, payload: String },
    /// 托盘 / 托盘菜单请求
    Tray(tray::TrayAction),
}

#[derive(Deserialize)]
struct IpcRequest {
    id: u64,
    method: String,
    #[serde(default)]
    args: Vec<Value>,
}

fn main() {
    let app = match App::bootstrap() {
        Ok(app) => app,
        Err(err) => {
            eprintln!("启动失败: {err}");
            dialogs::error_dialog("启动失败", &format!("无法初始化数据目录或数据库：\n\n{err}"));
            std::process::exit(1);
        }
    };
    let app = Arc::new(Mutex::new(app));

    // WebView2 的缓存目录放到 %LOCALAPPDATA%，不污染便携 data/ 目录
    // （前端是内联 HTML，WebView2 缓存对应用无价值；data/ 只存 vault.json + settings.json）
    {
        let _guard = app.lock().expect("初始化后不应被污染");
        if let Err(err) = paths::apply_portable_webview_profile() {
            // 只是影响缓存位置，失败不阻塞启动
            eprintln!("警告: {err}，将使用系统默认的 WebView2 数据目录");
        }
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let mut window_builder = WindowBuilder::new()
        .with_title("密码管理器")
        .with_inner_size(LogicalSize::new(900.0, 700.0))
        .with_min_inner_size(LogicalSize::new(720.0, 520.0));
    if let Some(icon) = load_app_icon() {
        window_builder = window_builder.with_window_icon(Some(icon));
    }

    let window = window_builder.build(&event_loop).expect("创建窗口失败");
    let host = WindowHost(window_hwnd(&window));

    let ipc_proxy = proxy.clone();
    let builder = WebViewBuilder::new()
        .with_html(APP_HTML)
        .with_background_color((245, 240, 232, 255))
        .with_initialization_script(bridge_script())
        .with_ipc_handler(move |request| {
            if let Ok(message) = serde_json::from_str::<IpcRequest>(request.body()) {
                let _ = ipc_proxy.send_event(UserEvent::Rpc {
                    id: message.id,
                    method: message.method,
                    args: message.args,
                });
            }
        });

    let webview = with_browser_args(builder)
        .build(&window)
        .expect("创建 WebView 失败");

    // 系统托盘：关闭按钮不再退出，而是隐藏到托盘后台运行。
    // 托盘创建失败（极罕见，仅 Shell 通知区不可用时）会降级为「关闭即退出」。
    let tray = match tray::build(APP_ICON_PNG, "密码管理器") {
        Ok(tray) => Some(tray),
        Err(err) => {
            eprintln!("警告: 创建托盘失败，将退化为关闭即退出: {err}");
            None
        }
    };
    let has_tray = tray.is_some();

    // 托盘事件转发：左键单击 → 显示主窗口
    if has_tray {
        let tray_proxy = proxy.clone();
        tray_icon::TrayIconEvent::set_event_handler(Some(move |event| {
            if let tray_icon::TrayIconEvent::Click {
                button: tray_icon::MouseButton::Left,
                button_state: tray_icon::MouseButtonState::Up,
                ..
            } = event
            {
                let _ = tray_proxy.send_event(UserEvent::Tray(tray::TrayAction::ShowWindow));
            }
        }));
        let menu_proxy = proxy.clone();
        tray_icon::menu::MenuEvent::set_event_handler(Some(move |event: tray_icon::menu::MenuEvent| {
            let action = match event.id().as_ref() {
                tray::MENU_ID_SHOW => Some(tray::TrayAction::ShowWindow),
                tray::MENU_ID_QUIT => Some(tray::TrayAction::Quit),
                _ => None,
            };
            if let Some(action) = action {
                let _ = menu_proxy.send_event(UserEvent::Tray(action));
            }
        }));
    }

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if has_tray {
                    // 关闭按钮 → 隐藏到托盘后台运行，不退出
                    window.set_visible(false);
                } else {
                    // 无托盘退化：真正退出前清零密钥并关闭数据库
                    if let Ok(mut app) = app.lock() {
                        app.shutdown();
                    }
                    *control_flow = ControlFlow::Exit;
                }
            }

            Event::UserEvent(UserEvent::Tray(action)) => match action {
                tray::TrayAction::ShowWindow => {
                    window.set_visible(true);
                    window.set_focus();
                }
                tray::TrayAction::Quit => {
                    if let Ok(mut app) = app.lock() {
                        app.shutdown();
                    }
                    // 退出后 event_loop 闭包被 drop，tray 随之释放，
                    // Shell_NotifyIcon(NIM_DELETE) 由 tray-icon 在 Drop 里发出。
                    *control_flow = ControlFlow::Exit;
                }
            },

            Event::UserEvent(UserEvent::Rpc { id, method, args }) => {
                // 业务逻辑放到工作线程，避免 Argon2id / 数据库 / 文件对话框卡住界面
                let app = Arc::clone(&app);
                let proxy = proxy.clone();
                std::thread::spawn(move || {
                    let (ok, payload) = {
                        let mut guard = match app.lock() {
                            Ok(guard) => guard,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                        match rpc::dispatch(&mut guard, host, &method, &args) {
                            Ok(value) => (true, to_json(&value)),
                            Err(err) => (false, to_json(&Value::String(err))),
                        }
                    };
                    let _ = proxy.send_event(UserEvent::Reply { id, ok, payload });
                });
            }

            Event::UserEvent(UserEvent::Reply { id, ok, payload }) => {
                let script = format!("window.__vaultBridgeSettle({id}, {ok}, {payload});");
                if let Err(err) = webview.evaluate_script(&script) {
                    eprintln!("回填 RPC 结果失败: {err}");
                }
            }

            _ => {}
        }
    });
}

fn to_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

/// 组装 WebView2 的启动参数。
///
/// wry 默认只传 `--disable-features=...`，并且一旦通过 API 传参，
/// WebView2 就会**忽略** `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` 环境变量。
/// 这里把该环境变量显式拼接进去，让 WebView2 的标准调试开关（例如
/// `--remote-debugging-port=9222`、`--disable-gpu`）依然可用。
fn with_browser_args(builder: WebViewBuilder<'_>) -> WebViewBuilder<'_> {
    const WRYS_DEFAULT: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";
    let mut args = String::from(WRYS_DEFAULT);

    if let Ok(extra) = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS") {
        let extra = extra.trim();
        if !extra.is_empty() {
            args.push(' ');
            args.push_str(extra);
        }
    }

    #[cfg(windows)]
    {
        use wry::WebViewBuilderExtWindows;
        builder.with_additional_browser_args(args)
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        builder
    }
}

/// 生成注入脚本：方法名清单以 `rpc::METHODS` 为唯一来源
fn bridge_script() -> String {
    let methods = rpc::METHODS
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(",");
    BRIDGE_TEMPLATE.replace("/*__METHODS__*/ []", &format!("[{methods}]"))
}

/// 取窗口句柄，供文件对话框设置父窗口
fn window_hwnd(window: &tao::window::Window) -> isize {
    #[cfg(windows)]
    {
        use tao::platform::windows::WindowExtWindows;
        window.hwnd()
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        0
    }
}

/// 解码内置 PNG 图标并缩放到 256 像素以内（Windows 标题栏图标不适合直接用 1024px 原图）
fn load_app_icon() -> Option<Icon> {
    let (width, height, rgba) = decode_png(APP_ICON_PNG)?;

    const MAX: u32 = 256;
    let scale = if width > MAX || height > MAX {
        (width.max(height)).div_ceil(MAX)
    } else {
        1
    };

    let (width, height, rgba) = if scale > 1 {
        downscale_rgba(&rgba, width, height, scale)
    } else {
        (width, height, rgba)
    };

    Icon::from_rgba(rgba, width, height).ok()
}

/// 解码 8 位 RGB/RGBA PNG 为 RGBA8
pub(crate) fn decode_png(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let decoder = png::Decoder::new(bytes);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    buffer.truncate(info.buffer_size());

    let rgba = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        png::ColorType::Grayscale => buffer
            .iter()
            .flat_map(|&v| [v, v, v, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buffer
            .chunks_exact(2)
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        png::ColorType::Indexed => return None,
    };

    Some((info.width, info.height, rgba))
}

/// 整数倍盒式降采样
pub(crate) fn downscale_rgba(rgba: &[u8], width: u32, height: u32, scale: u32) -> (u32, u32, Vec<u8>) {
    let new_width = width / scale;
    let new_height = height / scale;
    let mut out = Vec::with_capacity((new_width * new_height * 4) as usize);
    let samples = (scale * scale) as u32;

    for y in 0..new_height {
        for x in 0..new_width {
            let mut sum = [0u32; 4];
            for dy in 0..scale {
                for dx in 0..scale {
                    let src_x = x * scale + dx;
                    let src_y = y * scale + dy;
                    let index = ((src_y * width + src_x) * 4) as usize;
                    if index + 3 < rgba.len() {
                        for channel in 0..4 {
                            sum[channel] += rgba[index + channel] as u32;
                        }
                    }
                }
            }
            for channel in sum {
                out.push((channel / samples) as u8);
            }
        }
    }
    (new_width, new_height, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_script_contains_all_methods() {
        let script = bridge_script();
        assert!(!script.contains("/*__METHODS__*/"), "占位符未被替换");
        for name in rpc::METHODS {
            assert!(script.contains(&format!("\"{name}\"")), "缺少方法 {name}");
        }
    }

    #[test]
    fn icon_is_decoded_and_downscaled() {
        let (width, height, rgba) = decode_png(APP_ICON_PNG).expect("内置图标应可解码");
        assert_eq!(rgba.len(), (width * height * 4) as usize);
        assert!(width >= 256);

        let (w, h, data) = downscale_rgba(&rgba, width, height, 4);
        assert_eq!(w, width / 4);
        assert_eq!(h, height / 4);
        assert_eq!(data.len(), (w * h * 4) as usize);
        // 缩放后仍应是不透明的图标，而不是全 0
        assert!(data.iter().any(|&b| b > 0));
    }

    #[test]
    fn load_app_icon_works_on_embedded_png() {
        assert!(load_app_icon().is_some());
    }
}

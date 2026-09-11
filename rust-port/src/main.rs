//! 密码管理器 —— Rust 便携版
//!
//! 骨架阶段：先把 WebView 窗口跑起来，验证 tao/wry 工具链。

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

const APP_HTML: &str = include_str!("../assets/app.html");

fn main() {
    let event_loop = EventLoopBuilder::<()>::with_user_event().build();

    let window = WindowBuilder::new()
        .with_title("密码管理器")
        .with_inner_size(LogicalSize::new(900.0, 700.0))
        .with_min_inner_size(LogicalSize::new(720.0, 520.0))
        .build(&event_loop)
        .expect("创建窗口失败");

    let _webview = WebViewBuilder::new()
        .with_html(APP_HTML)
        .with_background_color((245, 240, 232, 255))
        .build(&window)
        .expect("创建 WebView 失败");

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    });
}

//! 原生文件对话框（替代 Wails 的 runtime.SaveFileDialog / OpenFileDialog）。
//!
//! 对话框在工作线程上弹出，因此需要把主线程窗口的 HWND 传进来，
//! 这样文件对话框才会以我们的窗口为父窗口（模态、不会被主窗口挡住）。

use std::num::NonZeroIsize;
use std::path::PathBuf;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
    Win32WindowHandle, WindowHandle, WindowsDisplayHandle,
};

/// 主窗口句柄（0 表示无父窗口）
#[derive(Clone, Copy, Debug, Default)]
pub struct WindowHost(pub isize);

impl WindowHost {
    pub fn is_valid(self) -> bool {
        self.0 != 0
    }
}

impl HasWindowHandle for WindowHost {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = NonZeroIsize::new(self.0).ok_or(HandleError::Unavailable)?;
        let mut handle = Win32WindowHandle::new(raw);
        // hinstance 对文件对话框非必需，留空由系统处理
        handle.hinstance = None;
        // SAFETY: HWND 由 tao 窗口持有，生命周期覆盖整个对话框调用过程
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(handle)) })
    }
}

impl HasDisplayHandle for WindowHost {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        // SAFETY: Windows 显示句柄是无状态占位类型
        Ok(unsafe { DisplayHandle::borrow_raw(RawDisplayHandle::Windows(WindowsDisplayHandle::new())) })
    }
}

/// 保存文件对话框
pub fn save_file(
    host: WindowHost,
    title: &str,
    default_name: &str,
    filter: (&str, &[&str]),
) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_title(title).add_filter(filter.0, filter.1);
    if !default_name.is_empty() {
        dialog = dialog.set_file_name(default_name);
    }
    if host.is_valid() {
        dialog = dialog.set_parent(&host);
    }
    dialog.save_file()
}

/// 打开文件对话框（多个过滤器对应 Wails 的 Filters 列表）
pub fn open_file(host: WindowHost, title: &str, filters: &[(&str, &[&str])]) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_title(title);
    for (name, extensions) in filters {
        dialog = dialog.add_filter(*name, *extensions);
    }
    if host.is_valid() {
        dialog = dialog.set_parent(&host);
    }
    dialog.pick_file()
}

/// 启动失败等致命错误提示
pub fn error_dialog(title: &str, message: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(title)
        .set_description(message)
        .show();
}

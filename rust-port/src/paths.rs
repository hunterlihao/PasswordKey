//! 便携版数据目录解析。
//!
//! 优先级：
//! 1. 环境变量 `PASSWORD_MANAGER_DATA_DIR`（显式指定，便于把数据放到 U 盘任意位置）
//! 2. 可执行文件同级的 `data/` 目录 —— 便携版的核心：拷走 exe + data 就能带走全部数据
//! 3. 兜底 `%APPDATA%/PasswordManager`（exe 所在目录不可写时，例如装在 Program Files）

use std::fs;
use std::path::{Path, PathBuf};

/// 便携数据目录名
pub const DATA_DIR_NAME: &str = "data";
/// 环境变量覆盖
pub const ENV_DATA_DIR: &str = "PASSWORD_MANAGER_DATA_DIR";

/// 解析数据目录，并确保其存在且可写
pub fn resolve_data_dir() -> Result<PathBuf, String> {
    for candidate in candidates()? {
        match ensure_writable_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(_) => continue,
        }
    }
    Err("无法找到可写的数据目录（已尝试：环境变量、exe 同级 data、%APPDATA%）".to_string())
}

/// 按优先级列出候选目录
fn candidates() -> Result<Vec<PathBuf>, String> {
    let mut list = Vec::new();

    if let Ok(dir) = std::env::var(ENV_DATA_DIR) {
        let dir = dir.trim();
        if !dir.is_empty() {
            list.push(PathBuf::from(dir));
        }
    }

    let exe = std::env::current_exe().map_err(|e| format!("无法获取可执行文件路径: {e}"))?;
    if let Some(parent) = exe.parent() {
        list.push(parent.join(DATA_DIR_NAME));
    }

    list.push(roaming_app_data()?.join("PasswordManager"));
    Ok(list)
}

/// %APPDATA%（Windows）或 ~/.config（其他平台）
fn roaming_app_data() -> Result<PathBuf, String> {
    if let Ok(app_data) = std::env::var("APPDATA") {
        if !app_data.trim().is_empty() {
            return Ok(PathBuf::from(app_data));
        }
    }
    let home = home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
    #[cfg(windows)]
    {
        Ok(home.join("AppData").join("Roaming"))
    }
    #[cfg(not(windows))]
    {
        Ok(home.join(".config"))
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
}

/// 确保目录存在并可写：创建目录后写入一个探针文件再删除
fn ensure_writable_dir(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建目录 {} 失败: {e}", dir.display()))?;
    if !dir.is_dir() {
        return Err(format!("{} 不是目录", dir.display()));
    }

    let probe = dir.join(".write-test");
    fs::write(&probe, b"ok").map_err(|e| format!("目录不可写 {}: {e}", dir.display()))?;
    let _ = fs::remove_file(&probe);
    Ok(())
}

/// WebView2 用户数据目录名
pub const WEBVIEW_PROFILE_DIR: &str = "WebView2";

/// WebView2 profile 目录（便携版把它也放在数据目录里，
/// 避免在 %LOCALAPPDATA% 留下 WebView2 数据）
pub fn webview_profile_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(WEBVIEW_PROFILE_DIR)
}

/// 把 WebView2 的用户数据目录指向便携目录。
/// WebView2 会读取 `WEBVIEW2_USER_DATA_FOLDER` 环境变量，
/// 且必须在创建 WebView 环境之前设置。
pub fn apply_portable_webview_profile(data_dir: &Path) -> Result<PathBuf, String> {
    let dir = webview_profile_dir(data_dir);
    fs::create_dir_all(&dir).map_err(|e| format!("创建 WebView2 数据目录失败: {e}"))?;
    std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", &dir);
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_missing_directory() {
        let dir = std::env::temp_dir().join("pm-paths-test-created");
        let _ = fs::remove_dir_all(&dir);
        ensure_writable_dir(&dir).unwrap();
        assert!(dir.is_dir());
        // 目录里不应留下探针文件
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn rejects_non_directory_path() {
        let file = std::env::temp_dir().join("pm-paths-test-file");
        fs::write(&file, b"x").unwrap();
        assert!(ensure_writable_dir(&file).is_err());
        let _ = fs::remove_file(&file);
    }

    #[test]
    fn candidates_include_exe_sibling_and_appdata() {
        let list = candidates().unwrap();
        assert!(list.len() >= 2);
        let exe = std::env::current_exe().unwrap();
        let expected = exe.parent().unwrap().join(DATA_DIR_NAME);
        assert!(list.contains(&expected), "候选目录中应包含 exe 同级 data 目录");
    }

    #[test]
    fn portable_webview_profile_creates_dir_and_sets_env() {
        let dir = std::env::temp_dir().join("pm-paths-test-webview");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let profile = apply_portable_webview_profile(&dir).unwrap();
        assert_eq!(profile, webview_profile_dir(&dir));
        assert!(profile.is_dir());
        assert_eq!(std::env::var("WEBVIEW2_USER_DATA_FOLDER").unwrap(), profile.to_string_lossy());
    }
}

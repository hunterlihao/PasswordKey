//! 配置文件读写（对应 Go 的 internal/settings/settings.go）

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// 自定义数据文件路径；为空表示使用数据目录下的 vault.json
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub db_path: String,
}

impl Settings {
    /// 读取配置。文件不存在时返回默认配置；
    /// 文件损坏则报错，避免静默返回空配置导致误判 dbPath 为空、
    /// 进而引导用户重新设置主密钥覆盖原有自定义路径的数据库。
    pub fn load(dir: &Path) -> Result<Settings, String> {
        let path = dir.join(SETTINGS_FILE);
        let data = match fs::read(&path) {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
            Err(e) => return Err(format!("读取配置失败: {e}")),
        };

        serde_json::from_slice(&data)
            .map_err(|e| format!("配置文件解析失败（已损坏）: {e}"))
    }

    /// 保存配置，权限 0600
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        fs::create_dir_all(dir).map_err(|e| format!("创建配置目录失败: {e}"))?;
        let data = serde_json::to_vec_pretty(self).map_err(|e| format!("序列化配置失败: {e}"))?;
        write_private(dir.join(SETTINGS_FILE), &data)
    }
}

/// 以尽可能严格的权限写文件（Windows 上权限位不生效，仅 Unix 生效）
pub fn write_private(path: impl AsRef<Path>, data: &[u8]) -> Result<(), String> {
    use std::io::Write;

    let path = path.as_ref();
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("无法创建文件 {}: {e}", path.display()))?;
    file.write_all(data)
        .map_err(|e| format!("写入文件 {} 失败: {e}", path.display()))?;
    file.flush()
        .map_err(|e| format!("写入文件 {} 失败: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pm-settings-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_yields_default() {
        let dir = temp_dir("missing");
        let s = Settings::load(&dir).unwrap();
        assert!(s.db_path.is_empty());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = temp_dir("roundtrip");
        let s = Settings {
            db_path: "E:\\OwnerNode\\Data Save\\PasswordKey\\vault.json".to_string(),
        };
        s.save(&dir).unwrap();
        let loaded = Settings::load(&dir).unwrap();
        assert_eq!(loaded.db_path, s.db_path);
    }

    #[test]
    fn corrupt_file_is_reported() {
        let dir = temp_dir("corrupt");
        fs::write(dir.join(SETTINGS_FILE), b"{ not json").unwrap();
        let err = Settings::load(&dir).unwrap_err();
        assert!(err.contains("配置文件解析失败"), "实际: {err}");
    }

    #[test]
    fn empty_settings_serializes_compactly() {
        let json = serde_json::to_string(&Settings::default()).unwrap();
        assert_eq!(json, "{}");
    }
}

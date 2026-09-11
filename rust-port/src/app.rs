//! 应用状态与业务逻辑（对应 Go 的 app.go）。
//!
//! 设计要点：
//! - 密钥用定长数组 `[u8; 32]`（`Copy`），每次使用都是独立副本，
//!   天然规避 Go 版手工拷贝 + 清零所防的 TOCTOU 问题；
//! - 便携版数据目录解析见 [`crate::paths`]；
//! - 所有对外操作都返回 `Result<_, String>`，错误信息直接串到前端。

use std::path::{Path, PathBuf};

use chrono::{Duration, Local};
use zeroize::Zeroize;

use crate::crypto::{self, Key};
use crate::dialogs::{self, WindowHost};
use crate::export::{self, ExportCredential};
use crate::model::{self, Credential, CredentialView};
use crate::paths;
use crate::settings::{write_private, Settings};
use crate::store::Store;
use crate::timefmt;

pub struct App {
    key: Option<Key>,
    store: Option<Store>,
    settings: Settings,
    data_dir: PathBuf,
    db_path: PathBuf,
}

impl App {
    /// 启动：解析便携数据目录 -> 读配置 -> 打开数据库
    pub fn bootstrap() -> Result<App, String> {
        let data_dir = paths::resolve_data_dir()?;
        let settings = Settings::load(&data_dir)?;

        let db_path = if settings.db_path.trim().is_empty() {
            data_dir.join("data.db")
        } else {
            PathBuf::from(settings.db_path.trim())
        };

        let store = Store::open(&db_path)?;

        Ok(App {
            key: None,
            store: Some(store),
            settings,
            data_dir,
            db_path,
        })
    }

    /// 退出：清零密钥并关闭数据库
    pub fn shutdown(&mut self) {
        self.lock_key();
        if let Some(store) = self.store.take() {
            let _ = store.close();
        }
    }

    pub fn db_path_string(&self) -> String {
        self.db_path.to_string_lossy().into_owned()
    }

    // ===== 密钥 =====

    pub fn lock_key(&mut self) {
        if let Some(mut key) = self.key.take() {
            key.zeroize();
        }
    }

    pub fn is_unlocked(&self) -> bool {
        self.key.is_some()
    }

    /// 已解锁状态下取一份密钥副本执行 fn，执行完立即清零
    fn with_key<T, F>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Key) -> Result<T, String>,
    {
        let mut key = self.key.ok_or_else(|| "未解锁".to_string())?;
        let result = f(&key);
        key.zeroize();
        result
    }

    fn store(&self) -> Result<&Store, String> {
        self.store.as_ref().ok_or_else(|| "数据库未初始化".to_string())
    }

    fn store_mut(&mut self) -> Result<&mut Store, String> {
        self.store.as_mut().ok_or_else(|| "数据库未初始化".to_string())
    }

    // ===== 主密钥 =====

    pub fn is_master_set(&self) -> bool {
        self.store().map(|s| s.get_master_config().is_ok()).unwrap_or(false)
    }

    pub fn setup_master(&mut self, password: &str) -> Result<(), String> {
        let salt = crypto::generate_salt().map_err(String::from)?;
        let key = crypto::derive_key(password, &salt).map_err(String::from)?;
        let token = crypto::generate_verify_token().map_err(String::from)?;
        let encrypted_verify = crypto::encrypt(&key, &token).map_err(String::from)?;

        self.store()?.set_master_config(&salt, &encrypted_verify)?;

        self.key = Some(key);
        Ok(())
    }

    pub fn unlock(&mut self, password: &str) -> Result<(), String> {
        let store = self.store()?;

        // 暴力破解防护：状态持久化在数据库里，重启应用不会重置
        let mut state = store.get_unlock_state()?;
        if let Some(until) = state.locked_until {
            let now = Local::now();
            if now < until {
                let remaining = (until - now).num_seconds().max(0);
                return Err(format!("已锁定，请 {remaining} 秒后再试"));
            }
        }

        let mc = store.get_master_config()?;
        let key = crypto::derive_key(password, &mc.salt).map_err(String::from)?;

        if crypto::decrypt(&key, &mc.encrypted_verify).is_err() {
            state.fail_count += 1;
            if state.fail_count >= 5 {
                state.fail_count = 0;
                state.locked_until = Some(Local::now() + Duration::seconds(30));
                let _ = store.set_unlock_state(&state);
                return Err("连续错误 5 次，已锁定 30 秒".to_string());
            }
            let _ = store.set_unlock_state(&state);
            return Err("主密钥错误".to_string());
        }

        state.fail_count = 0;
        state.locked_until = None;
        let _ = store.set_unlock_state(&state);

        self.key = Some(key);
        Ok(())
    }

    /// 修改主密钥：事务内重新加密所有凭证，失败整体回滚
    pub fn change_master_password(&mut self, old_password: &str, new_password: &str) -> Result<(), String> {
        let mc = self.store()?.get_master_config()?;

        let mut old_key = crypto::derive_key(old_password, &mc.salt).map_err(String::from)?;
        if crypto::decrypt(&old_key, &mc.encrypted_verify).is_err() {
            old_key.zeroize();
            return Err("旧密码错误".to_string());
        }

        let new_salt = crypto::generate_salt().map_err(String::from)?;
        let mut new_key = crypto::derive_key(new_password, &new_salt).map_err(String::from)?;
        let token = crypto::generate_verify_token().map_err(String::from)?;
        let new_verify = crypto::encrypt(&new_key, &token).map_err(String::from)?;

        let outcome = {
            let store = self.store_mut()?;
            store.with_transaction(|tx| {
                // 先取出全部密文，再逐条重新加密（避免边读边写同一语句）
                let rows: Vec<(i64, Vec<u8>)> = {
                    let mut stmt = tx
                        .prepare("SELECT id, secret FROM credentials")
                        .map_err(|e| format!("读取凭证失败: {e}"))?;
                    let mapped = stmt
                        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)))
                        .map_err(|e| format!("读取凭证失败: {e}"))?;
                    let mut collected = Vec::new();
                    for row in mapped {
                        collected.push(row.map_err(|e| format!("读取凭证失败: {e}"))?);
                    }
                    collected
                };

                for (id, secret) in rows {
                    let mut plaintext = crypto::decrypt(&old_key, &secret)
                        .map_err(|e| format!("解密凭证 ID={id} 失败: {e}"))?;
                    let encrypted = crypto::encrypt(&new_key, &plaintext)
                        .map_err(|e| format!("重新加密凭证 ID={id} 失败: {e}"))?;
                    plaintext.zeroize();

                    tx.execute(
                        "UPDATE credentials SET secret=?, updated_at=? WHERE id=?",
                        rusqlite::params![encrypted, timefmt::now_text(), id],
                    )
                    .map_err(|e| format!("更新凭证 ID={id} 失败: {e}"))?;
                }

                tx.execute(
                    "INSERT OR REPLACE INTO master_config (id, salt, encrypted_verify) VALUES (1, ?, ?)",
                    rusqlite::params![new_salt, new_verify],
                )
                .map_err(|e| format!("更新主密钥配置失败: {e}"))?;

                Ok(())
            })
        };

        old_key.zeroize();

        if let Err(e) = outcome {
            // 事务失败：新密钥必须清零，不能留在内存里
            new_key.zeroize();
            return Err(e);
        }

        // 成功：旧密钥清零，新密钥转交
        self.lock_key();
        self.key = Some(new_key);
        Ok(())
    }

    // ===== 凭证读写 =====

    /// 列表查询。与 Go 版不同的一点加固：锁定状态下不再返回任何元数据
    /// （Go 版锁屏后仍能读到名称/URL，只有密码本身受保护）
    pub fn get_credentials(&self, cred_type: i32, keyword: &str) -> Vec<CredentialView> {
        if !self.is_unlocked() {
            return Vec::new();
        }
        let Ok(store) = self.store() else {
            return Vec::new();
        };
        let filter = if cred_type >= 0 { Some(cred_type) } else { None };
        store
            .list_credentials(filter, keyword)
            .map(|list| list.iter().map(Credential::to_view).collect())
            .unwrap_or_default()
    }

    pub fn get_favorites(&self, keyword: &str) -> Vec<CredentialView> {
        if !self.is_unlocked() {
            return Vec::new();
        }
        let Ok(store) = self.store() else {
            return Vec::new();
        };
        store
            .list_favorites(keyword)
            .map(|list| list.iter().map(Credential::to_view).collect())
            .unwrap_or_default()
    }

    pub fn get_credential_view_by_id(&self, id: i64) -> Result<CredentialView, String> {
        self.require_unlocked()?;
        Ok(self.store()?.get_credential(id)?.to_view())
    }

    pub fn get_credential_counts(&self) -> std::collections::BTreeMap<i32, i32> {
        if !self.is_unlocked() {
            return Default::default();
        }
        self.store()
            .and_then(|s| s.count_by_type())
            .unwrap_or_default()
    }

    pub fn add_credential(&mut self, mut credential: Credential) -> Result<(), String> {
        model::validate_credential(&credential)?;

        let encrypted = self.with_key(|key| {
            crypto::encrypt(key, credential.secret.as_bytes()).map_err(String::from)
        })?;

        credential.secret_bytes = encrypted;
        credential.secret.zeroize();
        self.store_mut()?.create_credential(&mut credential)
    }

    pub fn update_credential(&mut self, mut credential: Credential) -> Result<(), String> {
        model::validate_credential(&credential)?;

        let encrypted = self.with_key(|key| {
            crypto::encrypt(key, credential.secret.as_bytes()).map_err(String::from)
        })?;

        credential.secret_bytes = encrypted;
        credential.secret.zeroize();
        self.store_mut()?.update_credential(&mut credential)
    }

    pub fn delete_credential(&mut self, id: i64) -> Result<(), String> {
        self.require_unlocked()?;
        self.store_mut()?.delete_credential(id)
    }

    pub fn delete_credentials(&mut self, ids: &[i64]) -> Result<(), String> {
        self.require_unlocked()?;
        self.store_mut()?.delete_credentials(ids)
    }

    pub fn toggle_favorite(&mut self, id: i64) -> Result<(), String> {
        self.require_unlocked()?;
        self.store_mut()?.toggle_favorite(id)
    }

    fn require_unlocked(&self) -> Result<(), String> {
        if self.is_unlocked() {
            Ok(())
        } else {
            Err("未解锁".to_string())
        }
    }

    /// 解密单条凭证（无需二次验证）
    pub fn get_decrypted_secret_direct(&self, id: i64) -> Result<String, String> {
        let credential = self.store()?.get_credential(id)?;
        self.with_key(|key| decrypt_secret(key, &credential.secret_bytes))
    }

    /// 解密单条凭证（需二次验证主密钥）
    pub fn get_decrypted_secret(&self, id: i64, verify_password: &str) -> Result<String, String> {
        let store = self.store()?;
        let mc = store.get_master_config()?;

        let mut verify_key = crypto::derive_key(verify_password, &mc.salt).map_err(String::from)?;
        let verified = crypto::decrypt(&verify_key, &mc.encrypted_verify).is_ok();
        verify_key.zeroize();
        if !verified {
            return Err("主密钥验证失败".to_string());
        }

        let credential = store.get_credential(id)?;
        self.with_key(|key| decrypt_secret(key, &credential.secret_bytes))
    }

    // ===== 密码工具 =====

    pub fn generate_password(&self, length: i64) -> Result<String, String> {
        let length = if length <= 0 { 16 } else { length as usize };
        crypto::generate_random_password(length, true, true, true).map_err(String::from)
    }

    pub fn check_password_strength(&self, password: &str) -> i32 {
        model::check_password_strength(password)
    }

    // ===== 导入导出 =====

    pub fn export_data(&self, host: WindowHost, format: &str) -> Result<String, String> {
        let (ext, filter_name, patterns): (&str, &str, &[&str]) = if format == "excel" {
            ("xlsx", "Excel 文件 (*.xlsx)", &["xlsx"])
        } else {
            ("json", "JSON 文件 (*.json)", &["json"])
        };

        let default_name = format!("passwords_{}.{ext}", Local::now().format("%Y%m%d"));
        let path = dialogs::save_file(host, "导出凭证", &default_name, (filter_name, patterns))
            .ok_or_else(|| "已取消".to_string())?;

        let credentials = self.store()?.get_all_credentials()?;

        let mut export_credentials: Vec<ExportCredential> = Vec::new();
        let mut failed = 0usize;
        self.with_key(|key| {
            for c in &credentials {
                match crypto::decrypt(key, &c.secret_bytes).and_then(|pt| {
                    String::from_utf8(pt).map_err(|_| crypto::CryptoError::Decrypt("明文非 UTF-8".into()))
                }) {
                    Ok(plaintext) => export_credentials.push(export::credential_to_export(c, &plaintext)),
                    Err(_) => failed += 1,
                }
            }
            Ok(())
        })?;

        if format == "excel" {
            export::export_excel(&export_credentials, &path)?;
        } else {
            export::export_json(&export_credentials, &path)?;
        }

        if failed > 0 {
            // 文件已生成，仅提示部分凭证被跳过（与 Go 版行为一致）
            return Err(format!("导出完成，但有 {failed} 条凭证解密失败被跳过"));
        }
        Ok(path.to_string_lossy().into_owned())
    }

    pub fn export_single(&self, host: WindowHost, id: i64, format: &str) -> Result<String, String> {
        let credential = self.store()?.get_credential(id)?;
        let plaintext = self.with_key(|key| decrypt_secret(key, &credential.secret_bytes))?;

        let (ext, filter_name, patterns): (&str, &str, &[&str]) = if format == "excel" {
            ("xlsx", "Excel 文件 (*.xlsx)", &["xlsx"])
        } else {
            ("json", "JSON 文件 (*.json)", &["json"])
        };

        let default_name = format!("{}.{ext}", credential.name);
        let path = dialogs::save_file(host, "导出凭证", &default_name, (filter_name, patterns))
            .ok_or_else(|| "已取消".to_string())?;

        let single = vec![export::credential_to_export(&credential, &plaintext)];
        if format == "excel" {
            export::export_excel(&single, &path)?;
        } else {
            export::export_json(&single, &path)?;
        }
        Ok(path.to_string_lossy().into_owned())
    }

    pub fn import_data(&mut self, host: WindowHost) -> Result<i64, String> {
        self.require_unlocked()?;

        let path = dialogs::open_file(
            host,
            "导入凭证",
            &[
                ("支持的文件 (*.json;*.xlsx)", &["json", "xlsx"]),
                ("JSON 文件 (*.json)", &["json"]),
                ("Excel 文件 (*.xlsx)", &["xlsx"]),
            ],
        )
        .ok_or_else(|| "已取消".to_string())?;

        let path_str = path.to_string_lossy().into_owned();
        let imported = if export::guess_format(&path_str) == "excel" {
            export::import_excel(&path).map_err(|e| format!("解析文件失败: {e}"))?
        } else {
            export::import_json(&path).map_err(|e| format!("解析文件失败: {e}"))?
        };

        let mut count = 0i64;
        let mut failed = 0usize;

        // 先把 store 取出来，避免在 with_key 的闭包里同时借用 &mut self
        let mut store = self.store.take().ok_or_else(|| "数据库未初始化".to_string())?;
        let outcome = self.with_key(|key| {
            for ec in &imported {
                if ec.name.is_empty() || ec.secret.is_empty() {
                    continue;
                }
                let mut credential = export::export_to_credential(ec);
                let encrypted = match crypto::encrypt(key, credential.secret.as_bytes()) {
                    Ok(data) => data,
                    Err(_) => {
                        failed += 1;
                        continue;
                    }
                };
                credential.secret_bytes = encrypted;
                credential.secret.zeroize();

                match store.create_credential(&mut credential) {
                    Ok(()) => count += 1,
                    Err(_) => failed += 1,
                }
            }
            Ok(())
        });
        self.store = Some(store);
        outcome?;

        if failed > 0 {
            return Err(format!("成功导入 {count} 条，{failed} 条失败"));
        }
        Ok(count)
    }

    pub fn download_json_template(&self, host: WindowHost) -> Result<String, String> {
        let path = dialogs::save_file(
            host,
            "保存JSON模板",
            "password_template.json",
            ("JSON 文件 (*.json)", &["json"]),
        )
        .ok_or_else(|| "已取消".to_string())?;

        let data = export::json_template()?;
        write_private(&path, &data)?;
        Ok(path.to_string_lossy().into_owned())
    }

    pub fn download_excel_template(&self, host: WindowHost) -> Result<String, String> {
        let path = dialogs::save_file(
            host,
            "保存Excel模板",
            "password_template.xlsx",
            ("Excel 文件 (*.xlsx)", &["xlsx"]),
        )
        .ok_or_else(|| "已取消".to_string())?;

        export::excel_template(&path)?;
        Ok(path.to_string_lossy().into_owned())
    }

    // ===== 数据库路径 =====

    pub fn change_db_path(&mut self, host: WindowHost) -> Result<String, String> {
        if self.store.is_none() {
            return Err("数据库未初始化".to_string());
        }

        let new_path = dialogs::save_file(
            host,
            "选择数据库存储位置",
            "data.db",
            ("数据库文件 (*.db)", &["db"]),
        )
        .ok_or_else(|| "已取消".to_string())?;

        if new_path == self.db_path {
            return Ok(self.db_path_string());
        }

        let old_path = self.db_path.clone();

        // 先关闭连接，确保数据落盘
        if let Some(store) = self.store.take() {
            if let Err(e) = store.close() {
                self.reopen(&old_path)?;
                return Err(e);
            }
        }

        if let Err(e) = copy_db_files(&old_path, &new_path) {
            let rollback = self.reopen(&old_path);
            return Err(match rollback {
                Ok(()) => format!("无法复制数据库: {e}"),
                Err(re) => format!("复制数据库失败且回滚打开原库也失败: 复制错误={e}, 回滚错误={re}"),
            });
        }

        match Store::open(&new_path) {
            Ok(store) => {
                self.store = Some(store);
                self.db_path = new_path.clone();

                self.settings.db_path = new_path.to_string_lossy().into_owned();
                if let Err(e) = self.settings.save(&self.data_dir) {
                    return Err(format!("路径已更改但设置保存失败: {e}"));
                }
                Ok(self.db_path_string())
            }
            Err(e) => {
                let rollback = self.reopen(&old_path);
                Err(match rollback {
                    Ok(()) => format!("新数据库打开失败: {e}"),
                    Err(re) => format!("新数据库打开失败且回滚打开原库也失败: 打开错误={e}, 回滚错误={re}"),
                })
            }
        }
    }

    pub fn reset_db_path(&mut self) -> Result<(), String> {
        if self.store.is_none() {
            return Err("数据库未初始化".to_string());
        }

        let default_path = self.data_dir.join("data.db");
        if self.db_path == default_path {
            return Err("已经是默认路径".to_string());
        }

        let old_path = self.db_path.clone();

        if let Some(store) = self.store.take() {
            if let Err(e) = store.close() {
                self.reopen(&old_path)?;
                return Err(e);
            }
        }

        if !default_path.exists() {
            if let Err(e) = copy_db_files(&old_path, &default_path) {
                let rollback = self.reopen(&old_path);
                return Err(match rollback {
                    Ok(()) => format!("无法复制数据库到默认路径: {e}"),
                    Err(re) => format!("复制到默认路径失败且回滚打开原库也失败: 复制错误={e}, 回滚错误={re}"),
                });
            }
        }

        match Store::open(&default_path) {
            Ok(store) => {
                self.store = Some(store);
                self.db_path = default_path;

                self.settings.db_path.clear();
                self.settings
                    .save(&self.data_dir)
                    .map_err(|e| format!("设置保存失败: {e}"))?;
                Ok(())
            }
            Err(e) => {
                let rollback = self.reopen(&old_path);
                Err(match rollback {
                    Ok(()) => format!("默认数据库打开失败: {e}"),
                    Err(re) => format!("默认数据库打开失败且回滚打开原库也失败: 打开错误={e}, 回滚错误={re}"),
                })
            }
        }
    }

    /// 回滚：重新打开原路径
    fn reopen(&mut self, path: &Path) -> Result<(), String> {
        let store = Store::open(path)?;
        self.store = Some(store);
        Ok(())
    }
}

fn decrypt_secret(key: &Key, secret_bytes: &[u8]) -> Result<String, String> {
    let plaintext = crypto::decrypt(key, secret_bytes).map_err(String::from)?;
    match String::from_utf8(plaintext) {
        Ok(text) => Ok(text),
        Err(e) => {
            let mut bytes = e.into_bytes();
            bytes.zeroize();
            Err("解密结果不是合法文本".to_string())
        }
    }
}

/// 复制单个 SQLite 数据库文件到新位置，并限制副本权限。
/// DELETE journal 模式下没有 -wal / -shm 旁车文件，只需复制主文件。
pub fn copy_db_files(src_path: &Path, dst_path: &Path) -> Result<(), String> {
    let data = std::fs::read(src_path).map_err(|e| format!("无法读取当前数据库: {e}"))?;
    write_private(dst_path, &data)?;
    Ok(())
}

#[cfg(test)]
impl App {
    /// 测试用构造：直接注入内部状态，绕过数据目录探测
    pub fn for_test(store: Store, settings: Settings, data_dir: PathBuf, db_path: PathBuf) -> App {
        App {
            key: None,
            store: Some(store),
            settings,
            data_dir,
            db_path,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个运行在临时目录里的 App
    fn temp_app(name: &str) -> (App, PathBuf) {
        let dir = std::env::temp_dir().join(format!("pm-app-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("data.db");
        let store = Store::open(&db_path).unwrap();
        let app = App::for_test(store, Settings::default(), dir.clone(), db_path);
        (app, dir)
    }

    fn sample_credential(name: &str, secret: &str) -> Credential {
        Credential {
            name: name.to_string(),
            cred_type: model::TYPE_BROWSER_PASSWORD,
            secret: secret.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn setup_unlock_lock_flow() {
        let (mut app, _dir) = temp_app("lock");
        assert!(!app.is_master_set());
        assert!(!app.is_unlocked());

        app.setup_master("master-pass").unwrap();
        assert!(app.is_master_set());
        assert!(app.is_unlocked());
        assert!(app.is_master_set());

        app.lock_key();
        assert!(!app.is_unlocked());
        // 锁定后不能操作
        assert_eq!(app.delete_credential(1).unwrap_err(), "未解锁");

        app.unlock("wrong").unwrap_err();
        app.unlock("master-pass").unwrap();
        assert!(app.is_unlocked());
    }

    #[test]
    fn repeated_failures_lock_the_app() {
        let (mut app, _dir) = temp_app("bruteforce");
        app.setup_master("right").unwrap();
        app.lock_key();

        for _ in 0..4 {
            assert_eq!(app.unlock("wrong").unwrap_err(), "主密钥错误");
        }
        let err = app.unlock("wrong").unwrap_err();
        assert!(err.contains("已锁定 30 秒"), "实际: {err}");

        // 锁定期间即使密码正确也会被拒
        let err = app.unlock("right").unwrap_err();
        assert!(err.contains("已锁定"), "实际: {err}");
    }

    #[test]
    fn credential_encrypt_decrypt_roundtrip() {
        let (mut app, _dir) = temp_app("cred");
        app.setup_master("pw").unwrap();

        let mut c = sample_credential("GitHub", "s3cr3t-密码");
        app.add_credential(c.clone()).unwrap();
        c.id = 1;

        let list = app.get_credentials(-1, "");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "GitHub");

        assert_eq!(app.get_decrypted_secret_direct(1).unwrap(), "s3cr3t-密码");
        assert_eq!(app.get_decrypted_secret(1, "pw").unwrap(), "s3cr3t-密码");
        assert_eq!(app.get_decrypted_secret(1, "bad").unwrap_err(), "主密钥验证失败");
    }

    #[test]
    fn add_credential_rejects_invalid_input() {
        let (mut app, _dir) = temp_app("validate");
        app.setup_master("pw").unwrap();
        let mut c = sample_credential("", "x");
        assert_eq!(app.add_credential(c.clone()).unwrap_err(), "名称不能为空");
        c.name = "ok".into();
        c.cred_type = 42;
        assert_eq!(app.add_credential(c).unwrap_err(), "无效的凭证类型");
    }

    #[test]
    fn add_credential_requires_unlock() {
        let (mut app, _dir) = temp_app("locked-add");
        app.setup_master("pw").unwrap();
        app.lock_key();
        let err = app.add_credential(sample_credential("a", "b")).unwrap_err();
        assert_eq!(err, "未解锁");
    }

    #[test]
    fn search_and_counts() {
        let (mut app, _dir) = temp_app("search");
        app.setup_master("pw").unwrap();
        app.add_credential(sample_credential("GitHub", "a")).unwrap();
        app.add_credential(sample_credential("GitLab", "b")).unwrap();
        let mut api_key = sample_credential("OpenAI", "c");
        api_key.cred_type = model::TYPE_API_KEY;
        app.add_credential(api_key).unwrap();

        assert_eq!(app.get_credentials(-1, "").len(), 3);
        assert_eq!(app.get_credentials(model::TYPE_API_KEY, "").len(), 1);
        assert_eq!(app.get_credentials(-1, "git").len(), 2);

        let counts = app.get_credential_counts();
        assert_eq!(counts[&-1], 3);
        assert_eq!(counts[&model::TYPE_API_KEY], 1);

        app.toggle_favorite(1).unwrap();
        assert_eq!(app.get_favorites("").len(), 1);
        assert_eq!(app.get_credential_counts()[&-2], 1);
    }

    #[test]
    fn change_master_password_reencrypts_everything() {
        let (mut app, _dir) = temp_app("change-master");
        app.setup_master("old-pw").unwrap();
        app.add_credential(sample_credential("a", "secret-a")).unwrap();
        app.add_credential(sample_credential("b", "secret-b")).unwrap();

        assert_eq!(app.change_master_password("nope", "new-pw").unwrap_err(), "旧密码错误");

        app.change_master_password("old-pw", "new-pw").unwrap();
        app.lock_key();
        assert_eq!(app.unlock("new-pw").unwrap(), ());
        assert_eq!(app.get_decrypted_secret_direct(1).unwrap(), "secret-a");
        assert_eq!(app.get_decrypted_secret_direct(2).unwrap(), "secret-b");
    }

    #[test]
    fn locked_state_hides_metadata() {
        let (mut app, _dir) = temp_app("locked-read");
        app.setup_master("pw").unwrap();
        app.add_credential(sample_credential("GitHub", "s")).unwrap();
        assert_eq!(app.get_credentials(-1, "").len(), 1);

        app.lock_key();
        assert!(app.get_credentials(-1, "").is_empty(), "锁屏后不应返回列表");
        assert!(app.get_favorites("").is_empty());
        assert!(app.get_credential_counts().is_empty());
        assert_eq!(app.get_credential_view_by_id(1).unwrap_err(), "未解锁");
        assert_eq!(app.get_decrypted_secret_direct(1).unwrap_err(), "未解锁");
    }

    #[test]
    fn copy_db_files_copies_single_file() {
        let dir = std::env::temp_dir().join("pm-app-test-copy-single");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("data.db");
        std::fs::write(&src, b"sqlite data").unwrap();

        let target_dir = dir.join("copy");
        std::fs::create_dir_all(&target_dir).unwrap();
        let target = target_dir.join("data.db");
        copy_db_files(&src, &target).unwrap();

        assert!(target.exists());
        assert_eq!(std::fs::read(&target).unwrap(), b"sqlite data");
    }

    #[test]
    fn import_export_roundtrip_through_app() {
        let (mut app, dir) = temp_app("io");
        app.setup_master("pw").unwrap();
        app.add_credential(sample_credential("GitHub", "p@ss")).unwrap();

        // 直接调用导出逻辑（跳过文件对话框）
        let credentials = app.store().unwrap().get_all_credentials().unwrap();
        let plaintext = app.get_decrypted_secret_direct(1).unwrap();
        let exported = vec![export::credential_to_export(&credentials[0], &plaintext)];
        let json_path = dir.join("out.json");
        export::export_json(&exported, &json_path).unwrap();

        let fresh_dir = dir.join("fresh");
        std::fs::create_dir_all(&fresh_dir).unwrap();
        let mut fresh = App {
            key: None,
            store: Some(Store::open(&fresh_dir.join("data.db")).unwrap()),
            settings: Settings::default(),
            data_dir: fresh_dir.clone(),
            db_path: fresh_dir.join("data.db"),
        };
        fresh.setup_master("pw2").unwrap();

        let imported = export::import_json(&json_path).unwrap();
        for ec in &imported {
            let mut credential = export::export_to_credential(ec);
            let encrypted = crypto::encrypt(&fresh.key.unwrap(), credential.secret.as_bytes()).unwrap();
            credential.secret_bytes = encrypted;
            fresh.store_mut().unwrap().create_credential(&mut credential).unwrap();
        }
        assert_eq!(fresh.get_credentials(-1, "").len(), 1);
        assert_eq!(fresh.get_decrypted_secret_direct(1).unwrap(), "p@ss");
    }
}

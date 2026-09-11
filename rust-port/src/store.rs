//! JSON 文件存储层。
//!
//! 全部数据（主密钥配置、凭证列表、解锁失败状态、自增 ID）序列化到
//! 便携目录下的单个 `vault.json`。所有读写在内存里完成，写盘采用
//! 原子写（先写 `.tmp` 再 rename）保证崩溃时不损坏已有数据。
//!
//! 密文以 base64 编码存储，避免非 UTF-8 字节破坏 JSON。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::model::{Credential, MasterConfig};
use crate::timefmt::{lock_stamp, now_text, parse_lock_stamp, to_rfc3339};

/// 文件格式版本（破坏性改动时递增，便于未来迁移）
const VAULT_VERSION: u32 = 1;

/// 持久化的暴力破解防护状态
#[derive(Debug, Clone, Default)]
pub struct UnlockState {
    pub fail_count: i32,
    pub locked_until: Option<DateTime<Local>>,
}

// ===== 内部存储结构（仅 JSON 序列化用，不暴露到 crate 外） =====

#[derive(Default, Serialize, Deserialize, Debug)]
struct VaultFile {
    /// 文件格式版本；老文件无此字段则默认为 0，加载后会被改写为当前版本
    #[serde(default)]
    version: u32,
    #[serde(default)]
    master_config: Option<MasterConfigStored>,
    #[serde(default)]
    app_state: AppStateStored,
    /// 下一个凭证 ID（对应 SQLite 的 AUTOINCREMENT 语义）
    #[serde(default)]
    next_id: i64,
    #[serde(default)]
    credentials: Vec<CredentialStored>,
}

#[derive(Serialize, Deserialize, Debug)]
struct MasterConfigStored {
    /// base64 编码的 salt
    salt: String,
    /// base64 编码的 encrypted_verify
    encrypted_verify: String,
}

#[derive(Default, Serialize, Deserialize, Debug)]
struct AppStateStored {
    #[serde(default)]
    fail_count: i32,
    /// 空 = 未锁定；非空 = lock_stamp() 格式的时间串
    #[serde(default)]
    locked_until: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct CredentialStored {
    id: i64,
    name: String,
    #[serde(rename = "type")]
    cred_type: i32,
    url: String,
    username: String,
    /// base64 编码的密文
    secret: String,
    notes: String,
    is_favorite: i32,
    created_at: String,
    updated_at: String,
}

impl From<&Credential> for CredentialStored {
    fn from(c: &Credential) -> Self {
        Self {
            id: c.id,
            name: c.name.clone(),
            cred_type: c.cred_type,
            url: c.url.clone(),
            username: c.username.clone(),
            secret: B64.encode(&c.secret_bytes),
            notes: c.notes.clone(),
            is_favorite: c.is_favorite,
            created_at: c.created_at.clone(),
            updated_at: c.updated_at.clone(),
        }
    }
}

impl From<CredentialStored> for Credential {
    fn from(s: CredentialStored) -> Self {
        let secret_bytes = B64.decode(&s.secret).unwrap_or_default();
        Credential {
            id: s.id,
            name: s.name,
            cred_type: s.cred_type,
            url: s.url,
            username: s.username,
            secret: String::new(),
            secret_bytes,
            notes: s.notes,
            is_favorite: s.is_favorite,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

pub struct Store {
    path: PathBuf,
    vault: VaultFile,
    /// 是否有待落盘的修改
    dirty: bool,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.path)
            .field("next_id", &self.vault.next_id)
            .field("credentials_count", &self.vault.credentials.len())
            .field("dirty", &self.dirty)
            .finish()
    }
}

impl Store {
    /// 打开（必要时创建）数据文件。
    /// - 文件不存在 / 为空：创建空 vault 并**立即落盘**，让用户能直接看到 `vault.json`
    /// - 文件损坏：报错（避免静默重置导致数据丢失）
    pub fn open(path: &Path) -> Result<Store, String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建数据目录失败: {e}"))?;
            }
        }

        let (vault, needs_init_write) = match std::fs::read(path) {
            Ok(data) if data.is_empty() => (
                VaultFile {
                    version: VAULT_VERSION,
                    ..Default::default()
                },
                true,
            ),
            Ok(data) => {
                let mut v: VaultFile = serde_json::from_slice(&data)
                    .map_err(|e| format!("数据文件解析失败（已损坏）: {e}"))?;
                // 老文件 version=0，加载后补成当前版本并落盘
                let was_old = v.version == 0;
                if was_old {
                    v.version = VAULT_VERSION;
                }
                (v, was_old)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (
                VaultFile {
                    version: VAULT_VERSION,
                    ..Default::default()
                },
                true,
            ),
            Err(e) => return Err(format!("读取数据文件失败: {e}")),
        };

        let mut store = Store {
            path: path.to_path_buf(),
            vault,
            dirty: needs_init_write,
        };
        // 文件不存在 / 为空 / 老版本时，立即落盘，让用户能直接看到 vault.json
        if needs_init_write {
            store.flush()?;
        }
        Ok(store)
    }

    /// 关闭并落盘（如果有未保存改动）
    pub fn close(mut self) -> Result<(), String> {
        self.flush()
    }

    /// 原子写：写 `vault.json.tmp` 再 rename 成 `vault.json`，
    /// 保证写到一半崩溃不会损坏已有数据。
    pub fn flush(&mut self) -> Result<(), String> {
        if !self.dirty {
            return Ok(());
        }
        let tmp = self.path.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(&self.vault)
            .map_err(|e| format!("序列化数据失败: {e}"))?;
        crate::settings::write_private(&tmp, &data)?;
        std::fs::rename(&tmp, &self.path).map_err(|e| format!("原子写入失败: {e}"))?;
        self.dirty = false;
        Ok(())
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    // ===== 主密钥配置 =====

    pub fn get_master_config(&self) -> Result<MasterConfig, String> {
        let mc = self
            .vault
            .master_config
            .as_ref()
            .ok_or_else(|| "未设置主密钥".to_string())?;
        Ok(MasterConfig {
            salt: B64
                .decode(&mc.salt)
                .map_err(|e| format!("主密钥 salt 解码失败: {e}"))?,
            encrypted_verify: B64
                .decode(&mc.encrypted_verify)
                .map_err(|e| format!("主密钥验证值解码失败: {e}"))?,
        })
    }

    pub fn set_master_config(&mut self, salt: &[u8], encrypted_verify: &[u8]) -> Result<(), String> {
        self.vault.master_config = Some(MasterConfigStored {
            salt: B64.encode(salt),
            encrypted_verify: B64.encode(encrypted_verify),
        });
        self.mark_dirty();
        Ok(())
    }

    // ===== 凭证 CRUD =====

    pub fn create_credential(&mut self, c: &mut Credential) -> Result<(), String> {
        let now = now_text();
        let id = self.vault.next_id.max(1);
        // 自增：先取当前 next_id 作为新 id，再 +1
        self.vault.next_id = id + 1;

        let stored = CredentialStored {
            id,
            name: c.name.clone(),
            cred_type: c.cred_type,
            url: c.url.clone(),
            username: c.username.clone(),
            secret: B64.encode(&c.secret_bytes),
            notes: c.notes.clone(),
            is_favorite: c.is_favorite,
            created_at: to_rfc3339(&now),
            updated_at: to_rfc3339(&now),
        };
        self.vault.credentials.push(stored);

        c.id = id;
        c.created_at = to_rfc3339(&now);
        c.updated_at = to_rfc3339(&now);
        self.mark_dirty();
        Ok(())
    }

    pub fn get_credential(&self, id: i64) -> Result<Credential, String> {
        self.vault
            .credentials
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .map(Credential::from)
            .ok_or_else(|| "凭证不存在".to_string())
    }

    pub fn update_credential(&mut self, c: &mut Credential) -> Result<(), String> {
        let now = now_text();
        let idx = self
            .vault
            .credentials
            .iter()
            .position(|x| x.id == c.id)
            .ok_or_else(|| "凭证不存在".to_string())?;

        let stored = &mut self.vault.credentials[idx];
        stored.name = c.name.clone();
        stored.cred_type = c.cred_type;
        stored.url = c.url.clone();
        stored.username = c.username.clone();
        stored.secret = B64.encode(&c.secret_bytes);
        stored.notes = c.notes.clone();
        stored.is_favorite = c.is_favorite;
        stored.updated_at = to_rfc3339(&now);

        c.updated_at = to_rfc3339(&now);
        self.mark_dirty();
        Ok(())
    }

    pub fn delete_credential(&mut self, id: i64) -> Result<(), String> {
        let before = self.vault.credentials.len();
        self.vault.credentials.retain(|c| c.id != id);
        if self.vault.credentials.len() == before {
            return Err("凭证不存在".to_string());
        }
        self.mark_dirty();
        Ok(())
    }

    pub fn delete_credentials(&mut self, ids: &[i64]) -> Result<(), String> {
        if ids.is_empty() {
            return Ok(());
        }
        let set: std::collections::HashSet<i64> = ids.iter().copied().collect();
        self.vault.credentials.retain(|c| !set.contains(&c.id));
        self.mark_dirty();
        Ok(())
    }

    pub fn toggle_favorite(&mut self, id: i64) -> Result<(), String> {
        let c = self
            .vault
            .credentials
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| "凭证不存在".to_string())?;
        c.is_favorite = 1 - c.is_favorite;
        c.updated_at = to_rfc3339(&now_text());
        self.mark_dirty();
        Ok(())
    }

    /// 列表查询：按类型过滤 + 关键字模糊匹配（name/url/username，大小写不敏感）
    pub fn list_credentials(&self, cred_type: Option<i32>, keyword: &str) -> Result<Vec<Credential>, String> {
        let kw = keyword.to_lowercase();
        let mut out: Vec<Credential> = self
            .vault
            .credentials
            .iter()
            .filter(|c| {
                if let Some(t) = cred_type {
                    if c.cred_type != t {
                        return false;
                    }
                }
                if !kw.is_empty() {
                    let hay = format!("{} {} {}", c.name, c.url, c.username).to_lowercase();
                    if !hay.contains(&kw) {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .map(Credential::from)
            .collect();
        // 与 SQLite 版一致：收藏优先，再按 updated_at 倒序
        out.sort_by(|a, b| {
            b.is_favorite
                .cmp(&a.is_favorite)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });
        Ok(out)
    }

    /// 收藏列表
    pub fn list_favorites(&self, keyword: &str) -> Result<Vec<Credential>, String> {
        let kw = keyword.to_lowercase();
        let mut out: Vec<Credential> = self
            .vault
            .credentials
            .iter()
            .filter(|c| {
                if c.is_favorite != 1 {
                    return false;
                }
                if !kw.is_empty() {
                    let hay = format!("{} {} {}", c.name, c.url, c.username).to_lowercase();
                    if !hay.contains(&kw) {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .map(Credential::from)
            .collect();
        out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(out)
    }

    /// 全量导出用
    pub fn get_all_credentials(&self) -> Result<Vec<Credential>, String> {
        let mut out: Vec<Credential> = self
            .vault
            .credentials
            .iter()
            .cloned()
            .map(Credential::from)
            .collect();
        out.sort_by_key(|c| c.id);
        Ok(out)
    }

    /// 各类型数量，key -1=全部, -2=收藏
    pub fn count_by_type(&self) -> Result<BTreeMap<i32, i32>, String> {
        let mut counts = BTreeMap::new();
        let mut total = 0;
        let mut favorites = 0;
        for c in &self.vault.credentials {
            total += 1;
            if c.is_favorite == 1 {
                favorites += 1;
            }
            *counts.entry(c.cred_type).or_insert(0) += 1;
        }
        counts.insert(-1, total);
        counts.insert(-2, favorites);
        Ok(counts)
    }

    /// 修改主密钥时重新加密所有凭证的密文。
    /// 在内存里完成全部解密 + 重加密，全部成功才写回 vault；任何一条失败
    /// 都不修改 vault 内容（等价于事务回滚），已解密的明文立即清零。
    pub fn reencrypt_all(
        &mut self,
        old_key: &crate::crypto::Key,
        new_key: &crate::crypto::Key,
    ) -> Result<(), String> {
        use zeroize::Zeroize;

        // 先在临时缓冲区里完成全部解密 + 重加密，失败时不动 vault
        let mut reencrypted: Vec<Vec<u8>> = Vec::with_capacity(self.vault.credentials.len());
        for c in &self.vault.credentials {
            let ciphertext = B64
                .decode(&c.secret)
                .map_err(|e| format!("密文 base64 解码失败 ID={}: {e}", c.id))?;
            let mut plaintext = match crate::crypto::decrypt(old_key, &ciphertext) {
                Ok(pt) => pt,
                Err(e) => {
                    for p in reencrypted.iter_mut() {
                        p.zeroize();
                    }
                    return Err(format!("解密凭证 ID={} 失败: {e}", c.id));
                }
            };
            // reencrypted 存的是「重加密后的密文」；明文用完立即清零
            let encrypted = match crate::crypto::encrypt(new_key, &plaintext) {
                Ok(data) => data,
                Err(e) => {
                    plaintext.zeroize();
                    for p in reencrypted.iter_mut() {
                        p.zeroize();
                    }
                    return Err(format!("重新加密凭证 ID={} 失败: {e}", c.id));
                }
            };
            plaintext.zeroize();
            reencrypted.push(encrypted);
        }

        // 全部成功，写回 vault + 落盘标记
        let now = to_rfc3339(&now_text());
        for (idx, encrypted) in reencrypted.into_iter().enumerate() {
            self.vault.credentials[idx].secret = B64.encode(&encrypted);
            self.vault.credentials[idx].updated_at = now.clone();
        }
        self.mark_dirty();
        Ok(())
    }

    // ===== 解锁失败状态（暴力破解防护）=====

    pub fn get_unlock_state(&self) -> Result<UnlockState, String> {
        Ok(UnlockState {
            fail_count: self.vault.app_state.fail_count,
            locked_until: parse_lock_stamp(&self.vault.app_state.locked_until),
        })
    }

    pub fn set_unlock_state(&mut self, state: &UnlockState) -> Result<(), String> {
        self.vault.app_state.fail_count = state.fail_count;
        self.vault.app_state.locked_until = state.locked_until.map(lock_stamp).unwrap_or_default();
        self.mark_dirty();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{TYPE_API_KEY, TYPE_BROWSER_PASSWORD};

    fn temp_store(name: &str) -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("pm-store-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vault.json");
        (Store::open(&path).unwrap(), dir)
    }

    fn cred(name: &str, cred_type: i32, secret: &[u8]) -> Credential {
        Credential {
            name: name.to_string(),
            cred_type,
            secret_bytes: secret.to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn open_creates_empty_vault_and_is_idempotent() {
        let (mut store, dir) = temp_store("schema");
        assert!(store.get_master_config().is_err());
        // 写一次空改动让文件落盘
        store.set_master_config(&[1], &[2]).unwrap();
        store.flush().unwrap();
        drop(store);

        // 二次打开不应报错，且能读回配置
        let store = Store::open(&dir.join("vault.json")).unwrap();
        let mc = store.get_master_config().unwrap();
        assert_eq!(mc.salt, vec![1]);
        assert_eq!(mc.encrypted_verify, vec![2]);
    }

    #[test]
    fn crud_roundtrip_keeps_ciphertext_bytes() {
        let (mut store, _dir) = temp_store("crud");

        let mut c = cred("GitHub", TYPE_API_KEY, &[0x00, 0xFF, 0x80, 0x01]);
        store.create_credential(&mut c).unwrap();
        assert!(c.id > 0);
        assert!(!c.created_at.is_empty());

        let loaded = store.get_credential(c.id).unwrap();
        assert_eq!(loaded.secret_bytes, vec![0x00, 0xFF, 0x80, 0x01]);
        assert_eq!(loaded.name, "GitHub");
        assert_eq!(loaded.cred_type, TYPE_API_KEY);

        let mut loaded = loaded;
        loaded.name = "GitLab".into();
        store.update_credential(&mut loaded).unwrap();
        assert_eq!(store.get_credential(c.id).unwrap().name, "GitLab");

        store.toggle_favorite(c.id).unwrap();
        assert_eq!(store.get_credential(c.id).unwrap().is_favorite, 1);
        assert_eq!(store.list_favorites("").unwrap().len(), 1);

        store.delete_credential(c.id).unwrap();
        assert!(store.get_credential(c.id).is_err());
    }

    #[test]
    fn search_is_case_insensitive_and_matches_name_url_username() {
        let (mut store, _dir) = temp_store("search");
        store
            .create_credential(&mut cred("GitHub", TYPE_BROWSER_PASSWORD, b"a"))
            .unwrap();
        store
            .create_credential(&mut cred("gitlab", TYPE_BROWSER_PASSWORD, b"b"))
            .unwrap();
        store
            .create_credential(&mut cred("other", TYPE_BROWSER_PASSWORD, b"c"))
            .unwrap();

        // 大小写不敏感
        assert_eq!(store.list_credentials(None, "GIT").unwrap().len(), 2);
        // 匹配 url
        let mut with_url = cred("site", TYPE_BROWSER_PASSWORD, b"d");
        with_url.url = "https://example.com/login".into();
        store.create_credential(&mut with_url).unwrap();
        assert_eq!(store.list_credentials(None, "example.com").unwrap().len(), 1);
        // 匹配 username
        let mut with_user = cred("user", TYPE_BROWSER_PASSWORD, b"e");
        with_user.username = "alice@example.com".into();
        store.create_credential(&mut with_user).unwrap();
        assert_eq!(store.list_credentials(None, "alice").unwrap().len(), 1);
        // 类型过滤
        assert_eq!(
            store.list_credentials(Some(TYPE_BROWSER_PASSWORD), "").unwrap().len(),
            5
        );
        assert_eq!(store.list_credentials(Some(TYPE_API_KEY), "").unwrap().len(), 0);
    }

    #[test]
    fn counts_and_batch_delete() {
        let (mut store, _dir) = temp_store("counts");
        let mut a = cred("a", TYPE_BROWSER_PASSWORD, b"x");
        let mut b = cred("b", TYPE_API_KEY, b"y");
        store.create_credential(&mut a).unwrap();
        store.create_credential(&mut b).unwrap();
        store.toggle_favorite(b.id).unwrap();

        let counts = store.count_by_type().unwrap();
        assert_eq!(counts[&-1], 2);
        assert_eq!(counts[&-2], 1);
        assert_eq!(counts[&TYPE_BROWSER_PASSWORD], 1);
        assert_eq!(counts[&TYPE_API_KEY], 1);

        store.delete_credentials(&[a.id, b.id]).unwrap();
        assert_eq!(store.count_by_type().unwrap()[&-1], 0);
        // 空列表不应报错
        store.delete_credentials(&[]).unwrap();
    }

    #[test]
    fn master_config_and_unlock_state() {
        let (mut store, _dir) = temp_store("master");
        store.set_master_config(&[1, 2, 3], &[4, 5, 6]).unwrap();
        let mc = store.get_master_config().unwrap();
        assert_eq!(mc.salt, vec![1, 2, 3]);
        assert_eq!(mc.encrypted_verify, vec![4, 5, 6]);

        let empty = store.get_unlock_state().unwrap();
        assert_eq!(empty.fail_count, 0);
        assert!(empty.locked_until.is_none());

        let state = UnlockState {
            fail_count: 3,
            locked_until: Some(Local::now() + chrono::Duration::seconds(30)),
        };
        store.set_unlock_state(&state).unwrap();
        let loaded = store.get_unlock_state().unwrap();
        assert_eq!(loaded.fail_count, 3);
        assert!(loaded.locked_until.is_some());
    }

    #[test]
    fn flush_writes_atomic_and_reload_reads_back() {
        let (mut store, dir) = temp_store("flush");
        store
            .create_credential(&mut cred("GitHub", TYPE_API_KEY, &[0xAB, 0xCD]))
            .unwrap();
        store.flush().unwrap();

        // 原子写后会留有 vault.json，没有残留 .tmp
        assert!(dir.join("vault.json").exists());
        assert!(!dir.join("vault.json.tmp").exists());

        // 重新打开能读回
        let reopened = Store::open(&dir.join("vault.json")).unwrap();
        let list = reopened.list_credentials(None, "").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "GitHub");
        assert_eq!(list[0].secret_bytes, vec![0xAB, 0xCD]);
    }

    #[test]
    fn next_id_persists_across_reopen() {
        let (mut store, dir) = temp_store("nextid");
        let mut a = cred("a", TYPE_BROWSER_PASSWORD, b"x");
        store.create_credential(&mut a).unwrap();
        store.flush().unwrap();
        drop(store);

        let mut store = Store::open(&dir.join("vault.json")).unwrap();
        let mut b = cred("b", TYPE_BROWSER_PASSWORD, b"y");
        store.create_credential(&mut b).unwrap();
        // 重开后新增的 id 必须大于之前最大 id
        assert!(b.id > a.id);
    }

    #[test]
    fn corrupt_file_is_reported_not_silently_reset() {
        let dir = std::env::temp_dir().join("pm-store-test-corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("vault.json"), b"{ not valid json").unwrap();

        let err = Store::open(&dir.join("vault.json")).unwrap_err();
        assert!(err.contains("已损坏"), "实际: {err}");
    }
}

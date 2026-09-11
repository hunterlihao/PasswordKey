//! SQLite 存储层（对应 Go 的 internal/store/store.go）。
//!
//! 表结构与 Go 版完全一致，因此可以直接打开旧版 `data.db`。

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Local};
use rusqlite::types::ToSql;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Transaction};

use crate::model::{Credential, MasterConfig};
use crate::timefmt::{lock_stamp, now_text, parse_lock_stamp, to_rfc3339};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS master_config (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    salt            BLOB NOT NULL,
    encrypted_verify BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS credentials (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL,
    type        INTEGER NOT NULL DEFAULT 0,
    url         TEXT NOT NULL DEFAULT '',
    username    TEXT NOT NULL DEFAULT '',
    secret      BLOB NOT NULL,
    notes       TEXT NOT NULL DEFAULT '',
    is_favorite INTEGER NOT NULL DEFAULT 0,
    created_at  DATETIME NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at  DATETIME NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS app_state (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    fail_count    INTEGER NOT NULL DEFAULT 0,
    locked_until  TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_credentials_type ON credentials(type);
CREATE INDEX IF NOT EXISTS idx_credentials_name ON credentials(name);
CREATE INDEX IF NOT EXISTS idx_credentials_favorite ON credentials(is_favorite);
"#;

const SELECT_COLUMNS: &str = "id, name, type, url, username, secret, notes, is_favorite, created_at, updated_at";

/// 转义 SQL LIKE 中的通配符（% _ \），配合 ESCAPE '\' 使用
pub fn escape_like(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' | '%' | '_' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// 持久化的暴力破解防护状态
#[derive(Debug, Clone, Default)]
pub struct UnlockState {
    pub fail_count: i32,
    pub locked_until: Option<DateTime<Local>>,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// 打开（必要时创建）数据库并建表。
    /// 连接参数与 Go 版 `?_journal_mode=WAL&_busy_timeout=5000` 等价。
    pub fn open(db_path: &Path) -> Result<Store, String> {
        if let Some(parent) = db_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建数据库目录失败: {e}"))?;
            }
        }

        let conn = Connection::open(db_path)
            .map_err(|e| format!("数据库打开失败: {e}"))?;

        // journal_mode 会返回一行结果，需用 query_row 读取
        conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0))
            .map_err(|e| format!("开启 WAL 模式失败: {e}"))?;
        conn.execute_batch("PRAGMA busy_timeout=5000;")
            .map_err(|e| format!("设置 busy_timeout 失败: {e}"))?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| format!("初始化数据表失败: {e}"))?;

        Ok(Store { conn })
    }

    /// 关闭连接（触发 WAL checkpoint）
    pub fn close(self) -> Result<(), String> {
        self.conn.close().map_err(|(_, e)| format!("关闭数据库失败: {e}"))
    }

    /// 在事务中执行，成功提交、失败回滚
    pub fn with_transaction<T, F>(&mut self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Transaction<'_>) -> Result<T, String>,
    {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("开启事务失败: {e}"))?;
        match f(&tx) {
            Ok(value) => {
                tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
                Ok(value)
            }
            Err(e) => {
                let _ = tx.rollback();
                Err(e)
            }
        }
    }

    // ===== 主密钥配置 =====

    pub fn get_master_config(&self) -> Result<MasterConfig, String> {
        self.conn
            .query_row(
                "SELECT salt, encrypted_verify FROM master_config WHERE id = 1",
                [],
                |row| {
                    Ok(MasterConfig {
                        salt: row.get(0)?,
                        encrypted_verify: row.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(|e| format!("读取主密钥配置失败: {e}"))?
            .ok_or_else(|| "未设置主密钥".to_string())
    }

    pub fn set_master_config(&self, salt: &[u8], encrypted_verify: &[u8]) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO master_config (id, salt, encrypted_verify) VALUES (1, ?, ?)",
                params![salt, encrypted_verify],
            )
            .map_err(|e| format!("保存主密钥配置失败: {e}"))?;
        Ok(())
    }

    // ===== 凭证 CRUD =====

    pub fn create_credential(&mut self, c: &mut Credential) -> Result<(), String> {
        let now = now_text();
        self.conn
            .execute(
                "INSERT INTO credentials (name, type, url, username, secret, notes, is_favorite, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    c.name,
                    c.cred_type,
                    c.url,
                    c.username,
                    c.secret_bytes,
                    c.notes,
                    c.is_favorite,
                    now,
                    now
                ],
            )
            .map_err(|e| format!("新增凭证失败: {e}"))?;

        c.id = self.conn.last_insert_rowid();
        c.created_at = to_rfc3339(&now);
        c.updated_at = to_rfc3339(&now);
        Ok(())
    }

    pub fn get_credential(&self, id: i64) -> Result<Credential, String> {
        let sql = format!("SELECT {SELECT_COLUMNS} FROM credentials WHERE id = ?");
        self.conn
            .query_row(&sql, params![id], map_credential)
            .optional()
            .map_err(|e| format!("查询凭证失败: {e}"))?
            .ok_or_else(|| "凭证不存在".to_string())
    }

    pub fn update_credential(&mut self, c: &mut Credential) -> Result<(), String> {
        let now = now_text();
        let affected = self
            .conn
            .execute(
                "UPDATE credentials SET name=?, type=?, url=?, username=?, secret=?, notes=?, is_favorite=?, updated_at=?
                 WHERE id=?",
                params![
                    c.name,
                    c.cred_type,
                    c.url,
                    c.username,
                    c.secret_bytes,
                    c.notes,
                    c.is_favorite,
                    now,
                    c.id
                ],
            )
            .map_err(|e| format!("更新凭证失败: {e}"))?;

        if affected == 0 {
            return Err("凭证不存在".to_string());
        }
        c.updated_at = to_rfc3339(&now);
        Ok(())
    }

    pub fn delete_credential(&mut self, id: i64) -> Result<(), String> {
        let affected = self
            .conn
            .execute("DELETE FROM credentials WHERE id = ?", params![id])
            .map_err(|e| format!("删除凭证失败: {e}"))?;
        if affected == 0 {
            return Err("凭证不存在".to_string());
        }
        Ok(())
    }

    pub fn delete_credentials(&mut self, ids: &[i64]) -> Result<(), String> {
        if ids.is_empty() {
            return Ok(());
        }
        self.with_transaction(|tx| {
            for id in ids {
                tx.execute("DELETE FROM credentials WHERE id = ?", params![id])
                    .map_err(|e| format!("删除凭证失败: {e}"))?;
            }
            Ok(())
        })
    }

    pub fn toggle_favorite(&mut self, id: i64) -> Result<(), String> {
        let now = now_text();
        let affected = self
            .conn
            .execute(
                "UPDATE credentials SET is_favorite = 1 - is_favorite, updated_at = ? WHERE id = ?",
                params![now, id],
            )
            .map_err(|e| format!("切换收藏失败: {e}"))?;
        if affected == 0 {
            return Err("凭证不存在".to_string());
        }
        Ok(())
    }

    /// 列表查询：按类型过滤 + 关键字模糊匹配（name/url/username）
    pub fn list_credentials(&self, cred_type: Option<i32>, keyword: &str) -> Result<Vec<Credential>, String> {
        let mut sql = format!("SELECT {SELECT_COLUMNS} FROM credentials WHERE 1=1");
        let mut args: Vec<Box<dyn ToSql>> = Vec::new();

        if let Some(t) = cred_type {
            sql.push_str(" AND type = ?");
            args.push(Box::new(t));
        }
        if !keyword.is_empty() {
            sql.push_str(
                r" AND (name LIKE ? ESCAPE '\' OR url LIKE ? ESCAPE '\' OR username LIKE ? ESCAPE '\')",
            );
            let kw = format!("%{}%", escape_like(keyword));
            for _ in 0..3 {
                args.push(Box::new(kw.clone()));
            }
        }
        sql.push_str(" ORDER BY is_favorite DESC, updated_at DESC");

        self.query_credentials(&sql, args)
    }

    /// 收藏列表
    pub fn list_favorites(&self, keyword: &str) -> Result<Vec<Credential>, String> {
        let mut sql = format!("SELECT {SELECT_COLUMNS} FROM credentials WHERE is_favorite = 1");
        let mut args: Vec<Box<dyn ToSql>> = Vec::new();

        if !keyword.is_empty() {
            sql.push_str(
                r" AND (name LIKE ? ESCAPE '\' OR url LIKE ? ESCAPE '\' OR username LIKE ? ESCAPE '\')",
            );
            let kw = format!("%{}%", escape_like(keyword));
            for _ in 0..3 {
                args.push(Box::new(kw.clone()));
            }
        }
        sql.push_str(" ORDER BY updated_at DESC");

        self.query_credentials(&sql, args)
    }

    /// 全量导出用
    pub fn get_all_credentials(&self) -> Result<Vec<Credential>, String> {
        let sql = format!("SELECT {SELECT_COLUMNS} FROM credentials ORDER BY id");
        self.query_credentials(&sql, Vec::new())
    }

    fn query_credentials(&self, sql: &str, args: Vec<Box<dyn ToSql>>) -> Result<Vec<Credential>, String> {
        let mut stmt = self.conn.prepare(sql).map_err(|e| format!("查询凭证失败: {e}"))?;
        let rows = stmt
            .query_map(params_from_iter(args.iter().map(|b| b.as_ref())), map_credential)
            .map_err(|e| format!("查询凭证失败: {e}"))?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| format!("读取凭证失败: {e}"))?);
        }
        Ok(out)
    }

    /// 各类型数量，key -1=全部, -2=收藏
    pub fn count_by_type(&self) -> Result<BTreeMap<i32, i32>, String> {
        let mut counts = BTreeMap::new();

        let total: i32 = self
            .conn
            .query_row("SELECT COUNT(*) FROM credentials", [], |row| row.get(0))
            .map_err(|e| format!("统计失败: {e}"))?;
        counts.insert(-1, total);

        let favorites: i32 = self
            .conn
            .query_row("SELECT COUNT(*) FROM credentials WHERE is_favorite = 1", [], |row| row.get(0))
            .map_err(|e| format!("统计失败: {e}"))?;
        counts.insert(-2, favorites);

        let mut stmt = self
            .conn
            .prepare("SELECT type, COUNT(*) FROM credentials GROUP BY type")
            .map_err(|e| format!("统计失败: {e}"))?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, i32>(0)?, row.get::<_, i32>(1)?)))
            .map_err(|e| format!("统计失败: {e}"))?;
        for row in rows {
            let (t, count) = row.map_err(|e| format!("统计失败: {e}"))?;
            counts.insert(t, count);
        }
        Ok(counts)
    }

    // ===== 解锁失败状态（暴力破解防护）=====

    pub fn get_unlock_state(&self) -> Result<UnlockState, String> {
        let row = self
            .conn
            .query_row(
                "SELECT fail_count, locked_until FROM app_state WHERE id = 1",
                [],
                |row| Ok((row.get::<_, i32>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|e| format!("读取锁定状态失败: {e}"))?;

        let Some((fail_count, locked_str)) = row else {
            return Ok(UnlockState::default());
        };

        Ok(UnlockState {
            fail_count,
            locked_until: parse_lock_stamp(&locked_str),
        })
    }

    pub fn set_unlock_state(&self, state: &UnlockState) -> Result<(), String> {
        let locked = state.locked_until.map(lock_stamp).unwrap_or_default();
        self.conn
            .execute(
                "INSERT OR REPLACE INTO app_state (id, fail_count, locked_until) VALUES (1, ?, ?)",
                params![state.fail_count, locked],
            )
            .map_err(|e| format!("保存锁定状态失败: {e}"))?;
        Ok(())
    }
}

fn map_credential(row: &rusqlite::Row<'_>) -> rusqlite::Result<Credential> {
    let created: String = row.get(8)?;
    let updated: String = row.get(9)?;
    // TEXT/BLOB 都能读成 Vec<u8>，兼容 Go 版写入的 TEXT 密文
    let secret: Vec<u8> = row.get(5)?;

    Ok(Credential {
        id: row.get(0)?,
        name: row.get(1)?,
        cred_type: row.get(2)?,
        url: row.get(3)?,
        username: row.get(4)?,
        secret: String::new(),
        secret_bytes: secret,
        notes: row.get(6)?,
        is_favorite: row.get(7)?,
        created_at: to_rfc3339(&created),
        updated_at: to_rfc3339(&updated),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{TYPE_API_KEY, TYPE_BROWSER_PASSWORD};

    fn temp_store(name: &str) -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("pm-store-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("data.db");
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
    fn open_creates_tables_and_is_idempotent() {
        let (store, dir) = temp_store("schema");
        assert!(store.get_master_config().is_err());
        store.close().unwrap();

        // 二次打开不应报错
        let store = Store::open(&dir.join("data.db")).unwrap();
        store.get_unlock_state().unwrap();
        store.close().unwrap();
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
    fn search_escapes_like_wildcards() {
        let (mut store, _dir) = temp_store("search");
        store.create_credential(&mut cred("100%_safe", TYPE_BROWSER_PASSWORD, b"a")).unwrap();
        store.create_credential(&mut cred("other", TYPE_BROWSER_PASSWORD, b"b")).unwrap();

        assert_eq!(store.list_credentials(None, "100%").unwrap().len(), 1);
        assert_eq!(store.list_credentials(None, "_safe").unwrap().len(), 1);
        // 通配符已被转义，纯 '%' 不应匹配到全部
        assert_eq!(store.list_credentials(None, "zzz").unwrap().len(), 0);
        assert_eq!(store.list_credentials(Some(TYPE_BROWSER_PASSWORD), "").unwrap().len(), 2);
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
    fn transaction_rolls_back_on_error() {
        let (mut store, _dir) = temp_store("tx");
        let mut a = cred("a", TYPE_BROWSER_PASSWORD, b"x");
        store.create_credential(&mut a).unwrap();

        let result: Result<(), String> = store.with_transaction(|tx| {
            tx.execute("DELETE FROM credentials WHERE id = ?", params![a.id]).unwrap();
            Err("故意失败".to_string())
        });
        assert!(result.is_err());
        // 回滚后数据仍在
        assert!(store.get_credential(a.id).is_ok());
    }
}

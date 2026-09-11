//! 数据结构与业务规则（对应 Go 的 internal/model/model.go）

use serde::{Deserialize, Serialize};

/// 凭证类型：0 浏览器密码 1 API Key 2 SSH 密钥 3 安全笔记 4 数据库 5 邮箱 6 银行卡
pub const TYPE_BROWSER_PASSWORD: i32 = 0;
pub const TYPE_API_KEY: i32 = 1;
pub const TYPE_SSH_KEY: i32 = 2;
pub const TYPE_SECURE_NOTE: i32 = 3;
pub const TYPE_DATABASE: i32 = 4;
pub const TYPE_EMAIL: i32 = 5;
pub const TYPE_BANK_CARD: i32 = 6;

pub const MAX_NAME_BYTES: usize = 256;
pub const MAX_SECRET_BYTES: usize = 65536;

/// 凭证。`secret` 字段在数据库层存放的是密文；
/// 列表接口通过 [`CredentialView`] 转换，避免把密文送到前端。
///
/// 所有字段都带 `#[serde(default)]`：前端保存时只提交
/// `{id, name, type, url, username, secret, notes, is_favorite}`，
/// 不包含时间戳，Go 版依赖零值容错，这里用 serde 默认值等价实现。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Credential {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "type")]
    pub cred_type: i32,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub secret: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub is_favorite: i32,
    /// RFC3339 时间串（与 Go 的 time.Time JSON 表现一致）
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    /// 密文载体。密文是任意字节序列，不能塞进 String（会破坏非 UTF-8 字节），
    /// 因此数据库层只用这个字段；`secret` 仅承载前端传入/取走的明文。
    #[serde(skip)]
    pub secret_bytes: Vec<u8>,
}

impl Credential {
    /// 转成不含密钥的前端视图
    pub fn to_view(&self) -> CredentialView {
        CredentialView {
            id: self.id,
            name: self.name.clone(),
            cred_type: self.cred_type,
            url: self.url.clone(),
            username: self.username.clone(),
            notes: self.notes.clone(),
            is_favorite: self.is_favorite,
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
        }
    }
}

/// 前端展示用凭证（不含密文）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CredentialView {
    pub id: i64,
    pub name: String,
    #[serde(rename = "type")]
    pub cred_type: i32,
    pub url: String,
    pub username: String,
    pub notes: String,
    pub is_favorite: i32,
    pub created_at: String,
    pub updated_at: String,
}

/// 主密钥配置
#[derive(Debug, Clone, Default)]
pub struct MasterConfig {
    pub salt: Vec<u8>,
    pub encrypted_verify: Vec<u8>,
}

/// 凭证校验（对应 Go 的 validateCredential）
pub fn validate_credential(c: &Credential) -> Result<(), String> {
    if c.name.is_empty() {
        return Err("名称不能为空".to_string());
    }
    if c.name.len() > MAX_NAME_BYTES {
        return Err("名称过长（最多 256 字符）".to_string());
    }
    if c.secret.len() > MAX_SECRET_BYTES {
        return Err("密钥/密码过长（最多 64KB）".to_string());
    }
    if c.cred_type < TYPE_BROWSER_PASSWORD || c.cred_type > TYPE_BANK_CARD {
        return Err("无效的凭证类型".to_string());
    }
    Ok(())
}

/// 密码强度：0 弱 1 中 2 强
pub const STRENGTH_WEAK: i32 = 0;
pub const STRENGTH_MEDIUM: i32 = 1;
pub const STRENGTH_STRONG: i32 = 2;

/// 基于熵估计的强度判定，与 Go 版 `model.CheckPasswordStrength` 完全一致：
/// 按实际用到的字符集估算 poolSize，再以 bits/字符 × 长度 分级。
///
/// 注意：长度沿用 Go 版的**字节长度**（Go 的 `len(password)`），
/// 以保证与旧版本判定结果一致。
pub fn check_password_strength(password: &str) -> i32 {
    const ENTROPY_WEAK: f64 = 40.0;
    const ENTROPY_MEDIUM: f64 = 60.0;
    const ENTROPY_STRONG: f64 = 80.0;

    let mut has_lower = false;
    let mut has_upper = false;
    let mut has_digit = false;
    let mut has_special_ascii = false;
    let mut has_other = false;

    for ch in password.chars() {
        match ch {
            'a'..='z' => has_lower = true,
            'A'..='Z' => has_upper = true,
            '0'..='9' => has_digit = true,
            '\u{21}'..='\u{7E}' => has_special_ascii = true,
            _ => has_other = true,
        }
    }

    let mut pool_size: u32 = 0;
    if has_lower {
        pool_size += 26;
    }
    if has_upper {
        pool_size += 26;
    }
    if has_digit {
        pool_size += 10;
    }
    if has_special_ascii {
        pool_size += 33;
    }
    if has_other {
        pool_size += 94;
    }

    let length = password.len();
    if pool_size == 0 || length == 0 {
        return STRENGTH_WEAK;
    }

    let entropy = (pool_size as f64).log2() * length as f64;
    if entropy >= ENTROPY_STRONG {
        STRENGTH_STRONG
    } else if entropy >= ENTROPY_MEDIUM {
        STRENGTH_MEDIUM
    } else if entropy >= ENTROPY_WEAK {
        if length < 8 {
            STRENGTH_WEAK
        } else {
            STRENGTH_MEDIUM
        }
    } else {
        STRENGTH_WEAK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_rules() {
        let mut c = Credential {
            name: "GitHub".into(),
            secret: "x".into(),
            cred_type: TYPE_API_KEY,
            ..Default::default()
        };
        assert!(validate_credential(&c).is_ok());

        c.name = String::new();
        assert_eq!(validate_credential(&c).unwrap_err(), "名称不能为空");

        c.name = "a".repeat(MAX_NAME_BYTES + 1);
        assert!(validate_credential(&c).is_err());

        c.name = "ok".into();
        c.cred_type = 99;
        assert_eq!(validate_credential(&c).unwrap_err(), "无效的凭证类型");

        c.cred_type = 0;
        c.secret = "s".repeat(MAX_SECRET_BYTES + 1);
        assert!(validate_credential(&c).is_err());
    }

    #[test]
    fn strength_levels() {
        assert_eq!(check_password_strength(""), STRENGTH_WEAK);
        assert_eq!(check_password_strength("123"), STRENGTH_WEAK);
        // 12 位全小写 = 26^12 ≈ 56 bits -> 中
        assert_eq!(check_password_strength("abcdefghijkl"), STRENGTH_MEDIUM);
        // 20 位混合字符 -> 强
        assert_eq!(check_password_strength("Abcdef123456!@#$xyzq"), STRENGTH_STRONG);
    }

    #[test]
    fn view_drops_secret() {
        let c = Credential {
            secret: "top-secret".into(),
            name: "n".into(),
            ..Default::default()
        };
        let v = serde_json::to_string(&c.to_view()).unwrap();
        assert!(!v.contains("top-secret"));
        assert!(!v.contains("secret"));
    }

    #[test]
    fn credential_json_roundtrip_tolerates_missing_timestamps() {
        let json = r#"{"id":3,"name":"GitHub","type":1,"url":"u","username":"x","secret":"s","notes":"n","is_favorite":0}"#;
        let c: Credential = serde_json::from_str(json).unwrap();
        assert_eq!(c.cred_type, TYPE_API_KEY);
        assert_eq!(c.url, "u");
        assert!(c.created_at.is_empty());
    }
}

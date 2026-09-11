//! 加密原语：Argon2id 密钥派生 + AES-256-GCM 加解密 + 随机密码生成。
//!
//! 所有参数与 Go 版（internal/crypto/crypto.go）逐一对齐，
//! 因此新旧版本可以互相读写同一份 data.db。

use std::fmt;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroize;

/// 盐长度（与 Go 版一致）
pub const SALT_LENGTH: usize = 16;
/// AES-256 密钥长度
pub const KEY_LENGTH: usize = 32;
/// GCM nonce 长度
pub const NONCE_LENGTH: usize = 12;
/// Argon2id 迭代次数
pub const TIME_COST: u32 = 3;
/// Argon2id 内存开销（KiB，即 64MB）
pub const MEMORY_COST_KIB: u32 = 64 * 1024;
/// Argon2id 并行度
pub const PARALLELISM: u32 = 4;
/// 校验令牌长度
pub const VERIFY_TOKEN_LENGTH: usize = 32;

/// 派生后的 AES-256 密钥。使用固定长度数组，`Copy` 语义天然避免共享底层缓冲区。
pub type Key = [u8; KEY_LENGTH];

#[derive(Debug)]
pub enum CryptoError {
    Random(String),
    Derive(String),
    Encrypt(String),
    Decrypt(String),
    CiphertextTooShort,
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CryptoError::Random(e) => write!(f, "随机数生成失败: {e}"),
            CryptoError::Derive(e) => write!(f, "密钥派生失败: {e}"),
            CryptoError::Encrypt(_) => write!(f, "加密失败"),
            CryptoError::Decrypt(_) => write!(f, "解密失败（密文损坏或密钥不匹配）"),
            CryptoError::CiphertextTooShort => write!(f, "密文长度不足"),
        }
    }
}

impl std::error::Error for CryptoError {}

impl From<CryptoError> for String {
    fn from(value: CryptoError) -> Self {
        value.to_string()
    }
}

/// 生成 16 字节随机盐
pub fn generate_salt() -> Result<[u8; SALT_LENGTH], CryptoError> {
    let mut salt = [0u8; SALT_LENGTH];
    getrandom::fill(&mut salt).map_err(|e| CryptoError::Random(e.to_string()))?;
    Ok(salt)
}

/// 生成 32 字节随机校验令牌
pub fn generate_verify_token() -> Result<[u8; VERIFY_TOKEN_LENGTH], CryptoError> {
    let mut token = [0u8; VERIFY_TOKEN_LENGTH];
    getrandom::fill(&mut token).map_err(|e| CryptoError::Random(e.to_string()))?;
    Ok(token)
}

/// Argon2id 派生 32 字节密钥。参数与 Go 版 argon2.IDKey(password, salt, 3, 64*1024, 4, 32) 相同。
pub fn derive_key(password: &str, salt: &[u8]) -> Result<Key, CryptoError> {
    let params = Params::new(MEMORY_COST_KIB, TIME_COST, PARALLELISM, Some(KEY_LENGTH))
        .map_err(|e| CryptoError::Derive(e.to_string()))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; KEY_LENGTH];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| CryptoError::Derive(e.to_string()))?;
    Ok(key)
}

/// AES-256-GCM 加密，输出布局为 `nonce(12) || ciphertext || tag(16)`，与 Go 的 gcm.Seal 布局一致。
pub fn encrypt(key: &Key, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| CryptoError::Encrypt(e.to_string()))?;
    let mut nonce_bytes = [0u8; NONCE_LENGTH];
    getrandom::fill(&mut nonce_bytes).map_err(|e| CryptoError::Random(e.to_string()))?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| CryptoError::Encrypt(e.to_string()))?;

    let mut out = Vec::with_capacity(NONCE_LENGTH + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// AES-256-GCM 解密
pub fn decrypt(key: &Key, ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if ciphertext.len() < NONCE_LENGTH {
        return Err(CryptoError::CiphertextTooShort);
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| CryptoError::Decrypt(e.to_string()))?;
    let (nonce_bytes, body) = ciphertext.split_at(NONCE_LENGTH);
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), body)
        .map_err(|e| CryptoError::Decrypt(e.to_string()))
}

/// 均匀分布的随机下标（拒绝采样，避免取模偏置）
fn random_index(bound: usize) -> Result<usize, CryptoError> {
    if bound <= 1 {
        return Ok(0);
    }
    let bound64 = bound as u64;
    let total = u32::MAX as u64 + 1;
    let zone = total - (total % bound64);
    loop {
        let v = getrandom::u32().map_err(|e| CryptoError::Random(e.to_string()))? as u64;
        if v < zone {
            return Ok((v % bound64) as usize);
        }
    }
}

const CHARSET_LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const CHARSET_UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const CHARSET_DIGITS: &str = "0123456789";
const CHARSET_SPECIAL: &str = "!@#$%^&*()_+-=[]{}|;:,.<>?";

/// 生成随机密码：先按选定字符集填充，再保证每个启用的字符类别至少出现一次，
/// 最后做 Fisher-Yates 洗牌。逻辑与 Go 版逐行一致（含最小长度 4 的钳制）。
pub fn generate_random_password(
    length: usize,
    use_upper: bool,
    use_digit: bool,
    use_special: bool,
) -> Result<String, CryptoError> {
    let mut charset = String::from(CHARSET_LOWER);
    if use_upper {
        charset.push_str(CHARSET_UPPER);
    }
    if use_digit {
        charset.push_str(CHARSET_DIGITS);
    }
    if use_special {
        charset.push_str(CHARSET_SPECIAL);
    }

    let length = if length < 4 { 4 } else { length };
    let charset_bytes = charset.as_bytes();
    let mut password = vec![0u8; length];

    for slot in password.iter_mut() {
        let idx = random_index(charset_bytes.len())?;
        *slot = charset_bytes[idx];
    }

    // 保证每种启用的字符类别至少出现一次
    let mut pos = 0usize;
    let ensure_chars = |pool: &str, password: &mut Vec<u8>, pos: &mut usize| -> Result<(), CryptoError> {
        if *pos >= length {
            return Ok(());
        }
        let bytes = pool.as_bytes();
        let idx = random_index(bytes.len())?;
        password[*pos] = bytes[idx];
        *pos += 1;
        Ok(())
    };

    ensure_chars(CHARSET_LOWER, &mut password, &mut pos)?;
    if use_upper {
        ensure_chars(CHARSET_UPPER, &mut password, &mut pos)?;
    }
    if use_digit {
        ensure_chars(CHARSET_DIGITS, &mut password, &mut pos)?;
    }
    if use_special {
        ensure_chars(CHARSET_SPECIAL, &mut password, &mut pos)?;
    }

    // Fisher-Yates 洗牌
    for i in (1..password.len()).rev() {
        let j = random_index(i + 1)?;
        password.swap(i, j);
    }

    let result = String::from_utf8(password);
    match result {
        Ok(s) => Ok(s),
        Err(e) => {
            let mut bytes = e.into_bytes();
            bytes.zeroize();
            Err(CryptoError::Random("生成的密码不是合法 UTF-8".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = derive_key("correct horse battery staple", &[7u8; SALT_LENGTH]).unwrap();
        let plaintext = "sk-abcdefghijklmnopqrstuvwxyz-中文".as_bytes();
        let ciphertext = encrypt(&key, plaintext).unwrap();
        assert_eq!(ciphertext.len(), NONCE_LENGTH + plaintext.len() + 16);
        assert_eq!(decrypt(&key, &ciphertext).unwrap(), plaintext);
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let salt = [3u8; SALT_LENGTH];
        let key = derive_key("password-a", &salt).unwrap();
        let other = derive_key("password-b", &salt).unwrap();
        let ciphertext = encrypt(&key, b"secret").unwrap();
        assert!(decrypt(&other, &ciphertext).is_err());
    }

    #[test]
    fn decrypt_rejects_short_ciphertext() {
        let key = derive_key("password", &[1u8; SALT_LENGTH]).unwrap();
        assert!(decrypt(&key, &[0u8; 4]).is_err());
    }

    #[test]
    fn derive_key_is_deterministic_and_32_bytes() {
        let salt = [9u8; SALT_LENGTH];
        let a = derive_key("同一个主密钥", &salt).unwrap();
        let b = derive_key("同一个主密钥", &salt).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), KEY_LENGTH);
        assert_ne!(a, derive_key("同一个主密钥!", &salt).unwrap());
    }

    #[test]
    fn random_password_respects_length_and_classes() {
        let pwd = generate_random_password(16, true, true, true).unwrap();
        assert_eq!(pwd.chars().count(), 16);
        assert!(pwd.chars().any(|c| c.is_ascii_lowercase()));
        assert!(pwd.chars().any(|c| c.is_ascii_uppercase()));
        assert!(pwd.chars().any(|c| c.is_ascii_digit()));
        assert!(pwd.chars().any(|c| CHARSET_SPECIAL.contains(c)));

        // 长度钳制
        assert_eq!(generate_random_password(1, false, false, false).unwrap().len(), 4);

        // 关闭的类别不应出现
        let only_lower = generate_random_password(32, false, false, false).unwrap();
        assert!(only_lower.chars().all(|c| c.is_ascii_lowercase()));
    }

    #[test]
    fn random_password_is_not_constant() {
        let a = generate_random_password(24, true, true, true).unwrap();
        let b = generate_random_password(24, true, true, true).unwrap();
        assert_ne!(a, b);
    }
}

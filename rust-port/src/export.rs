//! JSON / Excel 导入导出（对应 Go 的 internal/export/export.go）

use std::path::Path;

use calamine::{open_workbook_auto, Data, Reader};
use rust_xlsxwriter::Workbook;
use serde::{Deserialize, Serialize};

use crate::model::{self, Credential};
use crate::settings::write_private;

/// 导出/导入的中间格式。字段命名与 Go 版 JSON 标签一致。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportCredential {
    pub name: String,
    #[serde(rename = "type")]
    pub cred_type: String,
    pub url: String,
    pub username: String,
    pub secret: String,
    pub notes: String,
}

const HEADERS: [&str; 6] = ["name", "type", "url", "username", "secret", "notes"];

/// 凭证类型 -> 可读字符串
pub fn credential_type_str(t: i32) -> &'static str {
    match t {
        model::TYPE_API_KEY => "api_key",
        model::TYPE_SSH_KEY => "ssh_key",
        model::TYPE_SECURE_NOTE => "secure_note",
        model::TYPE_DATABASE => "database",
        model::TYPE_EMAIL => "email",
        model::TYPE_BANK_CARD => "bank_card",
        _ => "browser",
    }
}

/// 字符串 -> 凭证类型（未知一律按浏览器密码处理）
pub fn parse_credential_type(s: &str) -> i32 {
    match s.trim() {
        "api_key" | "API Key" => model::TYPE_API_KEY,
        "ssh_key" => model::TYPE_SSH_KEY,
        "secure_note" => model::TYPE_SECURE_NOTE,
        "database" => model::TYPE_DATABASE,
        "email" => model::TYPE_EMAIL,
        "bank_card" => model::TYPE_BANK_CARD,
        _ => model::TYPE_BROWSER_PASSWORD,
    }
}

/// 数据库模型 -> 导出模型
pub fn credential_to_export(c: &Credential, secret: &str) -> ExportCredential {
    ExportCredential {
        name: c.name.clone(),
        cred_type: credential_type_str(c.cred_type).to_string(),
        url: c.url.clone(),
        username: c.username.clone(),
        secret: secret.to_string(),
        notes: c.notes.clone(),
    }
}

/// 导出模型 -> 数据库模型（secret 为明文，落库前由调用方加密）
pub fn export_to_credential(e: &ExportCredential) -> Credential {
    Credential {
        name: e.name.clone(),
        cred_type: parse_credential_type(&e.cred_type),
        url: e.url.clone(),
        username: e.username.clone(),
        secret: e.secret.clone(),
        notes: e.notes.clone(),
        ..Default::default()
    }
}

/// 按文件扩展名判断格式
pub fn guess_format(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".xlsx") {
        "excel"
    } else {
        "json"
    }
}

// ===== 导入导出 =====

pub fn export_json(credentials: &[ExportCredential], path: &Path) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(credentials).map_err(|e| format!("生成 JSON 失败: {e}"))?;
    write_private(path, &data)
}

pub fn import_json(path: &Path) -> Result<Vec<ExportCredential>, String> {
    let data = std::fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?;
    serde_json::from_slice(&data).map_err(|e| format!("解析 JSON 失败: {e}"))
}

/// 导出为 xlsx。列顺序：name, type, url, username, secret, notes
pub fn export_excel(credentials: &[ExportCredential], path: &Path) -> Result<(), String> {
    let mut workbook = Workbook::new();
    {
        let sheet = workbook.add_worksheet();
        for (col, title) in HEADERS.iter().enumerate() {
            sheet
                .write_string(0, col as u16, *title)
                .map_err(|e| format!("写入表头失败: {e}"))?;
        }
        for (i, c) in credentials.iter().enumerate() {
            let row = i as u32 + 1;
            let cells = [&c.name, &c.cred_type, &c.url, &c.username, &c.secret, &c.notes];
            for (col, value) in cells.iter().enumerate() {
                sheet
                    .write_string(row, col as u16, sanitize_xlsx_text(value))
                    .map_err(|e| format!("写入第 {} 行失败: {e}", row + 1))?;
            }
        }
    }
    workbook.save(path).map_err(|e| format!("保存 Excel 失败: {e}"))
}

/// 从 xlsx 导入：跳过表头，忽略 name 或 secret 为空的行
pub fn import_excel(path: &Path) -> Result<Vec<ExportCredential>, String> {
    let mut workbook = open_workbook_auto(path).map_err(|e| format!("打开 Excel 失败: {e}"))?;
    let range = workbook
        .worksheet_range_at(0)
        .ok_or_else(|| "Excel 中没有任何工作表".to_string())?
        .map_err(|e| format!("读取工作表失败: {e}"))?;

    let mut out = Vec::new();
    for (index, row) in range.rows().enumerate() {
        if index == 0 {
            continue;
        }
        let cred = ExportCredential {
            name: cell_text(row, 0),
            cred_type: cell_text(row, 1),
            url: cell_text(row, 2),
            username: cell_text(row, 3),
            secret: cell_text(row, 4),
            notes: cell_text(row, 5),
        };
        if !cred.name.is_empty() && !cred.secret.is_empty() {
            out.push(cred);
        }
    }
    Ok(out)
}

/// 模板：与 Go 版内容一致
fn template_rows() -> Vec<ExportCredential> {
    vec![
        ExportCredential {
            name: "GitHub".into(),
            cred_type: "browser".into(),
            url: "https://github.com".into(),
            username: "user@example.com".into(),
            secret: "your_password_here".into(),
            notes: "备注信息".into(),
        },
        ExportCredential {
            name: "OpenAI".into(),
            cred_type: "api_key".into(),
            secret: "sk-xxxxxxxxxxxxxxxxxxxx".into(),
            notes: "API Key 备注".into(),
            ..Default::default()
        },
        ExportCredential {
            name: "生产服务器".into(),
            cred_type: "ssh_key".into(),
            url: "192.168.1.100".into(),
            username: "root".into(),
            secret: "-----BEGIN OPENSSH PRIVATE KEY-----\n...\n-----END OPENSSH PRIVATE KEY-----".into(),
            notes: "端口 22".into(),
        },
        ExportCredential {
            name: "WiFi密码".into(),
            cred_type: "secure_note".into(),
            secret: "MyWiFi@2024".into(),
            notes: "客厅路由器密码".into(),
            ..Default::default()
        },
        ExportCredential {
            name: "MySQL".into(),
            cred_type: "database".into(),
            url: "localhost:3306".into(),
            username: "admin".into(),
            secret: "db_password".into(),
            notes: "生产数据库".into(),
        },
        ExportCredential {
            name: "Gmail".into(),
            cred_type: "email".into(),
            url: "smtp.gmail.com".into(),
            username: "user@gmail.com".into(),
            secret: "app_password".into(),
            notes: "IMAP: imap.gmail.com".into(),
        },
        ExportCredential {
            name: "招商银行".into(),
            cred_type: "bank_card".into(),
            username: "6225 **** **** 1234".into(),
            secret: "***".into(),
            notes: "储蓄卡，持卡人: 张三".into(),
            ..Default::default()
        },
    ]
}

pub fn json_template() -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(&template_rows()).map_err(|e| format!("生成模板失败: {e}"))
}

pub fn excel_template(path: &Path) -> Result<(), String> {
    export_excel(&template_rows(), path)
}

/// XLSX 不允许控制字符，这里把非法字符替换为空格，保留 \t \n \r
fn sanitize_xlsx_text(input: &str) -> String {
    input
        .chars()
        .map(|ch| {
            if (ch as u32) < 0x20 && ch != '\t' && ch != '\n' && ch != '\r' {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

/// 单元格 -> 字符串
fn cell_text(row: &[Data], index: usize) -> String {
    match row.get(index) {
        Some(Data::String(s)) => s.clone(),
        Some(Data::Float(f)) => {
            if f.fract() == 0.0 && f.abs() < 9e15 {
                format!("{}", *f as i64)
            } else {
                f.to_string()
            }
        }
        Some(Data::Int(i)) => i.to_string(),
        Some(Data::Bool(b)) => b.to_string(),
        Some(Data::DateTime(dt)) => dt.to_string(),
        Some(Data::DateTimeIso(s)) | Some(Data::DurationIso(s)) => s.clone(),
        Some(Data::Error(e)) => e.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("pm-export-test");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn sample() -> Vec<ExportCredential> {
        vec![
            ExportCredential {
                name: "GitHub".into(),
                cred_type: "browser".into(),
                url: "https://github.com".into(),
                username: "user@example.com".into(),
                secret: "p@ssw0rd".into(),
                notes: "多行\n备注".into(),
            },
            ExportCredential {
                name: "OpenAI".into(),
                cred_type: "api_key".into(),
                secret: "sk-xxx".into(),
                notes: String::new(),
                ..Default::default()
            },
        ]
    }

    #[test]
    fn json_roundtrip() {
        let path = temp_path("creds.json");
        export_json(&sample(), &path).unwrap();
        let loaded = import_json(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "GitHub");
        assert_eq!(loaded[0].cred_type, "browser");
        assert_eq!(loaded[1].secret, "sk-xxx");
    }

    #[test]
    fn excel_roundtrip() {
        let path = temp_path("creds.xlsx");
        export_excel(&sample(), &path).unwrap();
        let loaded = import_excel(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "GitHub");
        assert_eq!(loaded[0].secret, "p@ssw0rd");
        assert_eq!(loaded[1].cred_type, "api_key");
    }

    #[test]
    fn excel_import_skips_rows_without_name_or_secret() {
        let path = temp_path("skip.xlsx");
        let rows = vec![
            ExportCredential { name: "ok".into(), secret: "s".into(), ..Default::default() },
            ExportCredential { name: "".into(), secret: "s".into(), ..Default::default() },
            ExportCredential { name: "n".into(), secret: "".into(), ..Default::default() },
        ];
        export_excel(&rows, &path).unwrap();
        assert_eq!(import_excel(&path).unwrap().len(), 1);
    }

    #[test]
    fn templates_are_generated() {
        let json = json_template().unwrap();
        let parsed: Vec<ExportCredential> = serde_json::from_slice(&json).unwrap();
        assert_eq!(parsed.len(), 7);

        let path = temp_path("template.xlsx");
        excel_template(&path).unwrap();
        let loaded = import_excel(&path).unwrap();
        assert_eq!(loaded.len(), 7, "模板 7 行的 name/secret 都非空，应全部导入");
        assert_eq!(loaded[6].cred_type, "bank_card");
    }

    #[test]
    fn type_mapping_is_stable() {
        for t in 0..=6 {
            let s = credential_type_str(t);
            assert_eq!(parse_credential_type(s), t, "类型 {t} 映射往返失败");
        }
        assert_eq!(parse_credential_type("API Key"), model::TYPE_API_KEY);
        assert_eq!(parse_credential_type("未知类型"), model::TYPE_BROWSER_PASSWORD);
    }

    #[test]
    fn guess_format_detects_excel() {
        assert_eq!(guess_format("a/b/C.XLSX"), "excel");
        assert_eq!(guess_format("a/b/c.json"), "json");
        assert_eq!(guess_format("noext"), "json");
    }

    #[test]
    fn template_with_long_secret_is_written() {
        let path = temp_path("long.xlsx");
        let rows = vec![ExportCredential {
            name: "long".into(),
            secret: "x".repeat(5000),
            ..Default::default()
        }];
        export_excel(&rows, &path).unwrap();
        assert_eq!(import_excel(&path).unwrap()[0].secret.len(), 5000);
    }

    #[test]
    fn control_chars_are_sanitized() {
        let dirty = "a\u{0}b\tc\nd";
        let clean = sanitize_xlsx_text(dirty);
        assert_eq!(clean, "a b\tc\nd");
    }
}

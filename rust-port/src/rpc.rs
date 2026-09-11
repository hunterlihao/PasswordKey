//! RPC 分发：把前端传来的方法名映射到 [`App`] 的方法。
//!
//! 方法名与 Go 版 `App` 的导出方法一一对应，
//! 前端（wailsjs/go/main/App.js）因此不需要任何改动。

use serde_json::{json, Value};

use crate::app::App;
use crate::dialogs::WindowHost;
use crate::model::Credential;

/// 前端可以调用的方法名清单（用于注入 JS 桥）
pub const METHODS: &[&str] = &[
    "IsMasterSet",
    "SetupMaster",
    "Unlock",
    "Lock",
    "GetCredentials",
    "GetCredentialViewById",
    "GetFavorites",
    "GetCredentialCounts",
    "AddCredential",
    "UpdateCredential",
    "DeleteCredential",
    "DeleteCredentials",
    "ToggleFavorite",
    "GetDecryptedSecret",
    "GetDecryptedSecretDirect",
    "GeneratePassword",
    "CheckPasswordStrength",
    "ChangeMasterPassword",
    "ExportData",
    "ExportSingle",
    "ImportData",
    "DownloadJSONTemplate",
    "DownloadExcelTemplate",
    "GetDbPath",
    "ChangeDbPath",
    "ResetDbPath",
];

/// 执行一次 RPC 调用：成功返回 JSON 结果，失败返回给用户看的错误信息
pub fn dispatch(app: &mut App, host: WindowHost, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        // ===== 主密钥 =====
        "IsMasterSet" => Ok(json!(app.is_master_set())),
        "SetupMaster" => {
            app.setup_master(&arg_string(args, 0)?)?;
            Ok(Value::Null)
        }
        "Unlock" => {
            app.unlock(&arg_string(args, 0)?)?;
            Ok(Value::Null)
        }
        "Lock" => {
            app.lock_key();
            Ok(Value::Null)
        }
        "ChangeMasterPassword" => {
            let old = arg_string(args, 0)?;
            let new = arg_string(args, 1)?;
            app.change_master_password(&old, &new)?;
            Ok(Value::Null)
        }

        // ===== 凭证 =====
        "GetCredentials" => {
            let cred_type = arg_i64(args, 0)? as i32;
            let keyword = arg_string(args, 1)?;
            Ok(json!(app.get_credentials(cred_type, &keyword)))
        }
        "GetFavorites" => {
            let keyword = arg_string(args, 0)?;
            Ok(json!(app.get_favorites(&keyword)))
        }
        "GetCredentialViewById" => {
            let view = app.get_credential_view_by_id(arg_i64(args, 0)?)?;
            Ok(json!(view))
        }
        "GetCredentialCounts" => Ok(json!(app.get_credential_counts())),
        "AddCredential" => {
            let credential = arg_credential(args, 0)?;
            app.add_credential(credential)?;
            Ok(Value::Null)
        }
        "UpdateCredential" => {
            let credential = arg_credential(args, 0)?;
            app.update_credential(credential)?;
            Ok(Value::Null)
        }
        "DeleteCredential" => {
            app.delete_credential(arg_i64(args, 0)?)?;
            Ok(Value::Null)
        }
        "DeleteCredentials" => {
            app.delete_credentials(&arg_i64_list(args, 0)?)?;
            Ok(Value::Null)
        }
        "ToggleFavorite" => {
            app.toggle_favorite(arg_i64(args, 0)?)?;
            Ok(Value::Null)
        }
        "GetDecryptedSecretDirect" => Ok(json!(app.get_decrypted_secret_direct(arg_i64(args, 0)?)?)),
        "GetDecryptedSecret" => {
            let id = arg_i64(args, 0)?;
            let password = arg_string(args, 1)?;
            Ok(json!(app.get_decrypted_secret(id, &password)?))
        }

        // ===== 工具 =====
        "GeneratePassword" => Ok(json!(app.generate_password(arg_i64(args, 0)?)?)),
        "CheckPasswordStrength" => {
            let password = arg_string(args, 0)?;
            Ok(json!(app.check_password_strength(&password)))
        }

        // ===== 导入导出 =====
        "ExportData" => Ok(json!(app.export_data(host, &arg_string(args, 0)?)?)),
        "ExportSingle" => {
            let id = arg_i64(args, 0)?;
            let format = arg_string(args, 1)?;
            Ok(json!(app.export_single(host, id, &format)?))
        }
        "ImportData" => Ok(json!(app.import_data(host)?)),
        "DownloadJSONTemplate" => Ok(json!(app.download_json_template(host)?)),
        "DownloadExcelTemplate" => Ok(json!(app.download_excel_template(host)?)),

        // ===== 数据库路径 =====
        "GetDbPath" => Ok(json!(app.db_path_string())),
        "ChangeDbPath" => Ok(json!(app.change_db_path(host)?)),
        "ResetDbPath" => {
            app.reset_db_path()?;
            Ok(Value::Null)
        }

        other => Err(format!("未知的方法: {other}")),
    }
}

fn arg<'a>(args: &'a [Value], index: usize) -> Result<&'a Value, String> {
    args.get(index).ok_or_else(|| format!("缺少第 {} 个参数", index + 1))
}

/// 字符串参数：null / 缺失 当作空串（与 Go 的零值语义一致）
fn arg_string(args: &[Value], index: usize) -> Result<String, String> {
    match args.get(index) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Ok(other.to_string()),
    }
}

fn arg_i64(args: &[Value], index: usize) -> Result<i64, String> {
    match arg(args, index)? {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .ok_or_else(|| format!("第 {} 个参数不是合法整数", index + 1)),
        Value::String(s) => s
            .trim()
            .parse::<i64>()
            .map_err(|_| format!("第 {} 个参数不是合法整数: {s}", index + 1)),
        other => Err(format!("第 {} 个参数不是合法整数: {other}", index + 1)),
    }
}

fn arg_i64_list(args: &[Value], index: usize) -> Result<Vec<i64>, String> {
    match arg(args, index)? {
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(i, v)| {
                v.as_i64()
                    .ok_or_else(|| format!("第 {} 个参数的第 {} 项不是整数", index + 1, i + 1))
            })
            .collect(),
        other => Err(format!("第 {} 个参数应为数组，实际: {other}", index + 1)),
    }
}

fn arg_credential(args: &[Value], index: usize) -> Result<Credential, String> {
    serde_json::from_value(arg(args, index)?.clone())
        .map_err(|e| format!("凭证参数解析失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model;
    use crate::settings::Settings;
    use crate::store::Store;
    use std::path::PathBuf;

    fn temp_app(name: &str) -> App {
        let dir = std::env::temp_dir().join(format!("pm-rpc-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db_path: PathBuf = dir.join("vault.json");
        let store = Store::open(&db_path).unwrap();
        App::for_test(store, Settings::default(), dir, db_path)
    }

    fn call(app: &mut App, method: &str, args: Value) -> Result<Value, String> {
        let list = match args {
            Value::Array(items) => items,
            other => vec![other],
        };
        dispatch(app, WindowHost::default(), method, &list)
    }

    #[test]
    fn method_list_matches_dispatch() {
        // 需要弹原生文件对话框的方法不在此测试内调用（会阻塞测试进程）
        const DIALOG_METHODS: [&str; 6] = [
            "ExportData",
            "ExportSingle",
            "ImportData",
            "DownloadJSONTemplate",
            "DownloadExcelTemplate",
            "ChangeDbPath",
        ];

        for m in METHODS {
            if DIALOG_METHODS.contains(m) {
                continue;
            }
            // 无参调用要么成功（无参方法），要么报参数错误；唯独不能是“未知的方法”
            if let Err(err) = call(&mut temp_app("unknown-check"), m, json!([])) {
                assert!(!err.contains("未知的方法"), "{m} 未在 dispatch 中实现: {err}");
            }
        }

        // 未知方法必须被拒绝
        let err = call(&mut temp_app("unknown-check"), "NoSuchMethod", json!([])).unwrap_err();
        assert!(err.contains("未知的方法"), "实际: {err}");
    }

    #[test]
    fn full_flow_through_rpc() {
        let mut app = temp_app("flow");

        assert_eq!(call(&mut app, "IsMasterSet", json!([])).unwrap(), json!(false));
        call(&mut app, "SetupMaster", json!(["master"])).unwrap();
        assert_eq!(call(&mut app, "IsMasterSet", json!([])).unwrap(), json!(true));

        call(
            &mut app,
            "AddCredential",
            json!([{
                "id": 0, "name": "GitHub", "type": 0, "url": "https://github.com",
                "username": "u", "secret": "p@ss", "notes": "", "is_favorite": 0
            }]),
        )
        .unwrap();

        let list = call(&mut app, "GetCredentials", json!([-1, ""])).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["name"], "GitHub");
        assert!(list[0].get("secret").is_none(), "列表不应返回密文");

        assert_eq!(
            call(&mut app, "GetDecryptedSecretDirect", json!([1])).unwrap(),
            json!("p@ss")
        );

        let counts = call(&mut app, "GetCredentialCounts", json!([])).unwrap();
        assert_eq!(counts["-1"], 1);

        call(&mut app, "ToggleFavorite", json!([1])).unwrap();
        assert_eq!(
            call(&mut app, "GetFavorites", json!([""])).unwrap().as_array().unwrap().len(),
            1
        );

        assert_eq!(
            call(&mut app, "CheckPasswordStrength", json!(["Abcdef123456!@#$xyzq"])).unwrap(),
            json!(model::STRENGTH_STRONG)
        );
        assert!(call(&mut app, "GeneratePassword", json!([20]))
            .unwrap()
            .as_str()
            .unwrap()
            .chars()
            .count()
            == 20);

        call(&mut app, "DeleteCredentials", json!([[1]])).unwrap();
        assert_eq!(call(&mut app, "GetCredentialCounts", json!([])).unwrap()["-1"], 0);
    }

    #[test]
    fn lock_unlock_over_rpc() {
        let mut app = temp_app("lock");
        call(&mut app, "SetupMaster", json!(["pw"])).unwrap();
        call(&mut app, "Lock", json!([])).unwrap();

        let err = call(&mut app, "DeleteCredential", json!([1])).unwrap_err();
        assert_eq!(err, "未解锁");

        assert_eq!(call(&mut app, "Unlock", json!(["bad"])).unwrap_err(), "主密钥错误");
        call(&mut app, "Unlock", json!(["pw"])).unwrap();
    }

    #[test]
    fn db_path_roundtrip() {
        let mut app = temp_app("dbpath");
        let path = call(&mut app, "GetDbPath", json!([])).unwrap();
        assert!(path.as_str().unwrap().ends_with("vault.json"));
        // 已经是默认路径
        assert_eq!(call(&mut app, "ResetDbPath", json!([])).unwrap_err(), "已经是默认路径");
    }

    #[test]
    fn argument_validation_errors_are_readable() {
        let mut app = temp_app("args");
        assert!(call(&mut app, "GetCredentials", json!([])).unwrap_err().contains("缺少"));
        assert!(call(&mut app, "GetCredentials", json!([{}])).unwrap_err().contains("整数"));
        assert!(call(&mut app, "AddCredential", json!([{"type": "x"}]))
            .unwrap_err()
            .contains("凭证参数解析失败"));
    }
}

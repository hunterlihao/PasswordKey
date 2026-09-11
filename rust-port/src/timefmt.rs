//! 时间格式工具。
//!
//! 数据库里的时间以 TEXT 存放，格式与 Go 版保持一致：
//! - 应用写入：`2026-09-11 11:02:49.123+08:00`（本地时间 + 偏移，Go 的 database/sql 与 SQLite 都能解析）
//! - 建表默认值：`2026-09-11 11:02:49`（SQLite 的 `datetime('now','localtime')`）
//!
//! 读取时统一转成 RFC3339 再交给前端，形状与 Go 的 time.Time JSON 一致。

use chrono::{DateTime, FixedOffset, Local, LocalResult, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone};

/// 当前时间的数据库文本表示
pub fn now_text() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S%.3f%:z").to_string()
}

/// 数据库时间文本 -> 带偏移的时间
pub fn parse_db_time(raw: &str) -> Option<DateTime<FixedOffset>> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt);
    }

    const WITH_OFFSET: [&str; 4] = [
        "%Y-%m-%d %H:%M:%S%.f%:z",
        "%Y-%m-%d %H:%M:%S%:z",
        "%Y-%m-%dT%H:%M:%S%.f%:z",
        "%Y-%m-%dT%H:%M:%S%:z",
    ];
    for fmt in WITH_OFFSET {
        if let Ok(dt) = DateTime::parse_from_str(s, fmt) {
            return Some(dt);
        }
    }

    const NAIVE: [&str; 2] = ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S"];
    for fmt in NAIVE {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, fmt) {
            return localize(naive);
        }
    }

    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return localize(date.and_hms_opt(0, 0, 0)?);
    }

    None
}

fn localize(naive: NaiveDateTime) -> Option<DateTime<FixedOffset>> {
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Some(dt.fixed_offset()),
        LocalResult::Ambiguous(a, _) => Some(a.fixed_offset()),
        LocalResult::None => None,
    }
}

/// 数据库时间文本 -> RFC3339（失败时原样返回，保证前端至少能显示）
pub fn to_rfc3339(raw: &str) -> String {
    match parse_db_time(raw) {
        Some(dt) => dt.to_rfc3339_opts(SecondsFormat::Millis, false),
        None => raw.to_string(),
    }
}

/// 用于持久化「锁定到某时刻」的紧凑格式（与 Go 版 `2006-01-02 15:04:05` 一致）
pub fn lock_stamp(dt: DateTime<Local>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 解析锁定时间戳；按本地时区理解
pub fn parse_lock_stamp(raw: &str) -> Option<DateTime<Local>> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let naive = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()?;
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Some(dt),
        LocalResult::Ambiguous(a, _) => Some(a),
        LocalResult::None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_supported_shapes() {
        assert!(parse_db_time("2026-09-11 11:02:49.123+08:00").is_some());
        assert!(parse_db_time("2026-09-11T11:02:49.123456789+08:00").is_some());
        assert!(parse_db_time("2026-09-11 11:02:49").is_some());
        assert!(parse_db_time("2026-09-11 11:02:49.123456789+08:00").is_some());
        assert!(parse_db_time("2026-09-11").is_some());
        assert!(parse_db_time("").is_none());
        assert!(parse_db_time("不是时间").is_none());
    }

    #[test]
    fn rfc3339_output_is_js_friendly() {
        assert_eq!(
            to_rfc3339("2026-09-11 11:02:49.123+08:00"),
            "2026-09-11T11:02:49.123+08:00"
        );
        // 无偏移的时间按本地时区解释
        assert!(to_rfc3339("2026-09-11 11:02:49").starts_with("2026-09-11T11:02:49"));
        // 无法解析时原样返回
        assert_eq!(to_rfc3339("garbage"), "garbage");
    }

    #[test]
    fn now_text_is_parseable() {
        let text = now_text();
        assert!(parse_db_time(&text).is_some(), "无法解析: {text}");
    }

    #[test]
    fn lock_stamp_roundtrip() {
        let now = Local::now();
        let stamp = lock_stamp(now);
        let parsed = parse_lock_stamp(&stamp).expect("应可解析");
        assert!((parsed - now).num_seconds().abs() < 60);
    }
}

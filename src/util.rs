use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

pub const MAX_TEXT: usize = 200_000;
pub const MAX_TOOL_OUTPUT: usize = 100_000;
pub const MAX_TOOL_INPUT: usize = 20_000;
pub const MAX_TITLE: usize = 160;

pub fn clip(text: &str, limit: usize) -> (String, bool) {
    if text.chars().count() <= limit {
        (text.to_string(), false)
    } else {
        (text.chars().take(limit).collect(), true)
    }
}

pub fn squeeze(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn first_line(text: &str) -> Option<String> {
    let value = squeeze(text);
    if value.is_empty() {
        return None;
    }
    Some(value.chars().take(MAX_TITLE).collect())
}

pub fn is_prompt_text(text: &str) -> bool {
    let trimmed = text.trim_start();
    if !trimmed.starts_with('<') {
        return true;
    }
    ![
        "<task-notification",
        "<system-reminder",
        "<command-name",
        "<local-command",
        "<environment_context",
        "<user_info",
        "<mode_notice",
        "<user_input",
    ]
    .iter()
    .any(|prefix| trimmed.starts_with(prefix))
}

pub fn normalize_ts(value: &Value) -> Option<String> {
    match value {
        Value::Number(number) => {
            let mut seconds = number.as_f64()?;
            if seconds > 10_000_000_000.0 {
                seconds /= 1000.0;
            }
            let dt = Utc
                .timestamp_opt(seconds as i64, ((seconds.fract()) * 1e9) as u32)
                .single()?;
            Some(format_ts(dt))
        }
        Value::String(text) => normalize_ts_str(text),
        _ => None,
    }
}

pub fn normalize_ts_str(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(format_ts(dt.with_timezone(&Utc)));
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(trimmed, format) {
            return Some(format_ts(Utc.from_utc_datetime(&dt)));
        }
        if let Ok(dt) = chrono::NaiveDate::parse_from_str(trimmed, format) {
            return Some(format_ts(Utc.from_utc_datetime(&dt.and_hms_opt(0, 0, 0)?)));
        }
    }
    None
}

pub fn format_ts(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn now_ts() -> String {
    format_ts(Utc::now())
}

pub fn stable_id(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0x1f]);
    }
    let digest = hasher.finalize();
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

pub fn content_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            if text.is_empty() {
                None
            } else {
                Some(text.clone())
            }
        }
        Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                if let Some(text) = content_text(item) {
                    parts.push(text);
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        Value::Object(map) => {
            if let Some(Value::String(text)) = map.get("text") {
                return Some(text.clone());
            }
            if let Some(Value::String(text)) = map.get("thinking") {
                return Some(text.clone());
            }
            if let Some(content) = map.get("content") {
                return content_text(content);
            }
            match map.get("type").and_then(Value::as_str) {
                Some("image") | Some("input_image") | Some("image_url") => Some("[image]".into()),
                _ => None,
            }
        }
        _ => None,
    }
}

pub fn text_blocks(value: &Value) -> (String, String) {
    let mut text = Vec::new();
    let mut thinking = Vec::new();
    if let Value::Array(items) = value {
        for item in items {
            let Some(map) = item.as_object() else {
                continue;
            };
            match map.get("type").and_then(Value::as_str) {
                Some("text") | Some("input_text") | Some("output_text") => {
                    if let Some(Value::String(value)) = map.get("text") {
                        if !value.is_empty() {
                            text.push(value.clone());
                        }
                    }
                }
                Some("thinking") | Some("reasoning") => {
                    if let Some(value) = map.get("thinking").and_then(Value::as_str) {
                        thinking.push(value.to_string());
                    } else if let Some(value) = map.get("text").and_then(Value::as_str) {
                        thinking.push(value.to_string());
                    }
                }
                Some("image") | Some("input_image") | Some("image_url") => {
                    text.push("[image]".into())
                }
                _ => {}
            }
        }
    } else if let Some(value) = value.as_str() {
        text.push(value.to_string());
    }
    (text.join("\n"), thinking.join("\n"))
}

pub fn project_name(cwd: &str) -> Option<String> {
    if cwd.is_empty() {
        return None;
    }
    Path::new(cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToString::to_string)
        .or_else(|| Some(cwd.to_string()))
}

pub fn snippet(text: &str, terms: &[String], limit: usize) -> Option<String> {
    let flat = squeeze(text);
    if flat.is_empty() {
        return None;
    }
    let lower = flat.to_lowercase();
    let mut best: Option<usize> = None;
    for term in terms {
        let needle = term.to_lowercase();
        if needle.is_empty() {
            continue;
        }
        if let Some(index) = lower.find(&needle) {
            if lower.len() == flat.len() {
                best = Some(best.map_or(index, |current| current.min(index)));
            }
        }
    }
    let position = best.unwrap_or(0);
    let chars: Vec<char> = flat.chars().collect();
    let byte_to_char = |byte: usize| flat[..byte].chars().count();
    let center = if lower.len() == flat.len() {
        byte_to_char(position)
    } else {
        0
    };
    let start = center.saturating_sub(limit / 3);
    let end = (start + limit).min(chars.len());
    let mut out: String = chars[start..end].iter().collect();
    if start > 0 {
        out.insert(0, '…');
    }
    if end < chars.len() {
        out.push('…');
    }
    Some(out)
}

pub fn json_string(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

pub fn get_str<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str()
}

pub fn get_i64(value: &Value, path: &[&str]) -> Option<i64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_i64()
}

pub fn get_value<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

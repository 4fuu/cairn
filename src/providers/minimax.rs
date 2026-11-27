use crate::providers::base::{value_text, SessionBuilder, Usage};
use crate::util::{get_i64, get_str};
use anyhow::Result;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub fn discover(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .max_depth(8)
        .into_iter()
        .flatten()
    {
        if entry.file_type().is_file() {
            let name = entry.file_name().to_string_lossy();
            if name.starts_with("messages") && name.ends_with(".jsonl") {
                out.push(entry.path().to_path_buf());
            }
        }
    }
    out
}

pub fn parse(path: &Path) -> Result<Vec<crate::model::ParsedSession>> {
    let folder = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("session")
        .to_string();
    let session_id = folder
        .split_once("-session_")
        .map(|(_, encoded)| decode_id(encoded))
        .unwrap_or(folder.clone());
    let mut builder = SessionBuilder::new("minimax", path);
    let mut model: Option<String> = None;
    for (index, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(message) = obj.get("message") else {
            continue;
        };
        let role = get_str(message, &["role"]).unwrap_or("user");
        let timestamp = message
            .get("timestamp")
            .and_then(|value| crate::util::normalize_ts(value));
        let usage = Usage {
            input: message.get("usage").and_then(|u| get_i64(u, &["input"])),
            output: message.get("usage").and_then(|u| get_i64(u, &["output"])),
            cache_read: message
                .get("usage")
                .and_then(|u| get_i64(u, &["cacheRead"])),
            cache_write: message
                .get("usage")
                .and_then(|u| get_i64(u, &["cacheWrite"])),
        };
        let msg_model = get_str(message, &["model"]).map(ToString::to_string);
        if let Some(value) = &msg_model {
            model = Some(value.clone());
        }
        let base_uuid = get_str(&obj, &["message_id"])
            .map(|id| format!("minimax:{session_id}:{id}"))
            .unwrap_or_else(|| format!("minimax:{session_id}:{index}"));
        let content = message.get("content");
        let text = content.and_then(value_text);
        if let Some(text) = text {
            if !text.trim().is_empty() {
                builder.add_message(
                    Some(&base_uuid),
                    role,
                    Some(text),
                    "text",
                    None,
                    timestamp.as_deref(),
                    false,
                    msg_model.as_deref(),
                    usage,
                    None,
                    None,
                );
            }
        }
        if let Some(Value::Array(calls)) = message.get("tool_calls") {
            for (tool_index, call) in calls.iter().enumerate() {
                builder.add_tool_use(
                    &format!("{base_uuid}:tool:{tool_index}"),
                    None,
                    get_str(call, &["name"]).unwrap_or("tool"),
                    call.get("arguments").unwrap_or(&Value::Null),
                    get_str(call, &["id"]),
                    timestamp.as_deref(),
                    false,
                    None,
                );
            }
        }
    }
    builder.set_id(&session_id);
    builder.session.model = model;
    Ok(vec![builder.finish()])
}

fn decode_id(encoded: &str) -> String {
    use base64_engine::decode;
    decode(encoded).unwrap_or_else(|| encoded.to_string())
}

mod base64_engine {
    pub fn decode(value: &str) -> Option<String> {
        let mut padded = value.replace('-', "+").replace('_', "/");
        while padded.len() % 4 != 0 {
            padded.push('=');
        }
        let bytes = decode_std(&padded)?;
        String::from_utf8(bytes).ok()
    }

    fn decode_std(value: &str) -> Option<Vec<u8>> {
        let table = |byte: u8| -> Option<u8> {
            match byte {
                b'A'..=b'Z' => Some(byte - b'A'),
                b'a'..=b'z' => Some(byte - b'a' + 26),
                b'0'..=b'9' => Some(byte - b'0' + 52),
                b'+' => Some(62),
                b'/' => Some(63),
                _ => None,
            }
        };
        let mut out = Vec::new();
        let mut buffer = 0u32;
        let mut bits = 0;
        for byte in value.bytes() {
            if byte == b'=' {
                break;
            }
            let value = table(byte)?;
            buffer = (buffer << 6) | value as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buffer >> bits) as u8);
                buffer &= (1 << bits) - 1;
            }
        }
        Some(out)
    }
}

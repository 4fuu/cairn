use crate::providers::base::{value_text, SessionBuilder, Usage};
use crate::util::{get_i64, get_str, get_value, json_string, project_name};
use anyhow::{Context, Result};
use regex::Regex;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

pub fn discover(root: &Path) -> Vec<PathBuf> {
    let pattern = Regex::new(r"^session(?:\.v(\d+))?\.jsonl(\.zstd)?$").unwrap();
    let mut by_dir: std::collections::HashMap<PathBuf, (i64, bool, PathBuf)> =
        std::collections::HashMap::new();
    for entry in walkdir::WalkDir::new(root)
        .max_depth(4)
        .into_iter()
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(caps) = pattern.captures(&name) else {
            continue;
        };
        let version = caps
            .get(1)
            .and_then(|m| m.as_str().parse::<i64>().ok())
            .unwrap_or(0);
        let compressed = caps.get(2).is_some();
        let parent = path.parent().unwrap_or(root).to_path_buf();
        let better = match by_dir.get(&parent) {
            None => true,
            Some((best_version, best_compressed, _)) => {
                version > *best_version
                    || (version == *best_version && *best_compressed && !compressed)
            }
        };
        if better {
            by_dir.insert(parent, (version, compressed, path.to_path_buf()));
        }
    }
    by_dir.into_values().map(|(_, _, path)| path).collect()
}

pub fn parse(path: &Path) -> Result<Vec<crate::model::ParsedSession>> {
    let mut builder = SessionBuilder::new("dsh", path);
    let mut session_id = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("session")
        .to_string();
    let mut cwd: Option<String> = None;
    let mut title: Option<String> = None;
    let mut model: Option<String> = None;

    for line in reader(path)?.lines() {
        let line = line?;
        let Ok(obj) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let kind = get_str(&obj, &["type"]).unwrap_or("");
        let timestamp = obj
            .get("time")
            .and_then(|value| crate::util::normalize_ts(value));
        if kind == "session" {
            session_id = get_str(&obj, &["id"]).unwrap_or(&session_id).to_string();
            cwd = get_str(&obj, &["cwd"]).map(ToString::to_string).or(cwd);
            continue;
        }
        let Some(data) = obj.get("data") else {
            continue;
        };
        if kind == "session/title" {
            title = get_str(data, &["title"]).map(ToString::to_string).or(title);
            continue;
        }
        if kind == "user/message" {
            let Some(content) = data.get("content").and_then(value_text) else {
                continue;
            };
            let source_kind = get_str(data, &["source", "kind"]).unwrap_or("user");
            let visible = source_kind == "user" || source_kind == "skill-invocation";
            let uuid = seq_id(&obj, &session_id, "");
            builder.add_message(
                Some(&uuid),
                "user",
                Some(content),
                "text",
                None,
                timestamp.as_deref(),
                !visible,
                None,
                Usage::default(),
                None,
                Some(serde_json::json!({"source": source_kind})),
            );
            continue;
        }
        if kind == "assistant/message" {
            let message = get_value(data, &["message"]).unwrap_or(&Value::Null);
            let content = message.get("content").and_then(value_text);
            let model_name = get_str(message, &["source", "model"]).map(ToString::to_string);
            if let Some(value) = &model_name {
                model = Some(value.clone());
            }
            let usage = Usage {
                input: data.get("usage").and_then(|u| get_i64(u, &["inputTokens"])),
                output: data
                    .get("usage")
                    .and_then(|u| get_i64(u, &["outputTokens"])),
                cache_read: data
                    .get("usage")
                    .and_then(|u| get_i64(u, &["cacheReadTokens"])),
                cache_write: data
                    .get("usage")
                    .and_then(|u| get_i64(u, &["cacheWriteTokens"])),
            };
            let uuid = seq_id(&obj, &session_id, "");
            if let Some(content) = content {
                builder.add_message(
                    Some(&uuid),
                    "assistant",
                    Some(content),
                    "text",
                    None,
                    timestamp.as_deref(),
                    false,
                    model_name.as_deref(),
                    usage,
                    None,
                    None,
                );
            }
            continue;
        }
        if kind == "tool/call" {
            let name = get_str(data, &["name"]).unwrap_or("tool");
            let arguments = data
                .get("arguments")
                .and_then(Value::as_str)
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                .or_else(|| data.get("arguments").cloned())
                .unwrap_or_else(|| Value::Object(Default::default()));
            let uuid = seq_id(&obj, &session_id, ":call");
            builder.add_tool_use(
                &uuid,
                None,
                name,
                &arguments,
                get_str(data, &["id"]).or_else(|| get_str(data, &["callId"])),
                timestamp.as_deref(),
                false,
                None,
            );
            continue;
        }
        if kind == "tool/result" {
            let Some(call_id) =
                get_str(data, &["toolCallId"]).or_else(|| get_str(data, &["callId"]))
            else {
                continue;
            };
            let content = data
                .get("result")
                .map(json_string)
                .or_else(|| data.get("content").and_then(value_text));
            let uuid = seq_id(&obj, &session_id, ":result");
            builder.add_tool_result(
                call_id,
                &uuid,
                None,
                content,
                timestamp.as_deref(),
                data.get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            );
        }
    }

    builder.set_id(&session_id);
    builder.session.title = title;
    builder.session.model = model;
    builder.set_project(cwd.as_deref());
    Ok(vec![builder.finish()])
}

fn seq_id(obj: &Value, session_id: &str, suffix: &str) -> String {
    obj.get("seq")
        .and_then(Value::as_i64)
        .map(|seq| format!("dsh:{session_id}:{seq}{suffix}"))
        .unwrap_or_else(|| format!("dsh:{session_id}:line{suffix}"))
}

fn reader(path: &Path) -> Result<Box<dyn BufRead>> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    if path.extension().and_then(|ext| ext.to_str()) == Some("zstd") {
        let decoder = zstd::stream::read::Decoder::new(file)?;
        Ok(Box::new(BufReader::new(decoder)))
    } else {
        Ok(Box::new(BufReader::new(file)))
    }
}

#[allow(dead_code)]
fn _unused(mut reader: impl Read) -> std::io::Result<usize> {
    let mut buffer = [0u8; 1];
    reader.read(&mut buffer)
}

#[allow(dead_code)]
fn _project(value: Option<&String>) -> Option<String> {
    value.and_then(|value| project_name(value))
}

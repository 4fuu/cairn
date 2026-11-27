use crate::providers::base::{value_text, SessionBuilder, Usage};
use crate::util::{get_i64, get_str, get_value};
use anyhow::Result;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub fn discover(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .max_depth(5)
        .into_iter()
        .flatten()
    {
        if entry.file_type().is_file() && entry.file_name() == "session.jsonl" {
            out.push(entry.path().to_path_buf());
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
    let mut builder = SessionBuilder::new("liveagent", path);
    let mut session_id = folder.clone();
    let mut title: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut model: Option<String> = None;
    let mut last_assistant: Option<String> = None;

    for (index, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let kind = get_str(&obj, &["type"]).unwrap_or("");
        let payload = obj.get("payload").unwrap_or(&Value::Null);
        if kind == "session_meta" {
            session_id = get_str(payload, &["id"]).unwrap_or(&folder).to_string();
            title = get_str(payload, &["title"])
                .map(ToString::to_string)
                .or(title);
            cwd = get_str(payload, &["cwd"]).map(ToString::to_string).or(cwd);
            model = get_str(payload, &["model"])
                .map(ToString::to_string)
                .or(model);
            continue;
        }
        if kind != "message" {
            continue;
        }
        let Some(message) = get_value(payload, &["message"]) else {
            continue;
        };
        let role = get_str(message, &["role"]).unwrap_or("user");
        let timestamp = get_str(message, &["sent_at"]).or_else(|| get_str(payload, &["sent_at"]));
        let base_uuid = get_str(message, &["id"])
            .map(|id| format!("liveagent:{session_id}:{id}"))
            .unwrap_or_else(|| format!("liveagent:{session_id}:{index}"));
        let text = message.get("content").and_then(value_text);
        if role == "tool" {
            let Some(call_id) = get_str(message, &["tool_call_id"]) else {
                continue;
            };
            builder.add_tool_result(call_id, &base_uuid, None, text, timestamp, false);
            continue;
        }
        if let Some(text) = text {
            if !text.trim().is_empty() {
                let uuid = builder.add_message(
                    Some(&base_uuid),
                    role,
                    Some(text),
                    "text",
                    None,
                    timestamp,
                    false,
                    model.as_deref(),
                    Usage::default(),
                    None,
                    None,
                );
                if role == "assistant" {
                    last_assistant = Some(uuid);
                }
            }
        }
        if let Some(Value::Array(calls)) = message.get("tool_calls") {
            for (tool_index, call) in calls.iter().enumerate() {
                builder.add_tool_use(
                    &format!("{base_uuid}:tool:{tool_index}"),
                    None,
                    get_str(call, &["function", "name"])
                        .or_else(|| get_str(call, &["name"]))
                        .unwrap_or("tool"),
                    call.get("function")
                        .and_then(|f| f.get("arguments"))
                        .or_else(|| call.get("arguments"))
                        .unwrap_or(&Value::Null),
                    get_str(call, &["id"]),
                    timestamp,
                    false,
                    None,
                );
            }
        }
    }

    if let Some(uuid) = last_assistant.as_deref() {
        // The meta line of a K-brain session holds the running totals.
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Some(first) = text.lines().next() {
                if let Ok(meta) = serde_json::from_str::<Value>(first) {
                    let payload = meta.get("payload").unwrap_or(&Value::Null);
                    builder.set_last_usage(
                        uuid,
                        Usage {
                            input: get_i64(payload, &["usage_in"]),
                            output: get_i64(payload, &["usage_out"]),
                            cache_read: get_i64(payload, &["usage_cached"]),
                            cache_write: get_i64(payload, &["usage_cache_write"]),
                        },
                    );
                }
            }
        }
    }

    builder.set_id(&session_id);
    builder.session.title = title;
    builder.session.model = model;
    builder.set_project(cwd.as_deref());
    Ok(vec![builder.finish()])
}

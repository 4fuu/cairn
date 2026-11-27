use crate::providers::base::{value_text, SessionBuilder, Usage};
use crate::util::{get_i64, get_str, get_value};
use anyhow::Result;
use serde_json::Value;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub fn discover(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .max_depth(3)
        .into_iter()
        .flatten()
    {
        if entry.file_type().is_file() && entry.file_name() == "log.jsonl" {
            out.push(entry.path().to_path_buf());
        }
    }
    out
}

pub fn parse(path: &Path) -> Result<Vec<crate::model::ParsedSession>> {
    let dir = path.parent().unwrap_or(path);
    let folder = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("session")
        .to_string();
    let mut builder = SessionBuilder::new("devin", path);
    let mut session_id = folder.clone();
    let mut title: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut model: Option<String> = None;

    if let Ok(text) = std::fs::read_to_string(dir.join("meta.json")) {
        if let Ok(meta) = serde_json::from_str::<Value>(&text) {
            session_id = get_str(&meta, &["id"]).unwrap_or(&folder).to_string();
            title = get_str(&meta, &["title"]).map(ToString::to_string);
            cwd = get_str(&meta, &["working_directory"]).map(ToString::to_string);
            model = get_str(&meta, &["model"]).map(ToString::to_string);
        }
    }

    let mut seen = HashSet::new();
    for (index, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(node) = get_value(&obj, &["node"]) else {
            continue;
        };
        let Some(message) = get_value(node, &["message"]) else {
            continue;
        };
        let message_id = get_str(message, &["message_id"])
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("devin:{session_id}:{index}"));
        if !seen.insert(message_id.clone()) {
            continue;
        }
        let role = get_str(message, &["role"]).unwrap_or("user");
        let metadata = message.get("metadata").unwrap_or(&Value::Null);
        let timestamp = get_str(metadata, &["created_at"]);
        let metrics = metadata.get("metrics").unwrap_or(&Value::Null);
        let usage = Usage {
            input: get_i64(metrics, &["input_tokens"]),
            output: get_i64(metrics, &["output_tokens"]),
            cache_read: get_i64(metrics, &["cache_read_tokens"]),
            cache_write: get_i64(metrics, &["cache_creation_tokens"]),
        };
        let msg_model = get_str(metadata, &["generation_model"]).map(ToString::to_string);
        if let Some(value) = &msg_model {
            model = Some(value.clone());
        }
        let text = message.get("content").and_then(value_text);
        if role == "tool" {
            let Some(call_id) =
                get_str(message, &["tool_call_id"]).or_else(|| get_str(message, &["toolCallId"]))
            else {
                continue;
            };
            builder.add_tool_result(call_id, &message_id, None, text, timestamp, false);
            continue;
        }
        if let Some(text) = text {
            if !text.trim().is_empty() {
                builder.add_message(
                    Some(&message_id),
                    role,
                    Some(text),
                    "text",
                    None,
                    timestamp,
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
                    &format!("{message_id}:tool:{tool_index}"),
                    None,
                    get_str(call, &["name"]).unwrap_or("tool"),
                    call.get("arguments").unwrap_or(&Value::Null),
                    get_str(call, &["id"]),
                    timestamp,
                    false,
                    None,
                );
            }
        }
    }

    builder.set_id(&session_id);
    builder.session.title = title;
    builder.session.model = model;
    builder.set_project(cwd.as_deref());
    Ok(vec![builder.finish()])
}

use crate::providers::base::{value_text, SessionBuilder, Usage};
use crate::util::{get_i64, get_str, get_value, project_name};
use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn discover(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .max_depth(3)
        .into_iter()
        .flatten()
    {
        if entry.file_type().is_file()
            && entry
                .file_name()
                .to_string_lossy()
                .ends_with(".messages.json")
        {
            files.push(entry.path().to_path_buf());
        }
    }
    files
}

pub fn parse(path: &Path) -> Result<Vec<crate::model::ParsedSession>> {
    let folder = path.parent().unwrap_or(path);
    let folder_name = folder
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("session");
    let is_main = path.file_name().and_then(|name| name.to_str())
        == Some(&format!("{folder_name}.messages.json"));
    let mut builder = SessionBuilder::new("cline", path);
    let mut title: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut model: Option<String> = None;

    if is_main {
        let manifest = folder.join(format!("{folder_name}.json"));
        if let Ok(text) = std::fs::read_to_string(&manifest) {
            if let Ok(obj) = serde_json::from_str::<Value>(&text) {
                title = get_str(&obj, &["metadata", "title"]).map(ToString::to_string);
                cwd = get_str(&obj, &["cwd"]).map(ToString::to_string);
                model = get_str(&obj, &["model"]).map(ToString::to_string);
            }
        }
    }

    let text = std::fs::read_to_string(path)?;
    let obj: Value = serde_json::from_str(&text)?;
    if let Some(Value::Array(messages)) = obj.get("messages") {
        for (index, message) in messages.iter().enumerate() {
            let role = get_str(message, &["role"]).unwrap_or("user");
            let timestamp = message
                .get("ts")
                .and_then(|value| crate::util::normalize_ts(value));
            let msg_model = get_str(message, &["modelInfo", "id"]).map(ToString::to_string);
            if let Some(value) = &msg_model {
                model = Some(value.clone());
            }
            let usage = Usage {
                input: message
                    .get("metrics")
                    .and_then(|m| get_i64(m, &["inputTokens"])),
                output: message
                    .get("metrics")
                    .and_then(|m| get_i64(m, &["outputTokens"])),
                cache_read: message
                    .get("metrics")
                    .and_then(|m| get_i64(m, &["cacheReadTokens"])),
                cache_write: message
                    .get("metrics")
                    .and_then(|m| get_i64(m, &["cacheWriteTokens"])),
            };
            let content = message.get("content");
            if let Some(Value::String(text)) = content {
                if !text.trim().is_empty() {
                    builder.add_message(
                        Some(&format!("cline:{folder_name}:{index}")),
                        role,
                        Some(text.clone()),
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
                continue;
            }
            let Some(blocks) = content.and_then(Value::as_array) else {
                continue;
            };
            let mut texts = Vec::new();
            let mut tools = Vec::new();
            let mut results = Vec::new();
            for block in blocks {
                match get_str(block, &["type"]) {
                    Some("text") => {
                        if let Some(text) = get_str(block, &["text"]) {
                            texts.push(text.to_string());
                        }
                    }
                    Some("tool_use") => tools.push(block),
                    Some("tool_result") => results.push(block),
                    Some("image") => texts.push("[image]".into()),
                    _ => {}
                }
            }
            let text = texts.join("\n");
            if !text.trim().is_empty() {
                builder.add_message(
                    Some(&format!("cline:{folder_name}:{index}")),
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
            for (tool_index, block) in tools.iter().enumerate() {
                builder.add_tool_use(
                    &format!("cline:{folder_name}:{index}:tool:{tool_index}"),
                    None,
                    get_str(block, &["name"]).unwrap_or("tool"),
                    block.get("input").unwrap_or(&Value::Null),
                    get_str(block, &["id"]),
                    timestamp.as_deref(),
                    false,
                    None,
                );
            }
            for (result_index, block) in results.iter().enumerate() {
                let Some(call_id) = get_str(block, &["tool_use_id"]) else {
                    continue;
                };
                builder.add_tool_result(
                    call_id,
                    &format!("cline:{folder_name}:{index}:result:{result_index}"),
                    None,
                    block.get("content").and_then(value_text),
                    timestamp.as_deref(),
                    block
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                );
            }
        }
    }

    builder.set_id(folder_name);
    builder.session.title = title;
    builder.session.model = model;
    builder.set_project(cwd.as_deref());
    Ok(vec![builder.finish()])
}

#[allow(dead_code)]
fn _unused(value: &Value) -> Option<String> {
    get_value(value, &["messages"]).and_then(value_text)
}

#[allow(dead_code)]
fn _project(value: Option<&String>) -> Option<String> {
    value.and_then(|value| project_name(value))
}

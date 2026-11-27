use crate::providers::base::{value_text, SessionBuilder, Usage};
use crate::util::{get_i64, get_str, get_value, normalize_ts, project_name};
use anyhow::Result;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub fn discover(root: &Path) -> Vec<std::path::PathBuf> {
    crate::providers::walk_jsonl(root)
}

pub fn parse(path: &Path, source: &str) -> Result<Vec<crate::model::ParsedSession>> {
    let mut builder = SessionBuilder::new(source, path);
    let mut session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session")
        .to_string();
    let mut title: Option<String> = None;
    let mut summary: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut model: Option<String> = None;
    let mut provider: Option<String> = None;

    for (index, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(&line) else {
            // omp writes a fixed-width title line before the JSON header.
            continue;
        };
        let kind = get_str(&obj, &["type"]).unwrap_or("");
        if kind == "session" {
            if let Some(value) = get_str(&obj, &["id"]) {
                session_id = value.to_string();
            }
            cwd = get_str(&obj, &["cwd"]).map(ToString::to_string).or(cwd);
            model = get_str(&obj, &["modelId"])
                .map(ToString::to_string)
                .or(model);
            provider = get_str(&obj, &["provider"])
                .map(ToString::to_string)
                .or(provider);
            title = get_str(&obj, &["title"]).map(ToString::to_string).or(title);
            continue;
        }
        if kind == "model_change" {
            model = get_str(&obj, &["modelId"])
                .map(ToString::to_string)
                .or(model);
            provider = get_str(&obj, &["provider"])
                .map(ToString::to_string)
                .or(provider);
            continue;
        }
        if kind == "compaction" || kind == "branch_summary" {
            if let Some(value) = get_str(&obj, &["summary"]) {
                summary = Some(value.to_string());
                builder.add_message(
                    Some(&format!("pi:{session_id}:{}", index + 1)),
                    "system",
                    Some(value.to_string()),
                    "summary",
                    get_str(&obj, &["parentId"]),
                    get_str(&obj, &["timestamp"]),
                    true,
                    model.as_deref(),
                    Usage::default(),
                    None,
                    None,
                );
            }
            continue;
        }
        if kind != "message" {
            continue;
        }
        let Some(message) = get_value(&obj, &["message"]) else {
            continue;
        };
        let entry_id = get_str(&obj, &["id"])
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("pi:{session_id}:{}", index + 1));
        let parent = get_str(&obj, &["parentId"]);
        let timestamp = get_str(&obj, &["timestamp"]).or_else(|| get_str(message, &["timestamp"]));
        let role = get_str(message, &["role"]).unwrap_or("user");

        if role == "user" {
            if let Some(text) = message.get("content").and_then(value_text) {
                if !text.trim().is_empty() {
                    builder.add_message(
                        Some(&entry_id),
                        "user",
                        Some(text),
                        "text",
                        parent,
                        timestamp,
                        false,
                        model.as_deref(),
                        Usage::default(),
                        None,
                        None,
                    );
                }
            }
            continue;
        }
        if role == "assistant" {
            let content = message.get("content");
            let mut texts = Vec::new();
            let mut thinking = Vec::new();
            let mut tools = Vec::new();
            if let Some(text) = content.and_then(Value::as_str) {
                texts.push(text.to_string());
            }
            if let Some(blocks) = content.and_then(Value::as_array) {
                for block in blocks {
                    match get_str(block, &["type"]) {
                        Some("text") => {
                            if let Some(text) = get_str(block, &["text"]) {
                                texts.push(text.to_string());
                            }
                        }
                        Some("thinking") => {
                            if let Some(text) =
                                get_str(block, &["thinking"]).or_else(|| get_str(block, &["text"]))
                            {
                                thinking.push(text.to_string());
                            }
                        }
                        Some("toolCall") | Some("tool_call") => tools.push(block),
                        Some("image") | Some("image_url") => texts.push("[image]".into()),
                        _ => {}
                    }
                }
            }
            let usage = pi_usage(message);
            let model_name = get_str(message, &["model"])
                .map(ToString::to_string)
                .or(model.clone());
            let text = texts.join("\n");
            if !text.trim().is_empty() {
                builder.add_message(
                    Some(&entry_id),
                    "assistant",
                    Some(text),
                    "text",
                    parent,
                    timestamp,
                    false,
                    model_name.as_deref(),
                    usage,
                    None,
                    None,
                );
            }
            if !thinking.is_empty() {
                builder.add_message(
                    Some(&format!("{entry_id}:thinking")),
                    "assistant",
                    Some(thinking.join("\n")),
                    "thinking",
                    parent,
                    timestamp,
                    true,
                    model_name.as_deref(),
                    Usage::default(),
                    None,
                    None,
                );
            }
            for (tool_index, block) in tools.iter().enumerate() {
                let name = get_str(block, &["name"]).unwrap_or("tool");
                let arguments = block
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Default::default()));
                builder.add_tool_use(
                    &format!("{entry_id}:tool:{tool_index}"),
                    parent,
                    name,
                    &arguments,
                    get_str(block, &["id"]),
                    timestamp,
                    false,
                    None,
                );
            }
            continue;
        }
        if role == "toolResult" || role == "tool_result" {
            let Some(call_id) =
                get_str(message, &["toolCallId"]).or_else(|| get_str(message, &["tool_call_id"]))
            else {
                continue;
            };
            let content = message.get("content").and_then(value_text);
            builder.add_tool_result(
                call_id,
                &entry_id,
                parent,
                content,
                timestamp,
                message
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            );
            continue;
        }
        if role == "bashExecution" {
            let command = get_str(message, &["command"]).unwrap_or("");
            let output = get_str(message, &["output"]).unwrap_or("");
            let text = format!("$ {command}\n{output}").trim().to_string();
            builder.add_message(
                Some(&entry_id),
                "tool",
                Some(text),
                "bash",
                parent,
                timestamp,
                true,
                model.as_deref(),
                Usage::default(),
                None,
                Some(serde_json::json!({
                    "command": command,
                    "exit_code": message.get("exitCode"),
                    "cancelled": message.get("cancelled"),
                })),
            );
            continue;
        }
        if role == "compactionSummary" || role == "branchSummary" {
            if let Some(value) = get_str(message, &["summary"]) {
                summary = Some(value.to_string());
                builder.add_message(
                    Some(&entry_id),
                    "system",
                    Some(value.to_string()),
                    "summary",
                    parent,
                    timestamp,
                    true,
                    model.as_deref(),
                    Usage::default(),
                    None,
                    None,
                );
            }
        }
    }

    builder.set_id(&session_id);
    builder.session.title = title;
    builder.session.summary = summary;
    builder.session.model = model;
    builder.set_project(cwd.as_deref());
    builder.session.meta_json =
        provider.map(|provider| serde_json::json!({"provider": provider}).to_string());
    Ok(vec![builder.finish()])
}

fn pi_usage(message: &Value) -> Usage {
    let usage = message.get("usage");
    Usage {
        input: usage.and_then(|u| get_i64(u, &["input"]).or_else(|| get_i64(u, &["input_tokens"]))),
        output: usage
            .and_then(|u| get_i64(u, &["output"]).or_else(|| get_i64(u, &["output_tokens"]))),
        cache_read: usage.and_then(|u| {
            get_i64(u, &["cacheRead"]).or_else(|| get_i64(u, &["cache_read_tokens"]))
        }),
        cache_write: usage.and_then(|u| {
            get_i64(u, &["cacheWrite"]).or_else(|| get_i64(u, &["cache_write_tokens"]))
        }),
    }
}

#[allow(dead_code)]
fn _unused(value: &Value) -> Option<String> {
    normalize_ts(value)
}

#[allow(dead_code)]
fn _project(value: Option<&String>) -> Option<String> {
    value.and_then(|value| project_name(value))
}

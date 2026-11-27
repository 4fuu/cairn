use crate::providers::base::{value_text, SessionBuilder, Usage};
use crate::util::{get_i64, get_str, get_value};
use anyhow::Result;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub fn discover(root: &Path) -> Vec<PathBuf> {
    crate::providers::walk_jsonl(root)
}

pub fn parse(path: &Path) -> Result<Vec<crate::model::ParsedSession>> {
    let mut builder = SessionBuilder::new("droid", path);
    let mut session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session")
        .to_string();
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
        if kind == "session_start" {
            session_id = get_str(&obj, &["id"]).unwrap_or(&session_id).to_string();
            title = get_str(&obj, &["title"]).map(ToString::to_string).or(title);
            cwd = get_str(&obj, &["cwd"]).map(ToString::to_string).or(cwd);
            continue;
        }
        if kind != "message" {
            continue;
        }
        let Some(message) = get_value(&obj, &["message"]) else {
            continue;
        };
        let role = get_str(message, &["role"]).unwrap_or("user");
        let timestamp = get_str(&obj, &["timestamp"]).or_else(|| get_str(message, &["timestamp"]));
        let base_uuid = get_str(&obj, &["id"])
            .or_else(|| get_str(message, &["id"]))
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("droid:{session_id}:{index}"));
        let content = message.get("content");
        if let Some(Value::String(text)) = content {
            let uuid = builder.add_message(
                Some(&base_uuid),
                role,
                Some(text.clone()),
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
            continue;
        }
        let Some(blocks) = content.and_then(Value::as_array) else {
            continue;
        };
        let mut texts = Vec::new();
        let mut thinking = Vec::new();
        let mut tools = Vec::new();
        let mut results = Vec::new();
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
                Some("tool_use") => tools.push(block),
                Some("tool_result") => results.push(block),
                Some("image") => texts.push("[image]".into()),
                _ => {}
            }
        }
        let text = texts.join("\n");
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
        if !thinking.is_empty() {
            builder.add_message(
                Some(&format!("{base_uuid}:thinking")),
                "assistant",
                Some(thinking.join("\n")),
                "thinking",
                None,
                timestamp,
                true,
                model.as_deref(),
                Usage::default(),
                None,
                None,
            );
        }
        for (tool_index, block) in tools.iter().enumerate() {
            builder.add_tool_use(
                &format!("{base_uuid}:tool:{tool_index}"),
                None,
                get_str(block, &["name"]).unwrap_or("tool"),
                block.get("input").unwrap_or(&Value::Null),
                get_str(block, &["id"]),
                timestamp,
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
                &format!("{base_uuid}:result:{result_index}"),
                None,
                block.get("content").and_then(value_text),
                timestamp,
                block
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            );
        }
    }

    // The transcript's settings hold one running token total for the model.
    let settings_path = path.with_extension("settings.json");
    if let Ok(text) = std::fs::read_to_string(&settings_path) {
        if let Ok(settings) = serde_json::from_str::<Value>(&text) {
            if let Some(value) = get_str(&settings, &["model"]) {
                model = Some(value.to_string());
            }
            if let Some(uuid) = last_assistant.as_deref() {
                let usage = settings.get("tokenUsage").unwrap_or(&Value::Null);
                builder.set_last_usage(
                    uuid,
                    Usage {
                        input: get_i64(usage, &["input"]),
                        output: get_i64(usage, &["output"]),
                        cache_read: get_i64(usage, &["cacheRead"]),
                        cache_write: get_i64(usage, &["cacheWrite"]),
                    },
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

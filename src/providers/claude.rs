use crate::providers::base::{value_text, SessionBuilder, Usage};
use crate::util::{get_i64, get_str, get_value, json_string, project_name};
use anyhow::Result;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub fn discover(root: &Path) -> Vec<std::path::PathBuf> {
    crate::providers::walk_jsonl(root)
}

pub fn parse_with_source(path: &Path, source: &str) -> Result<Vec<crate::model::ParsedSession>> {
    let mut builder = SessionBuilder::new(source, path);
    let file = File::open(path)?;
    let mut session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session")
        .to_string();
    let mut title: Option<String> = None;
    let mut summary: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut version: Option<String> = None;
    let mut model: Option<String> = None;

    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(&line) else {
            builder.warn(format!("line {}: invalid JSON", index + 1));
            continue;
        };
        if let Some(id) = get_str(&obj, &["sessionId"]) {
            session_id = id.to_string();
        }
        match get_str(&obj, &["type"]) {
            Some("custom-title") => {
                title = get_str(&obj, &["customTitle"])
                    .map(ToString::to_string)
                    .or(title);
                continue;
            }
            Some("ai-title") => {
                title = get_str(&obj, &["aiTitle"])
                    .map(ToString::to_string)
                    .or(title);
                continue;
            }
            Some("summary") => {
                summary = get_str(&obj, &["summary"])
                    .map(ToString::to_string)
                    .or(summary);
                continue;
            }
            Some("user") | Some("assistant") | Some("system") => {}
            _ => continue,
        }
        cwd = get_str(&obj, &["cwd"]).map(ToString::to_string).or(cwd);
        branch = get_str(&obj, &["gitBranch"])
            .map(ToString::to_string)
            .or(branch);
        version = get_str(&obj, &["version"])
            .map(ToString::to_string)
            .or(version);
        let message = get_value(&obj, &["message"]);
        let Some(message) = message else { continue };
        let role = get_str(message, &["role"]).unwrap_or("assistant");
        let timestamp = get_str(&obj, &["timestamp"]);
        let base_uuid = get_str(&obj, &["uuid"])
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("{session_id}:{}", index + 1));
        let parent = get_str(&obj, &["parentUuid"]);
        let agent_id = get_str(&obj, &["agentId"]);
        let is_meta =
            obj.get("isMeta").and_then(Value::as_bool).unwrap_or(false) || role == "system";
        let msg_model = get_str(message, &["model"]);
        if let Some(value) = msg_model {
            model = Some(value.to_string());
        }
        let usage = usage_of(message);
        let content = message.get("content");

        if let Some(Value::String(text)) = content {
            if !text.trim().is_empty() {
                builder.add_message(
                    Some(&base_uuid),
                    role,
                    Some(text.clone()),
                    "text",
                    parent,
                    timestamp,
                    is_meta,
                    msg_model,
                    usage,
                    agent_id,
                    None,
                );
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
                Some("image") | Some("input_image") => texts.push("[image]".into()),
                _ => {}
            }
        }
        let text = texts.join("\n");
        if !text.trim().is_empty() {
            builder.add_message(
                Some(&base_uuid),
                role,
                Some(text),
                "text",
                parent,
                timestamp,
                is_meta,
                msg_model,
                usage,
                agent_id,
                None,
            );
        }
        if !thinking.is_empty() {
            builder.add_message(
                Some(&format!("{base_uuid}:thinking")),
                "assistant",
                Some(thinking.join("\n")),
                "thinking",
                parent,
                timestamp,
                true,
                msg_model,
                Usage::default(),
                agent_id,
                None,
            );
        }
        for (tool_index, block) in tools.iter().enumerate() {
            let name = get_str(block, &["name"]).unwrap_or("tool");
            let arguments = block.get("input").cloned().unwrap_or(Value::Null);
            builder.add_tool_use(
                &format!("{base_uuid}:tool:{tool_index}"),
                parent,
                name,
                &arguments,
                get_str(block, &["id"]),
                timestamp,
                is_meta,
                agent_id,
            );
        }
        for (result_index, block) in results.iter().enumerate() {
            let Some(call_id) = get_str(block, &["tool_use_id"]) else {
                continue;
            };
            let content = block.get("content").and_then(value_text);
            builder.add_tool_result(
                call_id,
                &format!("{base_uuid}:result:{result_index}"),
                parent,
                content,
                timestamp,
                block
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            );
        }
    }

    builder.set_id(&session_id);
    builder.session.title = title;
    builder.session.summary = summary;
    builder.session.git_branch = branch;
    builder.session.version = version;
    builder.session.model = model;
    builder.set_project(cwd.as_deref());
    builder.session.meta_json = Some(
        serde_json::json!({
            "source": source,
        })
        .to_string(),
    );
    Ok(vec![builder.finish()])
}

fn usage_of(message: &Value) -> Usage {
    let usage = message.get("usage");
    Usage {
        input: usage.and_then(|u| get_i64(u, &["input_tokens"])),
        output: usage.and_then(|u| get_i64(u, &["output_tokens"])),
        cache_read: usage.and_then(|u| get_i64(u, &["cache_read_input_tokens"])),
        cache_write: usage.and_then(|u| get_i64(u, &["cache_creation_input_tokens"])),
    }
}

#[allow(dead_code)]
fn _unused(value: &Value) -> String {
    json_string(value)
}

#[allow(dead_code)]
fn _project(path: Option<&String>) -> Option<String> {
    path.and_then(|value| project_name(value))
}

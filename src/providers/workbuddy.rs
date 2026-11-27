use crate::providers::base::{value_text, SessionBuilder, Usage};
use crate::util::{get_i64, get_str, project_name};
use anyhow::Result;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub fn discover(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .max_depth(6)
        .into_iter()
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if name.ends_with(".jsonl") && !name.ends_with(".meta.json") {
            out.push(entry.path().to_path_buf());
        }
    }
    out
}

pub fn parse(path: &Path, source: &str) -> Result<Vec<crate::model::ParsedSession>> {
    let mut builder = SessionBuilder::new(source, path);
    let mut session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session")
        .to_string();
    let mut title: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut model: Option<String> = None;

    for (index, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(id) = get_str(&obj, &["sessionId"]) {
            session_id = id.to_string();
        }
        cwd = get_str(&obj, &["cwd"]).map(ToString::to_string).or(cwd);
        let timestamp = obj
            .get("timestamp")
            .and_then(|value| crate::util::normalize_ts(value));
        match get_str(&obj, &["type"]) {
            Some("ai-title") => {
                title = get_str(&obj, &["aiTitle"])
                    .map(ToString::to_string)
                    .or(title);
                continue;
            }
            Some("custom-title") => {
                title = get_str(&obj, &["customTitle"])
                    .map(ToString::to_string)
                    .or(title);
                continue;
            }
            Some("message") => {
                let role = get_str(&obj, &["role"]).unwrap_or("user");
                let provider = obj.get("providerData").unwrap_or(&Value::Null);
                let msg_model = get_str(provider, &["model"]).map(ToString::to_string);
                if let Some(value) = &msg_model {
                    model = Some(value.clone());
                }
                let usage = usage_of(provider);
                let text = obj.get("content").and_then(value_text);
                let parent = get_str(&obj, &["parentId"]);
                if let Some(text) = text {
                    if !text.trim().is_empty() {
                        builder.add_message(
                            Some(
                                get_str(&obj, &["id"])
                                    .unwrap_or(&format!("wb:{session_id}:{index}")),
                            ),
                            role,
                            Some(text),
                            "text",
                            parent,
                            timestamp.as_deref(),
                            false,
                            msg_model.as_deref(),
                            usage,
                            None,
                            None,
                        );
                    }
                }
            }
            Some("reasoning") => {
                let provider = obj.get("providerData").unwrap_or(&Value::Null);
                let msg_model = get_str(provider, &["model"]).map(ToString::to_string);
                if let Some(text) = obj.get("content").and_then(value_text) {
                    builder.add_message(
                        Some(
                            get_str(&obj, &["id"])
                                .unwrap_or(&format!("wb:{session_id}:{index}:thinking")),
                        ),
                        "assistant",
                        Some(text),
                        "thinking",
                        get_str(&obj, &["parentId"]),
                        timestamp.as_deref(),
                        true,
                        msg_model.as_deref(),
                        Usage::default(),
                        None,
                        None,
                    );
                }
            }
            Some("function_call") => {
                let name = get_str(&obj, &["name"]).unwrap_or("tool");
                let raw = get_str(&obj, &["arguments"]).unwrap_or("{}");
                let arguments = serde_json::from_str::<Value>(raw)
                    .unwrap_or_else(|_| serde_json::json!({"input": raw}));
                builder.add_tool_use(
                    &format!("wb:{session_id}:{index}:call"),
                    get_str(&obj, &["parentId"]),
                    name,
                    &arguments,
                    get_str(&obj, &["callId"]),
                    timestamp.as_deref(),
                    false,
                    None,
                );
            }
            Some("function_call_result") => {
                let Some(call_id) = get_str(&obj, &["callId"]) else {
                    continue;
                };
                let content = obj
                    .get("output")
                    .and_then(value_text)
                    .or_else(|| obj.get("content").and_then(value_text));
                builder.add_tool_result(
                    call_id,
                    &format!("wb:{session_id}:{index}:result"),
                    get_str(&obj, &["parentId"]),
                    content,
                    timestamp.as_deref(),
                    get_str(&obj, &["status"]) == Some("failed"),
                );
            }
            _ => {}
        }
    }

    builder.set_id(&session_id);
    builder.session.title = title;
    builder.session.model = model;
    builder.set_project(cwd.as_deref());
    Ok(vec![builder.finish()])
}

fn usage_of(provider: &Value) -> Usage {
    let usage = provider.get("usage").unwrap_or(&Value::Null);
    let input = usage
        .get("inputTokens")
        .and_then(Value::as_i64)
        .or_else(|| usage.get("input_tokens").and_then(Value::as_i64));
    let output = usage
        .get("outputTokens")
        .and_then(Value::as_i64)
        .or_else(|| usage.get("output_tokens").and_then(Value::as_i64));
    let cache_read = usage
        .get("inputTokensDetails")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| get_i64(item, &["cached_tokens"]));
    Usage {
        input,
        output,
        cache_read,
        cache_write: None,
    }
}

#[allow(dead_code)]
fn _project(value: Option<&String>) -> Option<String> {
    value.and_then(|value| project_name(value))
}

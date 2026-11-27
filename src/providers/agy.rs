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
        .max_depth(6)
        .into_iter()
        .flatten()
    {
        if entry.file_type().is_file() && entry.file_name() == "transcript_full.jsonl" {
            out.push(entry.path().to_path_buf());
        }
    }
    out
}

pub fn parse(path: &Path, source: &str) -> Result<Vec<crate::model::ParsedSession>> {
    let session_id = path
        .ancestors()
        .nth(3)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("session")
        .to_string();
    let mut builder = SessionBuilder::new(source, path);
    let model: Option<String> = None;
    let mut last_assistant: Option<String> = None;

    for (index, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(step) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let step_type = get_str(&step, &["type"]).unwrap_or("");
        let source_kind = get_str(&step, &["source"]).unwrap_or("");
        let timestamp = get_str(&step, &["created_at"]);
        let step_index = step
            .get("step_index")
            .and_then(Value::as_i64)
            .unwrap_or(index as i64);
        let base_uuid = format!("{source}:{session_id}:{step_index}");
        if step_type == "USER_INPUT" {
            let content = get_str(&step, &["content"]).unwrap_or("");
            let text = extract_user_request(content).unwrap_or_else(|| content.to_string());
            if !text.trim().is_empty() {
                builder.add_message(
                    Some(&base_uuid),
                    "user",
                    Some(text),
                    "text",
                    None,
                    timestamp,
                    false,
                    None,
                    Usage::default(),
                    None,
                    None,
                );
            }
            continue;
        }
        if source_kind == "MODEL" {
            let thinking = get_str(&step, &["thinking"]).map(ToString::to_string);
            let content = get_str(&step, &["content"]).map(ToString::to_string);
            let usage = Usage {
                input: get_i64(&step, &["input_tokens"]),
                output: get_i64(&step, &["output_tokens"]),
                cache_read: get_i64(&step, &["cache_read_tokens"]),
                cache_write: None,
            };
            if let Some(text) = content {
                if !text.trim().is_empty() {
                    let uuid = builder.add_message(
                        Some(&base_uuid),
                        "assistant",
                        Some(text),
                        "text",
                        None,
                        timestamp,
                        false,
                        model.as_deref(),
                        usage,
                        None,
                        None,
                    );
                    last_assistant = Some(uuid);
                }
            }
            if let Some(text) = thinking {
                if !text.trim().is_empty() {
                    builder.add_message(
                        Some(&format!("{base_uuid}:thinking")),
                        "assistant",
                        Some(text),
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
            }
            if let Some(Value::Array(calls)) = step.get("tool_calls") {
                for (tool_index, call) in calls.iter().enumerate() {
                    builder.add_tool_use(
                        &format!("{base_uuid}:tool:{tool_index}"),
                        None,
                        get_str(call, &["name"]).unwrap_or("tool"),
                        call.get("args")
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
    }

    if let Some(uuid) = last_assistant.as_deref() {
        // A step's own usage is already attached; this is a no-op unless the
        // transcript put the total on the first model step.
        let _ = uuid;
    }

    builder.set_id(&session_id);
    builder.session.model = model;
    Ok(vec![builder.finish()])
}

fn extract_user_request(content: &str) -> Option<String> {
    let start = content.find("<USER_REQUEST>")?;
    let end = content.find("</USER_REQUEST>")?;
    let inner = &content[start + "<USER_REQUEST>".len()..end];
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[allow(dead_code)]
fn _unused(value: &Value) -> Option<String> {
    value_text(value)
}

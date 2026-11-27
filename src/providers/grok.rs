use crate::providers::base::{value_text, SessionBuilder, Usage};
use crate::util::{get_i64, get_str, normalize_ts, project_name};
use anyhow::Result;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub fn discover(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .max_depth(4)
        .into_iter()
        .flatten()
    {
        if entry.file_type().is_file() && entry.file_name() == "updates.jsonl" {
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
    let mut builder = SessionBuilder::new("grok", path);
    let mut title: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut model: Option<String> = None;
    let mut session_id = folder.clone();

    if let Ok(text) = std::fs::read_to_string(dir.join("summary.json")) {
        if let Ok(summary) = serde_json::from_str::<Value>(&text) {
            session_id = get_str(&summary, &["info", "id"])
                .unwrap_or(&folder)
                .to_string();
            cwd = get_str(&summary, &["info", "cwd"]).map(ToString::to_string);
            title = get_str(&summary, &["generated_title"])
                .or_else(|| get_str(&summary, &["session_summary"]))
                .map(ToString::to_string);
            model = get_str(&summary, &["current_model_id"]).map(ToString::to_string);
        }
    }

    #[derive(Default)]
    struct Pending {
        role: Option<String>,
        text: String,
        timestamp: Option<String>,
    }
    fn flush(
        builder: &mut SessionBuilder,
        pending: &mut Pending,
        model: &Option<String>,
        session_id: &str,
    ) -> Option<String> {
        let role = pending.role.take()?;
        if pending.text.trim().is_empty() {
            pending.text.clear();
            pending.timestamp = None;
            return None;
        }
        let uuid = format!(
            "grok:{session_id}:{}",
            builder.session.id.len() + pending.text.len()
        );
        let uuid = builder.add_message(
            Some(&uuid),
            &role,
            Some(std::mem::take(&mut pending.text)),
            "text",
            None,
            pending.timestamp.take().as_deref(),
            false,
            model.as_deref(),
            Usage::default(),
            None,
            None,
        );
        (role == "assistant").then_some(uuid)
    }

    let mut pending = Pending::default();
    let mut last_assistant: Option<String> = None;

    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let update = obj
            .get("params")
            .and_then(|p| p.get("update"))
            .unwrap_or(&Value::Null);
        let update_type = get_str(update, &["sessionUpdate"]).unwrap_or("");
        let timestamp = obj.get("timestamp").and_then(|value| normalize_ts(value));
        match update_type {
            "user_message_chunk" | "agent_message_chunk" => {
                let role = if update_type == "user_message_chunk" {
                    "user"
                } else {
                    "assistant"
                };
                let Some(text) = update.get("content").and_then(value_text) else {
                    continue;
                };
                if pending.role.as_deref() != Some(role) {
                    if let Some(uuid) = flush(&mut builder, &mut pending, &model, &session_id) {
                        last_assistant = Some(uuid);
                    }
                    pending.role = Some(role.to_string());
                }
                if pending.timestamp.is_none() {
                    pending.timestamp = timestamp;
                }
                pending.text.push_str(&text);
            }
            "tool_call" => {
                if let Some(uuid) = flush(&mut builder, &mut pending, &model, &session_id) {
                    last_assistant = Some(uuid);
                }
                let name = get_str(update, &["_meta", "x.ai/tool", "name"])
                    .or_else(|| get_str(update, &["title"]))
                    .unwrap_or("tool");
                let args = update
                    .get("rawInput")
                    .or_else(|| {
                        update
                            .get("_meta")
                            .and_then(|m| m.get("x.ai/tool"))
                            .and_then(|t| t.get("input"))
                    })
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Default::default()));
                let uuid = format!(
                    "grok:{session_id}:call:{}",
                    get_str(update, &["toolCallId"]).unwrap_or("")
                );
                builder.add_tool_use(
                    &uuid,
                    None,
                    name,
                    &args,
                    get_str(update, &["toolCallId"]),
                    timestamp.as_deref(),
                    false,
                    None,
                );
            }
            "tool_call_update" => {
                let status = get_str(update, &["status"]).unwrap_or("");
                if status == "completed" || status == "failed" {
                    let Some(call_id) = get_str(update, &["toolCallId"]) else {
                        continue;
                    };
                    let content = update
                        .get("content")
                        .and_then(value_text)
                        .or_else(|| update.get("rawOutput").and_then(value_text))
                        .or_else(|| {
                            update
                                .get("rawOutput")
                                .and_then(|r| r.get("output_for_prompt"))
                                .and_then(Value::as_str)
                                .map(ToString::to_string)
                        });
                    builder.add_tool_result(
                        call_id,
                        &format!(
                            "grok:{session_id}:result:{}",
                            get_str(update, &["toolCallId"]).unwrap_or("")
                        ),
                        None,
                        content,
                        timestamp.as_deref(),
                        status == "failed",
                    );
                }
            }
            "turn_completed" => {
                if let Some(uuid) = flush(&mut builder, &mut pending, &model, &session_id) {
                    last_assistant = Some(uuid);
                }
                if let Some(uuid) = last_assistant.as_deref() {
                    let usage = update.get("usage").unwrap_or(&Value::Null);
                    builder.set_last_usage(
                        uuid,
                        Usage {
                            input: get_i64(usage, &["inputTokens"]),
                            output: get_i64(usage, &["outputTokens"]),
                            cache_read: get_i64(usage, &["cachedReadTokens"]),
                            cache_write: get_i64(usage, &["cacheCreationTokens"]),
                        },
                    );
                }
            }
            _ => {}
        }
    }
    let _ = flush(&mut builder, &mut pending, &model, &session_id);

    builder.set_id(&session_id);
    builder.session.title = title;
    builder.session.model = model;
    builder.set_project(cwd.as_deref());
    Ok(vec![builder.finish()])
}

#[allow(dead_code)]
fn _project(value: Option<&String>) -> Option<String> {
    value.and_then(|value| project_name(value))
}

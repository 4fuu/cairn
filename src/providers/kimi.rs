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
        .max_depth(6)
        .into_iter()
        .flatten()
    {
        if entry.file_type().is_file() && entry.file_name() == "wire.jsonl" {
            out.push(entry.path().to_path_buf());
        }
    }
    out
}

pub fn parse(path: &Path) -> Result<Vec<crate::model::ParsedSession>> {
    let agent_dir = path.parent().unwrap_or(path);
    let session_dir = agent_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or(agent_dir);
    let session_id = session_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("session")
        .to_string();
    let mut builder = SessionBuilder::new("kimi", path);
    let mut title: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut model: Option<String> = None;
    let mut last_assistant: Option<String> = None;

    if let Ok(text) = std::fs::read_to_string(session_dir.join("state.json")) {
        if let Ok(state) = serde_json::from_str::<Value>(&text) {
            title = get_str(&state, &["title"]).map(ToString::to_string);
            cwd = get_str(&state, &["cwd"]).map(ToString::to_string);
            model = get_str(&state, &["modelAlias"]).map(ToString::to_string);
        }
    }

    for (index, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let kind = get_str(&obj, &["type"]).unwrap_or("");
        let timestamp = obj
            .get("time")
            .and_then(|value| crate::util::normalize_ts(value));
        match kind {
            "config.update" => {
                cwd = get_str(&obj, &["cwd"]).map(ToString::to_string).or(cwd);
                model = get_str(&obj, &["modelAlias"])
                    .map(ToString::to_string)
                    .or(model);
            }
            "context.append_message" => {
                let Some(message) = get_value(&obj, &["message"]) else {
                    continue;
                };
                let role = get_str(message, &["role"]).unwrap_or("user");
                let origin = get_str(message, &["origin", "kind"]).unwrap_or("user");
                let text = message.get("content").and_then(value_text);
                if let Some(text) = text {
                    if !text.trim().is_empty() {
                        let uuid = format!("kimi:{session_id}:{index}");
                        let message_uuid = builder.add_message(
                            Some(&uuid),
                            role,
                            Some(text),
                            "text",
                            None,
                            timestamp.as_deref(),
                            origin != "user",
                            model.as_deref(),
                            Usage::default(),
                            None,
                            None,
                        );
                        if role == "assistant" {
                            last_assistant = Some(message_uuid);
                        }
                    }
                }
                if let Some(Value::Array(calls)) = message.get("toolCalls") {
                    for (tool_index, call) in calls.iter().enumerate() {
                        builder.add_tool_use(
                            &format!("kimi:{session_id}:{index}:tool:{tool_index}"),
                            None,
                            get_str(call, &["name"]).unwrap_or("tool"),
                            call.get("arguments").unwrap_or(&Value::Null),
                            get_str(call, &["id"]),
                            timestamp.as_deref(),
                            false,
                            None,
                        );
                    }
                }
            }
            "context.append_loop_event" => {
                let Some(event) = get_value(&obj, &["event"]) else {
                    continue;
                };
                let event_type = get_str(event, &["type"]).unwrap_or("");
                let uuid = get_str(event, &["uuid"])
                    .map(|value| format!("kimi:{session_id}:{value}"))
                    .unwrap_or_else(|| format!("kimi:{session_id}:{index}"));
                match event_type {
                    "content.part" => {
                        let Some(part) = get_value(event, &["part"]) else {
                            continue;
                        };
                        let part_type = get_str(part, &["type"]).unwrap_or("text");
                        let (content_type, text) = if part_type == "think" {
                            (
                                "thinking",
                                get_str(part, &["think"]).map(ToString::to_string),
                            )
                        } else {
                            ("text", get_str(part, &["text"]).map(ToString::to_string))
                        };
                        if let Some(text) = text {
                            let message_uuid = builder.add_message(
                                Some(&uuid),
                                "assistant",
                                Some(text),
                                content_type,
                                None,
                                timestamp.as_deref(),
                                content_type == "thinking",
                                model.as_deref(),
                                Usage::default(),
                                None,
                                None,
                            );
                            if content_type == "text" {
                                last_assistant = Some(message_uuid);
                            }
                        }
                    }
                    "tool.call" => {
                        builder.add_tool_use(
                            &format!("{uuid}:call"),
                            None,
                            get_str(event, &["name"]).unwrap_or("tool"),
                            event.get("args").unwrap_or(&Value::Null),
                            get_str(event, &["toolCallId"]),
                            timestamp.as_deref(),
                            false,
                            None,
                        );
                    }
                    "tool.result" => {
                        let Some(call_id) = get_str(event, &["toolCallId"]) else {
                            continue;
                        };
                        let result = event.get("result").unwrap_or(&Value::Null);
                        let content = get_str(result, &["output"]).map(ToString::to_string);
                        builder.add_tool_result(
                            call_id,
                            &format!("{uuid}:result"),
                            None,
                            content,
                            timestamp.as_deref(),
                            result
                                .get("isError")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        );
                    }
                    "step.end" => {
                        if let Some(uuid) = last_assistant.as_deref() {
                            let usage = event.get("usage").unwrap_or(&Value::Null);
                            builder.set_last_usage(
                                uuid,
                                Usage {
                                    input: get_i64(usage, &["inputOther"]),
                                    output: get_i64(usage, &["output"]),
                                    cache_read: get_i64(usage, &["inputCacheRead"]),
                                    cache_write: get_i64(usage, &["inputCacheCreation"]),
                                },
                            );
                        }
                    }
                    _ => {}
                }
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

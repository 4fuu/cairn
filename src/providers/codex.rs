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

pub fn parse(path: &Path) -> Result<Vec<crate::model::ParsedSession>> {
    let mut builder = SessionBuilder::new("codex", path);
    let mut session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session")
        .to_string();
    let mut title: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut version: Option<String> = None;
    let mut model: Option<String> = None;

    // Pass one: which visible messages the event stream already carries.
    let mut event_digests = std::collections::HashSet::new();
    let mut probe = SessionBuilder::new("codex", path);
    for (index, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(&line) else {
            probe.warn(format!("line {}: invalid JSON", index + 1));
            continue;
        };
        if get_str(&obj, &["type"]) != Some("event_msg") {
            continue;
        }
        let payload = obj.get("payload").unwrap_or(&Value::Null);
        if matches!(
            get_str(payload, &["type"]),
            Some("user_message") | Some("agent_message")
        ) {
            if let Some(text) = event_text(payload) {
                let role = if get_str(payload, &["type"]) == Some("user_message") {
                    "user"
                } else {
                    "assistant"
                };
                event_digests.insert((role.to_string(), digest(&text)));
            }
        }
    }

    let mut last_assistant: Option<String> = None;
    for (index, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(&line) else {
            builder.warn(format!("line {}: invalid JSON", index + 1));
            continue;
        };
        let kind = get_str(&obj, &["type"]).unwrap_or("");
        let payload = obj.get("payload").unwrap_or(&Value::Null);
        let timestamp = get_str(&obj, &["timestamp"]);
        match kind {
            "session_meta" => {
                if let Some(value) = get_str(payload, &["id"]) {
                    session_id = value.to_string();
                }
                cwd = get_str(payload, &["cwd"]).map(ToString::to_string).or(cwd);
                version = get_str(payload, &["cli_version"])
                    .map(ToString::to_string)
                    .or(version);
                branch = get_str(payload, &["git", "branch"])
                    .map(ToString::to_string)
                    .or(branch);
                continue;
            }
            "turn_context" => {
                cwd = get_str(payload, &["cwd"]).map(ToString::to_string).or(cwd);
                model = get_str(payload, &["model"])
                    .map(ToString::to_string)
                    .or(model);
                continue;
            }
            "event_msg" => {
                let event_type = get_str(payload, &["type"]).unwrap_or("");
                if matches!(
                    event_type,
                    "user_message" | "agent_message" | "agent_reasoning"
                ) {
                    let Some(text) = event_text(payload) else {
                        continue;
                    };
                    let role = if event_type == "user_message" {
                        "user"
                    } else {
                        "assistant"
                    };
                    let is_reasoning = event_type == "agent_reasoning";
                    let uuid = builder.add_message(
                        Some(&format!("codex:{session_id}:{}", index + 1)),
                        role,
                        Some(text),
                        if is_reasoning { "thinking" } else { "text" },
                        None,
                        timestamp,
                        is_reasoning,
                        model.as_deref(),
                        Usage::default(),
                        None,
                        None,
                    );
                    if role == "assistant" && !is_reasoning {
                        last_assistant = Some(uuid);
                    }
                } else if event_type == "thread_name_updated" {
                    title = get_str(payload, &["thread_name"])
                        .map(ToString::to_string)
                        .or(title);
                } else if event_type == "token_count" {
                    if let Some(uuid) = last_assistant.as_deref() {
                        builder.set_last_usage(uuid, codex_usage(payload));
                    }
                }
                continue;
            }
            "response_item" => {}
            _ => continue,
        }

        let item_type = get_str(payload, &["type"]).unwrap_or("");
        if item_type == "message" {
            if get_str(payload, &["role"]) == Some("developer") {
                continue;
            }
            let Some(text) = message_text(payload) else {
                continue;
            };
            let role = get_str(payload, &["role"]).unwrap_or("assistant");
            if event_digests.contains(&(role.to_string(), digest(&text))) {
                continue;
            }
            let uuid = builder.add_message(
                Some(&format!("codex:{session_id}:{}", index + 1)),
                role,
                Some(text),
                "text",
                None,
                timestamp,
                role == "system",
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
        if matches!(
            item_type,
            "function_call"
                | "custom_tool_call"
                | "tool_search_call"
                | "web_search_call"
                | "local_shell_call"
        ) {
            let call_id = get_str(payload, &["call_id"]).or_else(|| get_str(payload, &["id"]));
            let name = get_str(payload, &["name"])
                .or_else(|| get_str(payload, &["tool"]))
                .unwrap_or(item_type);
            let mut arguments = payload
                .get("arguments")
                .or_else(|| payload.get("input"))
                .or_else(|| payload.get("action"))
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default()));
            if let Some(raw) = arguments.as_str() {
                arguments =
                    serde_json::from_str(raw).unwrap_or_else(|_| serde_json::json!({"input": raw}));
            }
            builder.add_tool_use(
                &format!("codex:{session_id}:{}:call", index + 1),
                None,
                name,
                &arguments,
                call_id,
                timestamp,
                false,
                None,
            );
            continue;
        }
        if matches!(
            item_type,
            "function_call_output"
                | "custom_tool_call_output"
                | "tool_search_output"
                | "local_shell_call_output"
        ) {
            let Some(call_id) =
                get_str(payload, &["call_id"]).or_else(|| get_str(payload, &["id"]))
            else {
                continue;
            };
            builder.add_tool_result(
                call_id,
                &format!("codex:{session_id}:{}:result", index + 1),
                None,
                output_text(payload),
                timestamp,
                payload
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            );
        }
    }

    builder.set_id(&session_id);
    builder.session.title = title;
    builder.session.git_branch = branch;
    builder.session.version = version;
    builder.session.model = model;
    builder.set_project(cwd.as_deref());
    builder.session.meta_json = Some(serde_json::json!({"source": "codex"}).to_string());
    Ok(vec![builder.finish()])
}

fn event_text(payload: &Value) -> Option<String> {
    if let Some(text) = get_str(payload, &["message"]) {
        return Some(text.to_string());
    }
    if let Some(Value::Array(items)) = payload.get("text_elements") {
        let parts: Vec<String> = items
            .iter()
            .filter_map(|item| {
                item.as_str()
                    .map(ToString::to_string)
                    .or_else(|| get_str(item, &["text"]).map(ToString::to_string))
            })
            .collect();
        if !parts.is_empty() {
            return Some(parts.join("\n"));
        }
    }
    get_str(payload, &["text"]).map(ToString::to_string)
}

fn message_text(payload: &Value) -> Option<String> {
    let content = payload.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    let items = content.as_array()?;
    let parts: Vec<String> = items
        .iter()
        .filter_map(|block| get_str(block, &["text"]).map(ToString::to_string))
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

fn output_text(payload: &Value) -> Option<String> {
    payload
        .get("output")
        .and_then(value_text)
        .or_else(|| payload.get("content").and_then(value_text))
}

fn codex_usage(payload: &Value) -> Usage {
    let info = payload.get("info").unwrap_or(&Value::Null);
    let usage = info
        .get("last_token_usage")
        .or_else(|| info.get("total_token_usage"))
        .or_else(|| payload.get("last_token_usage"));
    Usage {
        input: usage.and_then(|u| get_i64(u, &["input_tokens"])),
        output: usage.and_then(|u| get_i64(u, &["output_tokens"])),
        cache_read: usage.and_then(|u| get_i64(u, &["cache_read_tokens"])),
        cache_write: usage.and_then(|u| get_i64(u, &["cache_write_tokens"])),
    }
}

fn digest(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join("")
}

#[allow(dead_code)]
fn _unused(value: &Value) -> Option<String> {
    get_value(value, &["payload"]).and_then(value_text)
}

#[allow(dead_code)]
fn _project(value: Option<&String>) -> Option<String> {
    value.and_then(|value| project_name(value))
}

#[allow(dead_code)]
fn _ts(value: &Value) -> Option<String> {
    normalize_ts(value)
}

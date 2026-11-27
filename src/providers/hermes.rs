use crate::providers::base::{value_text, SessionBuilder, Usage};
use crate::util::{get_i64, get_str};
use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn discover(path: &Path) -> Vec<PathBuf> {
    if path.is_file() {
        vec![path.to_path_buf()]
    } else {
        Vec::new()
    }
}

pub fn parse(path: &Path, source: &str) -> Result<Vec<crate::model::ParsedSession>> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    let store = &crate::util::stable_id(&[&path.to_string_lossy()])[..8];
    let mut stmt = conn.prepare(
        "SELECT id, COALESCE(title,''), COALESCE(cwd,''), COALESCE(model,''),
                started_at, ended_at, COALESCE(input_tokens,0), COALESCE(output_tokens,0),
                COALESCE(cache_read_tokens,0), COALESCE(cache_write_tokens,0)
         FROM sessions ORDER BY COALESCE(ended_at, started_at) DESC",
    )?;
    let sessions = stmt
        .query_map([], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                title: row.get(1)?,
                cwd: row.get(2)?,
                model: row.get(3)?,
                started: row.get(4)?,
                ended: row.get(5)?,
                input: row.get(6)?,
                output: row.get(7)?,
                cache_read: row.get(8)?,
                cache_write: row.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::new();
    for row in sessions {
        let mut builder = SessionBuilder::new(source, path);
        builder.set_id(&format!("{store}-{}", row.id));
        builder.session.title = non_empty(&row.title);
        builder.session.model = non_empty(&row.model);
        builder.set_project(non_empty(&row.cwd).as_deref());
        builder.session.started_at = row.started.and_then(ts);
        builder.session.ended_at = row.ended.and_then(ts);
        let mut messages_stmt = conn.prepare(
            "SELECT id, COALESCE(role,''), COALESCE(content,''), COALESCE(tool_calls,'[]'), timestamp
             FROM messages WHERE session_id = ? ORDER BY id",
        )?;
        let mut last_assistant: Option<String> = None;
        let mut pending_calls: std::collections::VecDeque<String> =
            std::collections::VecDeque::new();
        let mut rows = messages_stmt.query([&row.id])?;
        while let Some(message) = rows.next()? {
            let id: i64 = message.get(0)?;
            let role: String = message.get(1)?;
            let content: String = message.get(2)?;
            let tool_calls: String = message.get(3)?;
            let timestamp = message.get::<_, Option<f64>>(4)?.and_then(ts);
            let text = hermes_content(&content);
            let uuid = format!("{source}:{store}-{}:{id}", row.id);
            if role == "tool" {
                if let Some(text) = text {
                    let call_id = pending_calls.pop_front().unwrap_or_else(|| uuid.clone());
                    builder.add_tool_result(
                        &call_id,
                        &uuid,
                        None,
                        Some(text),
                        timestamp.as_deref(),
                        false,
                    );
                }
                continue;
            }
            if let Some(text) = text {
                if !text.trim().is_empty() {
                    let message_uuid = builder.add_message(
                        Some(&uuid),
                        &role,
                        Some(text),
                        "text",
                        None,
                        timestamp.as_deref(),
                        false,
                        None,
                        Usage::default(),
                        None,
                        None,
                    );
                    if role == "assistant" {
                        last_assistant = Some(message_uuid);
                    }
                }
            }
            if let Ok(Value::Array(calls)) = serde_json::from_str::<Value>(&tool_calls) {
                for (index, call) in calls.iter().enumerate() {
                    let function = call.get("function").unwrap_or(&Value::Null);
                    let call_id = get_str(call, &["id"])
                        .map(ToString::to_string)
                        .unwrap_or_else(|| format!("{uuid}:tool:{index}"));
                    builder.add_tool_use(
                        &format!("{uuid}:tool:{index}"),
                        None,
                        get_str(function, &["name"]).unwrap_or("tool"),
                        function.get("arguments").unwrap_or(&Value::Null),
                        Some(&call_id),
                        timestamp.as_deref(),
                        false,
                        None,
                    );
                    pending_calls.push_back(call_id);
                }
            }
        }
        if let Some(uuid) = last_assistant.as_deref() {
            builder.set_last_usage(
                uuid,
                Usage {
                    input: Some(row.input),
                    output: Some(row.output),
                    cache_read: Some(row.cache_read),
                    cache_write: Some(row.cache_write),
                },
            );
        }
        out.push(builder.finish());
    }
    Ok(out)
}

struct SessionRow {
    id: String,
    title: String,
    cwd: String,
    model: String,
    started: Option<f64>,
    ended: Option<f64>,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
}

fn hermes_content(content: &str) -> Option<String> {
    if let Some(json) = content.strip_prefix("\u{0}json:") {
        if let Ok(value) = serde_json::from_str::<Value>(json) {
            return value_text(&value);
        }
    }
    if content.trim().is_empty() {
        None
    } else {
        Some(content.to_string())
    }
}

fn ts(value: f64) -> Option<String> {
    crate::util::normalize_ts(&serde_json::json!(value))
}

fn non_empty(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[allow(dead_code)]
fn _unused(value: &Value) -> Option<i64> {
    get_i64(value, &["id"])
}

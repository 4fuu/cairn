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
    let has_v2 = table_exists(&conn, "session_v2")?;
    let mut out = Vec::new();
    if has_v2 {
        let mut stmt = conn.prepare(
            "SELECT id, COALESCE(parent_id, ''), directory, COALESCE(title, ''),
                    time_created, time_updated
             FROM session_v2 ORDER BY time_updated DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                _parent: row.get(1)?,
                directory: row.get(2)?,
                title: row.get(3)?,
                _created: row.get(4)?,
                _updated: row.get(5)?,
            })
        })?;
        for row in rows {
            let row = row?;
            out.push(parse_v2(&conn, source, path, &row)?);
        }
    } else if table_exists(&conn, "session")? {
        let mut stmt = conn.prepare(
            "SELECT id, COALESCE(parent_id, ''), directory, COALESCE(title, ''),
                    time_created, time_updated
             FROM session ORDER BY time_updated DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                _parent: row.get(1)?,
                directory: row.get(2)?,
                title: row.get(3)?,
                _created: row.get(4)?,
                _updated: row.get(5)?,
            })
        })?;
        for row in rows {
            let row = row?;
            out.push(parse_v1(&conn, source, path, &row)?);
        }
    }
    Ok(out)
}

#[derive(Debug)]
struct SessionRow {
    id: String,
    _parent: String,
    directory: String,
    title: String,
    _created: i64,
    _updated: i64,
}

fn parse_v1(
    conn: &Connection,
    source: &str,
    path: &Path,
    row: &SessionRow,
) -> Result<crate::model::ParsedSession> {
    let mut builder = SessionBuilder::new(source, path);
    builder.set_id(&row.id);
    builder.session.title = non_empty(&row.title);
    builder.set_project(non_empty(&row.directory).as_deref());
    let mut stmt = conn.prepare("SELECT data FROM message WHERE session_id = ?")?;
    let mut messages: Vec<Value> = stmt
        .query_map([&row.id], |r| r.get::<_, String>(0))?
        .filter_map(|text| text.ok())
        .filter_map(|text| serde_json::from_str::<Value>(&text).ok())
        .collect();
    messages.sort_by_key(|m| {
        (
            get_i64(m, &["time", "created"]).unwrap_or(0),
            get_str(m, &["id"]).unwrap_or("").to_string(),
        )
    });
    let mut model: Option<String> = None;
    let mut last_assistant: Option<String> = None;
    for message in &messages {
        let id = get_str(message, &["id"]).unwrap_or("message").to_string();
        let role = get_str(message, &["role"]).unwrap_or("user");
        let model_id = get_str(message, &["modelID"]).or_else(|| get_str(message, &["modelId"]));
        if let Some(value) = model_id {
            model = Some(value.to_string());
        }
        let timestamp = message
            .get("time")
            .and_then(|t| t.get("created"))
            .and_then(|v| crate::util::normalize_ts(v));
        let usage = Usage {
            input: message.get("tokens").and_then(|t| get_i64(t, &["input"])),
            output: message.get("tokens").and_then(|t| get_i64(t, &["output"])),
            cache_read: message
                .get("tokens")
                .and_then(|t| get_i64(t, &["cache", "read"])),
            cache_write: message
                .get("tokens")
                .and_then(|t| get_i64(t, &["cache", "write"])),
        };
        let mut parts_stmt =
            conn.prepare("SELECT data FROM part WHERE message_id = ? ORDER BY id")?;
        let parts: Vec<Value> = parts_stmt
            .query_map([&id], |r| r.get::<_, String>(0))?
            .filter_map(|text| text.ok())
            .filter_map(|text| serde_json::from_str::<Value>(&text).ok())
            .filter(|part| {
                !part
                    .get("synthetic")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    && !part
                        .get("ignored")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
            })
            .collect();
        let mut text_parts = Vec::new();
        let mut thinking_parts = Vec::new();
        let mut tools = Vec::new();
        for (part_index, part) in parts.iter().enumerate() {
            match get_str(part, &["type"]) {
                Some("text") => {
                    if let Some(text) = get_str(part, &["text"]) {
                        text_parts.push(text.to_string());
                    }
                }
                Some("reasoning") => {
                    if let Some(text) = get_str(part, &["text"]) {
                        thinking_parts.push(text.to_string());
                    }
                }
                Some("tool") => tools.push((part_index, part)),
                _ => {}
            }
        }
        let text = text_parts.join("\n");
        if !text.trim().is_empty() {
            let uuid = builder.add_message(
                Some(&format!("{source}:{}:{id}", row.id)),
                role,
                Some(text),
                "text",
                None,
                timestamp.as_deref(),
                false,
                model.as_deref(),
                usage,
                None,
                None,
            );
            if role == "assistant" {
                last_assistant = Some(uuid);
            }
        }
        if !thinking_parts.is_empty() {
            builder.add_message(
                Some(&format!("{source}:{}:{id}:thinking", row.id)),
                "assistant",
                Some(thinking_parts.join("\n")),
                "thinking",
                None,
                timestamp.as_deref(),
                true,
                model.as_deref(),
                Usage::default(),
                None,
                None,
            );
        }
        for (tool_index, (_, part)) in tools.iter().enumerate() {
            let name = get_str(part, &["name"])
                .or_else(|| get_str(part, &["tool"]))
                .unwrap_or("tool");
            let args = part
                .get("state")
                .and_then(|s| s.get("input"))
                .unwrap_or(&Value::Null);
            builder.add_tool_use(
                &format!("{source}:{}:{id}:tool:{tool_index}", row.id),
                None,
                name,
                args,
                get_str(part, &["id"]),
                timestamp.as_deref(),
                false,
                None,
            );
            if let Some(state) = part.get("state") {
                let output = state
                    .get("output")
                    .and_then(value_text)
                    .or_else(|| state.get("content").and_then(value_text));
                if let Some(output) = output {
                    builder.add_tool_result(
                        get_str(part, &["id"])
                            .unwrap_or(&format!("{source}:{}:{id}:tool:{tool_index}", row.id)),
                        &format!("{source}:{}:{id}:tool:{tool_index}:result", row.id),
                        None,
                        Some(output),
                        timestamp.as_deref(),
                        false,
                    );
                }
            }
        }
    }
    if let Some(uuid) = last_assistant.as_deref() {
        let _ = uuid;
    }
    builder.session.model = model;
    Ok(builder.finish())
}

fn parse_v2(
    conn: &Connection,
    source: &str,
    path: &Path,
    row: &SessionRow,
) -> Result<crate::model::ParsedSession> {
    let mut builder = SessionBuilder::new(source, path);
    builder.set_id(&row.id);
    builder.session.title = non_empty(&row.title);
    builder.set_project(non_empty(&row.directory).as_deref());
    let mut stmt = conn.prepare(
        "SELECT id, type, data FROM session_message
         WHERE session_id = ? AND type IN ('user','assistant','compaction','skill')
         ORDER BY seq",
    )?;
    let mut rows = stmt.query([&row.id])?;
    let mut model: Option<String> = None;
    while let Some(item) = rows.next()? {
        let id: String = item.get(0)?;
        let kind: String = item.get(1)?;
        let raw: String = item.get(2)?;
        let Ok(data) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let timestamp = data
            .get("time")
            .and_then(|t| t.get("created"))
            .and_then(|v| crate::util::normalize_ts(v));
        if let Some(value) = get_str(&data, &["model", "id"]) {
            model = Some(value.to_string());
        }
        if kind == "user" || kind == "skill" {
            let text = get_str(&data, &["text"])
                .map(ToString::to_string)
                .or_else(|| data.get("content").and_then(value_text));
            if let Some(text) = text {
                builder.add_message(
                    Some(&format!("{source}:{}:{id}", row.id)),
                    "user",
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
            }
            continue;
        }
        let usage = Usage {
            input: get_i64(&data, &["tokens", "input"]),
            output: get_i64(&data, &["tokens", "output"]),
            cache_read: get_i64(&data, &["tokens", "cache", "read"]),
            cache_write: get_i64(&data, &["tokens", "cache", "write"]),
        };
        let mut text_parts = Vec::new();
        let mut thinking_parts = Vec::new();
        let mut tools = Vec::new();
        if let Some(Value::Array(content)) = data.get("content") {
            for part in content {
                match get_str(part, &["type"]) {
                    Some("text") => {
                        if let Some(text) = get_str(part, &["text"]) {
                            text_parts.push(text.to_string());
                        }
                    }
                    Some("reasoning") => {
                        if let Some(text) = get_str(part, &["text"]) {
                            thinking_parts.push(text.to_string());
                        }
                    }
                    Some("tool") => tools.push(part),
                    _ => {}
                }
            }
        }
        let text = text_parts.join("\n");
        if !text.trim().is_empty() {
            builder.add_message(
                Some(&format!("{source}:{}:{id}", row.id)),
                "assistant",
                Some(text),
                "text",
                None,
                timestamp.as_deref(),
                false,
                model.as_deref(),
                usage,
                None,
                None,
            );
        }
        if !thinking_parts.is_empty() {
            builder.add_message(
                Some(&format!("{source}:{}:{id}:thinking", row.id)),
                "assistant",
                Some(thinking_parts.join("\n")),
                "thinking",
                None,
                timestamp.as_deref(),
                true,
                model.as_deref(),
                Usage::default(),
                None,
                None,
            );
        }
        for (tool_index, part) in tools.iter().enumerate() {
            let name = get_str(part, &["name"]).unwrap_or("tool");
            let args = part
                .get("state")
                .and_then(|s| s.get("input"))
                .unwrap_or(&Value::Null);
            builder.add_tool_use(
                &format!("{source}:{}:{id}:tool:{tool_index}", row.id),
                None,
                name,
                args,
                get_str(part, &["id"]),
                timestamp.as_deref(),
                false,
                None,
            );
        }
    }
    builder.session.model = model;
    Ok(builder.finish())
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
        [name],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn non_empty(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[allow(dead_code)]
fn _unused(a: i64, b: i64) -> (i64, i64) {
    (a, b)
}

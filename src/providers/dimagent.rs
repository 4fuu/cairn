use crate::providers::base::{value_text, SessionBuilder, Usage};
use crate::util::{get_i64, get_str, normalize_ts_str};
use anyhow::Result;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::path::{Path, PathBuf};

const DATABASE_NAMES: [&str; 2] = ["dimcode.sqlite", "dimglobal.sqlite"];

pub fn discover(path: &Path) -> Vec<PathBuf> {
    if path.is_file() {
        return vec![path.to_path_buf()];
    }
    DATABASE_NAMES
        .iter()
        .map(|name| path.join(name))
        .find(|candidate| candidate.is_file())
        .into_iter()
        .collect()
}

pub fn parse(path: &Path, source: &str) -> Result<Vec<crate::model::ParsedSession>> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    if !table_exists(&conn, "sessions")? || !table_exists(&conn, "messages")? {
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare(
        "SELECT sessionId, COALESCE(cwd, ''), COALESCE(title, ''),
                COALESCE(status, ''), createdAt, updatedAt
         FROM sessions ORDER BY updatedAt DESC",
    )?;
    let sessions = stmt
        .query_map([], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                cwd: row.get(1)?,
                title: row.get(2)?,
                _status: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::new();
    for row in sessions {
        out.push(parse_session(&conn, source, path, &row)?);
    }
    Ok(out)
}

struct SessionRow {
    id: String,
    cwd: String,
    title: String,
    _status: String,
    created_at: String,
    updated_at: String,
}

fn parse_session(
    conn: &Connection,
    source: &str,
    path: &Path,
    row: &SessionRow,
) -> Result<crate::model::ParsedSession> {
    let mut builder = SessionBuilder::new(source, path);
    builder.set_id(&row.id);
    builder.session.title = non_empty(&row.title);
    builder.session.model = session_model(conn, &row.id);
    builder.session.started_at = normalize_ts_str(&row.created_at);
    builder.session.ended_at = normalize_ts_str(&row.updated_at);
    builder.set_project(non_empty(&row.cwd).as_deref());

    let model = builder.session.model.clone();
    let mut stmt = conn.prepare(
        "SELECT messageId, role, parts, createdAt
         FROM messages WHERE sessionId = ?1 ORDER BY orderKey",
    )?;
    let mut rows = stmt.query([&row.id])?;
    let mut last_assistant: Option<String> = None;
    while let Some(message) = rows.next()? {
        let message_id: String = message.get(0)?;
        let role: String = message.get(1)?;
        let raw_parts: String = message.get(2)?;
        let created_at: String = message.get(3)?;
        let timestamp = normalize_ts_str(&created_at);
        let parts = match serde_json::from_str::<Vec<Value>>(&raw_parts) {
            Ok(parts) => parts,
            Err(error) => {
                builder.warn(format!(
                    "dimagent message {message_id} has invalid parts: {error}"
                ));
                continue;
            }
        };
        for (index, part) in parts.iter().enumerate() {
            let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
            let base = format!("{source}:{}:{message_id}:{index}", row.id);
            match kind {
                "text" => {
                    let Some(text) = get_str(part, &["text"]) else {
                        continue;
                    };
                    if text.trim().is_empty() || (role != "user" && role != "assistant") {
                        continue;
                    }
                    let uuid = builder.add_message(
                        Some(&base),
                        &role,
                        Some(text.to_string()),
                        "text",
                        None,
                        timestamp.as_deref(),
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
                "thinking" => {
                    let Some(text) = get_str(part, &["thinking"]) else {
                        continue;
                    };
                    if text.trim().is_empty() {
                        continue;
                    }
                    builder.add_message(
                        Some(&format!("{base}:thinking")),
                        "assistant",
                        Some(text.to_string()),
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
                "tool_use" => {
                    let call_id = get_str(part, &["id"])
                        .map(ToString::to_string)
                        .unwrap_or_else(|| format!("{base}:call"));
                    let name = get_str(part, &["name"]).unwrap_or("tool");
                    let input = part.get("input").unwrap_or(&Value::Null);
                    let uuid = builder.add_tool_use(
                        &format!("{base}:tool"),
                        None,
                        name,
                        input,
                        Some(&call_id),
                        timestamp.as_deref(),
                        false,
                        None,
                    );
                    last_assistant = Some(uuid);
                }
                "tool_result" => {
                    let Some(call_id) = get_str(part, &["tool_use_id"]) else {
                        continue;
                    };
                    if call_id.trim().is_empty() {
                        continue;
                    }
                    let content = part.get("content").and_then(value_text);
                    let is_error = part
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    builder.add_tool_result(
                        call_id,
                        &format!("{base}:result"),
                        None,
                        content,
                        timestamp.as_deref(),
                        is_error,
                    );
                }
                _ => {}
            }
        }
    }

    if let Some(uuid) = last_assistant.as_deref() {
        builder.set_last_usage(uuid, session_usage(conn, &row.id));
    }
    Ok(builder.finish())
}

fn session_model(conn: &Connection, session_id: &str) -> Option<String> {
    if !table_exists(conn, "session_states").ok()? {
        return None;
    }
    conn.query_row(
        "SELECT selectedModelId FROM session_states WHERE sessionId = ?1",
        [session_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .optional()
    .ok()
    .flatten()
    .flatten()
    .filter(|model| !model.trim().is_empty())
}

fn session_usage(conn: &Connection, session_id: &str) -> Usage {
    if !table_exists(conn, "usage_ledger").unwrap_or(false) {
        return Usage::default();
    }
    let Ok(mut stmt) = conn.prepare("SELECT usage FROM usage_ledger WHERE sessionId = ?1") else {
        return Usage::default();
    };
    let Ok(rows) = stmt.query_map([session_id], |row| row.get::<_, String>(0)) else {
        return Usage::default();
    };
    let mut input = 0;
    let mut output = 0;
    let mut cache_read = 0;
    let mut cache_write = 0;
    let mut found = false;
    for row in rows.flatten() {
        let Ok(usage) = serde_json::from_str::<Value>(&row) else {
            continue;
        };
        if let Some(value) = get_i64(&usage, &["promptTokens"]) {
            input += value;
            found = true;
        }
        if let Some(value) = get_i64(&usage, &["completionTokens"]) {
            output += value;
            found = true;
        }
        if let Some(value) = get_i64(&usage, &["cacheReadTokens"]) {
            cache_read += value;
            found = true;
        }
        if let Some(value) = get_i64(&usage, &["cacheWriteTokens"]) {
            cache_write += value;
            found = true;
        }
    }
    if !found {
        return Usage::default();
    }
    Usage {
        input: Some(input),
        output: Some(output),
        cache_read: Some(cache_read),
        cache_write: Some(cache_write),
    }
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
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

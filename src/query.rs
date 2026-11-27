use crate::store::Store;
use crate::util::{normalize_ts_str, squeeze};
use anyhow::{Context, Result};
use rusqlite::types::Value as SqlValue;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct SessionOptions {
    pub project: Option<String>,
    pub source: Option<String>,
    pub query: Option<String>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

pub fn overview(
    store: &Store,
    cwd: Option<&str>,
    project: Option<&str>,
    limit: usize,
) -> Result<Value> {
    let conn = store.open_readonly()?;
    let totals = json!({
        "sessions": conn.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get::<_, i64>(0))?,
        "messages": conn.query_row("SELECT COUNT(*) FROM messages WHERE is_meta = 0", [], |row| row.get::<_, i64>(0))?,
        "tool_calls": conn.query_row("SELECT COUNT(*) FROM tool_calls", [], |row| row.get::<_, i64>(0))?,
        "projects": conn.query_row("SELECT COUNT(DISTINCT project) FROM sessions WHERE project IS NOT NULL", [], |row| row.get::<_, i64>(0))?,
        "last_indexed": conn.query_row("SELECT MAX(indexed_at) FROM sessions", [], |row| row.get::<_, Option<String>>(0))?,
    });

    let mut clauses = Vec::new();
    let mut params = Vec::new();
    if let Some(project) = project {
        project_clause(&mut clauses, &mut params, project, "s");
    }
    clauses.push("s.project IS NOT NULL".to_string());
    let projects = conn
        .prepare(&format!(
            "SELECT s.project, COUNT(*) AS sessions, MAX(COALESCE(s.ended_at, s.started_at)) AS last_seen,
                    SUM(s.message_count) AS messages
             FROM sessions s WHERE {} GROUP BY s.project ORDER BY last_seen DESC LIMIT ?",
            clauses.join(" AND ")
        ))?
        .query_map(rusqlite::params_from_iter({
            let mut values = params.clone();
            values.push(SqlValue::Integer(limit as i64));
            values
        }), |row| {
            Ok(json!({
                "project": row.get::<_, String>(0)?,
                "sessions": row.get::<_, i64>(1)?,
                "last_seen": row.get::<_, Option<String>>(2)?,
                "messages": row.get::<_, Option<i64>>(3)?,
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let current = cwd
        .map(|cwd| current_project(&conn, cwd, limit))
        .transpose()?;
    let recent = {
        let mut clauses = Vec::new();
        let mut params = Vec::new();
        if let Some(project) = project {
            project_clause(&mut clauses, &mut params, project, "s");
        }
        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        let mut values = params;
        values.push(SqlValue::Integer(limit as i64));
        session_rows(
            &conn,
            &format!(
                "SELECT s.id, s.source, s.title, s.project, s.project_path, s.started_at,
                        s.ended_at, s.git_branch, s.model, s.version, s.summary, s.message_count
                 FROM sessions s {where_sql}
                 ORDER BY COALESCE(s.ended_at, s.started_at) DESC, s.id DESC LIMIT ?"
            ),
            values,
        )?
    };
    let sources = conn
        .prepare(
            "SELECT source, COUNT(*) AS sessions, SUM(message_count) AS messages
             FROM sessions GROUP BY source ORDER BY source",
        )?
        .query_map([], |row| {
            Ok(json!({
                "source": row.get::<_, String>(0)?,
                "sessions": row.get::<_, i64>(1)?,
                "messages": row.get::<_, Option<i64>>(2)?,
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(json!({
        "totals": totals,
        "current_project": current,
        "projects": projects,
        "recent": recent,
        "sources": sources,
    }))
}

fn current_project(conn: &Connection, cwd: &str, limit: usize) -> Result<Value> {
    let normalized = std::fs::canonicalize(Path::new(cwd))
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|_| cwd.to_string());
    let row = conn
        .query_row(
            "SELECT project, project_path, COUNT(*) AS sessions,
                    MAX(COALESCE(ended_at, started_at)) AS last_seen
             FROM sessions WHERE project_path = ? OR ? LIKE project_path || '/%'
             GROUP BY project, project_path ORDER BY sessions DESC LIMIT 1",
            [&normalized, &normalized],
            |row| {
                Ok(json!({
                    "project": row.get::<_, Option<String>>(0)?,
                    "project_path": row.get::<_, Option<String>>(1)?,
                    "sessions": row.get::<_, i64>(2)?,
                    "last_seen": row.get::<_, Option<String>>(3)?,
                }))
            },
        )
        .ok();
    let Some(mut current) = row else {
        return Ok(Value::Null);
    };
    let mut stmt = conn.prepare(
        "SELECT id, source, title, project, project_path, started_at, ended_at,
                git_branch, model, version, summary, message_count
         FROM sessions WHERE project_path = ? OR ? LIKE project_path || '/%'
         ORDER BY COALESCE(ended_at, started_at) DESC LIMIT ?",
    )?;
    current["recent"] = Value::Array(
        stmt.query_map(
            rusqlite::params![normalized, normalized, limit as i64],
            |row| session_json(row),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?,
    );
    Ok(current)
}

pub fn sessions(store: &Store, options: &SessionOptions) -> Result<Value> {
    let conn = store.open_readonly()?;
    let mut clauses = Vec::new();
    let mut params = Vec::new();
    if let Some(project) = &options.project {
        project_clause(&mut clauses, &mut params, project, "s");
    }
    if let Some(source) = &options.source {
        clauses.push("s.source = ?".to_string());
        params.push(SqlValue::Text(source.clone()));
    }
    if let Some(query) = &options.query {
        clauses.push("(s.title LIKE ? ESCAPE '\\' OR s.summary LIKE ? ESCAPE '\\')".to_string());
        let pattern = format!("%{}%", escape_like(query));
        params.push(SqlValue::Text(pattern.clone()));
        params.push(SqlValue::Text(pattern));
    }
    if let Some(after) = &options.after {
        clauses.push("COALESCE(s.ended_at, s.started_at) >= ?".to_string());
        params.push(SqlValue::Text(
            normalize_ts_str(after).context("invalid --after timestamp")?,
        ));
    }
    if let Some(before) = &options.before {
        clauses.push("COALESCE(s.ended_at, s.started_at) <= ?".to_string());
        params.push(SqlValue::Text(
            normalize_ts_str(before).context("invalid --before timestamp")?,
        ));
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    params.push(SqlValue::Integer(options.limit.clamp(1, 200) as i64));
    params.push(SqlValue::Integer(options.offset.max(0) as i64));
    let rows = session_rows(
        &conn,
        &format!(
            "SELECT s.id, s.source, s.title, s.project, s.project_path, s.started_at,
                    s.ended_at, s.git_branch, s.model, s.version, s.summary, s.message_count
             FROM sessions s {where_sql}
             ORDER BY COALESCE(s.ended_at, s.started_at) DESC, s.id DESC
             LIMIT ? OFFSET ?"
        ),
        params,
    )?;
    Ok(Value::Array(rows))
}

fn session_rows(conn: &Connection, sql: &str, params: Vec<SqlValue>) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), session_json)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn session_json(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "source": row.get::<_, String>(1)?,
        "title": row.get::<_, Option<String>>(2)?,
        "project": row.get::<_, Option<String>>(3)?,
        "project_path": row.get::<_, Option<String>>(4)?,
        "started_at": row.get::<_, Option<String>>(5)?,
        "ended_at": row.get::<_, Option<String>>(6)?,
        "git_branch": row.get::<_, Option<String>>(7)?,
        "model": row.get::<_, Option<String>>(8)?,
        "version": row.get::<_, Option<String>>(9)?,
        "summary": row.get::<_, Option<String>>(10)?,
        "message_count": row.get::<_, i64>(11)?,
    }))
}

pub fn show(
    store: &Store,
    session_id: &str,
    around: Option<&str>,
    before: usize,
    after: usize,
    limit: usize,
    include_tools: bool,
) -> Result<Value> {
    let conn = store.open_readonly()?;
    let session = find_session(&conn, session_id)?;
    let actual_id = session["id"].as_str().unwrap_or(session_id).to_string();
    let mut clauses = vec!["session_id = ?".to_string()];
    let mut params: Vec<SqlValue> = vec![SqlValue::Text(actual_id.clone())];
    if !include_tools {
        clauses.push("is_meta = 0 AND content_type = 'text'".to_string());
    }
    if let Some(around) = around {
        let anchor: Option<i64> = conn
            .query_row(
                "SELECT seq FROM messages WHERE session_id = ? AND uuid = ?",
                rusqlite::params![actual_id, around],
                |row| row.get(0),
            )
            .ok();
        let Some(anchor) = anchor else {
            anyhow::bail!("message {around:?} is not in session {actual_id}");
        };
        clauses.push("seq BETWEEN ? AND ?".to_string());
        params.push(SqlValue::Integer(anchor - before as i64));
        params.push(SqlValue::Integer(anchor + after as i64));
    }
    let limit = limit.clamp(1, 1000);
    params.push(SqlValue::Integer(limit as i64));
    let sql = format!(
        "SELECT * FROM messages WHERE {} ORDER BY seq LIMIT ?",
        clauses.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(params))?;
    let mut messages = Vec::new();
    while let Some(row) = rows.next()? {
        messages.push(message_json(&conn, row)?);
    }
    let count = messages.len();
    Ok(json!({
        "session": session,
        "messages": messages,
        "count": count,
        "truncated": count >= limit,
    }))
}

fn find_session(conn: &Connection, value: &str) -> Result<Value> {
    let exact = conn
        .query_row(
            "SELECT id, source, title, project, project_path, started_at, ended_at,
                    git_branch, model, version, summary, message_count
             FROM sessions WHERE id = ?",
            [value],
            session_json,
        )
        .ok();
    if let Some(session) = exact {
        return Ok(session);
    }
    let mut stmt = conn.prepare(
        "SELECT id, source, title, project, project_path, started_at, ended_at,
                git_branch, model, version, summary, message_count
         FROM sessions WHERE id LIKE ? ORDER BY COALESCE(ended_at, started_at) DESC LIMIT 2",
    )?;
    let rows = stmt
        .query_map([format!("{value}%")], session_json)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    match rows.as_slice() {
        [] => anyhow::bail!("no session matches {value:?}"),
        [session] => Ok(session.clone()),
        _ => anyhow::bail!(
            "session prefix {value:?} is ambiguous: {}",
            rows.iter()
                .filter_map(|row| row["id"].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn message_json(conn: &Connection, row: &rusqlite::Row<'_>) -> Result<Value> {
    let uuid: String = row.get("uuid")?;
    let content_type: String = row.get("content_type")?;
    let extra_text: Option<String> = row.get("extra_json")?;
    let extra: Option<Value> = extra_text
        .as_deref()
        .and_then(|text| serde_json::from_str(text).ok());
    let mut message = json!({
        "uuid": uuid,
        "seq": row.get::<_, i64>("seq")?,
        "timestamp": row.get::<_, Option<String>>("timestamp")?,
        "role": row.get::<_, Option<String>>("role")?,
        "content_type": content_type,
        "text": row.get::<_, Option<String>>("text")?,
        "is_meta": row.get::<_, i64>("is_meta")? != 0,
        "model": row.get::<_, Option<String>>("model")?,
        "input_tokens": row.get::<_, Option<i64>>("input_tokens")?,
        "output_tokens": row.get::<_, Option<i64>>("output_tokens")?,
        "cache_read_tokens": row.get::<_, Option<i64>>("cache_read_tokens")?,
        "cache_write_tokens": row.get::<_, Option<i64>>("cache_write_tokens")?,
        "agent_id": row.get::<_, Option<String>>("agent_id")?,
    });
    if let Some(extra) = &extra {
        message["extra"] = extra.clone();
    }
    if content_type == "tool_use" {
        if let Ok(call) = conn.query_row(
            "SELECT id, name, input_json, file_path FROM tool_calls WHERE message_uuid = ?",
            [&uuid],
            |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "name": row.get::<_, String>(1)?,
                    "input": serde_json::from_str::<Value>(&row.get::<_, String>(2)?).ok(),
                    "file_path": row.get::<_, Option<String>>(3)?,
                }))
            },
        ) {
            message["tool_call"] = call;
        }
    } else if content_type == "tool_result" {
        if let Some(call_id) = extra
            .as_ref()
            .and_then(|value| value.get("tool_call_id"))
            .and_then(Value::as_str)
        {
            if let Ok(call) = conn.query_row(
                "SELECT id, name, input_json, file_path FROM tool_calls WHERE id = ?",
                [call_id],
                |row| {
                    Ok(json!({
                        "id": row.get::<_, String>(0)?,
                        "name": row.get::<_, String>(1)?,
                        "input": serde_json::from_str::<Value>(&row.get::<_, String>(2)?).ok(),
                        "file_path": row.get::<_, Option<String>>(3)?,
                    }))
                },
            ) {
                message["tool_call"] = call;
            }
            message["is_error"] = json!(extra
                .as_ref()
                .and_then(|value| value.get("is_error"))
                .and_then(Value::as_bool)
                .unwrap_or(false));
        }
    }
    Ok(message)
}

fn project_clause(
    clauses: &mut Vec<String>,
    params: &mut Vec<SqlValue>,
    project: &str,
    alias: &str,
) {
    clauses.push(format!(
        "({alias}.project = ? OR {alias}.project_path = ? OR {alias}.project_path LIKE ? ESCAPE '\\')"
    ));
    params.push(SqlValue::Text(project.to_string()));
    params.push(SqlValue::Text(project.to_string()));
    params.push(SqlValue::Text(format!(
        "{}/%",
        escape_like(project.trim_end_matches('/'))
    )));
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[allow(dead_code)]
fn _unused(value: &str) -> Option<String> {
    Some(squeeze(value))
}

use crate::store::Store;
use anyhow::{Context, Result};
use rusqlite::hooks::{AuthAction, Authorization};
use rusqlite::types::Value as SqlValue;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const ALLOWED_PRAGMAS: &[&str] = &[
    "data_version",
    "table_info",
    "table_list",
    "database_list",
    "collation_list",
    "function_list",
    "module_list",
    "pragma_list",
    "index_list",
    "index_info",
    "foreign_key_list",
];

const DENIED_FUNCTIONS: &[&str] = &[
    "load_extension",
    "readfile",
    "writefile",
    "edit",
    "fts3_tokenizer",
    "zipfile",
];

pub fn run(
    store: &Store,
    sql: &str,
    params: &[Value],
    max_rows: usize,
    timeout_s: f64,
) -> Result<Value> {
    let sql = sql.trim();
    if sql.is_empty() {
        anyhow::bail!("empty SQL statement");
    }
    assert_single_statement(sql)?;
    assert_read_only_prefix(sql)?;
    let max_rows = max_rows.clamp(1, 2000);
    let started = Instant::now();
    let deadline = started + Duration::from_secs_f64(timeout_s.max(0.001));

    let mut conn = store.open_readonly()?;
    conn.authorizer(Some(
        move |ctx: rusqlite::hooks::AuthContext<'_>| match ctx.action {
            AuthAction::Select | AuthAction::Read { .. } | AuthAction::Recursive => {
                Authorization::Allow
            }
            AuthAction::Function { function_name } => {
                if DENIED_FUNCTIONS.contains(&function_name.to_ascii_lowercase().as_str()) {
                    Authorization::Deny
                } else {
                    Authorization::Allow
                }
            }
            AuthAction::Pragma { pragma_name, .. } => {
                if ALLOWED_PRAGMAS.contains(&pragma_name.to_ascii_lowercase().as_str()) {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            _ => Authorization::Deny,
        },
    ))?;
    conn.progress_handler(1000, Some(move || Instant::now() > deadline))?;

    let sql_values: Vec<SqlValue> = params.iter().map(json_to_sql).collect();
    let mut stmt = conn.prepare(sql).context("prepare SQL")?;
    let columns: Vec<String> = stmt
        .column_names()
        .into_iter()
        .map(ToString::to_string)
        .collect();
    let mut rows = stmt.query(rusqlite::params_from_iter(sql_values))?;
    let mut out = Vec::new();
    let mut truncated = false;
    while let Some(row) = rows.next()? {
        if out.len() >= max_rows {
            truncated = true;
            break;
        }
        let mut values = Vec::with_capacity(columns.len());
        for index in 0..columns.len() {
            values.push(value_ref_to_json(row.get_ref(index)?));
        }
        out.push(Value::Array(values));
    }
    Ok(json!({
        "columns": columns,
        "rows": out,
        "truncated": truncated,
        "row_count": out.len(),
        "elapsed_ms": started.elapsed().as_millis() as u64,
    }))
}

pub fn assert_read_only_prefix(sql: &str) -> Result<()> {
    let body = strip_literals(sql);
    let body = body.trim_start().to_ascii_lowercase();
    if !body.starts_with("select") && !body.starts_with("with") {
        anyhow::bail!("only read-only SELECT/WITH queries are allowed");
    }
    Ok(())
}

pub fn assert_single_statement(sql: &str) -> Result<()> {
    let body = strip_literals(sql);
    let body = body.trim();
    if body.is_empty() {
        anyhow::bail!("empty SQL statement");
    }
    let semicolons = body.matches(';').count();
    if semicolons > 1 || (semicolons == 1 && !body.ends_with(';')) {
        anyhow::bail!("exactly one SQL statement is allowed per call");
    }
    Ok(())
}

fn strip_literals(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let bytes = sql.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'-' && bytes.get(index + 1) == Some(&b'-') {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            out.push(' ');
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            let quote = byte;
            index += 1;
            while index < bytes.len() {
                if bytes[index] == quote {
                    if bytes.get(index + 1) == Some(&quote) {
                        index += 2;
                        continue;
                    }
                    index += 1;
                    break;
                }
                index += 1;
            }
            out.push(' ');
            continue;
        }
        if byte == b'[' {
            index += 1;
            while index < bytes.len() && bytes[index] != b']' {
                index += 1;
            }
            index = (index + 1).min(bytes.len());
            out.push(' ');
            continue;
        }
        out.push(byte as char);
        index += 1;
    }
    out
}

fn json_to_sql(value: &Value) -> SqlValue {
    match value {
        Value::Null => SqlValue::Null,
        Value::Bool(value) => SqlValue::Integer(*value as i64),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                SqlValue::Integer(value)
            } else {
                SqlValue::Real(number.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(value) => SqlValue::Text(value.clone()),
        _ => SqlValue::Text(value.to_string()),
    }
}

fn value_ref_to_json(value: rusqlite::types::ValueRef<'_>) -> Value {
    match value {
        rusqlite::types::ValueRef::Null => Value::Null,
        rusqlite::types::ValueRef::Integer(value) => json!(value),
        rusqlite::types::ValueRef::Real(value) => json!(value),
        rusqlite::types::ValueRef::Text(value) => {
            Value::String(String::from_utf8_lossy(value).to_string())
        }
        rusqlite::types::ValueRef::Blob(value) => Value::String(hex(value)),
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[allow(dead_code)]
fn _unused(_conn: &Connection) {}

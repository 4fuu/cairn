use crate::store;
use crate::store::Store;
use crate::util::{expand_content_types, normalize_ts_str, normalized_tool_names, snippet};
use anyhow::{Context, Result};
use rusqlite::types::Value as SqlValue;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct SearchOptions {
    pub project: Option<String>,
    pub session: Option<String>,
    pub source: Option<String>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub limit: usize,
    pub context: usize,
    pub include_meta: bool,
    pub full_text: bool,
    pub all: bool,
    pub content_types: Vec<String>,
    pub tool_names: Vec<String>,
}

#[derive(Clone, Debug)]
struct Hit {
    uuid: String,
    session_id: String,
    session_title: Option<String>,
    project: Option<String>,
    source: Option<String>,
    timestamp: Option<String>,
    role: Option<String>,
    content_type: Option<String>,
    text: String,
    snippet: Option<String>,
    score: f64,
    priority: u8,
    seq: i64,
}

pub fn search(store: &Store, query: &str, options: &SearchOptions) -> Result<Value> {
    let query = query.trim();
    if query.is_empty() {
        anyhow::bail!("search query is empty");
    }
    let limit = options.limit.clamp(1, 200);
    let terms: Vec<String> = store::segment(query)
        .split_whitespace()
        .map(ToString::to_string)
        .collect();
    let mut hits: HashMap<String, Hit> = HashMap::new();
    let conn = store.open_readonly()?;

    let (clauses, params) = filters(options)?;
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!(" AND {}", clauses.join(" AND "))
    };

    if query.chars().count() >= 3 {
        if let Ok(rows) = fts_rows(
            &conn,
            "messages_trigram",
            &phrase(query),
            &where_sql,
            &params,
            limit * 4,
        ) {
            for hit in rows {
                merge_hit(&mut hits, hit, query, &terms, options.full_text);
            }
        }
    }
    if !terms.is_empty() {
        let match_query = terms
            .iter()
            .map(|term| phrase(term))
            .collect::<Vec<_>>()
            .join(" AND ");
        if let Ok(rows) = fts_rows(
            &conn,
            "messages_fts",
            &match_query,
            &where_sql,
            &params,
            limit * 4,
        ) {
            for hit in rows {
                merge_hit(&mut hits, hit, query, &terms, options.full_text);
            }
        }
    }
    if query.chars().count() < 3 || hits.is_empty() {
        if let Ok(rows) = like_rows(&conn, query, &where_sql, &params, limit * 4) {
            for hit in rows {
                merge_hit(&mut hits, hit, query, &terms, options.full_text);
            }
        }
    }

    let mut ranked: Vec<Hit> = hits.into_values().collect();
    ranked.sort_by(|a, b| {
        (a.priority, -a.score)
            .partial_cmp(&(b.priority, -b.score))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.timestamp.cmp(&a.timestamp))
    });

    if options.all {
        let selected: Vec<Hit> = ranked.into_iter().take(limit).collect();
        let mut output = Vec::new();
        for hit in selected {
            let mut value = hit_json(&hit, options.full_text);
            if options.context > 0 {
                value["context"] =
                    Value::Array(context(&conn, &hit.session_id, hit.seq, options.context)?);
            }
            output.push(value);
        }
        return Ok(json!({
            "query": query,
            "count": output.len(),
            "hits": output,
        }));
    }

    let counts = exact_counts(&conn, query, &terms, &where_sql, &params).unwrap_or_default();
    let mut groups: HashMap<String, (Hit, usize)> = HashMap::new();
    for hit in ranked {
        match groups.get_mut(&hit.session_id) {
            Some((best, count)) => {
                *count += 1;
                if better(&hit, best) {
                    *best = hit;
                }
            }
            None => {
                groups.insert(hit.session_id.clone(), (hit, 1));
            }
        }
    }
    let mut sessions: Vec<Value> = groups
        .into_values()
        .map(|(best, count)| {
            let mut value = hit_json(&best, options.full_text);
            let exact = counts.get(&best.session_id).copied().unwrap_or(count);
            value["hits"] = json!(exact);
            value["score"] =
                json!(-best.score + (count as f64).ln() * 0.25 + recency(&best.timestamp));
            if options.context > 0 {
                if let Ok(rows) = context(&conn, &best.session_id, best.seq, options.context) {
                    value["context"] = Value::Array(rows);
                }
            }
            value
        })
        .collect();
    sessions.sort_by(|a, b| {
        b["score"]
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&a["score"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    sessions.truncate(limit);
    Ok(json!({
        "query": query,
        "count": sessions.len(),
        "sessions": sessions,
    }))
}

fn exact_counts(
    conn: &Connection,
    query: &str,
    terms: &[String],
    where_sql: &str,
    params: &[SqlValue],
) -> Result<HashMap<String, usize>> {
    let mut subqueries = Vec::new();
    let mut values: Vec<SqlValue> = Vec::new();
    if query.chars().count() >= 3 {
        subqueries.push(format!(
            "SELECT m.session_id AS session_id, m.uuid AS uuid FROM messages_trigram
             JOIN messages m ON m.rowid = messages_trigram.rowid
             JOIN sessions s ON s.id = m.session_id
             WHERE messages_trigram MATCH ?{where_sql}"
        ));
        values.push(SqlValue::Text(phrase(query)));
        values.extend(params.iter().cloned());
    }
    if !terms.is_empty() {
        let match_query = terms
            .iter()
            .map(|term| phrase(term))
            .collect::<Vec<_>>()
            .join(" AND ");
        subqueries.push(format!(
            "SELECT m.session_id AS session_id, m.uuid AS uuid FROM messages_fts
             JOIN messages m ON m.rowid = messages_fts.rowid
             JOIN sessions s ON s.id = m.session_id
             WHERE messages_fts MATCH ?{where_sql}"
        ));
        values.push(SqlValue::Text(match_query));
        values.extend(params.iter().cloned());
    }
    if query.chars().count() < 3 {
        subqueries.push(format!(
            "SELECT m.session_id AS session_id, m.uuid AS uuid FROM messages m
             JOIN sessions s ON s.id = m.session_id
             WHERE m.text LIKE ? ESCAPE '\\'{where_sql}"
        ));
        values.push(SqlValue::Text(format!("%{}%", escape_like(query))));
        values.extend(params.iter().cloned());
    }
    if subqueries.is_empty() {
        return Ok(HashMap::new());
    }
    let sql = format!(
        "SELECT session_id, COUNT(*) FROM ({}) GROUP BY session_id",
        subqueries.join(" UNION ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(values), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
    })?;
    Ok(rows.collect::<rusqlite::Result<HashMap<_, _>>>()?)
}

fn better(a: &Hit, b: &Hit) -> bool {
    (a.priority, -a.score) < (b.priority, -b.score)
}

fn merge_hit(
    hits: &mut HashMap<String, Hit>,
    mut hit: Hit,
    query: &str,
    terms: &[String],
    full_text: bool,
) {
    let mut search_terms = terms.to_vec();
    search_terms.push(query.to_string());
    hit.snippet = snippet(&hit.text, &search_terms, 280);
    if !full_text {
        hit.text.clear();
    }
    match hits.get_mut(&hit.uuid) {
        Some(existing) if better(&hit, existing) => *existing = hit,
        Some(_) => {}
        None => {
            hits.insert(hit.uuid.clone(), hit);
        }
    }
}

fn fts_rows(
    conn: &Connection,
    table: &str,
    match_query: &str,
    where_sql: &str,
    params: &[SqlValue],
    limit: usize,
) -> Result<Vec<Hit>> {
    let sql = format!(
        "SELECT m.uuid, m.session_id, m.seq, m.timestamp, m.role, m.content_type, m.text,
                s.title, s.project, s.source, bm25({table}) AS score
         FROM {table}
         JOIN messages m ON m.rowid = {table}.rowid
         JOIN sessions s ON s.id = m.session_id
         WHERE {table} MATCH ?{where_sql}
         ORDER BY score LIMIT ?"
    );
    let mut values = vec![SqlValue::Text(match_query.to_string())];
    values.extend(params.iter().cloned());
    values.push(SqlValue::Integer(limit as i64));
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(values), |row| {
        Ok(Hit {
            uuid: row.get(0)?,
            session_id: row.get(1)?,
            seq: row.get(2)?,
            timestamp: row.get(3)?,
            role: row.get(4)?,
            content_type: row.get(5)?,
            text: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
            session_title: row.get(7)?,
            project: row.get(8)?,
            source: row.get(9)?,
            score: row.get::<_, f64>(10)?,
            snippet: None,
            priority: if table == "messages_trigram" { 0 } else { 1 },
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn like_rows(
    conn: &Connection,
    query: &str,
    where_sql: &str,
    params: &[SqlValue],
    limit: usize,
) -> Result<Vec<Hit>> {
    let pattern = format!("%{}%", escape_like(query));
    let sql = format!(
        "SELECT m.uuid, m.session_id, m.seq, m.timestamp, m.role, m.content_type, m.text,
                s.title, s.project, s.source, 0.0 AS score
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         WHERE m.text LIKE ? ESCAPE '\\'{where_sql}
         ORDER BY COALESCE(m.timestamp, '') DESC LIMIT ?"
    );
    let mut values = vec![SqlValue::Text(pattern)];
    values.extend(params.iter().cloned());
    values.push(SqlValue::Integer(limit as i64));
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(values), |row| {
        Ok(Hit {
            uuid: row.get(0)?,
            session_id: row.get(1)?,
            seq: row.get(2)?,
            timestamp: row.get(3)?,
            role: row.get(4)?,
            content_type: row.get(5)?,
            text: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
            session_title: row.get(7)?,
            project: row.get(8)?,
            source: row.get(9)?,
            score: 0.0,
            snippet: None,
            priority: 2,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn filters(options: &SearchOptions) -> Result<(Vec<String>, Vec<SqlValue>)> {
    let mut clauses = Vec::new();
    let mut params = Vec::new();
    if let Some(project) = &options.project {
        clauses.push(
            "(s.project = ? OR s.project_path = ? OR s.project_path LIKE ? ESCAPE '\\')"
                .to_string(),
        );
        params.push(SqlValue::Text(project.clone()));
        params.push(SqlValue::Text(project.clone()));
        params.push(SqlValue::Text(format!(
            "{}/%",
            escape_like(project.trim_end_matches('/'))
        )));
    }
    if let Some(session) = &options.session {
        clauses.push("m.session_id = ?".to_string());
        params.push(SqlValue::Text(session.clone()));
    }
    if let Some(source) = &options.source {
        clauses.push("s.source = ?".to_string());
        params.push(SqlValue::Text(source.clone()));
    }
    if let Some(after) = &options.after {
        let stamp = normalize_ts_str(after).context("invalid --after timestamp")?;
        clauses.push("m.timestamp >= ?".to_string());
        params.push(SqlValue::Text(stamp));
    }
    if let Some(before) = &options.before {
        let stamp = normalize_ts_str(before).context("invalid --before timestamp")?;
        clauses.push("m.timestamp <= ?".to_string());
        params.push(SqlValue::Text(stamp));
    }
    let content_types = expand_content_types(&options.content_types);
    let tool_names = normalized_tool_names(&options.tool_names);
    if !content_types.is_empty() {
        let marks = vec!["?"; content_types.len()].join(", ");
        clauses.push(format!("m.content_type IN ({marks})"));
        for value in content_types {
            params.push(SqlValue::Text(value));
        }
    } else if tool_names.is_empty() && !options.include_meta {
        clauses.push("m.is_meta = 0 AND m.content_type = 'text'".to_string());
    }
    if !tool_names.is_empty() {
        let marks = vec!["?"; tool_names.len()].join(", ");
        clauses.push(format!(
            "(EXISTS (SELECT 1 FROM tool_calls tc
                      WHERE tc.message_uuid = m.uuid
                        AND LOWER(tc.name) IN ({marks}))
              OR EXISTS (SELECT 1 FROM tool_calls tc
                         JOIN tool_results tr ON tr.tool_call_id = tc.id
                         WHERE tr.message_uuid = m.uuid
                           AND LOWER(tc.name) IN ({marks})))"
        ));
        for _ in 0..2 {
            for name in &tool_names {
                params.push(SqlValue::Text(name.clone()));
            }
        }
    }
    Ok((clauses, params))
}

fn hit_json(hit: &Hit, full_text: bool) -> Value {
    let mut value = json!({
        "session_id": hit.session_id,
        "session_title": hit.session_title,
        "project": hit.project,
        "source": hit.source,
        "uuid": hit.uuid,
        "seq": hit.seq,
        "timestamp": hit.timestamp,
        "role": hit.role,
        "content_type": hit.content_type,
        "snippet": hit.snippet,
        "score": -hit.score + recency(&hit.timestamp),
    });
    if full_text {
        value["text"] = json!(hit.text);
    }
    value
}

fn context(conn: &Connection, session_id: &str, seq: i64, radius: usize) -> Result<Vec<Value>> {
    let radius = radius.clamp(0, 6) as i64;
    let mut stmt = conn.prepare(
        "SELECT uuid, seq, timestamp, role, content_type, text, is_meta
         FROM messages WHERE session_id = ? AND seq BETWEEN ? AND ? AND seq != ?
         ORDER BY seq",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![session_id, seq - radius, seq + radius, seq],
        |row| {
            Ok(json!({
                "uuid": row.get::<_, String>(0)?,
                "seq": row.get::<_, i64>(1)?,
                "timestamp": row.get::<_, Option<String>>(2)?,
                "role": row.get::<_, Option<String>>(3)?,
                "content_type": row.get::<_, Option<String>>(4)?,
                "text": snippet(&row.get::<_, Option<String>>(5)?.unwrap_or_default(), &[], 400),
                "is_meta": row.get::<_, i64>(6)? != 0,
            }))
        },
    )?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn phrase(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn recency(timestamp: &Option<String>) -> f64 {
    let Some(timestamp) = timestamp else {
        return 0.0;
    };
    let Ok(dt) = chrono::DateTime::parse_from_rfc3339(timestamp) else {
        return 0.0;
    };
    let days = (chrono::Utc::now() - dt.with_timezone(&chrono::Utc))
        .num_days()
        .max(0) as f64;
    0.5 / (1.0 + days / 30.0)
}

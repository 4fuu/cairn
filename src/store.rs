use crate::model::ParsedSession;
use crate::util::{clip, now_ts, stable_id, MAX_TEXT};
use anyhow::{Context, Result};
use jieba_rs::Jieba;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS index_meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS source_files (
  path        TEXT PRIMARY KEY,
  source      TEXT NOT NULL,
  session_id  TEXT,
  mtime_ns    INTEGER NOT NULL,
  size        INTEGER NOT NULL,
  prefix_hash TEXT,
  indexed_at  TEXT NOT NULL,
  error       TEXT
);

CREATE TABLE IF NOT EXISTS sessions (
  id            TEXT PRIMARY KEY,
  source        TEXT NOT NULL,
  title         TEXT,
  project       TEXT,
  project_path  TEXT,
  started_at    TEXT,
  ended_at      TEXT,
  git_branch    TEXT,
  model         TEXT,
  version       TEXT,
  summary       TEXT,
  message_count INTEGER NOT NULL DEFAULT 0,
  source_path   TEXT,
  meta_json     TEXT,
  indexed_at    TEXT
);
CREATE INDEX IF NOT EXISTS sessions_order ON sessions(COALESCE(ended_at, started_at) DESC, id DESC);
CREATE INDEX IF NOT EXISTS sessions_project ON sessions(project, started_at);
CREATE INDEX IF NOT EXISTS sessions_source ON sessions(source);
CREATE INDEX IF NOT EXISTS sessions_path ON sessions(source_path);

CREATE TABLE IF NOT EXISTS messages (
  uuid              TEXT PRIMARY KEY,
  session_id        TEXT NOT NULL,
  parent_uuid       TEXT,
  seq               INTEGER NOT NULL,
  timestamp         TEXT,
  role              TEXT,
  content_type      TEXT NOT NULL DEFAULT 'text',
  text              TEXT,
  text_words        TEXT,
  is_meta           INTEGER NOT NULL DEFAULT 0,
  model             TEXT,
  input_tokens      INTEGER,
  output_tokens     INTEGER,
  cache_read_tokens INTEGER,
  cache_write_tokens INTEGER,
  agent_id          TEXT,
  source            TEXT,
  source_path       TEXT,
  extra_json        TEXT
);
CREATE INDEX IF NOT EXISTS messages_session_order ON messages(session_id, seq);
CREATE INDEX IF NOT EXISTS messages_time ON messages(timestamp);
CREATE INDEX IF NOT EXISTS messages_role ON messages(session_id, role);
CREATE INDEX IF NOT EXISTS messages_path ON messages(source_path);
CREATE INDEX IF NOT EXISTS messages_agent ON messages(agent_id);
CREATE INDEX IF NOT EXISTS messages_type ON messages(content_type);

CREATE TABLE IF NOT EXISTS tool_calls (
  id           TEXT PRIMARY KEY,
  session_id   TEXT NOT NULL,
  message_uuid TEXT,
  name         TEXT,
  input_json   TEXT,
  file_path    TEXT,
  timestamp    TEXT,
  source       TEXT,
  source_path  TEXT
);
CREATE INDEX IF NOT EXISTS tool_calls_session ON tool_calls(session_id, timestamp);
CREATE INDEX IF NOT EXISTS tool_calls_name ON tool_calls(name);
CREATE INDEX IF NOT EXISTS tool_calls_file ON tool_calls(file_path);
CREATE INDEX IF NOT EXISTS tool_calls_path ON tool_calls(source_path);

CREATE TABLE IF NOT EXISTS tool_results (
  tool_call_id TEXT PRIMARY KEY,
  session_id   TEXT NOT NULL,
  message_uuid TEXT,
  content      TEXT,
  is_error     INTEGER NOT NULL DEFAULT 0,
  timestamp    TEXT,
  source       TEXT,
  source_path  TEXT
);
CREATE INDEX IF NOT EXISTS tool_results_session ON tool_results(session_id);
CREATE INDEX IF NOT EXISTS tool_results_error ON tool_results(is_error) WHERE is_error = 1;
CREATE INDEX IF NOT EXISTS tool_results_path ON tool_results(source_path);

CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
  text_words,
  content='messages',
  content_rowid='rowid',
  tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS messages_fts_ai AFTER INSERT ON messages BEGIN
  INSERT INTO messages_fts(rowid, text_words) VALUES (new.rowid, new.text_words);
END;
CREATE TRIGGER IF NOT EXISTS messages_fts_ad AFTER DELETE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, text_words) VALUES ('delete', old.rowid, old.text_words);
END;
CREATE TRIGGER IF NOT EXISTS messages_fts_au AFTER UPDATE OF text_words ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, text_words) VALUES ('delete', old.rowid, old.text_words);
  INSERT INTO messages_fts(rowid, text_words) VALUES (new.rowid, new.text_words);
END;

CREATE VIRTUAL TABLE IF NOT EXISTS messages_trigram USING fts5(
  text,
  content='messages',
  content_rowid='rowid',
  tokenize='trigram'
);

CREATE TRIGGER IF NOT EXISTS messages_trigram_ai AFTER INSERT ON messages BEGIN
  INSERT INTO messages_trigram(rowid, text) VALUES (new.rowid, new.text);
END;
CREATE TRIGGER IF NOT EXISTS messages_trigram_ad AFTER DELETE ON messages BEGIN
  INSERT INTO messages_trigram(messages_trigram, rowid, text) VALUES ('delete', old.rowid, old.text);
END;
CREATE TRIGGER IF NOT EXISTS messages_trigram_au AFTER UPDATE OF text ON messages BEGIN
  INSERT INTO messages_trigram(messages_trigram, rowid, text) VALUES ('delete', old.rowid, old.text);
  INSERT INTO messages_trigram(rowid, text) VALUES (new.rowid, new.text);
END;
"#;

pub fn default_db_path() -> PathBuf {
    if let Ok(home) = std::env::var("CAIRN_HOME") {
        return PathBuf::from(shellexpand(&home)).join("cairn.sqlite");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cairn")
        .join("cairn.sqlite")
}

fn shellexpand(value: &str) -> String {
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    value.to_string()
}

pub struct Store {
    pub path: PathBuf,
}

impl Store {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path: path.unwrap_or_else(default_db_path),
        }
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    pub fn open_readonly(&self) -> Result<Connection> {
        let uri = format!("file:{}?mode=ro", self.path.to_string_lossy());
        let conn = Connection::open_with_flags(
            uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .with_context(|| format!("open {}", self.path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        conn.execute_batch("PRAGMA query_only=ON;")?;
        Ok(conn)
    }

    pub fn open_write(&self) -> Result<Connection> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&self.path)
            .with_context(|| format!("open {}", self.path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(15))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;",
        )?;
        Ok(conn)
    }

    pub fn init(&self) -> Result<()> {
        let conn = self.open_write()?;
        conn.execute_batch(SCHEMA)?;
        let _ = conn.execute("ALTER TABLE source_files ADD COLUMN prefix_hash TEXT", []);
        conn.execute(
            "INSERT INTO index_meta(key, value) VALUES ('schema_version', ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![SCHEMA_VERSION.to_string()],
        )?;
        conn.execute(
            "INSERT INTO index_meta(key, value) VALUES ('trigram', '1')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )?;
        Ok(())
    }

    pub fn is_initialized(&self) -> bool {
        if !self.exists() {
            return false;
        }
        let Ok(conn) = self.open_readonly() else {
            return false;
        };
        conn.query_row(
            "SELECT value FROM index_meta WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
        .is_some_and(|value| value == SCHEMA_VERSION.to_string())
    }

    pub fn has_trigram(&self) -> bool {
        let Ok(conn) = self.open_readonly() else {
            return false;
        };
        conn.query_row(
            "SELECT value FROM index_meta WHERE key = 'trigram'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
        .is_some_and(|value| value == "1")
    }

    pub fn stats(&self) -> Result<serde_json::Value> {
        let conn = self.open_readonly()?;
        let sessions: i64 =
            conn.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;
        let messages: i64 = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE is_meta = 0",
            [],
            |row| row.get(0),
        )?;
        let tool_calls: i64 =
            conn.query_row("SELECT COUNT(*) FROM tool_calls", [], |row| row.get(0))?;
        let projects: i64 = conn.query_row(
            "SELECT COUNT(DISTINCT project) FROM sessions WHERE project IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        let last: Option<String> =
            conn.query_row("SELECT MAX(indexed_at) FROM sessions", [], |row| row.get(0))?;
        let mut stmt = conn.prepare(
            "SELECT source, COUNT(*) AS sessions, COALESCE(SUM(message_count), 0) AS messages
             FROM sessions GROUP BY source ORDER BY source",
        )?;
        let sources = stmt
            .query_map([], |row| {
                Ok(serde_json::json!({
                    "source": row.get::<_, String>(0)?,
                    "sessions": row.get::<_, i64>(1)?,
                    "messages": row.get::<_, i64>(2)?,
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(serde_json::json!({
            "db": self.path,
            "sessions": sessions,
            "messages": messages,
            "tool_calls": tool_calls,
            "projects": projects,
            "last_indexed": last,
            "trigram": self.has_trigram(),
            "sources": sources,
        }))
    }

    pub fn write_session(
        &self,
        conn: &Connection,
        parsed: &mut ParsedSession,
        mtime_ns: i64,
        size: i64,
    ) -> Result<String> {
        let ids = self.write_many(
            conn,
            std::slice::from_mut(parsed),
            mtime_ns,
            size,
            false,
            "",
        )?;
        Ok(ids.into_iter().next().unwrap_or_default())
    }

    pub fn write_many(
        &self,
        conn: &Connection,
        sessions: &mut [ParsedSession],
        mtime_ns: i64,
        size: i64,
        append: bool,
        prefix_hash: &str,
    ) -> Result<Vec<String>> {
        let Some(first) = sessions.first() else {
            return Ok(Vec::new());
        };
        let source_path = first.session.source_path.clone();
        let source = first.session.source.clone();
        if !append {
            delete_source_path(conn, &source_path)?;
        }
        let now = now_ts();
        let mut ids = Vec::new();
        for parsed in sessions.iter_mut() {
            ids.push(self.insert_session(conn, parsed, append)?);
        }
        conn.execute(
            "INSERT INTO source_files(path, source, session_id, mtime_ns, size, prefix_hash, indexed_at, error)
             VALUES (?, ?, ?, ?, ?, ?, ?, NULL)
             ON CONFLICT(path) DO UPDATE SET
               source = excluded.source,
               session_id = excluded.session_id,
               mtime_ns = excluded.mtime_ns,
               size = excluded.size,
               prefix_hash = excluded.prefix_hash,
               indexed_at = excluded.indexed_at,
               error = NULL",
            params![source_path, source, ids.first(), mtime_ns, size, prefix_hash, now],
        )?;
        Ok(ids)
    }

    fn insert_session(
        &self,
        conn: &Connection,
        parsed: &mut ParsedSession,
        append: bool,
    ) -> Result<String> {
        let session = &mut parsed.session;
        session.id = resolve_session_id(conn, &session.id, &session.source_path)?;
        let session_id = session.id.clone();
        let source_path = session.source_path.clone();
        let now = now_ts();
        let existing: std::collections::HashSet<String> = if append {
            let mut stmt = conn.prepare("SELECT uuid FROM messages WHERE source_path = ?")?;
            let rows = stmt.query_map([&source_path], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        } else {
            std::collections::HashSet::new()
        };

        conn.execute(
            "INSERT INTO sessions (
               id, source, title, project, project_path, started_at, ended_at,
               git_branch, model, version, summary, message_count, source_path,
               meta_json, indexed_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               source = excluded.source,
               title = COALESCE(excluded.title, sessions.title),
               project = COALESCE(excluded.project, sessions.project),
               project_path = COALESCE(excluded.project_path, sessions.project_path),
               git_branch = COALESCE(excluded.git_branch, sessions.git_branch),
               model = COALESCE(excluded.model, sessions.model),
               version = COALESCE(excluded.version, sessions.version),
               summary = COALESCE(excluded.summary, sessions.summary),
               source_path = COALESCE(sessions.source_path, excluded.source_path),
               meta_json = COALESCE(excluded.meta_json, sessions.meta_json),
               indexed_at = excluded.indexed_at",
            params![
                session_id,
                session.source,
                session.title,
                session.project,
                session.project_path,
                session.started_at,
                session.ended_at,
                session.git_branch,
                session.model,
                session.version,
                session.summary,
                source_path,
                session.meta_json,
                now
            ],
        )?;

        let mut insert_message = conn.prepare_cached(
            "INSERT OR IGNORE INTO messages(
               uuid, session_id, parent_uuid, seq, timestamp, role, content_type,
               text, text_words, is_meta, model, input_tokens, output_tokens,
               cache_read_tokens, cache_write_tokens, agent_id, source, source_path,
               extra_json
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )?;
        for message in &parsed.messages {
            if existing.contains(&message.uuid) {
                continue;
            }
            let (text, _) = match &message.text {
                Some(text) => {
                    let (clipped, truncated) = clip(text, MAX_TEXT);
                    (Some(clipped), truncated)
                }
                None => (None, false),
            };
            let words = text.as_deref().map(segment);
            insert_message.execute(params![
                message.uuid,
                session_id,
                message.parent_uuid,
                message.seq,
                message.timestamp,
                message.role,
                message.content_type,
                text,
                words,
                message.is_meta as i64,
                message.model,
                message.input_tokens,
                message.output_tokens,
                message.cache_read_tokens,
                message.cache_write_tokens,
                message.agent_id,
                session.source,
                source_path,
                message.extra_json
            ])?;
        }

        let mut insert_call = conn.prepare_cached(
            "INSERT OR IGNORE INTO tool_calls(
               id, session_id, message_uuid, name, input_json, file_path,
               timestamp, source, source_path
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )?;
        for call in &parsed.tool_calls {
            insert_call.execute(params![
                call.id,
                session_id,
                call.message_uuid,
                call.name,
                call.input_json,
                call.file_path,
                call.timestamp,
                session.source,
                source_path
            ])?;
        }

        let mut insert_result = conn.prepare_cached(
            "INSERT OR IGNORE INTO tool_results(
               tool_call_id, session_id, message_uuid, content, is_error,
               timestamp, source, source_path
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )?;
        for result in &parsed.tool_results {
            insert_result.execute(params![
                result.tool_call_id,
                session_id,
                result.message_uuid,
                result.content,
                result.is_error as i64,
                result.timestamp,
                session.source,
                source_path
            ])?;
        }

        refresh_session(conn, &session_id)?;
        Ok(session_id)
    }
}

pub fn resolve_session_id(conn: &Connection, raw: &str, source_path: &str) -> Result<String> {
    let existing: Option<Option<String>> = conn
        .query_row(
            "SELECT source_path FROM sessions WHERE id = ?",
            params![raw],
            |row| row.get(0),
        )
        .optional()?;
    match existing {
        Some(Some(path)) if path != source_path => {
            Ok(format!("{raw}#{}", &stable_id(&[source_path])[..8]))
        }
        _ => Ok(raw.to_string()),
    }
}

pub fn delete_source_path(conn: &Connection, path: &str) -> Result<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT DISTINCT session_id FROM messages WHERE source_path = ?")?;
    let sessions: Vec<String> = stmt
        .query_map(params![path], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    conn.execute("DELETE FROM messages WHERE source_path = ?", params![path])?;
    conn.execute(
        "DELETE FROM tool_calls WHERE source_path = ?",
        params![path],
    )?;
    conn.execute(
        "DELETE FROM tool_results WHERE source_path = ?",
        params![path],
    )?;
    conn.execute("DELETE FROM source_files WHERE path = ?", params![path])?;
    for session in &sessions {
        refresh_session(conn, session)?;
    }
    Ok(sessions)
}

pub fn refresh_session(conn: &Connection, session_id: &str) -> Result<()> {
    let (start, end, total, visible, model): (
        Option<String>,
        Option<String>,
        i64,
        i64,
        Option<String>,
    ) = conn.query_row(
        "SELECT MIN(timestamp), MAX(timestamp), COUNT(*),
                COALESCE(SUM(CASE WHEN is_meta = 0 THEN 1 ELSE 0 END), 0), MAX(model)
         FROM messages WHERE session_id = ?",
        params![session_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    if total == 0 {
        conn.execute("DELETE FROM sessions WHERE id = ?", params![session_id])?;
        return Ok(());
    }
    let first_user: Option<String> = conn
        .query_row(
            "SELECT text FROM messages WHERE session_id = ? AND role = 'user'
             AND is_meta = 0 AND text IS NOT NULL ORDER BY seq LIMIT 1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?;
    let title = first_user.and_then(|text| {
        crate::util::is_prompt_text(&text)
            .then(|| title_from(&text))
            .flatten()
    });
    conn.execute(
        "UPDATE sessions SET
           started_at = COALESCE(?, started_at),
           ended_at = COALESCE(?, ended_at),
           message_count = ?,
           title = COALESCE(title, ?),
           model = COALESCE(model, ?)
         WHERE id = ?",
        params![start, end, visible, title, model, session_id],
    )?;
    Ok(())
}

fn title_from(text: &str) -> Option<String> {
    let flat = crate::util::squeeze(text);
    if flat.is_empty() {
        return None;
    }
    Some(flat.chars().take(160).collect())
}

pub fn segment(text: &str) -> String {
    static JIEBA: OnceLock<Jieba> = OnceLock::new();
    let jieba = JIEBA.get_or_init(Jieba::new);
    jieba
        .cut_for_search(text, true)
        .into_iter()
        .map(|token| token.word)
        .filter(|word| !word.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn session_files(conn: &Connection) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT path FROM source_files")?;
    let paths = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    Ok(paths)
}

pub fn write_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO index_meta(key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn get_meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT value FROM index_meta WHERE key = ?",
            params![key],
            |row| row.get(0),
        )
        .optional()?)
}

pub fn clear_all(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM messages", [])?;
    conn.execute("DELETE FROM tool_calls", [])?;
    conn.execute("DELETE FROM tool_results", [])?;
    conn.execute("DELETE FROM sessions", [])?;
    conn.execute("DELETE FROM source_files", [])?;
    Ok(())
}

pub fn row_exists(conn: &Connection, sql: &str, value: &str) -> Result<bool> {
    Ok(conn
        .query_row(sql, params![value], |_| Ok(()))
        .optional()?
        .is_some())
}

pub fn session_source_path(conn: &Connection, id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT source_path FROM sessions WHERE id = ?",
            params![id],
            |row| row.get(0),
        )
        .optional()?
        .flatten())
}

pub fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

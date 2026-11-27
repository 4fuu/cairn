use cairn::indexer::build;
use cairn::providers::{Format, Root};
use cairn::query::{show, SessionOptions};
use cairn::search::{search, SearchOptions};
use cairn::sqlguard;
use cairn::store::Store;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

struct Fixture {
    _tmp: TempDir,
    store: Store,
    roots: Vec<Root>,
    db: PathBuf,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let roots = vec![
        Root {
            source: "claude".into(),
            path: base.join("claude"),
            format: Format::Claude,
        },
        Root {
            source: "codex".into(),
            path: base.join("codex"),
            format: Format::Codex,
        },
        Root {
            source: "pi".into(),
            path: base.join("pi"),
            format: Format::Pi,
        },
    ];
    let db = tmp.path().join("cairn.sqlite");
    let store = Store::new(Some(db.clone()));
    build(&store, &roots, None, true).unwrap();
    Fixture {
        _tmp: tmp,
        store,
        roots,
        db,
    }
}

#[test]
fn indexes_three_sources() {
    let fixture = fixture();
    let report = build(&fixture.store, &fixture.roots, None, true).unwrap();
    assert_eq!(report.sessions, 3);
    assert_eq!(report.messages, 11);
    let stats = fixture.store.stats().unwrap();
    assert_eq!(stats["tool_calls"], 3);
    assert_eq!(stats["trigram"], true);
}

#[test]
fn search_three_layers() {
    let fixture = fixture();
    let zh = search(&fixture.store, "重定向", &SearchOptions::default()).unwrap();
    assert_eq!(zh["count"], 1);
    assert!(zh["sessions"][0]["snippet"]
        .as_str()
        .unwrap()
        .contains("重定向"));
    let two = search(&fixture.store, "登录", &SearchOptions::default()).unwrap();
    assert_eq!(two["count"], 1);
    let english = search(&fixture.store, "rounding", &SearchOptions::default()).unwrap();
    assert_eq!(english["sessions"][0]["session_id"], "codex:codex-demo-1");
    let identifier = search(&fixture.store, "Date.now", &SearchOptions::default()).unwrap();
    assert_eq!(identifier["sessions"][0]["session_id"], "pi:pi-demo-1");
}

#[test]
fn search_scope_and_meta() {
    let fixture = fixture();
    let options = SearchOptions {
        project: Some("demo".into()),
        ..Default::default()
    };
    assert_eq!(
        search(&fixture.store, "redirect", &options).unwrap()["count"],
        1
    );
    let meta = search(
        &fixture.store,
        "Check the cache key",
        &SearchOptions::default(),
    )
    .unwrap();
    assert_eq!(meta["count"], 0);
    let with_meta = SearchOptions {
        include_meta: true,
        ..Default::default()
    };
    assert_eq!(
        search(&fixture.store, "Check the cache key", &with_meta).unwrap()["count"],
        1
    );
}

#[test]
fn show_default_and_tools() {
    let fixture = fixture();
    let plain = show(&fixture.store, "claude-demo-1", None, 0, 0, 200, false).unwrap();
    assert_eq!(plain["count"], 3);
    let tools = show(&fixture.store, "claude-demo-1", None, 0, 0, 200, true).unwrap();
    assert_eq!(tools["count"], 6);
    let call = tools["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["content_type"] == "tool_use")
        .unwrap();
    assert_eq!(call["tool_call"]["name"], "Read");
}

#[test]
fn sessions_filters() {
    let fixture = fixture();
    let rows = cairn::query::sessions(
        &fixture.store,
        &SessionOptions {
            project: Some("demo".into()),
            limit: 10,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["id"], "claude-demo-1");
}

#[test]
fn sql_guard() {
    let fixture = fixture();
    assert_eq!(
        sqlguard::run(
            &fixture.store,
            "SELECT COUNT(*) FROM sessions",
            &[],
            10,
            3.0
        )
        .unwrap()["rows"][0][0],
        3
    );
    assert!(sqlguard::run(
        &fixture.store,
        "INSERT INTO sessions(id, source) VALUES ('x','y')",
        &[],
        10,
        3.0
    )
    .is_err());
    assert!(sqlguard::run(&fixture.store, "PRAGMA table_info(sessions)", &[], 10, 3.0).is_err());
    assert!(sqlguard::run(&fixture.store, "SELECT 1; SELECT 2", &[], 10, 3.0).is_err());
    assert!(sqlguard::run(&fixture.store, "SELECT load_extension('x')", &[], 10, 3.0).is_err());
    let capped = sqlguard::run(&fixture.store, "SELECT id FROM sessions", &[], 1, 3.0).unwrap();
    assert_eq!(capped["truncated"], true);
    assert_eq!(capped["row_count"], 1);
}

#[test]
fn incremental_skips_and_prunes() {
    let fixture = fixture();
    let second = build(&fixture.store, &fixture.roots, None, false).unwrap();
    assert_eq!(second.skipped, 3);
    assert_eq!(second.indexed, 0);
    assert!(fixture.db.exists());
}

#[test]
fn append_update_is_incremental() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("claude");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude/session1.jsonl"),
        source.join("session1.jsonl"),
    )
    .unwrap();
    let roots = vec![Root {
        source: "claude".into(),
        path: source.clone(),
        format: Format::Claude,
    }];
    let store = Store::new(Some(tmp.path().join("cairn.sqlite")));
    assert_eq!(build(&store, &roots, None, true).unwrap().messages, 4);

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(source.join("session1.jsonl"))
        .unwrap();
    use std::io::Write;
    writeln!(
        file,
        r#"{{"parentUuid":"a2","type":"user","message":{{"role":"user","content":"append me"}},"uuid":"u9","timestamp":"2026-11-01T10:02:00.000Z","cwd":"/home/user/demo","sessionId":"claude-demo-1"}}"#
    )
    .unwrap();
    let report = build(&store, &roots, None, false).unwrap();
    assert_eq!(report.indexed, 1);
    assert_eq!(report.messages, 5);
    let hits = search(&store, "append me", &SearchOptions::default()).unwrap();
    assert_eq!(hits["count"], 1);
    let stats = store.stats().unwrap();
    assert_eq!(stats["sessions"], 1);
}

#[test]
fn indexes_dimagent_sqlite() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("dimcode.sqlite");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE sessions (
            sessionId TEXT PRIMARY KEY,
            cwd TEXT NOT NULL DEFAULT '',
            title TEXT,
            status TEXT NOT NULL,
            tags TEXT,
            heldBy TEXT,
            heldAt TEXT,
            createdAt TEXT NOT NULL,
            updatedAt TEXT NOT NULL,
            version INTEGER NOT NULL
        );
        CREATE TABLE messages (
            messageId TEXT PRIMARY KEY,
            sessionId TEXT NOT NULL,
            role TEXT NOT NULL,
            parts TEXT NOT NULL,
            attachments TEXT,
            toolMetadata TEXT,
            metadata TEXT,
            orderKey TEXT NOT NULL,
            createdAt TEXT NOT NULL,
            updatedAt TEXT NOT NULL
        );
        CREATE TABLE session_states (
            sessionId TEXT PRIMARY KEY,
            selectedProviderId TEXT,
            selectedModelId TEXT,
            insertionCursor TEXT,
            settings TEXT,
            permissionSettings TEXT,
            currentMode TEXT,
            version INTEGER NOT NULL
        );
        CREATE TABLE usage_ledger (
            ledgerId TEXT PRIMARY KEY,
            sessionId TEXT NOT NULL,
            runId TEXT,
            providerId TEXT NOT NULL,
            modelId TEXT NOT NULL,
            usage TEXT NOT NULL,
            cost REAL,
            createdAt TEXT NOT NULL
        );
        CREATE TABLE usage_run_stats (
            runId TEXT PRIMARY KEY,
            sessionId TEXT NOT NULL,
            inputTokens INTEGER NOT NULL,
            outputTokens INTEGER NOT NULL,
            cacheReadTokens INTEGER,
            cacheWriteTokens INTEGER
        );
        "#,
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sessions
         (sessionId, cwd, title, status, createdAt, updatedAt, version)
         VALUES (?1, ?2, ?3, 'active', ?4, ?5, 1)",
        params![
            "sess_demo",
            "/home/user/demo",
            "DimAgent demo",
            "2026-11-27T10:00:00.000Z",
            "2026-11-27T10:01:00.000Z"
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session_states
         (sessionId, selectedProviderId, selectedModelId, version)
         VALUES ('sess_demo', 'dim', 'dim-model', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO usage_ledger
         (ledgerId, sessionId, providerId, modelId, usage, createdAt)
         VALUES ('usage_1', 'sess_demo', 'dim', 'dim-model', ?1, ?2)",
        params![
            r#"{"promptTokens":10,"completionTokens":4,"cacheReadTokens":2}"#,
            "2026-11-27T10:01:00.000Z"
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO usage_run_stats
         (runId, sessionId, inputTokens, outputTokens, cacheReadTokens, cacheWriteTokens)
         VALUES ('run_1', 'sess_demo', 10, 4, 2, 0)",
        [],
    )
    .unwrap();
    let messages = [
        (
            "msg_user",
            "user",
            r#"[{"type":"text","text":"Find the needle"}]"#,
            "01",
        ),
        (
            "msg_assistant",
            "assistant",
            r#"[{"type":"thinking","thinking":"I should read the file"},{"type":"text","text":"The needle is in the haystack."},{"type":"tool_use","id":"call_1","name":"read","input":{"path":"/tmp/haystack.txt"}}]"#,
            "02",
        ),
        (
            "msg_tool",
            "tool_result",
            r#"[{"type":"tool_result","tool_use_id":"call_1","content":"needle\n","is_error":false}]"#,
            "03",
        ),
    ];
    for (id, role, parts, order) in messages {
        conn.execute(
            "INSERT INTO messages
             (messageId, sessionId, role, parts, orderKey, createdAt, updatedAt)
             VALUES (?1, 'sess_demo', ?2, ?3, ?4, ?5, ?5)",
            params![id, role, parts, order, "2026-11-27T10:00:30.000Z"],
        )
        .unwrap();
    }
    drop(conn);

    let roots = vec![Root {
        source: "dimagent".into(),
        path: db,
        format: Format::DimAgent,
    }];
    let store = Store::new(Some(tmp.path().join("cairn.sqlite")));
    let report = build(&store, &roots, None, true).unwrap();
    assert_eq!(report.sessions, 1);
    assert_eq!(report.messages, 3);
    assert_eq!(store.stats().unwrap()["tool_calls"], 1);

    let hits = search(&store, "needle", &SearchOptions::default()).unwrap();
    assert_eq!(hits["count"], 1);
    assert_eq!(hits["sessions"][0]["session_id"], "dimagent:sess_demo");

    let shown = show(&store, "dimagent:sess_demo", None, 0, 0, 200, true).unwrap();
    assert_eq!(shown["count"], 5);
    let messages = shown["messages"].as_array().unwrap();
    assert!(messages.iter().any(|message| {
        message["content_type"] == "tool_use" && message["tool_call"]["name"] == "read"
    }));
    assert!(messages
        .iter()
        .any(|message| message["content_type"] == "tool_result"));
    assert!(messages
        .iter()
        .any(|message| message["content_type"] == "thinking"));
    let assistant = messages
        .iter()
        .find(|message| message["content_type"] == "text" && message["role"] == "assistant")
        .unwrap();
    assert_eq!(assistant["input_tokens"], 10);
    assert_eq!(assistant["output_tokens"], 4);
    assert_eq!(assistant["cache_read_tokens"], 2);

    let sessions = cairn::query::sessions(
        &store,
        &SessionOptions {
            source: Some("dimagent".into()),
            limit: 10,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(sessions[0]["model"], "dim-model");
}

#[test]
fn install_writes_skill_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("skills/cairn");
    cairn::cli::install_for_test(&dest, true).unwrap();
    assert!(dest.join("SKILL.md").is_file());
    assert!(dest.join("references/schema.md").is_file());
}

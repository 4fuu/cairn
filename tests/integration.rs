use cairn::indexer::build;
use cairn::providers::{Format, Root};
use cairn::query::{show, SessionOptions};
use cairn::search::{search, SearchOptions};
use cairn::sqlguard;
use cairn::store::Store;
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
fn install_writes_skill_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("skills/cairn");
    cairn::cli::install_for_test(&dest, true).unwrap();
    assert!(dest.join("SKILL.md").is_file());
    assert!(dest.join("references/schema.md").is_file());
}

use crate::indexer;
use crate::providers::{parse_roots, source_format, source_names, Root};
use crate::query::{self, SessionOptions};
use crate::search::{self, SearchOptions};
use crate::sqlguard;
use crate::store::Store;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use std::path::PathBuf;

const SKILL_FILES: &[(&str, &str)] = &[
    ("SKILL.md", include_str!("../skill/SKILL.md")),
    (
        "references/schema.md",
        include_str!("../skill/references/schema.md"),
    ),
    (
        "references/query-patterns.md",
        include_str!("../skill/references/query-patterns.md"),
    ),
    (
        "references/retrieval-semantics.md",
        include_str!("../skill/references/retrieval-semantics.md"),
    ),
    (
        "references/pitfalls.md",
        include_str!("../skill/references/pitfalls.md"),
    ),
];

#[derive(Parser, Debug)]
#[command(
    name = "cairn",
    version,
    about = "Query local coding-agent session history"
)]
struct Cli {
    #[arg(
        long,
        global = true,
        help = "index path (default: ~/.cairn/cairn.sqlite)"
    )]
    db: Option<PathBuf>,
    #[arg(long, global = true, help = "print JSON")]
    json: bool,
    #[arg(long, global = true, help = "skip the incremental index refresh")]
    no_refresh: bool,
    #[arg(
        long = "root",
        global = true,
        value_name = "SOURCE=PATH",
        help = "override a source root; repeatable"
    )]
    root: Vec<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Build {
        #[arg(long)]
        full: bool,
        #[arg(long = "source")]
        source: Vec<String>,
        #[arg(long)]
        quiet: bool,
    },
    Status,
    Sources,
    Overview {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long, default_value_t = 8)]
        limit: usize,
    },
    Sessions {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        after: Option<String>,
        #[arg(long)]
        before: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
    Search {
        #[arg(required = true, num_args = 1..)]
        query: Vec<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        after: Option<String>,
        #[arg(long)]
        before: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long, default_value_t = 0)]
        context: usize,
        #[arg(long)]
        include_meta: bool,
        #[arg(long)]
        full_text: bool,
        #[arg(long, help = "return message hits instead of grouping by session")]
        all: bool,
    },
    Show {
        session: String,
        #[arg(long)]
        around: Option<String>,
        #[arg(long, default_value_t = 0)]
        before: usize,
        #[arg(long, default_value_t = 0)]
        after: usize,
        #[arg(long, default_value_t = 200)]
        limit: usize,
        #[arg(long)]
        tools: bool,
    },
    Sql {
        sql: String,
        #[arg(long = "param", help = "JSON scalar bound to ?")]
        param: Vec<String>,
        #[arg(long, default_value_t = 500)]
        max_rows: usize,
        #[arg(long, default_value_t = 3.0)]
        timeout: f64,
    },
    Schema,
    Skill,
    Install {
        #[arg(long, default_value = "amp")]
        agent: String,
        #[arg(long)]
        dest: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
}

pub fn main() {
    if let Err(error) = run() {
        let wants_json = std::env::args().any(|argument| argument == "--json");
        if wants_json {
            println!(
                "{}",
                serde_json::json!({"ok": false, "error": format!("{error:#}")})
            );
        } else {
            eprintln!("cairn: {error:#}");
        }
        std::process::exit(2);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Command::Skill => {
            let path = skill_dir();
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&json!({"path": path}))?);
            } else {
                println!("{path}");
            }
            return Ok(());
        }
        Command::Install { agent, dest, force } => {
            return install_skill(agent, dest.as_deref(), *force, cli.json);
        }
        Command::Sources => return print_sources(&cli),
        _ => {}
    }

    let store = Store::new(cli.db.clone());
    match &cli.command {
        Command::Build {
            full,
            source,
            quiet,
        } => return run_build(&cli, &store, *full, source, *quiet),
        Command::Status => {
            refresh(&cli, &store)?;
            let stats = store.stats()?;
            if cli.json {
                print_json(&stats)?;
            } else {
                println!("db:       {}", stats["db"].as_str().unwrap_or("-"));
                println!("sessions: {}", stats["sessions"]);
                println!("messages: {}", stats["messages"]);
                println!("tools:    {}", stats["tool_calls"]);
                println!("projects: {}", stats["projects"]);
                println!(
                    "updated:  {}",
                    stats["last_indexed"].as_str().unwrap_or("-")
                );
                println!(
                    "trigram:  {}",
                    if stats["trigram"].as_bool().unwrap_or(false) {
                        "yes"
                    } else {
                        "no"
                    }
                );
                if let Some(sources) = stats["sources"].as_array() {
                    println!("sources:");
                    for source in sources {
                        println!(
                            "  {:<12} {:>5} sessions",
                            source["source"].as_str().unwrap_or("-"),
                            source["sessions"]
                        );
                    }
                }
            }
            return Ok(());
        }
        Command::Schema => {
            store.init()?;
            let conn = store.open_readonly()?;
            let mut stmt = conn.prepare(
                "SELECT type, name, sql FROM sqlite_master
                 WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'
                 ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'index' THEN 1
                 WHEN 'trigger' THEN 2 ELSE 3 END, name",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(json!({
                        "type": row.get::<_, String>(0)?,
                        "name": row.get::<_, String>(1)?,
                        "sql": row.get::<_, String>(2)?,
                    }))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if cli.json {
                print_json(&Value::Array(rows))?;
            } else {
                for row in rows {
                    println!("{};\n", row["sql"].as_str().unwrap_or(""));
                }
            }
            return Ok(());
        }
        _ => {}
    }

    refresh(&cli, &store)?;
    match &cli.command {
        Command::Overview {
            project,
            cwd,
            limit,
        } => {
            let cwd_value = match cwd {
                Some(value) => value.clone(),
                None => std::env::current_dir()?.to_string_lossy().to_string(),
            };
            let value = query::overview(&store, Some(&cwd_value), project.as_deref(), *limit)?;
            if cli.json {
                print_json(&value)?;
            } else {
                print_overview(&value);
            }
        }
        Command::Sessions {
            project,
            source,
            query,
            after,
            before,
            limit,
            offset,
        } => {
            let value = query::sessions(
                &store,
                &SessionOptions {
                    project: project.clone(),
                    source: source.clone(),
                    query: query.clone(),
                    after: after.clone(),
                    before: before.clone(),
                    limit: *limit,
                    offset: *offset,
                },
            )?;
            if cli.json {
                print_json(&value)?;
            } else {
                print_sessions(&value);
            }
        }
        Command::Search {
            query,
            project,
            session,
            source,
            after,
            before,
            limit,
            context,
            include_meta,
            full_text,
            all,
        } => {
            let value = search::search(
                &store,
                &query.join(" "),
                &SearchOptions {
                    project: project.clone(),
                    session: session.clone(),
                    source: source.clone(),
                    after: after.clone(),
                    before: before.clone(),
                    limit: *limit,
                    context: *context,
                    include_meta: *include_meta,
                    full_text: *full_text,
                    all: *all,
                },
            )?;
            if cli.json {
                print_json(&value)?;
            } else {
                print_search(&value, *all);
            }
        }
        Command::Show {
            session,
            around,
            before,
            after,
            limit,
            tools,
        } => {
            let value = query::show(
                &store,
                session,
                around.as_deref(),
                *before,
                *after,
                *limit,
                *tools,
            )?;
            if cli.json {
                print_json(&value)?;
            } else {
                print_show(&value);
            }
        }
        Command::Sql {
            sql,
            param,
            max_rows,
            timeout,
        } => {
            let params: Vec<Value> = param
                .iter()
                .map(|value| {
                    serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.clone()))
                })
                .collect();
            let value = sqlguard::run(&store, sql, &params, *max_rows, *timeout)?;
            if cli.json {
                print_json(&value)?;
            } else {
                print_sql(&value);
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn run_build(cli: &Cli, store: &Store, full: bool, sources: &[String], quiet: bool) -> Result<()> {
    let roots = parse_roots(&cli.root)?;
    let report = indexer::build(
        store,
        &roots,
        if sources.is_empty() {
            None
        } else {
            Some(sources)
        },
        full,
    )?;
    if cli.json {
        print_json(&serde_json::to_value(&report)?)?;
    } else if !quiet {
        println!(
            "indexed {} files, skipped {}, removed {}, {} errors in {}ms",
            report.indexed,
            report.skipped,
            report.removed,
            report.errors.len(),
            report.duration_ms
        );
        println!(
            "sessions: {}  messages: {}",
            report.sessions, report.messages
        );
        for error in &report.errors {
            eprintln!("  ! {}: {}", error.path, error.error);
        }
    }
    if !report.errors.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn refresh(cli: &Cli, store: &Store) -> Result<()> {
    if cli.no_refresh {
        store.init()?;
        return Ok(());
    }
    let roots = parse_roots(&cli.root)?;
    let full = !store.is_initialized();
    let report = indexer::build(store, &roots, None, full)?;
    for error in &report.errors {
        eprintln!("cairn: {}: {}", error.path, error.error);
    }
    Ok(())
}

fn print_json(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_sources(cli: &Cli) -> Result<()> {
    let roots = parse_roots(&cli.root)?;
    let mut values = Vec::new();
    for root in &roots {
        let files = if root.path.is_dir() {
            walkdir::WalkDir::new(&root.path)
                .into_iter()
                .flatten()
                .filter(|entry| entry.file_type().is_file())
                .count()
        } else if root.path.is_file() {
            1
        } else {
            0
        };
        values.push(json!({
            "source": root.source,
            "path": root.path,
            "exists": root.path.exists(),
            "files": files,
        }));
    }
    if cli.json {
        print_json(&Value::Array(values))?;
    } else {
        println!("{:<12} {:>6}  path", "source", "files");
        for value in values {
            println!(
                "{:<12} {:>6}  {}{}",
                value["source"].as_str().unwrap_or("-"),
                value["files"],
                value["path"].as_str().unwrap_or("-"),
                if value["exists"].as_bool().unwrap_or(false) {
                    ""
                } else {
                    "  (missing)"
                }
            );
        }
    }
    Ok(())
}

fn print_overview(value: &Value) {
    let totals = &value["totals"];
    println!(
        "{} sessions · {} messages · {} tool calls · {} projects",
        totals["sessions"], totals["messages"], totals["tool_calls"], totals["projects"]
    );
    if let Some(current) = value
        .get("current_project")
        .filter(|value| !value.is_null())
    {
        println!(
            "\ncurrent: {}  ({} sessions)",
            current["project"].as_str().unwrap_or("-"),
            current["sessions"]
        );
        for session in current["recent"].as_array().into_iter().flatten().take(5) {
            println!(
                "  {}  [{}]  {}",
                short(session["title"].as_str(), 70),
                short(session["id"].as_str(), 12),
                session["ended_at"]
                    .as_str()
                    .or(session["started_at"].as_str())
                    .unwrap_or("")
            );
        }
    }
    if let Some(projects) = value["projects"].as_array().filter(|rows| !rows.is_empty()) {
        println!("\nprojects");
        for project in projects {
            println!(
                "  {:<26} {:>4} sessions  {}",
                project["project"].as_str().unwrap_or("-"),
                project["sessions"],
                project["last_seen"].as_str().unwrap_or("")
            );
        }
    }
    if let Some(recent) = value["recent"].as_array().filter(|rows| !rows.is_empty()) {
        println!("\nrecent");
        for session in recent {
            println!(
                "  {}  [{}]  {}  {}",
                short(session["title"].as_str(), 62),
                short(session["id"].as_str(), 12),
                session["source"].as_str().unwrap_or("-"),
                session["ended_at"]
                    .as_str()
                    .or(session["started_at"].as_str())
                    .unwrap_or("")
            );
        }
    }
}

fn print_sessions(value: &Value) {
    let Some(rows) = value.as_array() else { return };
    if rows.is_empty() {
        println!("(no sessions)");
        return;
    }
    println!("{:<14} {:<10} {:<20} title", "id", "source", "when");
    for row in rows {
        let when = row["ended_at"]
            .as_str()
            .or(row["started_at"].as_str())
            .unwrap_or("")
            .get(..19)
            .unwrap_or("")
            .replace('T', " ");
        println!(
            "{:<40} {:<10} {:<20} {}",
            row["id"].as_str().unwrap_or("-"),
            row["source"].as_str().unwrap_or("-"),
            when,
            short(row["title"].as_str(), 70)
        );
    }
}

fn print_search(value: &Value, all: bool) {
    if all {
        let hits = value["hits"].as_array().cloned().unwrap_or_default();
        println!(
            "{} hits for {:?}",
            hits.len(),
            value["query"].as_str().unwrap_or("")
        );
        for hit in hits {
            println!(
                "\n[{}] {} · {} · {}",
                short(hit["session_id"].as_str(), 12),
                short(hit["session_title"].as_str(), 60),
                hit["role"].as_str().unwrap_or("-"),
                hit["timestamp"].as_str().unwrap_or("")
            );
            println!(
                "  {}",
                highlight(
                    hit["snippet"].as_str().unwrap_or(""),
                    value["query"].as_str().unwrap_or("")
                )
            );
        }
        return;
    }
    let sessions = value["sessions"].as_array().cloned().unwrap_or_default();
    println!(
        "{} sessions for {:?}",
        sessions.len(),
        value["query"].as_str().unwrap_or("")
    );
    for session in sessions {
        println!(
            "\n[{}] {} · {} · {} hits",
            session["session_id"].as_str().unwrap_or("-"),
            short(session["session_title"].as_str(), 70),
            session["project"].as_str().unwrap_or("-"),
            session["hits"]
        );
        println!(
            "  {}",
            highlight(
                session["snippet"].as_str().unwrap_or(""),
                value["query"].as_str().unwrap_or("")
            )
        );
        if let Some(context) = session.get("context").and_then(Value::as_array) {
            for row in context {
                println!(
                    "    {:>5} {:<9} {}",
                    row["seq"],
                    row["role"].as_str().unwrap_or("-"),
                    highlight(
                        row["text"].as_str().unwrap_or(""),
                        value["query"].as_str().unwrap_or("")
                    )
                );
            }
        }
    }
}

fn print_show(value: &Value) {
    let session = &value["session"];
    println!(
        "{}",
        session["title"]
            .as_str()
            .unwrap_or(session["id"].as_str().unwrap_or("-"))
    );
    println!(
        "{} · {} · {} · {} → {}",
        session["id"].as_str().unwrap_or("-"),
        session["source"].as_str().unwrap_or("-"),
        session["project"].as_str().unwrap_or("-"),
        session["started_at"].as_str().unwrap_or(""),
        session["ended_at"].as_str().unwrap_or("")
    );
    for message in value["messages"].as_array().into_iter().flatten() {
        let stamp = message["timestamp"].as_str().unwrap_or("");
        println!(
            "\n[{:>5}] {:<10} {:<8} {}",
            message["seq"],
            message["role"].as_str().unwrap_or("-"),
            stamp.get(11..19).unwrap_or(""),
            message["content_type"].as_str().unwrap_or("-")
        );
        if let Some(text) = message["text"].as_str() {
            for line in text.lines() {
                println!("  {line}");
            }
        }
        if let Some(call) = message.get("tool_call") {
            let input = call["input"].to_string();
            println!(
                "  ↳ {} {}",
                call["name"].as_str().unwrap_or("-"),
                short(Some(&input), 300)
            );
        }
        if message["is_error"].as_bool().unwrap_or(false) {
            println!("  (error)");
        }
    }
}

fn print_sql(value: &Value) {
    let columns = value["columns"].as_array().cloned().unwrap_or_default();
    if columns.is_empty() {
        println!("(no result columns)");
        return;
    }
    let names: Vec<String> = columns
        .iter()
        .filter_map(|column| column.as_str().map(ToString::to_string))
        .collect();
    println!("{}", names.join(" | "));
    println!(
        "{}",
        names
            .iter()
            .map(|name| "-".repeat(name.chars().count()))
            .collect::<Vec<_>>()
            .join("-+-")
    );
    for row in value["rows"].as_array().into_iter().flatten() {
        let cells: Vec<String> = row
            .as_array()
            .into_iter()
            .flatten()
            .map(|cell| {
                if cell.is_null() {
                    String::new()
                } else {
                    cell.to_string().trim_matches('"').to_string()
                }
            })
            .collect();
        println!("{}", cells.join(" | "));
    }
    if value["truncated"].as_bool().unwrap_or(false) {
        println!("\n(truncated at {} rows)", value["row_count"]);
    }
}

fn highlight(text: &str, query: &str) -> String {
    use std::io::IsTerminal;
    if query.is_empty()
        || !std::io::stdout().is_terminal()
        || std::env::var_os("NO_COLOR").is_some()
    {
        return text.to_string();
    }
    text.replace(query, &format!("\x1b[1;33m{query}\x1b[0m"))
}

fn short(value: Option<&str>, limit: usize) -> String {
    let value = value.unwrap_or("");
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.chars().count() <= limit {
        value
    } else {
        format!("{}…", value.chars().take(limit).collect::<String>())
    }
}

fn skill_dir() -> String {
    if let Ok(value) = std::env::var("CAIRN_SKILL_DIR") {
        return value;
    }
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("skill");
    if source.is_dir() {
        source.to_string_lossy().to_string()
    } else {
        "embedded (run `cairn install --dest <dir>`)".to_string()
    }
}

pub fn install_for_test(destination: &std::path::Path, force: bool) -> Result<()> {
    install_skill("amp", Some(destination), force, false)
}

fn install_skill(
    agent: &str,
    dest: Option<&std::path::Path>,
    force: bool,
    json: bool,
) -> Result<()> {
    let destination = match dest {
        Some(path) => path.to_path_buf(),
        None => {
            let home = dirs::home_dir().context("home directory")?;
            match agent {
                "amp" => home
                    .join(".config")
                    .join("agents")
                    .join("skills")
                    .join("cairn"),
                "claude" => home.join(".claude").join("skills").join("cairn"),
                "pi" => home.join(".pi").join("agent").join("skills").join("cairn"),
                "codex" => home.join(".codex").join("skills").join("cairn"),
                other => anyhow::bail!("unknown agent {other:?}; choose amp, claude, pi, or codex"),
            }
        }
    };
    if destination.exists() && !force {
        anyhow::bail!(
            "{} already exists; pass --force to replace it",
            destination.display()
        );
    }
    for (relative, content) in SKILL_FILES {
        let path = destination.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
    }
    if json {
        print_json(&json!({"installed": destination}))?;
    } else {
        println!("installed skill to {}", destination.display());
    }
    Ok(())
}

#[allow(dead_code)]
fn _unused(source: &str) -> Option<crate::providers::Format> {
    source_format(source)
}

#[allow(dead_code)]
fn _names() -> &'static [&'static str] {
    source_names()
}

#[allow(dead_code)]
fn _roots(roots: &[Root]) -> usize {
    roots.len()
}

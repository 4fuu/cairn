pub mod agy;
pub mod base;
pub mod claude;
pub mod cline;
pub mod codex;
pub mod devin;
pub mod droid;
pub mod dsh;
pub mod grok;
pub mod hermes;
pub mod kimi;
pub mod liveagent;
pub mod minimax;
pub mod opencode;
pub mod pi;
pub mod workbuddy;

use crate::model::ParsedSession;
use anyhow::{bail, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Claude,
    Codex,
    Pi,
    Dsh,
    Cline,
    Grok,
    Droid,
    Kimi,
    WorkBuddy,
    OpenCode,
    Hermes,
    Agy,
    Devin,
    MiniMax,
    LiveAgent,
}

#[derive(Clone, Debug)]
pub struct Root {
    pub source: String,
    pub path: PathBuf,
    pub format: Format,
}

#[derive(Clone, Debug)]
pub struct SourceFile {
    pub source: String,
    pub path: PathBuf,
    pub format: Format,
}

pub fn source_format(source: &str) -> Option<Format> {
    Some(match source {
        "claude" | "qoder" | "qoder-cn" => Format::Claude,
        "codex" => Format::Codex,
        "pi" | "omp" | "raft" | "cindy" => Format::Pi,
        "dsh" => Format::Dsh,
        "cline" => Format::Cline,
        "grok" => Format::Grok,
        "droid" => Format::Droid,
        "kimi" => Format::Kimi,
        "workbuddy" | "workbuddy-ai" | "codebuddy" => Format::WorkBuddy,
        "opencode" | "zcode" => Format::OpenCode,
        "hermes" => Format::Hermes,
        "agy" | "antigravity" => Format::Agy,
        "devin" => Format::Devin,
        "minimax" => Format::MiniMax,
        "liveagent" => Format::LiveAgent,
        _ => return None,
    })
}

pub fn source_names() -> &'static [&'static str] {
    &[
        "claude",
        "qoder",
        "qoder-cn",
        "codex",
        "pi",
        "omp",
        "dsh",
        "cline",
        "grok",
        "droid",
        "kimi",
        "workbuddy",
        "workbuddy-ai",
        "codebuddy",
        "opencode",
        "zcode",
        "hermes",
        "agy",
        "antigravity",
        "devin",
        "minimax",
        "liveagent",
    ]
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn env_path(name: &str, fallback: impl FnOnce() -> PathBuf) -> PathBuf {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| expand_home(value.trim()))
        .unwrap_or_else(fallback)
}

fn expand_home(value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        return home().join(rest);
    }
    PathBuf::from(value)
}

fn root(source: &str, path: PathBuf) -> Root {
    Root {
        source: source.to_string(),
        path,
        format: source_format(source).expect("known source"),
    }
}

pub fn default_roots() -> Vec<Root> {
    let home = home();
    let mut roots = Vec::new();

    let claude_base = env_path("CLAUDE_CONFIG_DIR", || home.join(".claude"));
    roots.push(root("claude", claude_base.join("projects")));

    let qoder_base = env_path("QODER_CONFIG_DIR", || home.join(".qoder"));
    roots.push(root("qoder", qoder_base.join("projects")));
    let qoder_cn_base = env_path("QODERCN_CONFIG_DIR", || home.join(".qoder-cn"));
    roots.push(root("qoder-cn", qoder_cn_base.join("projects")));

    let codex_base = env_path("CODEX_HOME", || home.join(".codex"));
    roots.push(root("codex", codex_base.join("sessions")));
    roots.push(root("codex", codex_base.join("archived_sessions")));

    let pi_base = env_path("PI_CODING_AGENT_DIR", || home.join(".pi").join("agent"));
    let pi_sessions = env_path("PI_CODING_AGENT_SESSION_DIR", || pi_base.join("sessions"));
    roots.push(root("pi", pi_sessions));
    let omp_base = home.join(".omp").join("agent");
    roots.push(root("omp", omp_base.join("sessions")));
    if let Ok(entries) = std::fs::read_dir(home.join(".omp").join("profiles")) {
        for entry in entries.flatten() {
            let sessions = entry.path().join("agent").join("sessions");
            if sessions.is_dir() {
                roots.push(root("omp", sessions));
            }
        }
    }
    let slock = env_path("SLOCK_HOME", || home.join(".slock"));
    if let Ok(entries) = std::fs::read_dir(slock.join("agents")) {
        for entry in entries.flatten() {
            for sub in [".pi-sessions", ".builtin-sessions"] {
                let sessions = entry.path().join(sub);
                if sessions.is_dir() {
                    roots.push(root("raft", sessions));
                }
            }
        }
    }
    if let Some(config) = dirs::config_dir() {
        for build in ["Cindy", "CindyGlobal"] {
            let sessions = config.join(build).join("pi-agent-home").join("sessions");
            if sessions.is_dir() {
                roots.push(root("cindy", sessions));
            }
        }
    }

    let dsh_base = env_path("DSH_HOME", || home.join(".dsh"));
    roots.push(root("dsh", dsh_base.join("sessions")));

    let cline_base = env_path("CLINE_DATA_DIR", || {
        env_path("CLINE_DIR", || home.join(".cline")).join("data")
    });
    let cline_sessions = env_path("CLINE_SESSION_DATA_DIR", || cline_base.join("sessions"));
    roots.push(root("cline", cline_sessions));

    let grok_base = env_path("GROK_HOME", || home.join(".grok"));
    roots.push(root("grok", grok_base.join("sessions")));

    let droid_base = env_path("FACTORY_HOME_OVERRIDE", || home.clone()).join(".factory");
    roots.push(root("droid", droid_base.join("sessions")));

    let kimi_base = env_path("KIMI_CODE_HOME", || home.join(".kimi-code"));
    roots.push(root("kimi", kimi_base.join("sessions")));

    let wb_base = env_path("WORKBUDDY_CONFIG_DIR", || home.join(".workbuddy"));
    roots.push(root("workbuddy", wb_base.join("projects")));
    roots.push(root(
        "workbuddy-ai",
        home.join(".workbuddy-ai").join("projects"),
    ));
    let cb_base = env_path("CODEBUDDY_CONFIG_DIR", || home.join(".codebuddy"));
    roots.push(root("codebuddy", cb_base.join("projects")));

    let xdg = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local").join("share"));
    let opencode_db = std::env::var("OPENCODE_DB")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                xdg.join("opencode").join(path)
            }
        })
        .unwrap_or_else(|| xdg.join("opencode").join("opencode.db"));
    roots.push(root("opencode", opencode_db));

    roots.push(root(
        "zcode",
        home.join(".zcode").join("cli").join("db").join("db.sqlite"),
    ));

    let hermes_base = env_path("HERMES_HOME", || home.join(".hermes"));
    roots.push(root("hermes", hermes_base.join("state.db")));
    if let Ok(entries) = std::fs::read_dir(hermes_base.join("profiles")) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                roots.push(root("hermes", entry.path().join("state.db")));
            }
        }
    }

    let gemini = home.join(".gemini");
    roots.push(root("agy", gemini.join("antigravity-cli").join("brain")));
    roots.push(root(
        "antigravity",
        gemini.join("antigravity").join("brain"),
    ));

    let devin_base = xdg.join("devin").join("cli");
    roots.push(root("devin", devin_base.join("sessions")));

    let minimax_base = env_path("MINIMAX_DATA_DIR", || home.join(".minimax"));
    roots.push(root("minimax", minimax_base.join("v2").join("sessions")));

    let liveagent_base = env_path("LIVEAGENT_HOME", || {
        env_path("K_BRAIN_HOME", || home.join(".liveagent"))
    });
    roots.push(root("liveagent", liveagent_base.join("sessions")));

    roots
}

pub fn parse_roots(values: &[String]) -> Result<Vec<Root>> {
    if values.is_empty() {
        return Ok(default_roots());
    }
    let mut roots = Vec::new();
    for value in values {
        let (source, path) = value
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("--root expects SOURCE=PATH, got {value:?}"))?;
        let source = source.trim();
        let Some(format) = source_format(source) else {
            bail!(
                "unknown source {source:?}; choose from {}",
                source_names().join(", ")
            );
        };
        roots.push(Root {
            source: source.to_string(),
            path: expand_home(path.trim()),
            format,
        });
    }
    Ok(roots)
}

pub fn discover(roots: &[Root], sources: Option<&[String]>) -> Vec<SourceFile> {
    let allowed: Option<HashSet<&str>> =
        sources.map(|values| values.iter().map(String::as_str).collect());
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for root in roots {
        if let Some(allowed) = &allowed {
            if !allowed.contains(root.source.as_str()) {
                continue;
            }
        }
        let paths = match root.format {
            Format::Claude | Format::Codex | Format::Pi | Format::Droid => walk_jsonl(&root.path),
            Format::Dsh => dsh::discover(&root.path),
            Format::Cline => cline::discover(&root.path),
            Format::Grok => grok::discover(&root.path),
            Format::Kimi => kimi::discover(&root.path),
            Format::WorkBuddy => workbuddy::discover(&root.path),
            Format::OpenCode => opencode::discover(&root.path),
            Format::Hermes => hermes::discover(&root.path),
            Format::Agy => agy::discover(&root.path),
            Format::Devin => devin::discover(&root.path),
            Format::MiniMax => minimax::discover(&root.path),
            Format::LiveAgent => liveagent::discover(&root.path),
        };
        for path in paths {
            let Ok(path) = path.canonicalize() else {
                continue;
            };
            if !seen.insert(path.clone()) {
                continue;
            }
            out.push(SourceFile {
                source: root.source.clone(),
                path,
                format: root.format,
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

pub fn parse_file(file: &SourceFile) -> Result<Vec<ParsedSession>> {
    match file.format {
        Format::Claude => claude::parse_with_source(&file.path, &file.source),
        Format::Codex => codex::parse(&file.path),
        Format::Pi => pi::parse(&file.path, &file.source),
        Format::Dsh => dsh::parse(&file.path),
        Format::Cline => cline::parse(&file.path),
        Format::Grok => grok::parse(&file.path),
        Format::Droid => droid::parse(&file.path),
        Format::Kimi => kimi::parse(&file.path),
        Format::WorkBuddy => workbuddy::parse(&file.path, &file.source),
        Format::OpenCode => opencode::parse(&file.path, &file.source),
        Format::Hermes => hermes::parse(&file.path, &file.source),
        Format::Agy => agy::parse(&file.path, &file.source),
        Format::Devin => devin::parse(&file.path),
        Format::MiniMax => minimax::parse(&file.path),
        Format::LiveAgent => liveagent::parse(&file.path),
    }
}

pub fn walk_jsonl(root: &Path) -> Vec<PathBuf> {
    if !root.exists() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .flatten()
    {
        if entry.file_type().is_file()
            && entry.path().extension().and_then(|ext| ext.to_str()) == Some("jsonl")
        {
            out.push(entry.path().to_path_buf());
        }
    }
    out
}

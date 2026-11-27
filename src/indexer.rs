use crate::providers::{discover, parse_file, Format, Root, SourceFile};
use crate::store::{clear_all, delete_source_path, write_meta, Store};
use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Clone, Debug, Serialize)]
pub struct BuildError {
    pub path: String,
    pub error: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BuildReport {
    pub indexed: usize,
    pub skipped: usize,
    pub removed: usize,
    pub errors: Vec<BuildError>,
    pub files: usize,
    pub sessions: i64,
    pub messages: i64,
    pub duration_ms: u128,
}

pub fn build(
    store: &Store,
    roots: &[Root],
    sources: Option<&[String]>,
    full: bool,
) -> Result<BuildReport> {
    let started = Instant::now();
    store.init()?;
    let conn = store.open_write()?;
    let mut report = BuildReport {
        indexed: 0,
        skipped: 0,
        removed: 0,
        errors: Vec::new(),
        files: 0,
        sessions: 0,
        messages: 0,
        duration_ms: 0,
    };

    conn.execute("BEGIN IMMEDIATE", [])?;
    let result: Result<()> = (|| {
        if full {
            clear_all(&conn)?;
        }
        let existing: HashMap<String, (i64, i64, String)> = {
            let mut stmt = conn.prepare(
                "SELECT path, mtime_ns, size, COALESCE(prefix_hash, '') FROM source_files",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    (
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                    ),
                ))
            })?;
            rows.collect::<rusqlite::Result<HashMap<_, _>>>()?
        };

        let files = discover(roots, sources);
        report.files = files.len();
        let mut seen = HashSet::new();
        let mut changed: Vec<(SourceFile, i64, i64, String, bool)> = Vec::new();
        for file in files {
            let path = file.path.clone();
            seen.insert(path.to_string_lossy().to_string());
            let metadata = match std::fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    report.errors.push(BuildError {
                        path: path.to_string_lossy().to_string(),
                        error: error.to_string(),
                    });
                    continue;
                }
            };
            let mtime = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos() as i64)
                .unwrap_or(0);
            let size = metadata.len() as i64;
            if !full {
                if let Some((old_mtime, old_size, _)) =
                    existing.get(&path.to_string_lossy().to_string())
                {
                    if *old_mtime == mtime && *old_size == size {
                        report.skipped += 1;
                        continue;
                    }
                }
            }
            let hash = match prefix_hash(&path) {
                Ok(hash) => hash,
                Err(error) => {
                    report.errors.push(BuildError {
                        path: path.to_string_lossy().to_string(),
                        error: error.to_string(),
                    });
                    continue;
                }
            };
            let append = !full
                && !matches!(file.format, Format::OpenCode | Format::Hermes)
                && existing
                    .get(&path.to_string_lossy().to_string())
                    .map(|(_, old_size, old_hash)| {
                        size > *old_size && !old_hash.is_empty() && old_hash == &hash
                    })
                    .unwrap_or(false);
            changed.push((file, mtime, size, hash, append));
        }

        for chunk in changed.chunks(32) {
            let parsed: Vec<(
                SourceFile,
                i64,
                i64,
                String,
                bool,
                Result<Vec<crate::model::ParsedSession>>,
            )> = chunk
                .par_iter()
                .map(|(file, mtime, size, hash, append)| {
                    (
                        file.clone(),
                        *mtime,
                        *size,
                        hash.clone(),
                        *append,
                        parse_file(file).with_context(|| format!("parse {}", file.path.display())),
                    )
                })
                .collect();
            for (file, mtime, size, hash, append, result) in parsed {
                match result {
                    Ok(mut sessions) => {
                        if sessions.is_empty() {
                            report.skipped += 1;
                            continue;
                        }
                        match store.write_many(&conn, &mut sessions, mtime, size, append, &hash) {
                            Ok(_) => {
                                report.indexed += 1;
                            }
                            Err(error) => report.errors.push(BuildError {
                                path: file.path.to_string_lossy().to_string(),
                                error: error.to_string(),
                            }),
                        }
                    }
                    Err(error) => report.errors.push(BuildError {
                        path: file.path.to_string_lossy().to_string(),
                        error: error.to_string(),
                    }),
                }
            }
        }

        let removed = prune(&conn, roots, &seen, sources)?;
        report.removed = removed;
        report.sessions = conn.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;
        report.messages = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE is_meta = 0",
            [],
            |row| row.get(0),
        )?;
        write_meta(&conn, "last_build", &crate::util::now_ts())?;
        if full {
            write_meta(&conn, "last_full_build", &crate::util::now_ts())?;
        }
        write_meta(
            &conn,
            "last_build_stats",
            &format!(
                "indexed={} skipped={} removed={} errors={}",
                report.indexed,
                report.skipped,
                report.removed,
                report.errors.len()
            ),
        )?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            conn.execute("COMMIT", [])?;
        }
        Err(error) => {
            let _ = conn.execute("ROLLBACK", []);
            return Err(error);
        }
    }
    report.duration_ms = started.elapsed().as_millis();
    Ok(report)
}

fn prefix_hash(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut buffer = Vec::with_capacity(65_536);
    file.by_ref().take(65_536).read_to_end(&mut buffer)?;
    let digest = Sha256::digest(&buffer);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn prune(
    conn: &rusqlite::Connection,
    roots: &[Root],
    seen: &HashSet<String>,
    sources: Option<&[String]>,
) -> Result<usize> {
    let allowed: Option<HashSet<&str>> =
        sources.map(|values| values.iter().map(String::as_str).collect());
    let mut removed = 0;
    for root in roots {
        if let Some(allowed) = &allowed {
            if !allowed.contains(root.source.as_str()) {
                continue;
            }
        }
        if root.path.is_file() {
            let canonical = root
                .path
                .canonicalize()
                .unwrap_or_else(|_| root.path.clone());
            let path = canonical.to_string_lossy().to_string();
            if !canonical.exists() && !seen.contains(&path) {
                delete_source_path(conn, &path)?;
                removed += 1;
            }
            continue;
        }
        if !root.path.is_dir() {
            continue;
        }
        let canonical_root = root
            .path
            .canonicalize()
            .unwrap_or_else(|_| root.path.clone());
        let prefix = format!(
            "{}/",
            canonical_root.to_string_lossy().trim_end_matches('/')
        );
        let mut stmt = conn.prepare("SELECT path FROM source_files WHERE path LIKE ?")?;
        let paths: Vec<String> = stmt
            .query_map([format!("{prefix}%")], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for path in paths {
            if seen.contains(&path) {
                continue;
            }
            if PathBuf::from(&path).exists() {
                continue;
            }
            delete_source_path(conn, &path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

const RELEASE_API: &str = "https://api.github.com/repos/4fuu/cairn/releases/latest";
const RELEASES_URL: &str = "https://github.com/4fuu/cairn/releases/latest";
const CHECK_INTERVAL_SECS: i64 = 24 * 60 * 60;

#[derive(Clone, Debug)]
pub struct Status {
    pub current: String,
    pub latest: Option<String>,
    pub release_url: String,
    pub update_available: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct CheckState {
    last_checked: Option<i64>,
    latest_version: Option<String>,
    release_url: Option<String>,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn check(force: bool) -> Status {
    let mut state = load_state();
    let now = Utc::now().timestamp();
    let stale = state
        .last_checked
        .is_none_or(|last| now.saturating_sub(last) >= CHECK_INTERVAL_SECS);
    if force || stale {
        if let Some((latest, url)) = fetch_latest() {
            state.latest_version = Some(latest);
            state.release_url = Some(url);
        }
        state.last_checked = Some(now);
        save_state(&state);
    }
    status_from_state(state)
}

pub fn cached() -> Status {
    status_from_state(load_state())
}

fn status_from_state(state: CheckState) -> Status {
    let current = current_version().to_string();
    let update_available = state
        .latest_version
        .as_deref()
        .is_some_and(|latest| is_newer(&current, latest));
    Status {
        current,
        latest: state.latest_version,
        release_url: state
            .release_url
            .unwrap_or_else(|| RELEASES_URL.to_string()),
        update_available,
    }
}

fn fetch_latest() -> Option<(String, String)> {
    let response = minreq::get(RELEASE_API)
        .with_header("User-Agent", format!("cairn/{}", current_version()))
        .with_header("Accept", "application/vnd.github+json")
        .with_timeout(2)
        .send()
        .ok()?;
    if response.status_code != 200 {
        return None;
    }
    let value: Value = serde_json::from_str(response.as_str().ok()?).ok()?;
    let latest = value
        .get("tag_name")
        .and_then(Value::as_str)?
        .trim_start_matches('v')
        .to_string();
    parse_version(&latest)?;
    let url = value
        .get("html_url")
        .and_then(Value::as_str)
        .unwrap_or(RELEASES_URL)
        .to_string();
    Some((latest, url))
}

fn is_newer(current: &str, latest: &str) -> bool {
    match (parse_version(current), parse_version(latest)) {
        (Some(current), Some(latest)) => latest > current,
        _ => false,
    }
}

fn parse_version(value: &str) -> Option<(u64, u64, u64)> {
    let value = value.trim().trim_start_matches('v');
    let mut parts = value.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn state_path() -> PathBuf {
    crate::store::default_db_path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("update-check.json")
}

fn load_state() -> CheckState {
    std::fs::read_to_string(state_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_state(state: &CheckState) {
    let path = state_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(path, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_calendar_versions() {
        assert!(is_newer("2026.1127.2", "2026.1127.3"));
        assert!(is_newer("2026.1127.2", "2026.1128.0"));
        assert!(!is_newer("2026.1127.2", "2026.1127.2"));
        assert!(!is_newer("2026.1127.2", "2026.1127.1"));
        assert!(!is_newer("2026.1127.2", "not-a-version"));
    }

    #[test]
    fn parses_optional_v_prefix() {
        assert_eq!(parse_version("v2026.1127.2"), Some((2026, 1127, 2)));
        assert_eq!(parse_version("2026.1127.2"), Some((2026, 1127, 2)));
        assert_eq!(parse_version("2026.1127"), None);
        assert_eq!(parse_version("2026.1127.2.1"), None);
    }
}

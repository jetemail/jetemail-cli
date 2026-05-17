//! Once-per-day "you're behind" warning shown after a command completes.
//!
//! Cache lives at `<cache>/update-check.json` (per-user). When stale (>24h) we
//! best-effort refresh with a 2s timeout; failure is silent — this is a
//! courtesy nag, never something that should block a real command.
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CURRENT: &str = env!("CARGO_PKG_VERSION");
const TTL_SECS: u64 = 24 * 3600;
const FETCH_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Serialize, Deserialize, Default)]
struct Cache {
    checked_at: u64,
    latest_version: String,
}

/// Returns a formatted stderr-ready warning if a newer version is known.
/// Best-effort: any IO or network failure simply yields `None`.
pub async fn check_outdated() -> Option<String> {
    let path = cache_path()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();

    let mut cache: Cache = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    if now.saturating_sub(cache.checked_at) > TTL_SECS {
        if let Some(latest) = fetch_latest().await {
            cache = Cache {
                checked_at: now,
                latest_version: latest,
            };
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(s) = serde_json::to_string(&cache) {
                let _ = std::fs::write(&path, s);
            }
        }
    }

    let latest = cache.latest_version.trim_start_matches('v');
    if latest.is_empty() {
        return None;
    }
    let current_v = semver::Version::parse(CURRENT).ok()?;
    let latest_v = semver::Version::parse(latest).ok()?;
    if latest_v <= current_v {
        return None;
    }

    Some(format!(
        "\n\x1b[33mA new version of jetemail is available: v{latest_v} \
         (you have v{current_v})\x1b[0m\n  Run `jetemail update` to upgrade.\n"
    ))
}

async fn fetch_latest() -> Option<String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("jetemail-cli/", env!("CARGO_PKG_VERSION")))
        .timeout(FETCH_TIMEOUT)
        .build()
        .ok()?;
    let resp = client
        .get("https://api.github.com/repos/jetemail/jetemail-cli/releases/latest")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    json.get("tag_name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim_start_matches('v').to_string())
}

fn cache_path() -> Option<PathBuf> {
    let dirs = ProjectDirs::from("com", "JetEmail", "jetemail")?;
    Some(dirs.cache_dir().join("update-check.json"))
}

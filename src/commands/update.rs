use anyhow::{anyhow, Context, Result};
use clap::Args;
use std::path::Path;

use crate::output::OutputOpts;

const REPO: &str = "jetemail/jetemail-cli";
const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Args)]
pub struct Cmd {
    /// Don't install — just report whether an update is available.
    #[arg(long)]
    pub check: bool,
    /// Reinstall even if already on the latest version.
    #[arg(long)]
    pub force: bool,
}

pub async fn run(cmd: &Cmd, out: OutputOpts) -> Result<()> {
    let current_exe = std::env::current_exe().context("locating current binary")?;

    // If the binary is managed by a package manager, defer to it — racing
    // a self-replace against the package database is how installs rot.
    if let Some(hint) = package_manager_hint(&current_exe) {
        println!("{hint}");
        return Ok(());
    }

    let target = target_triple()
        .ok_or_else(|| anyhow!("no pre-built binary for this target — install from source"))?;

    let latest = fetch_latest_tag().await?;
    let latest_v = parse_version(&latest)?;
    let current_v = parse_version(CURRENT)?;

    if cmd.check {
        if latest_v > current_v {
            println!("update available: v{current_v} → v{latest_v}");
        } else {
            println!("up to date (v{current_v})");
        }
        return Ok(());
    }

    if latest_v <= current_v && !cmd.force {
        println!("already on the latest version (v{current_v})");
        return Ok(());
    }

    let tag = format!("v{latest_v}");
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let asset = format!("jetemail-{latest_v}-{target}{ext}");
    let url = format!("https://github.com/{REPO}/releases/download/{tag}/{asset}");

    if !out.quiet {
        println!("==> Downloading {asset}");
    }

    let client = github_client()?;
    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("downloading {url}"))?;
    if !resp.status().is_success() {
        return Err(anyhow!("download returned {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .with_context(|| format!("reading body from {url}"))?;

    // Stage in a temp file next to the target so the rename stays on one
    // filesystem (cross-fs rename can fail with EXDEV).
    let parent = current_exe
        .parent()
        .ok_or_else(|| anyhow!("current binary has no parent directory"))?;
    let staging = parent.join(format!(".jetemail-{latest_v}.download"));
    std::fs::write(&staging, &bytes)
        .with_context(|| format!("writing staged binary to {}", staging.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755))
            .context("setting executable bit on downloaded binary")?;
    }

    self_replace::self_replace(&staging)
        .with_context(|| format!("replacing {}", current_exe.display()))?;
    let _ = std::fs::remove_file(&staging);

    println!("==> Updated to v{latest_v}");
    Ok(())
}

/// Compile-time mapping from host triple to the asset name we publish.
fn target_triple() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("aarch64-apple-darwin")
    } else if cfg!(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    )) {
        Some("x86_64-unknown-linux-gnu")
    } else if cfg!(all(
        target_os = "linux",
        target_arch = "aarch64",
        target_env = "gnu"
    )) {
        Some("aarch64-unknown-linux-gnu")
    } else if cfg!(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "musl"
    )) {
        Some("x86_64-unknown-linux-musl")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("x86_64-pc-windows-msvc")
    } else {
        None
    }
}

fn package_manager_hint(path: &Path) -> Option<String> {
    let lower = path.to_string_lossy().to_lowercase();
    if lower.contains("/cellar/")
        || lower.contains("/homebrew/")
        || lower.contains("/opt/homebrew/")
        || lower.contains("/.linuxbrew/")
    {
        Some("Installed via Homebrew — update with: brew upgrade jetemail".into())
    } else if lower.contains("\\scoop\\") || lower.contains("/scoop/") {
        Some("Installed via Scoop — update with: scoop update jetemail-cli".into())
    } else if lower.contains("/.cargo/bin/") || lower.contains("\\.cargo\\bin\\") {
        Some(
            "Installed via cargo — update with:\n  \
             cargo install --git https://github.com/jetemail/jetemail-cli --force"
                .into(),
        )
    } else {
        None
    }
}

fn parse_version(s: &str) -> Result<semver::Version> {
    let trimmed = s.trim_start_matches('v');
    semver::Version::parse(trimmed).with_context(|| format!("parsing version string {s:?}"))
}

async fn fetch_latest_tag() -> Result<String> {
    let client = github_client()?;
    let resp = client
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .send()
        .await
        .context("contacting GitHub API")?;
    if !resp.status().is_success() {
        return Err(anyhow!("GitHub API returned {}", resp.status()));
    }
    let json: serde_json::Value = resp.json().await.context("decoding GitHub API response")?;
    json.get("tag_name")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("GitHub API response missing tag_name"))
}

fn github_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("jetemail-cli/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("building HTTP client")
}

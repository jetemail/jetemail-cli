use anyhow::{anyhow, Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const DEFAULT_BASE_URL: &str = "https://api.jetemail.com";
pub const DEFAULT_PROFILE: &str = "default";

/// On-disk config. Stored as TOML. Format:
///
/// ```toml
/// current_profile = "default"
///
/// [profiles.default]
/// api_key = "api_xxx"
/// transactional_key = "transactional_xxx"
/// ```
///
/// The pre-profile-block format (top-level `api_key` / `transactional_key`) is
/// still loadable — it's migrated into `profiles.default` on the next save.
/// `base_url` is no longer configurable (only one prod API exists); existing
/// `base_url` lines in config files are silently ignored.
// Note: `Debug` is implemented manually (below) for the secret-bearing structs
// so the key fields are masked — a derived `Debug` would print full keys if any
// future code ever `{:?}`-printed these. See `mask_key`.
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct FileConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_profile: Option<String>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, Profile>,

    // Legacy fields for backward-compat on load. Skipped on serialize so the
    // file rewrites cleanly once a profile section exists.
    #[serde(default, skip_serializing)]
    api_key: Option<String>,
    #[serde(default, skip_serializing)]
    transactional_key: Option<String>,
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transactional_key: Option<String>,
}

impl std::fmt::Debug for FileConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileConfig")
            .field("current_profile", &self.current_profile)
            .field("profiles", &self.profiles) // Profile masks its own keys
            .finish()
    }
}

impl std::fmt::Debug for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Profile")
            .field("api_key", &self.api_key.as_deref().map(mask_key))
            .field(
                "transactional_key",
                &self.transactional_key.as_deref().map(mask_key),
            )
            .finish()
    }
}

impl FileConfig {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file {}", path.display()))?;
        let mut cfg: FileConfig = toml::from_str(&text)
            .with_context(|| format!("parsing config file {}", path.display()))?;
        cfg.migrate_legacy();
        Ok(cfg)
    }

    fn migrate_legacy(&mut self) {
        let has_legacy = self.api_key.is_some() || self.transactional_key.is_some();
        if has_legacy && self.profiles.is_empty() {
            self.profiles.insert(
                DEFAULT_PROFILE.to_string(),
                Profile {
                    api_key: self.api_key.take(),
                    transactional_key: self.transactional_key.take(),
                },
            );
            if self.current_profile.is_none() {
                self.current_profile = Some(DEFAULT_PROFILE.to_string());
            }
        }
        // Clear any unmigrated legacy fields so they don't get re-serialized.
        self.api_key = None;
        self.transactional_key = None;
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating config dir {}", parent.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // Best-effort: tighten the config dir to owner-only so the file's
                // presence/siblings aren't enumerable by other local users.
                if let Ok(meta) = std::fs::metadata(parent) {
                    if meta.permissions().mode() & 0o077 != 0 {
                        let _ = std::fs::set_permissions(
                            parent,
                            std::fs::Permissions::from_mode(0o700),
                        );
                    }
                }
            }
        }
        let text = toml::to_string_pretty(self).context("serializing config")?;
        write_secret_file(path, text.as_bytes())
    }

    pub fn profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.get(name)
    }

    pub fn profile_mut(&mut self, name: &str) -> &mut Profile {
        self.profiles.entry(name.to_string()).or_default()
    }
}

pub fn config_path() -> Result<PathBuf> {
    // `JETEMAIL_CONFIG` is a *trusted-input* override: it is used verbatim for
    // both reading and writing the secret config. Point it only at a path you
    // control — a symlink/shared/world-readable target would place the keys
    // there (the 0600 mode below applies to the resolved target's file).
    if let Ok(custom) = std::env::var("JETEMAIL_CONFIG") {
        return Ok(PathBuf::from(custom));
    }
    let dirs = ProjectDirs::from("com", "JetEmail", "jetemail")
        .ok_or_else(|| anyhow!("could not determine a config directory for this OS"))?;
    Ok(dirs.config_dir().join("config.toml"))
}

/// Write `bytes` to `path` so the secret only ever touches disk with restrictive
/// permissions. On Unix the file is created `0600` from the start (and an existing
/// looser file is re-restricted to `0600` *before* the secret is written), and a
/// chmod failure is surfaced rather than swallowed — we refuse to write a secret
/// we couldn't protect. On Windows the file inherits the per-user `%APPDATA%`
/// ACLs (there is no portable chmod); see the README security note.
fn write_secret_file(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        // `mode(0o600)` applies only when the file is newly created, so for a
        // pre-existing file we explicitly re-restrict after truncation but
        // before writing any secret bytes — closing the old write-then-chmod race.
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("opening config file {}", path.display()))?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).with_context(
            || {
                format!(
                    "restricting permissions on {} (refusing to write an unprotected key)",
                    path.display()
                )
            },
        )?;
        f.write_all(bytes)
            .with_context(|| format!("writing config file {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let mut f = std::fs::File::create(path)
            .with_context(|| format!("creating config file {}", path.display()))?;
        f.write_all(bytes)
            .with_context(|| format!("writing config file {}", path.display()))?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Flag,
    Env,
    Profile,
    Missing,
}

impl KeySource {
    pub fn label(self) -> &'static str {
        match self {
            KeySource::Flag => "command-line flag",
            KeySource::Env => "environment variable",
            KeySource::Profile => "config file",
            KeySource::Missing => "not set",
        }
    }
}

#[derive(Clone)]
pub struct Resolved {
    pub base_url: String,
    pub api_key: Option<String>,
    pub transactional_key: Option<String>,
    pub api_key_source: KeySource,
    pub transactional_key_source: KeySource,
}

impl std::fmt::Debug for Resolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolved")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_deref().map(mask_key))
            .field("api_key_source", &self.api_key_source)
            .field(
                "transactional_key",
                &self.transactional_key.as_deref().map(mask_key),
            )
            .field("transactional_key_source", &self.transactional_key_source)
            .finish()
    }
}

impl Drop for Resolved {
    /// Scrub the in-memory key material on drop. This is best-effort hardening
    /// (transient clones — e.g. the `Bearer …` header string — are not covered),
    /// but it clears the longest-lived copy that is held for the whole process.
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.api_key.zeroize();
        self.transactional_key.zeroize();
    }
}

impl Resolved {
    pub fn from_layers(
        cli_api_key: Option<String>,
        cli_transactional_key: Option<String>,
    ) -> Result<Self> {
        let path = config_path()?;
        let file = FileConfig::load(&path)?;

        // Always read from the default profile slot.
        let profile_name = file
            .current_profile
            .clone()
            .unwrap_or_else(|| DEFAULT_PROFILE.to_string());
        let profile = file.profile(&profile_name).cloned().unwrap_or_default();

        let (api_key, api_key_source) =
            resolve_key(cli_api_key, "JETEMAIL_API_KEY", profile.api_key);
        let (transactional_key, transactional_key_source) = resolve_key(
            cli_transactional_key,
            "JETEMAIL_TRANSACTIONAL_KEY",
            profile.transactional_key,
        );

        Ok(Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key,
            transactional_key,
            api_key_source,
            transactional_key_source,
        })
    }

    pub fn require_api_key(&self) -> Result<&str> {
        self.api_key.as_deref().ok_or_else(|| {
            anyhow!(
                "no API key found. Run `jetemail login`, set JETEMAIL_API_KEY, or pass --api-key."
            )
        })
    }

    pub fn require_transactional_key(&self) -> Result<&str> {
        self.transactional_key.as_deref().ok_or_else(|| {
            anyhow!(
                "no transactional key found. Run `jetemail login --transactional`, set JETEMAIL_TRANSACTIONAL_KEY, or pass --transactional-key."
            )
        })
    }
}

fn resolve_key(
    cli: Option<String>,
    env: &str,
    profile: Option<String>,
) -> (Option<String>, KeySource) {
    if let Some(v) = cli {
        return (Some(v), KeySource::Flag);
    }
    if let Ok(v) = std::env::var(env) {
        if !v.is_empty() {
            return (Some(v), KeySource::Env);
        }
    }
    if let Some(v) = profile {
        return (Some(v), KeySource::Profile);
    }
    (None, KeySource::Missing)
}

/// Mask a key for safe display: shows the prefix and the last 4 chars.
///
/// Operates on `char`s (never raw byte indices) so a non-ASCII / underscore-less
/// value can't panic on a UTF-8 boundary.
pub fn mask_key(key: &str) -> String {
    let char_count = key.chars().count();
    if char_count <= 8 {
        return "*".repeat(char_count);
    }
    let (prefix, tail) = match key.split_once('_') {
        Some((p, t)) => (p.to_string(), t),
        // No underscore: use the first 3 chars as a pseudo-prefix.
        None => (key.chars().take(3).collect::<String>(), key),
    };
    let tail_chars: Vec<char> = tail.chars().collect();
    let start = tail_chars.len().saturating_sub(4);
    let suffix: String = tail_chars[start..].iter().collect();
    format!("{prefix}_…{suffix}")
}

#[cfg(test)]
mod tests {
    use super::mask_key;

    #[test]
    fn mask_normal_key() {
        assert_eq!(mask_key("api_1234567890abcd"), "api_…abcd");
        assert_eq!(mask_key("transactional_zzzzwxyz"), "transactional_…wxyz");
    }

    #[test]
    fn mask_short_keys_are_all_stars() {
        assert_eq!(mask_key(""), "");
        assert_eq!(mask_key("a"), "*");
        assert_eq!(mask_key("12345678"), "********");
    }

    #[test]
    fn mask_no_underscore_does_not_panic() {
        // Long, underscore-less value: first 3 chars + last 4 chars.
        assert_eq!(mask_key("abcdefghij"), "abc_…ghij");
    }

    #[test]
    fn mask_non_ascii_does_not_panic() {
        // A multi-byte leading char would previously panic on `&key[..3]`.
        let masked = mask_key("ééééééééé");
        assert!(masked.contains('…'));
        let masked2 = mask_key("🔑🔑🔑🔑🔑🔑🔑🔑🔑");
        assert!(masked2.contains('…'));
    }
}

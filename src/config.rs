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
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transactional_key: Option<String>,
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
        }
        let text = toml::to_string_pretty(self).context("serializing config")?;
        std::fs::write(path, text)
            .with_context(|| format!("writing config file {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(path)?.permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(path, perms).ok();
        }
        Ok(())
    }

    pub fn profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.get(name)
    }

    pub fn profile_mut(&mut self, name: &str) -> &mut Profile {
        self.profiles.entry(name.to_string()).or_default()
    }
}

pub fn config_path() -> Result<PathBuf> {
    if let Ok(custom) = std::env::var("JETEMAIL_CONFIG") {
        return Ok(PathBuf::from(custom));
    }
    let dirs = ProjectDirs::from("com", "JetEmail", "jetemail")
        .ok_or_else(|| anyhow!("could not determine a config directory for this OS"))?;
    Ok(dirs.config_dir().join("config.toml"))
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

#[derive(Debug, Clone)]
pub struct Resolved {
    pub base_url: String,
    pub api_key: Option<String>,
    pub transactional_key: Option<String>,
    pub api_key_source: KeySource,
    pub transactional_key_source: KeySource,
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
pub fn mask_key(key: &str) -> String {
    if key.len() <= 8 {
        return "*".repeat(key.len());
    }
    let (prefix, tail) = key.split_once('_').unwrap_or((&key[..3], key));
    let suffix = &tail[tail.len().saturating_sub(4)..];
    format!("{prefix}_…{suffix}")
}

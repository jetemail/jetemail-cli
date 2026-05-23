use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};
use std::io::IsTerminal;

use crate::config::{config_path, mask_key, FileConfig, DEFAULT_PROFILE};
use crate::output::OutputOpts;

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Print the config file location.
    Path,
    /// Print the contents of the config file (TOML). Secrets are masked unless `--reveal`.
    Show(ShowArgs),
    /// Read one value (`api_key`, `transactional_key`).
    Get(GetArgs),
    /// Set one value.
    Set(SetArgs),
    /// Clear one value.
    Unset(UnsetArgs),
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Print secret values in full instead of masked.
    #[arg(long)]
    pub reveal: bool,
}

#[derive(Debug, Args)]
pub struct GetArgs {
    pub key: String,
    /// Print the secret in full even when writing to a terminal (piped output is
    /// always raw, for scripting).
    #[arg(long)]
    pub reveal: bool,
}

#[derive(Debug, Args)]
pub struct SetArgs {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Args)]
pub struct UnsetArgs {
    pub key: String,
}

pub async fn run(cmd: &Cmd, _out: OutputOpts) -> Result<()> {
    let path = config_path()?;
    match &cmd.action {
        Action::Path => {
            println!("{}", path.display());
            Ok(())
        }
        Action::Show(a) => {
            let mut cfg = FileConfig::load(&path)?;
            let mut had_secret = false;
            if !a.reveal {
                // Mask in place before serializing so `config show` never dumps
                // raw keys into terminal scrollback / recordings / pipelines.
                for profile in cfg.profiles.values_mut() {
                    if let Some(k) = profile.api_key.as_deref() {
                        had_secret = true;
                        profile.api_key = Some(mask_key(k));
                    }
                    if let Some(k) = profile.transactional_key.as_deref() {
                        had_secret = true;
                        profile.transactional_key = Some(mask_key(k));
                    }
                }
            }
            let text = toml::to_string_pretty(&cfg)?;
            print!("{text}");
            if had_secret {
                eprintln!("note: secrets masked — pass --reveal to show full values");
            }
            Ok(())
        }
        Action::Get(a) => {
            let cfg = FileConfig::load(&path)?;
            let profile = cfg.profile(DEFAULT_PROFILE).cloned().unwrap_or_default();
            let is_secret = matches!(a.key.as_str(), "api_key" | "transactional_key");
            let value = match a.key.as_str() {
                "api_key" => profile.api_key,
                "transactional_key" => profile.transactional_key,
                other => return Err(anyhow!("unknown config key `{other}`")),
            };
            match value {
                // Mask only when displaying to a terminal without --reveal; piped
                // output stays raw so `KEY=$(jetemail config get api_key)` works.
                Some(v) if is_secret && !a.reveal && std::io::stdout().is_terminal() => {
                    println!("{}", mask_key(&v));
                    eprintln!("note: masked for terminal display — pass --reveal (or pipe) for the raw value");
                }
                Some(v) => println!("{v}"),
                None => println!(),
            }
            Ok(())
        }
        Action::Set(a) => {
            let mut cfg = FileConfig::load(&path)?;
            let v = Some(a.value.clone());
            let profile = cfg.profile_mut(DEFAULT_PROFILE);
            match a.key.as_str() {
                "api_key" => profile.api_key = v,
                "transactional_key" => profile.transactional_key = v,
                other => return Err(anyhow!("unknown config key `{other}`")),
            };
            cfg.current_profile = Some(DEFAULT_PROFILE.to_string());
            cfg.save(&path)?;
            eprintln!("saved {}", a.key);
            Ok(())
        }
        Action::Unset(a) => {
            let mut cfg = FileConfig::load(&path)?;
            let profile = cfg.profile_mut(DEFAULT_PROFILE);
            match a.key.as_str() {
                "api_key" => profile.api_key = None,
                "transactional_key" => profile.transactional_key = None,
                other => return Err(anyhow!("unknown config key `{other}`")),
            };
            cfg.save(&path)?;
            eprintln!("cleared {}", a.key);
            Ok(())
        }
    }
}

use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};

use crate::config::{config_path, FileConfig, DEFAULT_PROFILE};
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
    /// Print the contents of the config file (TOML).
    Show,
    /// Read one value (`api_key`, `transactional_key`).
    Get(GetArgs),
    /// Set one value.
    Set(SetArgs),
    /// Clear one value.
    Unset(UnsetArgs),
}

#[derive(Debug, Args)]
pub struct GetArgs {
    pub key: String,
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
        Action::Show => {
            let cfg = FileConfig::load(&path)?;
            let text = toml::to_string_pretty(&cfg)?;
            print!("{text}");
            Ok(())
        }
        Action::Get(a) => {
            let cfg = FileConfig::load(&path)?;
            let profile = cfg.profile(DEFAULT_PROFILE).cloned().unwrap_or_default();
            let value = match a.key.as_str() {
                "api_key" => profile.api_key,
                "transactional_key" => profile.transactional_key,
                other => return Err(anyhow!("unknown config key `{other}`")),
            };
            match value {
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

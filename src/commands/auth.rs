use anyhow::{Context, Result};
use clap::Args;
use dialoguer::{theme::ColorfulTheme, Confirm, Password};
use reqwest::Method;

use crate::client::{ApiClient, Auth};
use crate::config::{config_path, mask_key, FileConfig, KeySource, Resolved, DEFAULT_PROFILE};
use crate::output::OutputOpts;

// ─────────────────────────── login ───────────────────────────

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// API key (`api_…`). Prompts if omitted.
    #[arg(long, hide_env_values = true)]
    pub api_key: Option<String>,

    /// Transactional key (`transactional_…`). Optional.
    #[arg(long, hide_env_values = true)]
    pub transactional_key: Option<String>,

    /// Skip the network round-trip that validates the key after entry.
    #[arg(long)]
    pub skip_validation: bool,
}

pub async fn login(
    _client: &ApiClient,
    cfg: &Resolved,
    args: &LoginArgs,
    _out: OutputOpts,
) -> Result<()> {
    let path = config_path()?;
    let mut file = FileConfig::load(&path)?;

    let theme = ColorfulTheme::default();
    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdout())
        && std::io::IsTerminal::is_terminal(&std::io::stdin());

    // API key
    let api_key = if let Some(k) = args.api_key.clone() {
        k
    } else if interactive {
        Password::with_theme(&theme)
            .with_prompt("Enter your `api_…` key")
            .interact()?
    } else {
        anyhow::bail!("--api-key is required when stdin is not a TTY")
    };

    if !api_key.starts_with("api_") {
        eprintln!(
            "warning: management keys (/outbound, /inbound, /webhooks…) should start with `api_`."
        );
    }

    // Validate against a cheap authenticated endpoint.
    if !args.skip_validation {
        eprintln!("→ validating key against {} …", cfg.base_url);
        validate_key(&cfg.base_url, &api_key).await?;
        eprintln!("✓ key accepted");
    }

    // Optional transactional key
    let transactional_key = match args.transactional_key.clone() {
        Some(k) => Some(k),
        None if interactive
            && Confirm::with_theme(&theme)
                .with_prompt("Also save a transactional key for `email send`?")
                .default(false)
                .interact()? =>
        {
            Some(
                Password::with_theme(&theme)
                    .with_prompt("Enter your `transactional_…` key")
                    .interact()?,
            )
        }
        None => None,
    };

    // Save into the single (default) profile.
    let profile = file.profile_mut(DEFAULT_PROFILE);
    profile.api_key = Some(api_key.clone());
    if let Some(k) = transactional_key {
        profile.transactional_key = Some(k);
    }
    file.current_profile = Some(DEFAULT_PROFILE.to_string());
    file.save(&path)?;

    eprintln!("✓ saved to {}", path.display());
    eprintln!("  API key: {}", mask_key(&api_key));
    Ok(())
}

async fn validate_key(base_url: &str, key: &str) -> Result<()> {
    let url = format!("{}/outbound/domains", base_url.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(key)
        .send()
        .await
        .with_context(|| format!("sending validation request to {url}"))?;
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    let body = resp.text().await.unwrap_or_default();
    let snippet = if body.len() > 200 {
        format!("{}…", &body[..200])
    } else {
        body
    };
    anyhow::bail!("key rejected — HTTP {status}: {snippet}")
}

// ─────────────────────────── logout ───────────────────────────

#[derive(Debug, Args)]
pub struct LogoutArgs {
    /// Skip the confirmation prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,
}

pub fn logout(args: &LogoutArgs, _out: OutputOpts) -> Result<()> {
    let path = config_path()?;
    let mut file = FileConfig::load(&path)?;

    if file.profile(DEFAULT_PROFILE).is_none() {
        eprintln!("no saved credentials");
        return Ok(());
    }

    if !args.yes {
        let theme = ColorfulTheme::default();
        let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin());
        if interactive {
            let ok = Confirm::with_theme(&theme)
                .with_prompt("Clear saved credentials?")
                .default(false)
                .interact()?;
            if !ok {
                eprintln!("aborted");
                return Ok(());
            }
        }
    }

    let profile = file.profile_mut(DEFAULT_PROFILE);
    profile.api_key = None;
    profile.transactional_key = None;
    // Keep base_url so the user doesn't lose their endpoint override.
    file.save(&path)?;
    eprintln!("✓ cleared saved credentials");
    Ok(())
}

// ─────────────────────────── whoami ───────────────────────────

pub async fn whoami(client: &ApiClient, cfg: &Resolved, _out: OutputOpts) -> Result<()> {
    match &cfg.api_key {
        Some(k) => println!(
            "API key:           {} ({})",
            mask_key(k),
            cfg.api_key_source.label()
        ),
        None => println!("API key:           — (not set)"),
    }
    match &cfg.transactional_key {
        Some(k) => println!(
            "Transactional key: {} ({})",
            mask_key(k),
            cfg.transactional_key_source.label()
        ),
        None => println!("Transactional key: — (not set)"),
    }

    if cfg.api_key.is_some() {
        eprint!("\nvalidating against API … ");
        let res = client
            .request_json::<(), ()>(Method::GET, "/outbound/domains", Auth::Api, None, None, &[])
            .await;
        match res {
            Ok(_) => eprintln!("✓ key is valid"),
            Err(e) => eprintln!("✗ key rejected: {e}"),
        }
    } else if cfg.api_key_source == KeySource::Missing {
        eprintln!("\nNo API key configured — run `jetemail login` to set one.");
    }
    Ok(())
}

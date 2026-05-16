use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::client::ApiClient;
use crate::commands;
use crate::config::Resolved;
use crate::output::OutputOpts;

#[derive(Debug, Parser)]
#[command(
    name = "jetemail",
    version,
    about = "Command-line interface for the JetEmail API",
    long_about = "Command-line interface for the JetEmail API (https://api.jetemail.com).\n\
                  All 68 endpoints from the OpenAPI specification are supported."
)]
pub struct Cli {
    /// API key for account-management endpoints (overrides env/config).
    #[arg(long, global = true, env = "JETEMAIL_API_KEY", hide_env_values = true)]
    pub api_key: Option<String>,

    /// Transactional key used for `email send` / `email batch` (overrides env/config).
    #[arg(
        long,
        global = true,
        env = "JETEMAIL_TRANSACTIONAL_KEY",
        hide_env_values = true
    )]
    pub transactional_key: Option<String>,

    /// Force machine-readable JSON output (the default when stdout isn't a TTY).
    #[arg(long, global = true)]
    pub json: bool,

    /// Emit compact (single-line) JSON instead of pretty-printed.
    #[arg(long, global = true)]
    pub raw: bool,

    /// Suppress progress spinners and non-essential output.
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Authenticate with the JetEmail API. Stores keys in the local config.
    Login(commands::auth::LoginArgs),
    /// Remove saved credentials.
    Logout(commands::auth::LogoutArgs),
    /// Show the current authentication context and validate the API key.
    Whoami,
    /// Self-test: validates config and API key.
    Doctor,
    /// Generate a shell tab-completion script (run `jetemail completion --help` for setup).
    Completion(commands::completion::Cmd),
    /// Send transactional email (POST /email, POST /email-batch). Alias: `jetemail send …`.
    Email(commands::email::Cmd),
    /// Short form of `email send` — `jetemail send --to … --subject … …`.
    #[command(hide = true)]
    Send(commands::email::SendArgs),
    /// Inbound forwarding, filters, logs, settings.
    Inbound(commands::inbound::Cmd),
    /// Outbound domains, logs, SMTP users, suppression, transactional keys.
    Outbound(commands::outbound::Cmd),
    /// Webhook subscriptions, queries, and replay.
    Webhooks(commands::webhooks::Cmd),
    /// Manage the local config file directly.
    Config(commands::config_cmd::Cmd),
}

pub async fn run(cli: Cli) -> Result<()> {
    let out = OutputOpts {
        json: cli.json,
        raw: cli.raw,
        quiet: cli.quiet,
    };

    // Commands that don't need an authenticated client (and shouldn't fail
    // when no key is configured — e.g. `login`, `completion`, `config`).
    match &cli.command {
        Command::Config(cmd) => return commands::config_cmd::run(cmd, out).await,
        Command::Completion(cmd) => return commands::completion::run(cmd),
        Command::Logout(args) => return commands::auth::logout(args, out),
        _ => {}
    }

    let cfg = Resolved::from_layers(cli.api_key.clone(), cli.transactional_key.clone())?;
    let client = ApiClient::new(cfg.clone())?;

    match cli.command {
        Command::Login(args) => commands::auth::login(&client, &cfg, &args, out).await,
        Command::Whoami => commands::auth::whoami(&client, &cfg, out).await,
        Command::Doctor => commands::doctor::run(&client, &cfg, out).await,
        Command::Email(cmd) => commands::email::run(&client, &cmd, out).await,
        Command::Send(args) => {
            commands::email::run(
                &client,
                &commands::email::Cmd {
                    action: commands::email::Action::Send(args),
                },
                out,
            )
            .await
        }
        Command::Inbound(cmd) => commands::inbound::run(&client, &cmd, out).await,
        Command::Outbound(cmd) => commands::outbound::run(&client, &cmd, out).await,
        Command::Webhooks(cmd) => commands::webhooks::run(&client, &cmd, out).await,
        Command::Config(_) | Command::Completion(_) | Command::Logout(_) => unreachable!(),
    }
}

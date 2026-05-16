use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::ApiClient;
use crate::output::OutputOpts;

pub mod account;
pub mod destinations;
pub mod domains;
pub mod filters;
pub mod logs;
pub mod quarantine;
pub mod settings;

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Account-level allowlist, blocklist, and combined logs.
    Account(account::Cmd),
    /// Inbound domains, DNS check, forwarding rules.
    Domains(domains::Cmd),
    /// Per-domain allowlist / blocklist filters.
    Filters(filters::Cmd),
    /// Forwarding destinations (list, verify, resend verification).
    Destinations(destinations::Cmd),
    /// GET /inbound/logs — inbound email logs.
    Logs(logs::LogsArgs),
    /// POST /inbound/quarantine-release.
    Quarantine(quarantine::Cmd),
    /// GET / PATCH /inbound/settings/{uuid}.
    Settings(settings::Cmd),
}

pub async fn run(client: &ApiClient, cmd: &Cmd, out: OutputOpts) -> Result<()> {
    match &cmd.action {
        Action::Account(c) => account::run(client, c, out).await,
        Action::Domains(c) => domains::run(client, c, out).await,
        Action::Filters(c) => filters::run(client, c, out).await,
        Action::Destinations(c) => destinations::run(client, c, out).await,
        Action::Logs(args) => logs::run(client, args, out).await,
        Action::Quarantine(c) => quarantine::run(client, c, out).await,
        Action::Settings(c) => settings::run(client, c, out).await,
    }
}

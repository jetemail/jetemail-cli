use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::ApiClient;
use crate::output::OutputOpts;

pub mod domains;
pub mod keys;
pub mod logs;
pub mod smarthost;
pub mod suppression;

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Outbound domains, DNS check, per-domain settings.
    Domains(domains::Cmd),
    /// GET /outbound/logs and /outbound/logs/uid.
    Logs(logs::LogsCmd),
    /// SMTP smarthost users.
    Smarthost(smarthost::Cmd),
    /// Suppression list rules.
    Suppression(suppression::Cmd),
    /// Transactional API keys (token rotation, restrictions).
    Keys(keys::Cmd),
}

pub async fn run(client: &ApiClient, cmd: &Cmd, out: OutputOpts) -> Result<()> {
    match &cmd.action {
        Action::Domains(c) => domains::run(client, c, out).await,
        Action::Logs(c) => logs::run(client, c, out).await,
        Action::Smarthost(c) => smarthost::run(client, c, out).await,
        Action::Suppression(c) => suppression::run(client, c, out).await,
        Action::Keys(c) => keys::run(client, c, out).await,
    }
}

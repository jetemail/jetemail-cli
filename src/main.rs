#![allow(clippy::large_enum_variant, clippy::too_many_arguments)]

mod cli;
mod client;
mod commands;
mod config;
mod output;
mod tui;
mod version_check;

use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = cli::Cli::parse();

    // Decide whether to nag about a new release after the command runs.
    // Suppress when the user asked for machine-readable output (JSON) or
    // explicitly silenced output, and when they're already running `update`
    // (which surfaces version info itself).
    let nag = !cli.quiet && !cli.json && !matches!(cli.command, cli::Command::Update(_));

    let result = cli::run(cli).await;
    if let Err(err) = &result {
        eprintln!("error: {err:#}");
    }

    if nag {
        if let Some(msg) = version_check::check_outdated().await {
            eprintln!("{msg}");
        }
    }

    if result.is_err() {
        std::process::exit(1);
    }
}

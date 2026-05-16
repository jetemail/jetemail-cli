#![allow(clippy::large_enum_variant, clippy::too_many_arguments)]

mod cli;
mod client;
mod commands;
mod config;
mod output;
mod tui;

use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = cli::Cli::parse();
    if let Err(err) = cli::run(cli).await {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

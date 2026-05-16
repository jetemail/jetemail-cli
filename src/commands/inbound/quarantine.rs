use anyhow::Result;
use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::json;

use crate::client::{ApiClient, Auth};
use crate::output::{print_value, OutputOpts};

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// POST /inbound/quarantine-release — release email from quarantine by UID.
    Release(ReleaseArgs),
}

#[derive(Debug, Args)]
pub struct ReleaseArgs {
    /// UID of the quarantined message.
    pub uid: String,
}

pub async fn run(client: &ApiClient, cmd: &Cmd, out: OutputOpts) -> Result<()> {
    match &cmd.action {
        Action::Release(a) => {
            let body = json!({ "uid": a.uid });
            let v = client
                .request_json::<(), _>(
                    Method::POST,
                    "/inbound/quarantine-release",
                    Auth::Api,
                    None,
                    Some(&body),
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
    }
}

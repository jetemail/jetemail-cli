use anyhow::Result;
use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::json;

use crate::client::{ApiClient, Auth};
use crate::output::{extract_rows, print_table, print_value, truncate_value, Column, OutputOpts};

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// GET /inbound/forward-destinations.
    List,
    /// POST /inbound/forward-destinations/verify (no auth — consumes verification token).
    Verify(VerifyArgs),
    /// POST /inbound/forward-destinations/{uuid}/resend.
    Resend(UuidArg),
}

#[derive(Debug, Args)]
pub struct VerifyArgs {
    /// Verification token from the email link.
    pub token: String,
}

#[derive(Debug, Args)]
pub struct UuidArg {
    pub uuid: String,
}

fn destinations_columns() -> Vec<Column> {
    vec![
        Column::new("Email", "email").with_transform(|v| truncate_value(v, 50)),
        Column::new("Verified", "verified")
            .with_transform(crate::output::bool_check)
            .with_color(crate::output::bool_check_color),
        Column::new("UUID", "uuid"),
    ]
}

pub async fn run(client: &ApiClient, cmd: &Cmd, out: OutputOpts) -> Result<()> {
    match &cmd.action {
        Action::List => {
            let v = client
                .request_json::<(), ()>(
                    Method::GET,
                    "/inbound/forward-destinations",
                    Auth::Api,
                    None,
                    None,
                    &[],
                )
                .await?;
            print_table(&v, out, &destinations_columns(), extract_rows)
        }
        Action::Verify(a) => {
            let body = json!({ "token": a.token });
            let v = client
                .request_json::<(), _>(
                    Method::POST,
                    "/inbound/forward-destinations/verify",
                    Auth::None,
                    None,
                    Some(&body),
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
        Action::Resend(a) => {
            let path = format!("/inbound/forward-destinations/{}/resend", a.uuid);
            let v = client
                .request_json::<(), ()>(Method::POST, &path, Auth::Api, None, None, &[])
                .await?;
            print_value(&v, out)
        }
    }
}

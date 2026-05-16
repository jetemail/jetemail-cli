use anyhow::Result;
use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::{json, Map, Value};

use crate::client::{ApiClient, Auth};
use crate::commands::util::{insert_opt, insert_vec, merge, parse_body_json, parse_field_pairs};
use crate::output::{extract_rows, print_table, print_value, Column, OutputOpts};

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// GET /outbound/domains.
    List,
    /// POST /outbound/domains.
    #[command(visible_alias = "add")]
    Create(AddArgs),
    /// DELETE /outbound/domains (body: `{ "uuid": "..." }`).
    Delete(UuidArg),
    /// POST /outbound/domains/{uuid}/check.
    Check(UuidArg),
    /// Per-domain settings (GET/POST /outbound/domains/{uuid}/settings).
    Settings(SettingsCmd),
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// Domain to add (e.g. `mail.example.com`).
    pub domain: String,
}

#[derive(Debug, Args)]
pub struct UuidArg {
    pub uuid: String,
}

#[derive(Debug, Args)]
pub struct SettingsCmd {
    #[command(subcommand)]
    pub action: SettingsAction,
}

#[derive(Debug, Subcommand)]
pub enum SettingsAction {
    Get(UuidArg),
    Update(SettingsUpdateArgs),
}

#[derive(Debug, Args)]
pub struct SettingsUpdateArgs {
    pub uuid: String,
    /// BCC every outgoing email to this address. Repeatable.
    #[arg(long = "bcc")]
    pub bcc_emails: Vec<String>,
    #[arg(long = "open-tracking")]
    pub open_tracking: Option<bool>,
    #[arg(long = "click-tracking")]
    pub click_tracking: Option<bool>,
    #[arg(long = "body-json")]
    pub body_json: Option<String>,
    #[arg(long = "field", value_name = "KEY=VALUE")]
    pub field: Vec<String>,
}

fn outbound_domain_columns() -> Vec<Column> {
    vec![
        Column::new("Domain", "domain"),
        Column::new("Status", "status"),
        Column::new("SPF", "dns.spf.verified")
            .with_transform(crate::output::bool_check)
            .with_color(crate::output::bool_check_color),
        Column::new("DKIM", "dns.dkim.verified")
            .with_transform(crate::output::bool_check)
            .with_color(crate::output::bool_check_color),
        Column::new("DMARC", "dns.dmarc.verified")
            .with_transform(crate::output::bool_check)
            .with_color(crate::output::bool_check_color),
        Column::new("Tracking", "dns.tracking.verified")
            .with_transform(crate::output::bool_check)
            .with_color(crate::output::bool_check_color),
    ]
}

pub async fn run(client: &ApiClient, cmd: &Cmd, out: OutputOpts) -> Result<()> {
    match &cmd.action {
        Action::List => {
            let v = client
                .request_json::<(), ()>(
                    Method::GET,
                    "/outbound/domains",
                    Auth::Api,
                    None,
                    None,
                    &[],
                )
                .await?;
            print_table(&v, out, &outbound_domain_columns(), extract_rows)
        }
        Action::Create(a) => {
            let body = json!({ "domain": a.domain });
            let v = client
                .request_json::<(), _>(
                    Method::POST,
                    "/outbound/domains",
                    Auth::Api,
                    None,
                    Some(&body),
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
        Action::Delete(a) => {
            let body = json!({ "uuid": a.uuid });
            let v = client
                .request_json::<(), _>(
                    Method::DELETE,
                    "/outbound/domains",
                    Auth::Api,
                    None,
                    Some(&body),
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
        Action::Check(a) => {
            let path = format!("/outbound/domains/{}/check", a.uuid);
            let v = client
                .request_json::<(), ()>(Method::POST, &path, Auth::Api, None, None, &[])
                .await?;
            print_value(&v, out)
        }
        Action::Settings(c) => settings(client, c, out).await,
    }
}

async fn settings(client: &ApiClient, cmd: &SettingsCmd, out: OutputOpts) -> Result<()> {
    match &cmd.action {
        SettingsAction::Get(a) => {
            let path = format!("/outbound/domains/{}/settings", a.uuid);
            let v = client
                .request_json::<(), ()>(Method::GET, &path, Auth::Api, None, None, &[])
                .await?;
            print_value(&v, out)
        }
        SettingsAction::Update(a) => {
            let mut body = Map::new();
            if let Some(s) = &a.body_json {
                merge(&mut body, parse_body_json(s)?);
            }
            if !a.field.is_empty() {
                merge(&mut body, parse_field_pairs(&a.field)?);
            }
            insert_vec(&mut body, "bccEmails", &a.bcc_emails);
            insert_opt(&mut body, "openTracking", a.open_tracking);
            insert_opt(&mut body, "clickTracking", a.click_tracking);
            let path = format!("/outbound/domains/{}/settings", a.uuid);
            let v = client
                .request_json::<(), _>(
                    Method::POST,
                    &path,
                    Auth::Api,
                    None,
                    Some(&Value::Object(body)),
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
    }
}

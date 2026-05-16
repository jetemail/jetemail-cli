use anyhow::Result;
use clap::{Args, Subcommand};
use reqwest::Method;
use serde::Serialize;
use serde_json::Value;

use crate::client::{ApiClient, Auth};
use crate::commands::inbound::account::{
    build_rule_body, build_rule_update_body, RuleBody, RuleUpdate, UuidArg,
};
use crate::output::{print_value, OutputOpts};

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Domain allowlist rules.
    Allowlist(FilterCmd),
    /// Domain blocklist rules.
    Blocklist(FilterCmd),
}

#[derive(Debug, Args)]
pub struct FilterCmd {
    #[command(subcommand)]
    pub action: FilterAction,
}

#[derive(Debug, Subcommand)]
pub enum FilterAction {
    /// List filter rules (optionally scoped to a domain or subdomain).
    List(ListArgs),
    /// Get a single filter rule.
    Get(UuidArg),
    /// Create a filter rule.
    Create(CreateArgs),
    /// Update a filter rule.
    Update(RuleUpdate),
    /// Delete a filter rule.
    Delete(UuidArg),
}

#[derive(Debug, Args, Serialize)]
pub struct ListArgs {
    #[arg(long)]
    pub domain: Option<String>,
    #[arg(long = "subdomain-id")]
    #[serde(rename = "subdomain_id", skip_serializing_if = "Option::is_none")]
    pub subdomain_id: Option<i64>,
    #[arg(long = "subdomain-uuid")]
    #[serde(rename = "subdomain_uuid", skip_serializing_if = "Option::is_none")]
    pub subdomain_uuid: Option<String>,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[command(flatten)]
    pub query: ListArgs,
    #[command(flatten)]
    pub body: RuleBody,
}

pub async fn run(client: &ApiClient, cmd: &Cmd, out: OutputOpts) -> Result<()> {
    match &cmd.action {
        Action::Allowlist(c) => run_filter(client, c, "/inbound/filters/allowlist", out).await,
        Action::Blocklist(c) => run_filter(client, c, "/inbound/filters/blocklist", out).await,
    }
}

async fn run_filter(
    client: &ApiClient,
    cmd: &FilterCmd,
    base_path: &str,
    out: OutputOpts,
) -> Result<()> {
    match &cmd.action {
        FilterAction::List(q) => {
            let v = client
                .request_json::<_, ()>(Method::GET, base_path, Auth::Api, Some(q), None, &[])
                .await?;
            print_value(&v, out)
        }
        FilterAction::Get(a) => {
            let path = format!("{}/{}", base_path, a.uuid);
            let v = client
                .request_json::<(), ()>(Method::GET, &path, Auth::Api, None, None, &[])
                .await?;
            print_value(&v, out)
        }
        FilterAction::Create(c) => {
            // Filter create allows `domain` in the body too — pass through if user set it.
            let mut body = build_rule_body(&c.body)?;
            if let Some(d) = &c.query.domain {
                body.entry("domain".to_string())
                    .or_insert_with(|| Value::String(d.clone()));
            }
            let v = client
                .request_json::<_, _>(
                    Method::POST,
                    base_path,
                    Auth::Api,
                    Some(&c.query),
                    Some(&Value::Object(body)),
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
        FilterAction::Update(u) => {
            let body = build_rule_update_body(u)?;
            let path = format!("{}/{}", base_path, u.uuid);
            let v = client
                .request_json::<(), _>(
                    Method::PATCH,
                    &path,
                    Auth::Api,
                    None,
                    Some(&Value::Object(body)),
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
        FilterAction::Delete(a) => {
            let path = format!("{}/{}", base_path, a.uuid);
            let v = client
                .request_json::<(), ()>(Method::DELETE, &path, Auth::Api, None, None, &[])
                .await?;
            print_value(&v, out)
        }
    }
}

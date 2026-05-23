use anyhow::Result;
use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::{Map, Value};

use crate::client::{enc, ApiClient, Auth};
use crate::commands::util::{insert_opt, insert_vec, merge, parse_body_json, parse_field_pairs};
use crate::output::{print_value, OutputOpts};

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// GET /inbound/settings/{uuid}.
    Get(UuidArg),
    /// PATCH /inbound/settings/{uuid}.
    Update(UpdateArgs),
}

#[derive(Debug, Args)]
pub struct UuidArg {
    pub uuid: String,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    pub uuid: String,
    /// `active` or `disabled`.
    #[arg(long)]
    pub status: Option<String>,
    /// `smtp`, `webhook`, or `forward`.
    #[arg(long = "delivery-type")]
    pub delivery_type: Option<String>,
    #[arg(long = "smtp-server")]
    pub smtp_servers: Vec<String>,
    #[arg(long = "webhook-url")]
    pub webhook_url: Option<String>,
    #[arg(long = "webhook-method")]
    pub webhook_method: Option<String>,
    #[arg(long = "webhook-header", value_name = "NAME:VALUE")]
    pub webhook_headers: Vec<String>,
    #[arg(long = "webhook-timeout")]
    pub webhook_timeout: Option<i64>,
    #[arg(long = "webhook-retry-count")]
    pub webhook_retry_count: Option<i64>,
    #[arg(long = "webhook-auth-header")]
    pub webhook_auth_header: Option<String>,
    #[arg(long = "body-json")]
    pub body_json: Option<String>,
    #[arg(long = "field", value_name = "KEY=VALUE")]
    pub field: Vec<String>,
}

pub async fn run(client: &ApiClient, cmd: &Cmd, out: OutputOpts) -> Result<()> {
    match &cmd.action {
        Action::Get(a) => {
            let path = format!("/inbound/settings/{}", enc(&a.uuid));
            let v = client
                .request_json::<(), ()>(Method::GET, &path, Auth::Api, None, None, &[])
                .await?;
            print_value(&v, out)
        }
        Action::Update(a) => {
            let mut body = Map::new();
            if let Some(s) = &a.body_json {
                merge(&mut body, parse_body_json(s)?);
            }
            if !a.field.is_empty() {
                merge(&mut body, parse_field_pairs(&a.field)?);
            }
            insert_opt(&mut body, "status", a.status.clone());
            insert_opt(&mut body, "delivery_type", a.delivery_type.clone());
            insert_vec(&mut body, "smtp_servers", &a.smtp_servers);
            insert_opt(&mut body, "webhook_url", a.webhook_url.clone());
            insert_opt(&mut body, "webhook_method", a.webhook_method.clone());
            if !a.webhook_headers.is_empty() {
                let mut hmap = Map::new();
                for h in &a.webhook_headers {
                    let (k, v) = h
                        .split_once(':')
                        .ok_or_else(|| anyhow::anyhow!("--webhook-header expects `Name: Value`"))?;
                    hmap.insert(k.trim().to_string(), Value::String(v.trim().to_string()));
                }
                body.insert("webhook_headers".to_string(), Value::Object(hmap));
            }
            insert_opt(&mut body, "webhook_timeout", a.webhook_timeout);
            insert_opt(&mut body, "webhook_retry_count", a.webhook_retry_count);
            insert_opt(
                &mut body,
                "webhook_auth_header",
                a.webhook_auth_header.clone(),
            );
            let path = format!("/inbound/settings/{}", enc(&a.uuid));
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
    }
}

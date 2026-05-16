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
    /// GET /inbound/domains.
    List,
    /// POST /inbound/domains.
    #[command(visible_alias = "add")]
    Create(AddArgs),
    /// DELETE /inbound/domains (body: `{ "uuid": "..." }`).
    Delete(DeleteArgs),
    /// POST /inbound/domains/{uuid}/check.
    Check(UuidArg),
    /// Forwarding rules for a domain.
    ForwardRules(ForwardRulesCmd),
}

#[derive(Debug, Args)]
pub struct UuidArg {
    pub uuid: String,
}

#[derive(Debug, Args)]
pub struct DeleteArgs {
    pub uuid: String,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    #[arg(long)]
    pub domain: Option<String>,
    /// `smtp`, `webhook`, or `forward`.
    #[arg(long = "delivery-type")]
    pub delivery_type: Option<String>,
    /// SMTP server `host:port` (repeatable, only used with delivery-type=smtp).
    #[arg(long = "smtp-server")]
    pub smtp_servers: Vec<String>,
    #[arg(long = "webhook-url")]
    pub webhook_url: Option<String>,
    #[arg(long = "webhook-method")]
    pub webhook_method: Option<String>,
    /// Webhook header `Name: Value` (repeatable).
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

#[derive(Debug, Args)]
pub struct ForwardRulesCmd {
    #[command(subcommand)]
    pub action: ForwardRulesAction,
}

#[derive(Debug, Subcommand)]
pub enum ForwardRulesAction {
    /// List forwarding rules for a domain.
    List(UuidArg),
    /// Add a forwarding rule.
    Add(AddRuleArgs),
    /// Get a single forwarding rule.
    Get(RulePair),
    /// Update a forwarding rule.
    Update(UpdateRuleArgs),
    /// Delete a forwarding rule.
    Delete(RulePair),
}

#[derive(Debug, Args)]
pub struct RulePair {
    /// Domain UUID.
    pub uuid: String,
    /// Forwarding rule UUID.
    pub rule_uuid: String,
}

#[derive(Debug, Args)]
pub struct AddRuleArgs {
    /// Domain UUID.
    pub uuid: String,
    #[arg(long)]
    pub localpart: Option<String>,
    #[arg(long)]
    pub destination: Option<String>,
    #[arg(long)]
    pub active: Option<bool>,
    #[arg(long = "body-json")]
    pub body_json: Option<String>,
    #[arg(long = "field", value_name = "KEY=VALUE")]
    pub field: Vec<String>,
}

#[derive(Debug, Args)]
pub struct UpdateRuleArgs {
    pub uuid: String,
    pub rule_uuid: String,
    #[arg(long)]
    pub destination: Option<String>,
    #[arg(long)]
    pub active: Option<bool>,
    #[arg(long = "body-json")]
    pub body_json: Option<String>,
    #[arg(long = "field", value_name = "KEY=VALUE")]
    pub field: Vec<String>,
}

fn inbound_domain_columns() -> Vec<Column> {
    use serde_json::Value;
    vec![
        Column::new("Domain", "domain"),
        Column::new("Delivery", "delivery_type"),
        Column::new("Status", "status").with_transform(|v| match v {
            // The list endpoint returns numeric status; the update endpoint accepts
            // the string `active` / `disabled`. Normalize for display.
            Value::Number(n) => match n.as_i64() {
                Some(1) => "active".into(),
                Some(0) => "disabled".into(),
                Some(other) => other.to_string(),
                None => "—".into(),
            },
            Value::String(s) => s.clone(),
            _ => "—".into(),
        }),
        Column::new("UUID", "uuid"),
    ]
}

pub async fn run(client: &ApiClient, cmd: &Cmd, out: OutputOpts) -> Result<()> {
    match &cmd.action {
        Action::List => {
            let v = client
                .request_json::<(), ()>(Method::GET, "/inbound/domains", Auth::Api, None, None, &[])
                .await?;
            print_table(&v, out, &inbound_domain_columns(), extract_rows)
        }
        Action::Create(args) => {
            let body = build_add_body(args)?;
            let v = client
                .request_json::<(), _>(
                    Method::POST,
                    "/inbound/domains",
                    Auth::Api,
                    None,
                    Some(&Value::Object(body)),
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
        Action::Delete(args) => {
            let body = json!({ "uuid": args.uuid });
            let v = client
                .request_json::<(), _>(
                    Method::DELETE,
                    "/inbound/domains",
                    Auth::Api,
                    None,
                    Some(&body),
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
        Action::Check(a) => {
            let path = format!("/inbound/domains/{}/check", a.uuid);
            let v = client
                .request_json::<(), ()>(Method::POST, &path, Auth::Api, None, None, &[])
                .await?;
            print_value(&v, out)
        }
        Action::ForwardRules(fr) => forward_rules(client, fr, out).await,
    }
}

fn build_add_body(args: &AddArgs) -> Result<Map<String, Value>> {
    let mut body = Map::new();
    if let Some(s) = &args.body_json {
        merge(&mut body, parse_body_json(s)?);
    }
    if !args.field.is_empty() {
        merge(&mut body, parse_field_pairs(&args.field)?);
    }
    insert_opt(&mut body, "domain", args.domain.clone());
    insert_opt(&mut body, "delivery_type", args.delivery_type.clone());
    insert_vec(&mut body, "smtp_servers", &args.smtp_servers);
    insert_opt(&mut body, "webhook_url", args.webhook_url.clone());
    insert_opt(&mut body, "webhook_method", args.webhook_method.clone());
    if !args.webhook_headers.is_empty() {
        let mut hmap = Map::new();
        for h in &args.webhook_headers {
            let (k, v) = h
                .split_once(':')
                .ok_or_else(|| anyhow::anyhow!("--webhook-header expects `Name: Value`"))?;
            hmap.insert(k.trim().to_string(), Value::String(v.trim().to_string()));
        }
        body.insert("webhook_headers".to_string(), Value::Object(hmap));
    }
    insert_opt(&mut body, "webhook_timeout", args.webhook_timeout);
    insert_opt(&mut body, "webhook_retry_count", args.webhook_retry_count);
    insert_opt(
        &mut body,
        "webhook_auth_header",
        args.webhook_auth_header.clone(),
    );
    Ok(body)
}

async fn forward_rules(client: &ApiClient, fr: &ForwardRulesCmd, out: OutputOpts) -> Result<()> {
    match &fr.action {
        ForwardRulesAction::List(u) => {
            let path = format!("/inbound/domains/{}/forward-rules", u.uuid);
            let v = client
                .request_json::<(), ()>(Method::GET, &path, Auth::Api, None, None, &[])
                .await?;
            print_value(&v, out)
        }
        ForwardRulesAction::Add(args) => {
            let mut body = Map::new();
            if let Some(s) = &args.body_json {
                merge(&mut body, parse_body_json(s)?);
            }
            if !args.field.is_empty() {
                merge(&mut body, parse_field_pairs(&args.field)?);
            }
            insert_opt(&mut body, "localpart", args.localpart.clone());
            insert_opt(&mut body, "destination", args.destination.clone());
            insert_opt(&mut body, "active", args.active);
            let path = format!("/inbound/domains/{}/forward-rules", args.uuid);
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
        ForwardRulesAction::Get(p) => {
            let path = format!("/inbound/domains/{}/forward-rules/{}", p.uuid, p.rule_uuid);
            let v = client
                .request_json::<(), ()>(Method::GET, &path, Auth::Api, None, None, &[])
                .await?;
            print_value(&v, out)
        }
        ForwardRulesAction::Update(args) => {
            let mut body = Map::new();
            if let Some(s) = &args.body_json {
                merge(&mut body, parse_body_json(s)?);
            }
            if !args.field.is_empty() {
                merge(&mut body, parse_field_pairs(&args.field)?);
            }
            insert_opt(&mut body, "destination", args.destination.clone());
            insert_opt(&mut body, "active", args.active);
            let path = format!(
                "/inbound/domains/{}/forward-rules/{}",
                args.uuid, args.rule_uuid
            );
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
        ForwardRulesAction::Delete(p) => {
            let path = format!("/inbound/domains/{}/forward-rules/{}", p.uuid, p.rule_uuid);
            let v = client
                .request_json::<(), ()>(Method::DELETE, &path, Auth::Api, None, None, &[])
                .await?;
            print_value(&v, out)
        }
    }
}

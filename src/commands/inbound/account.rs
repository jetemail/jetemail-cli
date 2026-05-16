use anyhow::Result;
use clap::{Args, Subcommand};
use reqwest::Method;
use serde::Serialize;
use serde_json::Value;

use crate::client::{args_to_pairs, ApiClient, Auth};
use crate::commands::util::{insert_opt, insert_vec, merge, parse_body_json, parse_field_pairs};
use crate::output::{print_value, OutputOpts};
use crate::tui::logs::{LogKind, LogSource, TailOpts};

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Account-level allowlist rules.
    Allowlist(RuleCmd),
    /// Account-level blocklist rules.
    Blocklist(RuleCmd),
    /// GET /inbound/account/logs.
    Logs(LogsArgs),
}

#[derive(Debug, Args)]
pub struct RuleCmd {
    #[command(subcommand)]
    pub action: RuleAction,
}

#[derive(Debug, Subcommand)]
pub enum RuleAction {
    /// List rules.
    List,
    /// Get a single rule by UUID.
    Get(UuidArg),
    /// Create a rule.
    Create(RuleBody),
    /// Update a rule (PATCH).
    Update(RuleUpdate),
    /// Delete a rule.
    Delete(UuidArg),
}

#[derive(Debug, Args)]
pub struct UuidArg {
    pub uuid: String,
}

#[derive(Debug, Args)]
pub struct RuleBody {
    /// Human-readable rule name (required by API).
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
    /// Sender pattern (repeatable).
    #[arg(long = "sender")]
    pub senders: Vec<String>,
    /// Recipient pattern (repeatable).
    #[arg(long = "recipient")]
    pub recipients: Vec<String>,
    /// Subject pattern (repeatable).
    #[arg(long = "subject")]
    pub subjects: Vec<String>,
    /// Body pattern (repeatable).
    #[arg(long = "body")]
    pub bodies: Vec<String>,
    /// Header pattern (repeatable, free-form).
    #[arg(long = "header")]
    pub headers: Vec<String>,
    /// IP pattern (repeatable).
    #[arg(long = "ip")]
    pub ips: Vec<String>,
    /// Attachment pattern (repeatable).
    #[arg(long = "attachment")]
    pub attachments: Vec<String>,
    /// Free-form JSON merged into the body. `@file`, `-`, or literal.
    #[arg(long = "body-json")]
    pub body_json: Option<String>,
    /// Extra body field `key=value`.
    #[arg(long = "field", value_name = "KEY=VALUE")]
    pub field: Vec<String>,
}

#[derive(Debug, Args)]
pub struct RuleUpdate {
    pub uuid: String,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
    #[arg(long)]
    pub enabled: Option<bool>,
    #[arg(long = "sender")]
    pub senders: Vec<String>,
    #[arg(long = "recipient")]
    pub recipients: Vec<String>,
    #[arg(long = "subject")]
    pub subjects: Vec<String>,
    #[arg(long = "body")]
    pub bodies: Vec<String>,
    #[arg(long = "header")]
    pub headers: Vec<String>,
    #[arg(long = "ip")]
    pub ips: Vec<String>,
    #[arg(long = "attachment")]
    pub attachments: Vec<String>,
    #[arg(long = "body-json")]
    pub body_json: Option<String>,
    #[arg(long = "field", value_name = "KEY=VALUE")]
    pub field: Vec<String>,
}

#[derive(Debug, Args, Serialize)]
pub struct LogsArgs {
    /// Tail logs live in an interactive TUI. Only entries logged after the
    /// command starts are shown; pass --date-from to include earlier entries
    /// (use 0 to show everything in the buffer).
    #[arg(long)]
    #[serde(skip)]
    pub tail: bool,
    /// Poll interval in seconds when --tail is set.
    #[arg(long = "poll-secs", default_value_t = 5, requires = "tail")]
    #[serde(skip)]
    pub poll_secs: u64,
    /// Maximum entries kept in the tail buffer.
    #[arg(long, default_value_t = 500, requires = "tail")]
    #[serde(skip)]
    pub buffer: usize,
    #[arg(long)]
    pub domains: Option<String>,
    #[arg(long)]
    pub logtype: Option<String>,
    #[arg(long)]
    pub to: Option<String>,
    #[arg(long)]
    pub from: Option<String>,
    #[arg(long)]
    pub subject: Option<String>,
    #[arg(long)]
    pub ip: Option<String>,
    #[arg(long = "date-from")]
    #[serde(rename = "date_from", skip_serializing_if = "Option::is_none")]
    pub date_from: Option<i64>,
    #[arg(long = "date-to")]
    #[serde(rename = "date_to", skip_serializing_if = "Option::is_none")]
    pub date_to: Option<i64>,
    #[arg(long = "spamscore-min")]
    #[serde(rename = "spamscore_min", skip_serializing_if = "Option::is_none")]
    pub spamscore_min: Option<f64>,
    #[arg(long = "spamscore-max")]
    #[serde(rename = "spamscore_max", skip_serializing_if = "Option::is_none")]
    pub spamscore_max: Option<f64>,
    #[arg(long)]
    pub host: Option<String>,
    #[arg(long)]
    pub port: Option<i64>,
    #[arg(long)]
    pub mode: Option<String>,
    #[arg(long)]
    pub limit: Option<i64>,
    #[arg(long)]
    pub offset: Option<i64>,
    #[arg(long)]
    pub page: Option<i64>,
    #[arg(long = "sort-by")]
    #[serde(rename = "sort_by", skip_serializing_if = "Option::is_none")]
    pub sort_by: Option<String>,
    #[arg(long = "sort-order")]
    #[serde(rename = "sort_order", skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<String>,
}

pub async fn run(client: &ApiClient, cmd: &Cmd, out: OutputOpts) -> Result<()> {
    match &cmd.action {
        Action::Allowlist(c) => run_rule(client, c, "/inbound/account/allowlist", out).await,
        Action::Blocklist(c) => run_rule(client, c, "/inbound/account/blocklist", out).await,
        Action::Logs(args) => {
            if args.tail {
                let source = LogSource {
                    client: client.clone(),
                    path: "/inbound/account/logs".to_string(),
                    query: args_to_pairs(args)?,
                    auth: Auth::Api,
                    kind: LogKind::Account,
                };
                return crate::tui::logs::run(
                    source,
                    TailOpts {
                        poll_secs: args.poll_secs,
                        buffer: args.buffer,
                    },
                )
                .await;
            }
            let v = client
                .request_json::<_, ()>(
                    Method::GET,
                    "/inbound/account/logs",
                    Auth::Api,
                    Some(args),
                    None,
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
    }
}

async fn run_rule(
    client: &ApiClient,
    cmd: &RuleCmd,
    base_path: &str,
    out: OutputOpts,
) -> Result<()> {
    match &cmd.action {
        RuleAction::List => {
            let v = client
                .request_json::<(), ()>(Method::GET, base_path, Auth::Api, None, None, &[])
                .await?;
            print_value(&v, out)
        }
        RuleAction::Get(a) => {
            let path = format!("{}/{}", base_path, a.uuid);
            let v = client
                .request_json::<(), ()>(Method::GET, &path, Auth::Api, None, None, &[])
                .await?;
            print_value(&v, out)
        }
        RuleAction::Create(b) => {
            let body = build_rule_body(b)?;
            let v = client
                .request_json::<(), _>(
                    Method::POST,
                    base_path,
                    Auth::Api,
                    None,
                    Some(&Value::Object(body)),
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
        RuleAction::Update(u) => {
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
        RuleAction::Delete(a) => {
            let path = format!("{}/{}", base_path, a.uuid);
            let v = client
                .request_json::<(), ()>(Method::DELETE, &path, Auth::Api, None, None, &[])
                .await?;
            print_value(&v, out)
        }
    }
}

pub fn build_rule_body(b: &RuleBody) -> Result<serde_json::Map<String, Value>> {
    let mut body = serde_json::Map::new();
    if let Some(s) = &b.body_json {
        merge(&mut body, parse_body_json(s)?);
    }
    if !b.field.is_empty() {
        merge(&mut body, parse_field_pairs(&b.field)?);
    }
    insert_opt(&mut body, "name", b.name.clone());
    insert_opt(&mut body, "description", b.description.clone());
    insert_vec(&mut body, "senders", &b.senders);
    insert_vec(&mut body, "recipients", &b.recipients);
    insert_vec(&mut body, "subjects", &b.subjects);
    insert_vec(&mut body, "body", &b.bodies);
    insert_vec(&mut body, "headers", &b.headers);
    insert_vec(&mut body, "ips", &b.ips);
    insert_vec(&mut body, "attachments", &b.attachments);
    Ok(body)
}

pub fn build_rule_update_body(b: &RuleUpdate) -> Result<serde_json::Map<String, Value>> {
    let mut body = serde_json::Map::new();
    if let Some(s) = &b.body_json {
        merge(&mut body, parse_body_json(s)?);
    }
    if !b.field.is_empty() {
        merge(&mut body, parse_field_pairs(&b.field)?);
    }
    insert_opt(&mut body, "name", b.name.clone());
    insert_opt(&mut body, "description", b.description.clone());
    insert_opt(&mut body, "enabled", b.enabled);
    insert_vec(&mut body, "senders", &b.senders);
    insert_vec(&mut body, "recipients", &b.recipients);
    insert_vec(&mut body, "subjects", &b.subjects);
    insert_vec(&mut body, "body", &b.bodies);
    insert_vec(&mut body, "headers", &b.headers);
    insert_vec(&mut body, "ips", &b.ips);
    insert_vec(&mut body, "attachments", &b.attachments);
    Ok(body)
}

use anyhow::Result;
use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::{Map, Value};

use crate::client::{enc, ApiClient, Auth};
use crate::commands::util::{insert_opt, insert_vec, merge, parse_body_json, parse_field_pairs};
use crate::output::{extract_rows, print_table, print_value, truncate_value, Column, OutputOpts};

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// GET /webhooks.
    List,
    /// GET /webhooks/{uuid}.
    Get(UuidArg),
    /// POST /webhooks.
    Create(CreateArgs),
    /// PATCH /webhooks.
    Update(UpdateArgs),
    /// DELETE /webhooks/{uuid}.
    Delete(UuidArg),
    /// POST /webhooks/query — search webhook event history.
    Query(QueryArgs),
    /// POST /webhooks/replay.
    Replay(ReplayArgs),
}

#[derive(Debug, Args)]
pub struct UuidArg {
    pub uuid: String,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub url: Option<String>,
    /// Event type to subscribe to (repeatable). e.g. `outbound.delivered`, `inbound.received`.
    #[arg(long = "event")]
    pub events: Vec<String>,
    /// `0` (disabled) or `1` (enabled).
    #[arg(long)]
    pub status: Option<i64>,
    #[arg(long = "filter-user")]
    pub filter_users: Vec<String>,
    #[arg(long = "filter-domain")]
    pub filter_domains: Vec<String>,
    #[arg(long = "body-json")]
    pub body_json: Option<String>,
    #[arg(long = "field", value_name = "KEY=VALUE")]
    pub field: Vec<String>,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    #[arg(long)]
    pub uuid: String,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub url: Option<String>,
    #[arg(long = "event")]
    pub events: Vec<String>,
    #[arg(long)]
    pub status: Option<i64>,
    #[arg(long = "filter-user")]
    pub filter_users: Vec<String>,
    #[arg(long = "filter-domain")]
    pub filter_domains: Vec<String>,
    #[arg(long = "body-json")]
    pub body_json: Option<String>,
    #[arg(long = "field", value_name = "KEY=VALUE")]
    pub field: Vec<String>,
}

#[derive(Debug, Args)]
pub struct QueryArgs {
    #[arg(long)]
    pub uuid: Option<String>,
    #[arg(long = "event-id")]
    pub event_id: Option<String>,
    #[arg(long = "event-type")]
    pub event_type: Option<String>,
    #[arg(long = "source-uid")]
    pub source_uid: Option<String>,
    #[arg(long)]
    pub status: Option<String>,
    #[arg(long = "date-from")]
    pub date_from: Option<i64>,
    #[arg(long = "date-to")]
    pub date_to: Option<i64>,
    #[arg(long)]
    pub limit: Option<i64>,
    #[arg(long)]
    pub offset: Option<i64>,
    #[arg(long = "body-json")]
    pub body_json: Option<String>,
}

#[derive(Debug, Args)]
pub struct ReplayArgs {
    #[arg(long = "event-id")]
    pub event_id: Option<String>,
    #[arg(long = "source-uid")]
    pub source_uid: Option<String>,
    #[arg(long)]
    pub uuid: Option<String>,
    #[arg(long = "body-json")]
    pub body_json: Option<String>,
}

fn webhooks_columns() -> Vec<Column> {
    use serde_json::Value;
    vec![
        Column::new("Name", "name"),
        Column::new("URL", "url").with_transform(|v| truncate_value(v, 50)),
        Column::new("Events", "events").with_transform(|v| match v {
            Value::Array(a) => match a.len() {
                0 => "—".into(),
                1 => a[0].as_str().unwrap_or("?").to_string(),
                n => format!("{n} types"),
            },
            _ => "—".into(),
        }),
        Column::new("Status", "status").with_transform(|v| match v {
            Value::String(s) => s.clone(),
            Value::Number(_) => match v.as_i64() {
                Some(1) => "enabled".into(),
                Some(0) => "disabled".into(),
                Some(n) => n.to_string(),
                None => "—".into(),
            },
            _ => "—".into(),
        }),
        Column::new("UUID", "uuid"),
    ]
}

pub async fn run(client: &ApiClient, cmd: &Cmd, out: OutputOpts) -> Result<()> {
    match &cmd.action {
        Action::List => {
            let v = client
                .request_json::<(), ()>(Method::GET, "/webhooks", Auth::Api, None, None, &[])
                .await?;
            print_table(&v, out, &webhooks_columns(), extract_rows)
        }
        Action::Get(a) => {
            let path = format!("/webhooks/{}", enc(&a.uuid));
            let v = client
                .request_json::<(), ()>(Method::GET, &path, Auth::Api, None, None, &[])
                .await?;
            print_value(&v, out)
        }
        Action::Create(a) => {
            let mut body = Map::new();
            if let Some(s) = &a.body_json {
                merge(&mut body, parse_body_json(s)?);
            }
            if !a.field.is_empty() {
                merge(&mut body, parse_field_pairs(&a.field)?);
            }
            insert_opt(&mut body, "name", a.name.clone());
            insert_opt(&mut body, "url", a.url.clone());
            insert_vec(&mut body, "events", &a.events);
            insert_opt(&mut body, "status", a.status);
            insert_vec(&mut body, "filter_users", &a.filter_users);
            insert_vec(&mut body, "filter_domains", &a.filter_domains);
            let v = client
                .request_json::<(), _>(
                    Method::POST,
                    "/webhooks",
                    Auth::Api,
                    None,
                    Some(&Value::Object(body)),
                    &[],
                )
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
            body.insert("uuid".to_string(), Value::String(a.uuid.clone()));
            insert_opt(&mut body, "name", a.name.clone());
            insert_opt(&mut body, "url", a.url.clone());
            insert_vec(&mut body, "events", &a.events);
            insert_opt(&mut body, "status", a.status);
            insert_vec(&mut body, "filter_users", &a.filter_users);
            insert_vec(&mut body, "filter_domains", &a.filter_domains);
            let v = client
                .request_json::<(), _>(
                    Method::PATCH,
                    "/webhooks",
                    Auth::Api,
                    None,
                    Some(&Value::Object(body)),
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
        Action::Delete(a) => {
            let path = format!("/webhooks/{}", enc(&a.uuid));
            let v = client
                .request_json::<(), ()>(Method::DELETE, &path, Auth::Api, None, None, &[])
                .await?;
            print_value(&v, out)
        }
        Action::Query(a) => {
            let mut body = Map::new();
            if let Some(s) = &a.body_json {
                merge(&mut body, parse_body_json(s)?);
            }
            insert_opt(&mut body, "uuid", a.uuid.clone());
            insert_opt(&mut body, "event_id", a.event_id.clone());
            insert_opt(&mut body, "event_type", a.event_type.clone());
            insert_opt(&mut body, "source_uid", a.source_uid.clone());
            insert_opt(&mut body, "status", a.status.clone());
            insert_opt(&mut body, "date_from", a.date_from);
            insert_opt(&mut body, "date_to", a.date_to);
            insert_opt(&mut body, "limit", a.limit);
            insert_opt(&mut body, "offset", a.offset);
            let v = client
                .request_json::<(), _>(
                    Method::POST,
                    "/webhooks/query",
                    Auth::Api,
                    None,
                    Some(&Value::Object(body)),
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
        Action::Replay(a) => {
            let mut body = Map::new();
            if let Some(s) = &a.body_json {
                merge(&mut body, parse_body_json(s)?);
            }
            insert_opt(&mut body, "event_id", a.event_id.clone());
            insert_opt(&mut body, "source_uid", a.source_uid.clone());
            insert_opt(&mut body, "uuid", a.uuid.clone());
            let v = client
                .request_json::<(), _>(
                    Method::POST,
                    "/webhooks/replay",
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

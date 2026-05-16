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
    /// GET /outbound/transactional-keys.
    List,
    /// POST /outbound/transactional-keys.
    Create(CreateArgs),
    /// PATCH /outbound/transactional-keys.
    Update(UpdateArgs),
    /// DELETE /outbound/transactional-keys.
    Delete(TokenArg),
    /// POST /outbound/transactional-keys/rotate.
    Rotate(TokenArg),
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long = "allow-all-domains")]
    pub allow_all_domains: Option<bool>,
    #[arg(long = "approved-domain")]
    pub approved_domains: Vec<String>,
    #[arg(long)]
    pub quota: Option<i64>,
    #[arg(long = "ip-restriction")]
    pub ip_restrictions: Vec<String>,
    #[arg(long = "body-json")]
    pub body_json: Option<String>,
    #[arg(long = "field", value_name = "KEY=VALUE")]
    pub field: Vec<String>,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Existing `transactional_…` token to update.
    #[arg(long = "api-token")]
    pub api_token: String,
    /// `0` (disabled) or `1` (enabled).
    #[arg(long)]
    pub status: Option<i64>,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long = "allow-all-domains")]
    pub allow_all_domains: Option<bool>,
    #[arg(long = "approved-domain")]
    pub approved_domains: Vec<String>,
    #[arg(long)]
    pub quota: Option<i64>,
    #[arg(long = "ip-restriction")]
    pub ip_restrictions: Vec<String>,
    #[arg(long = "body-json")]
    pub body_json: Option<String>,
    #[arg(long = "field", value_name = "KEY=VALUE")]
    pub field: Vec<String>,
}

#[derive(Debug, Args)]
pub struct TokenArg {
    /// The `transactional_…` token.
    pub api_token: String,
}

fn keys_columns() -> Vec<Column> {
    use serde_json::Value;
    vec![
        Column::new("Name", "name"),
        Column::new("Token", "api_token"),
        Column::new("Status", "status").with_transform(|v| match v {
            Value::String(s) => s.clone(),
            Value::Number(_) => match v.as_i64() {
                Some(1) => "active".into(),
                Some(0) => "disabled".into(),
                Some(n) => n.to_string(),
                None => "—".into(),
            },
            _ => "—".into(),
        }),
        Column::new("All domains", "allowAllDomains")
            .with_transform(crate::output::bool_check)
            .with_color(crate::output::bool_check_color),
        Column::new("Approved domains", "approvedDomains")
            .with_transform(|v| crate::output::render_string_list(v, 80)),
        Column::new("IP restrictions", "ipRestrictions")
            .with_transform(|v| crate::output::render_string_list(v, 80)),
        Column::new("Quota", "quota").with_transform(crate::output::quota_label),
    ]
}

pub async fn run(client: &ApiClient, cmd: &Cmd, out: OutputOpts) -> Result<()> {
    match &cmd.action {
        Action::List => {
            let v = client
                .request_json::<(), ()>(
                    Method::GET,
                    "/outbound/transactional-keys",
                    Auth::Api,
                    None,
                    None,
                    &[],
                )
                .await?;
            print_table(&v, out, &keys_columns(), extract_rows)
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
            insert_opt(&mut body, "allowAllDomains", a.allow_all_domains);
            insert_vec(&mut body, "approvedDomains", &a.approved_domains);
            insert_opt(&mut body, "quota", a.quota);
            insert_vec(&mut body, "ipRestrictions", &a.ip_restrictions);
            let v = client
                .request_json::<(), _>(
                    Method::POST,
                    "/outbound/transactional-keys",
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
            body.insert("api_token".to_string(), Value::String(a.api_token.clone()));
            insert_opt(&mut body, "status", a.status);
            insert_opt(&mut body, "name", a.name.clone());
            insert_opt(&mut body, "allowAllDomains", a.allow_all_domains);
            insert_vec(&mut body, "approvedDomains", &a.approved_domains);
            insert_opt(&mut body, "quota", a.quota);
            insert_vec(&mut body, "ipRestrictions", &a.ip_restrictions);
            let v = client
                .request_json::<(), _>(
                    Method::PATCH,
                    "/outbound/transactional-keys",
                    Auth::Api,
                    None,
                    Some(&Value::Object(body)),
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
        Action::Delete(a) => {
            let body = json!({ "api_token": a.api_token });
            let v = client
                .request_json::<(), _>(
                    Method::DELETE,
                    "/outbound/transactional-keys",
                    Auth::Api,
                    None,
                    Some(&body),
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
        Action::Rotate(a) => {
            let body = json!({ "api_token": a.api_token });
            let v = client
                .request_json::<(), _>(
                    Method::POST,
                    "/outbound/transactional-keys/rotate",
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

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::{json, Map, Value};

use crate::client::{ApiClient, Auth};
use crate::commands::util::{insert_opt, insert_vec, merge, parse_body_json, parse_field_pairs};
use crate::output::{
    extract_rows, print_table, print_value, quota_label, render_string_list, Column, OutputOpts,
};

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// GET /outbound/smarthost.
    List,
    /// POST /outbound/smarthost.
    Create(CreateArgs),
    /// PATCH /outbound/smarthost.
    Update(UpdateArgs),
    /// DELETE /outbound/smarthost.
    Delete(DeleteArgs),
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[arg(long)]
    pub username: Option<String>,
    /// SMTP password. Pass `-` to read from stdin / prompt or `@file` to read
    /// from a file, keeping the secret out of argv (visible in `ps`/shell history).
    #[arg(long)]
    pub password: Option<String>,
    #[arg(long = "allow-any-domain")]
    pub allow_any_domain: Option<bool>,
    #[arg(long = "allow-all-domains")]
    pub allow_all_domains: Option<bool>,
    #[arg(long = "approved-domain")]
    pub approved_domains: Vec<String>,
    #[arg(long)]
    pub quota: Option<f64>,
    #[arg(long = "ip-restriction")]
    pub ip_restrictions: Vec<String>,
    #[arg(long = "body-json")]
    pub body_json: Option<String>,
    #[arg(long = "field", value_name = "KEY=VALUE")]
    pub field: Vec<String>,
}

/// Resolve a `--password` value without forcing the secret onto argv:
///   - `-`      → masked prompt on a TTY, otherwise read from stdin
///   - `@path`  → read (trimmed) from a file
///   - other    → used literally (discouraged: visible in `ps`/shell history)
fn resolve_password(spec: &Option<String>) -> Result<Option<String>> {
    let Some(spec) = spec else {
        return Ok(None);
    };
    let value = if spec == "-" {
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            dialoguer::Password::new()
                .with_prompt("SMTP password")
                .interact()
                .context("reading password")?
        } else {
            use std::io::Read;
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .context("reading password from stdin")?;
            s.trim_end_matches(['\n', '\r']).to_string()
        }
    } else if let Some(path) = spec.strip_prefix('@') {
        std::fs::read_to_string(path)
            .with_context(|| format!("reading password file {path}"))?
            .trim_end_matches(['\n', '\r'])
            .to_string()
    } else {
        spec.clone()
    };
    Ok(Some(value))
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    #[arg(long = "current-username")]
    pub current_username: Option<String>,
    #[arg(long = "new-username")]
    pub new_username: Option<String>,
    /// SMTP password. Pass `-` to read from stdin / prompt or `@file` to read
    /// from a file, keeping the secret out of argv (visible in `ps`/shell history).
    #[arg(long)]
    pub password: Option<String>,
    #[arg(long = "allow-any-domain")]
    pub allow_any_domain: Option<bool>,
    #[arg(long = "allow-all-domains")]
    pub allow_all_domains: Option<bool>,
    #[arg(long = "approved-domain")]
    pub approved_domains: Vec<String>,
    #[arg(long)]
    pub quota: Option<f64>,
    #[arg(long = "ip-restriction")]
    pub ip_restrictions: Vec<String>,
    /// One of: `inactive`, `active`, `suspended`, `quota-locked`, `quota-unlocked`.
    #[arg(long)]
    pub status: Option<String>,
    #[arg(long = "body-json")]
    pub body_json: Option<String>,
    #[arg(long = "field", value_name = "KEY=VALUE")]
    pub field: Vec<String>,
}

#[derive(Debug, Args)]
pub struct DeleteArgs {
    pub username: String,
}

fn smarthost_columns() -> Vec<Column> {
    vec![
        Column::new("Username", "username"),
        Column::new("Status", "status"),
        Column::new("Any domain", "allowAnyDomain")
            .with_transform(crate::output::bool_check)
            .with_color(crate::output::bool_check_color),
        Column::new("All domains", "allowAllDomains")
            .with_transform(crate::output::bool_check)
            .with_color(crate::output::bool_check_color),
        Column::new("Approved domains", "approvedDomains")
            .with_transform(|v| render_string_list(v, 24)),
        Column::new("IP restrictions", "ipRestrictions")
            .with_transform(|v| render_string_list(v, 24)),
        Column::new("Quota", "quota").with_transform(quota_label),
    ]
}

pub async fn run(client: &ApiClient, cmd: &Cmd, out: OutputOpts) -> Result<()> {
    match &cmd.action {
        Action::List => {
            let v = client
                .request_json::<(), ()>(
                    Method::GET,
                    "/outbound/smarthost",
                    Auth::Api,
                    None,
                    None,
                    &[],
                )
                .await?;
            print_table(&v, out, &smarthost_columns(), extract_rows)
        }
        Action::Create(a) => {
            let mut body = Map::new();
            if let Some(s) = &a.body_json {
                merge(&mut body, parse_body_json(s)?);
            }
            if !a.field.is_empty() {
                merge(&mut body, parse_field_pairs(&a.field)?);
            }
            insert_opt(&mut body, "username", a.username.clone());
            insert_opt(&mut body, "password", resolve_password(&a.password)?);
            insert_opt(&mut body, "allowAnyDomain", a.allow_any_domain);
            insert_opt(&mut body, "allowAllDomains", a.allow_all_domains);
            insert_vec(&mut body, "approvedDomains", &a.approved_domains);
            insert_opt(&mut body, "quota", a.quota);
            insert_vec(&mut body, "ipRestrictions", &a.ip_restrictions);
            let v = client
                .request_json::<(), _>(
                    Method::POST,
                    "/outbound/smarthost",
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
            insert_opt(&mut body, "currentUsername", a.current_username.clone());
            insert_opt(&mut body, "newUsername", a.new_username.clone());
            insert_opt(&mut body, "password", resolve_password(&a.password)?);
            insert_opt(&mut body, "allowAnyDomain", a.allow_any_domain);
            insert_opt(&mut body, "allowAllDomains", a.allow_all_domains);
            insert_vec(&mut body, "approvedDomains", &a.approved_domains);
            insert_opt(&mut body, "quota", a.quota);
            insert_vec(&mut body, "ipRestrictions", &a.ip_restrictions);
            insert_opt(&mut body, "status", a.status.clone());
            let v = client
                .request_json::<(), _>(
                    Method::PATCH,
                    "/outbound/smarthost",
                    Auth::Api,
                    None,
                    Some(&Value::Object(body)),
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
        Action::Delete(a) => {
            let body = json!({ "username": a.username });
            let v = client
                .request_json::<(), _>(
                    Method::DELETE,
                    "/outbound/smarthost",
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

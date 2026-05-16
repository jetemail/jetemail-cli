use anyhow::Result;
use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::Value;

use crate::client::{ApiClient, Auth};
use crate::commands::util::read_input;
use crate::output::{
    extract_rows, print_table, print_text, print_value, truncate_value, Column, OutputOpts,
};

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// GET /outbound/suppression.
    List,
    /// POST /outbound/suppression.
    Create(CreateArgs),
    /// DELETE /outbound/suppression/{id} — release a suppression rule.
    Delete(DeleteArgs),
    /// GET /outbound/suppression/export — print CSV to stdout.
    Export,
    /// POST /outbound/suppression/import — upload a CSV (`@path` or `-`).
    Import(ImportArgs),
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    /// Email address, domain, or other target to suppress.
    #[arg(long = "target")]
    pub target_value: String,
    #[arg(long)]
    pub reason: Option<String>,
}

#[derive(Debug, Args)]
pub struct DeleteArgs {
    pub id: i64,
}

#[derive(Debug, Args)]
pub struct ImportArgs {
    /// CSV content: `@path/to/file.csv` or `-` for stdin.
    pub source: String,
}

fn suppression_columns() -> Vec<Column> {
    use serde_json::Value;
    vec![
        Column::new("ID", "id"),
        Column::new("Target", "target_value").with_transform(|v| truncate_value(v, 50)),
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
        Column::new("Created", "created_date").with_transform(|v| truncate_value(v, 19)),
    ]
}

pub async fn run(client: &ApiClient, cmd: &Cmd, out: OutputOpts) -> Result<()> {
    match &cmd.action {
        Action::List => {
            let v = client
                .request_json::<(), ()>(
                    Method::GET,
                    "/outbound/suppression",
                    Auth::Api,
                    None,
                    None,
                    &[],
                )
                .await?;
            print_table(&v, out, &suppression_columns(), extract_rows)
        }
        Action::Create(a) => {
            let mut body = serde_json::Map::new();
            body.insert(
                "target_value".to_string(),
                Value::String(a.target_value.clone()),
            );
            if let Some(r) = &a.reason {
                body.insert("reason".to_string(), Value::String(r.clone()));
            }
            let v = client
                .request_json::<(), _>(
                    Method::POST,
                    "/outbound/suppression",
                    Auth::Api,
                    None,
                    Some(&Value::Object(body)),
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
        Action::Delete(a) => {
            let path = format!("/outbound/suppression/{}", a.id);
            let v = client
                .request_json::<(), ()>(Method::DELETE, &path, Auth::Api, None, None, &[])
                .await?;
            print_value(&v, out)
        }
        Action::Export => {
            let text = client
                .request_text::<(), ()>(
                    Method::GET,
                    "/outbound/suppression/export",
                    Auth::Api,
                    None,
                    None,
                    &[],
                )
                .await?;
            print_text(&text);
            Ok(())
        }
        Action::Import(a) => {
            let csv = read_input(&a.source)?;
            let v = client
                .request_csv::<()>(
                    Method::POST,
                    "/outbound/suppression/import",
                    Auth::Api,
                    None,
                    &csv,
                )
                .await?;
            print_value(&v, out)
        }
    }
}

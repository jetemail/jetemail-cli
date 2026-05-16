use anyhow::Result;
use clap::{Args, Subcommand};
use reqwest::Method;
use serde::Serialize;

use crate::client::{args_to_pairs, ApiClient, Auth};
use crate::output::{print_value, OutputOpts};
use crate::tui::logs::{LogKind, LogSource, TailOpts};

#[derive(Debug, Args)]
pub struct LogsCmd {
    #[command(subcommand)]
    pub action: Option<LogsAction>,
    #[command(flatten)]
    pub query: LogsQuery,
    /// Tail logs live in an interactive TUI. Only entries logged after the
    /// command starts are shown; pass --date-from to include earlier entries
    /// (use 0 to show everything in the buffer).
    #[arg(long, global = false)]
    pub tail: bool,
    /// Poll interval in seconds when --tail is set.
    #[arg(long = "poll-secs", default_value_t = 5, requires = "tail")]
    pub poll_secs: u64,
    /// Maximum entries kept in the tail buffer.
    #[arg(long, default_value_t = 500, requires = "tail")]
    pub buffer: usize,
}

#[derive(Debug, Subcommand)]
pub enum LogsAction {
    /// GET /outbound/logs/uid — lookup a single log entry by UID.
    Uid(UidArgs),
}

#[derive(Debug, Args)]
pub struct UidArgs {
    pub uid: String,
}

#[derive(Debug, Args, Serialize)]
pub struct LogsQuery {
    #[arg(long)]
    pub limit: Option<i64>,
    #[arg(long)]
    pub page: Option<i64>,
    #[arg(long)]
    pub offset: Option<i64>,
    #[arg(long = "sort-by")]
    #[serde(rename = "sort_by", skip_serializing_if = "Option::is_none")]
    pub sort_by: Option<String>,
    #[arg(long = "sort-order")]
    #[serde(rename = "sort_order", skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<String>,
    #[arg(long)]
    pub id: Option<String>,
    #[arg(long)]
    pub uid: Option<String>,
    #[arg(long)]
    pub action: Option<String>,
    #[arg(long)]
    pub zone: Option<String>,
    #[arg(long = "message-id")]
    #[serde(rename = "message_id", skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[arg(long = "from-address")]
    #[serde(rename = "from_address", skip_serializing_if = "Option::is_none")]
    pub from_address: Option<String>,
    #[arg(long = "to-address")]
    #[serde(rename = "to_address", skip_serializing_if = "Option::is_none")]
    pub to_address: Option<String>,
    #[arg(long)]
    pub mx: Option<String>,
    #[arg(long)]
    pub host: Option<String>,
    #[arg(long)]
    pub ip: Option<String>,
    #[arg(long)]
    pub response: Option<String>,
    #[arg(long)]
    pub user: Option<String>,
    #[arg(long)]
    pub protocol: Option<String>,
    #[arg(long)]
    pub src: Option<String>,
    #[arg(long)]
    pub subject: Option<String>,
    #[arg(long)]
    pub md5: Option<String>,
    #[arg(long = "interface")]
    pub interface_: Option<String>,
    #[arg(long)]
    pub originhost: Option<String>,
    #[arg(long)]
    pub transtype: Option<String>,
    #[arg(long = "header-from")]
    #[serde(rename = "headerFrom", skip_serializing_if = "Option::is_none")]
    pub header_from: Option<String>,
    #[arg(long = "date-from")]
    #[serde(rename = "date_from", skip_serializing_if = "Option::is_none")]
    pub date_from: Option<i64>,
    #[arg(long = "date-to")]
    #[serde(rename = "date_to", skip_serializing_if = "Option::is_none")]
    pub date_to: Option<i64>,
    #[arg(long)]
    pub from: Option<String>,
    #[arg(long)]
    pub to: Option<String>,
    #[arg(long = "rule-id")]
    #[serde(rename = "rule_id", skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    #[arg(long)]
    pub search: Option<String>,
}

pub async fn run(client: &ApiClient, cmd: &LogsCmd, out: OutputOpts) -> Result<()> {
    match &cmd.action {
        None => {
            if cmd.tail {
                let source = LogSource {
                    client: client.clone(),
                    path: "/outbound/logs".to_string(),
                    query: args_to_pairs(&cmd.query)?,
                    auth: Auth::Api,
                    kind: LogKind::Outbound,
                };
                return crate::tui::logs::run(
                    source,
                    TailOpts {
                        poll_secs: cmd.poll_secs,
                        buffer: cmd.buffer,
                    },
                )
                .await;
            }
            let v = client
                .request_json::<_, ()>(
                    Method::GET,
                    "/outbound/logs",
                    Auth::Api,
                    Some(&cmd.query),
                    None,
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
        Some(LogsAction::Uid(a)) => {
            #[derive(Serialize)]
            struct Q<'a> {
                uid: &'a str,
            }
            let v = client
                .request_json::<_, ()>(
                    Method::GET,
                    "/outbound/logs/uid",
                    Auth::Api,
                    Some(&Q { uid: &a.uid }),
                    None,
                    &[],
                )
                .await?;
            print_value(&v, out)
        }
    }
}

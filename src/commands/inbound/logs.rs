use anyhow::Result;
use clap::Args;
use reqwest::Method;
use serde::Serialize;

use crate::client::{args_to_pairs, ApiClient, Auth};
use crate::output::{print_value, OutputOpts};
use crate::tui::logs::{LogKind, LogSource, TailOpts};

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
    pub uuid: Option<String>,
    #[arg(long = "subdomain-id")]
    #[serde(rename = "subdomain_id", skip_serializing_if = "Option::is_none")]
    pub subdomain_id: Option<i64>,
    #[arg(long)]
    pub limit: Option<i64>,
    #[arg(long)]
    pub page: Option<i64>,
    #[arg(long)]
    pub offset: Option<i64>,
    #[arg(long)]
    pub uid: Option<String>,
    #[arg(long)]
    pub logtype: Option<String>,
    #[arg(long)]
    pub subject: Option<String>,
    #[arg(long)]
    pub from: Option<String>,
    #[arg(long)]
    pub to: Option<String>,
    #[arg(long = "spamscore-min")]
    #[serde(rename = "spamscore_min", skip_serializing_if = "Option::is_none")]
    pub spamscore_min: Option<f64>,
    #[arg(long = "spamscore-max")]
    #[serde(rename = "spamscore_max", skip_serializing_if = "Option::is_none")]
    pub spamscore_max: Option<f64>,
    #[arg(long)]
    pub host: Option<String>,
    #[arg(long)]
    pub ip: Option<String>,
    #[arg(long = "date-from")]
    #[serde(rename = "date_from", skip_serializing_if = "Option::is_none")]
    pub date_from: Option<i64>,
    #[arg(long = "date-to")]
    #[serde(rename = "date_to", skip_serializing_if = "Option::is_none")]
    pub date_to: Option<i64>,
    #[arg(long = "sort-by")]
    #[serde(rename = "sort_by", skip_serializing_if = "Option::is_none")]
    pub sort_by: Option<String>,
    #[arg(long = "sort-order")]
    #[serde(rename = "sort_order", skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<String>,
}

pub async fn run(client: &ApiClient, args: &LogsArgs, out: OutputOpts) -> Result<()> {
    if args.tail {
        let source = LogSource {
            client: client.clone(),
            path: "/inbound/logs".to_string(),
            query: args_to_pairs(args)?,
            auth: Auth::Api,
            kind: LogKind::Inbound,
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
            "/inbound/logs",
            Auth::Api,
            Some(args),
            None,
            &[],
        )
        .await?;
    print_value(&v, out)
}

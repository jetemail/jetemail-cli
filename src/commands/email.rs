use anyhow::Result;
use clap::{Args, Subcommand};
use dialoguer::{theme::ColorfulTheme, Input};
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Method;
use serde_json::{json, Map, Value};
use std::io::IsTerminal;
use std::time::Duration;

use crate::client::{ApiClient, Auth};
use crate::commands::util::{
    attachment_from_spec, insert_opt, insert_vec, merge, parse_body_json, parse_field_pairs,
};
use crate::output::{print_value, OutputOpts};

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// POST /email — Send a single transactional email.
    Send(SendArgs),
    /// POST /email-batch — Send multiple emails in one request.
    Batch(BatchArgs),
}

#[derive(Debug, Args)]
pub struct SendArgs {
    /// Sender address (e.g. `name@yourdomain.com`).
    #[arg(long)]
    pub from: Option<String>,
    /// Recipient address. Repeat for multiple.
    #[arg(long = "to")]
    pub to: Vec<String>,
    /// Subject line.
    #[arg(long)]
    pub subject: Option<String>,
    /// HTML body (literal string, `@file`, or `-` for stdin).
    #[arg(long)]
    pub html: Option<String>,
    /// Plain-text body (literal string, `@file`, or `-` for stdin).
    #[arg(long)]
    pub text: Option<String>,
    /// CC address. Repeat for multiple.
    #[arg(long = "cc")]
    pub cc: Vec<String>,
    /// BCC address. Repeat for multiple.
    #[arg(long = "bcc")]
    pub bcc: Vec<String>,
    /// Reply-To address.
    #[arg(long = "reply-to")]
    pub reply_to: Option<String>,
    /// Custom header `Name: Value`. Repeat for multiple.
    #[arg(long = "header", value_name = "NAME:VALUE")]
    pub headers: Vec<String>,
    /// Attachment — `@path/to/file` (auto-encoded base64) or a JSON object literal.
    #[arg(long = "attach")]
    pub attach: Vec<String>,
    /// Route via EU region.
    #[arg(long)]
    pub eu: bool,
    /// ISO-8601 timestamp to schedule the send for.
    #[arg(long = "scheduled-at")]
    pub scheduled_at: Option<String>,
    /// Idempotency-Key header (1–256 chars; repeats with same key replay original response).
    #[arg(long = "idempotency-key")]
    pub idempotency_key: Option<String>,
    /// JSON body (object). Merged underneath any explicit flags. `@file` or `-` accepted.
    #[arg(long = "body-json")]
    pub body_json: Option<String>,
    /// Extra body field `key=value` (value parsed as JSON, falls back to string).
    #[arg(long = "field", value_name = "KEY=VALUE")]
    pub field: Vec<String>,
}

#[derive(Debug, Args)]
pub struct BatchArgs {
    /// JSON file/stdin/literal containing either the full batch payload
    /// (`{"emails":[...]}`) or just the array of email objects.
    #[arg(long = "body-json")]
    pub body_json: String,
    /// Idempotency-Key header.
    #[arg(long = "idempotency-key")]
    pub idempotency_key: Option<String>,
}

pub async fn run(client: &ApiClient, cmd: &Cmd, out: OutputOpts) -> Result<()> {
    match &cmd.action {
        Action::Send(args) => send(client, args, out).await,
        Action::Batch(args) => batch(client, args, out).await,
    }
}

async fn send(client: &ApiClient, args: &SendArgs, out: OutputOpts) -> Result<()> {
    let interactive =
        !out.quiet && std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    let theme = ColorfulTheme::default();

    // Resolve required fields with prompting fallback.
    let mut from = args.from.clone();
    let mut to: Vec<String> = args.to.clone();
    let mut subject = args.subject.clone();

    // The body-json source may supply some of these, so only prompt for what's
    // still missing after merging the body sources.
    let has_body_from = args
        .body_json
        .as_ref()
        .map(|s| parse_body_json(s).ok())
        .and_then(|m| m)
        .map(|m| m.contains_key("from"))
        .unwrap_or(false);

    if from.is_none() && !has_body_from && interactive {
        let entered: String = Input::with_theme(&theme)
            .with_prompt("From")
            .interact_text()?;
        from = Some(entered);
    }
    if to.is_empty() && interactive {
        let entered: String = Input::with_theme(&theme)
            .with_prompt("To (comma-separate multiple)")
            .interact_text()?;
        to = entered
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if subject.is_none() && interactive {
        let entered: String = Input::with_theme(&theme)
            .with_prompt("Subject")
            .interact_text()?;
        subject = Some(entered);
    }

    let mut body: Map<String, Value> = Map::new();
    if let Some(spec) = &args.body_json {
        merge(&mut body, parse_body_json(spec)?);
    }
    if !args.field.is_empty() {
        merge(&mut body, parse_field_pairs(&args.field)?);
    }

    insert_opt(&mut body, "from", from.clone());
    insert_opt(&mut body, "subject", subject.clone());
    if !to.is_empty() {
        body.insert(
            "to".to_string(),
            if to.len() == 1 {
                Value::String(to[0].clone())
            } else {
                Value::Array(to.iter().cloned().map(Value::String).collect())
            },
        );
    }
    if let Some(spec) = &args.html {
        body.insert(
            "html".to_string(),
            Value::String(crate::commands::util::read_input(spec)?),
        );
    }
    if let Some(spec) = &args.text {
        body.insert(
            "text".to_string(),
            Value::String(crate::commands::util::read_input(spec)?),
        );
    }
    insert_vec(&mut body, "cc", &args.cc);
    insert_vec(&mut body, "bcc", &args.bcc);
    insert_opt(&mut body, "reply_to", args.reply_to.clone());

    if !args.headers.is_empty() {
        let mut hmap = Map::new();
        for h in &args.headers {
            let (k, v) = h
                .split_once(':')
                .ok_or_else(|| anyhow::anyhow!("--header expects `Name: Value`, got `{h}`"))?;
            hmap.insert(k.trim().to_string(), Value::String(v.trim().to_string()));
        }
        body.insert("headers".to_string(), Value::Object(hmap));
    }
    if !args.attach.is_empty() {
        let mut arr = Vec::new();
        for a in &args.attach {
            arr.push(attachment_from_spec(a)?);
        }
        body.insert("attachments".to_string(), Value::Array(arr));
    }
    if args.eu {
        body.insert("eu".to_string(), Value::Bool(true));
    }
    insert_opt(&mut body, "scheduledAt", args.scheduled_at.clone());

    let mut headers: Vec<(&str, &str)> = Vec::new();
    if let Some(k) = args.idempotency_key.as_deref() {
        headers.push(("Idempotency-Key", k));
    }

    // Spinner while we wait for the POST. Drawn on stderr so JSON on stdout is
    // pristine when piped.
    let spinner = if out.show_chrome() {
        let pb = ProgressBar::new_spinner();
        pb.enable_steady_tick(Duration::from_millis(80));
        pb.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {msg}")
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        pb.set_message("sending email…");
        Some(pb)
    } else {
        None
    };

    let result = client
        .request_json::<(), _>(
            Method::POST,
            "/email",
            Auth::Transactional,
            None,
            Some(&Value::Object(body)),
            &headers,
        )
        .await;

    if let Some(pb) = &spinner {
        pb.finish_and_clear();
    }

    let value = result?;

    // If the user wants JSON (piped or --json), just emit it.
    if !out.is_tty_view() {
        return print_value(&value, out);
    }

    // Pretty success line — pull common fields from the response.
    let id = extract_string(&value, &["id", "uid", "message_id", "messageId"]);
    let recipients = if to.is_empty() {
        value
            .get("to")
            .and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Array(a) => Some(
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
                _ => None,
            })
            .unwrap_or_default()
    } else {
        to.join(", ")
    };

    println!("✓ Email queued");
    if !recipients.is_empty() {
        println!("  to: {recipients}");
    }
    if let Some(s) = subject.as_deref().filter(|s| !s.is_empty()) {
        println!("  subject: {s}");
    }
    if let Some(id) = id {
        println!("  id: {id}");
    }
    Ok(())
}

fn extract_string(v: &Value, keys: &[&str]) -> Option<String> {
    let m = v.as_object()?;
    for k in keys {
        if let Some(Value::String(s)) = m.get(*k) {
            if !s.is_empty() {
                return Some(s.clone());
            }
        }
    }
    None
}

async fn batch(client: &ApiClient, args: &BatchArgs, out: OutputOpts) -> Result<()> {
    let text = crate::commands::util::read_input(&args.body_json)?;
    let parsed: Value = serde_json::from_str(&text)?;
    let body = match parsed {
        Value::Array(arr) => json!({ "emails": arr }),
        Value::Object(map) if map.contains_key("emails") => Value::Object(map),
        Value::Object(map) => {
            // Treat a single-object body as a batch of one email.
            json!({ "emails": [Value::Object(map)] })
        }
        other => anyhow::bail!("batch body must be an object or array, got {other:?}"),
    };

    let mut headers: Vec<(&str, &str)> = Vec::new();
    if let Some(k) = args.idempotency_key.as_deref() {
        headers.push(("Idempotency-Key", k));
    }

    let value = client
        .request_json::<(), _>(
            Method::POST,
            "/email-batch",
            Auth::Transactional,
            None,
            Some(&body),
            &headers,
        )
        .await?;
    print_value(&value, out)
}

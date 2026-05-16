use anyhow::Result;
use reqwest::Method;

use crate::client::{ApiClient, Auth};
use crate::config::{config_path, mask_key, KeySource, Resolved};
use crate::output::OutputOpts;

pub async fn run(client: &ApiClient, cfg: &Resolved, _out: OutputOpts) -> Result<()> {
    let mut failed = false;

    println!("jetemail doctor");
    println!("════════════════");
    print_check("config file", true, format!("{}", config_path()?.display()));

    // API key presence
    let api_ok = cfg.api_key_source != KeySource::Missing;
    print_check(
        "api key",
        api_ok,
        cfg.api_key
            .as_deref()
            .map(|k| format!("{} ({})", mask_key(k), cfg.api_key_source.label()))
            .unwrap_or_else(|| "missing — run `jetemail login`".to_string()),
    );
    if !api_ok {
        failed = true;
    }

    // Transactional key (informational only — not a hard requirement)
    let tx_ok = cfg.transactional_key_source != KeySource::Missing;
    print_check(
        "transactional key",
        tx_ok,
        cfg.transactional_key
            .as_deref()
            .map(|k| format!("{} ({})", mask_key(k), cfg.transactional_key_source.label()))
            .unwrap_or_else(|| "not set (only needed for `email send`)".to_string()),
    );

    // Live API check
    if api_ok {
        match client
            .request_json::<(), ()>(Method::GET, "/outbound/domains", Auth::Api, None, None, &[])
            .await
        {
            Ok(_) => print_check("api reachable", true, "GET /outbound/domains → 2xx".into()),
            Err(e) => {
                let msg = format!("{e:#}");
                let snippet = if msg.len() > 200 {
                    format!("{}…", &msg[..200])
                } else {
                    msg
                };
                print_check("api reachable", false, snippet);
                failed = true;
            }
        }
    }

    println!();
    if failed {
        println!("✗ some checks failed.");
        std::process::exit(1);
    }
    println!("✓ everything looks good.");
    Ok(())
}

fn print_check(label: &str, ok: bool, detail: String) {
    let mark = if ok { "✓" } else { "✗" };
    println!("  {mark}  {label:<20} {detail}");
}

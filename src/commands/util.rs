use anyhow::{anyhow, bail, Context, Result};
use serde_json::{Map, Value};
use std::path::Path;

/// Upper bound on a single `@file` / stdin read or attachment, to avoid loading
/// an unbounded (or `/dev/zero`-style) input entirely into memory.
const MAX_INPUT_BYTES: u64 = 25 * 1024 * 1024;

fn ensure_file_within_cap(path: &str) -> Result<()> {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.is_file() && meta.len() > MAX_INPUT_BYTES {
            bail!(
                "{path} is {} bytes, exceeding the {MAX_INPUT_BYTES}-byte input limit",
                meta.len()
            );
        }
    }
    Ok(())
}

/// Read a file or stdin and return its content as a string.
///
/// `spec` is the user-supplied value:
///   - `"-"`             → read from stdin
///   - `"@path"`         → read from file at `path`
///   - otherwise         → return the literal string
pub fn read_input(spec: &str) -> Result<String> {
    if spec == "-" {
        use std::io::Read;
        let mut buf = String::new();
        // `take` one byte past the cap so an over-limit stream is detectable
        // rather than silently truncated.
        std::io::stdin()
            .take(MAX_INPUT_BYTES + 1)
            .read_to_string(&mut buf)
            .context("reading stdin")?;
        if buf.len() as u64 > MAX_INPUT_BYTES {
            bail!("stdin input exceeds the {MAX_INPUT_BYTES}-byte limit");
        }
        Ok(buf)
    } else if let Some(path) = spec.strip_prefix('@') {
        ensure_file_within_cap(path)?;
        std::fs::read_to_string(path).with_context(|| format!("reading file {path}"))
    } else {
        Ok(spec.to_string())
    }
}

/// Parse `--body-json` (string, @file, or `-` for stdin) into a JSON object map.
pub fn parse_body_json(spec: &str) -> Result<Map<String, Value>> {
    let text = read_input(spec)?;
    let value: Value =
        serde_json::from_str(&text).with_context(|| format!("parsing JSON body from {spec}"))?;
    match value {
        Value::Object(m) => Ok(m),
        _ => bail!("--body-json must be a JSON object"),
    }
}

/// Parse `key=value` repeating flags into a JSON object. Values are parsed as
/// JSON first (so `count=5`, `active=true`, `tags=[\"a\",\"b\"]` all work), and
/// fall back to a plain string otherwise.
pub fn parse_field_pairs(pairs: &[String]) -> Result<Map<String, Value>> {
    let mut out = Map::new();
    for p in pairs {
        let (k, v) = p
            .split_once('=')
            .ok_or_else(|| anyhow!("--field expects key=value, got `{p}`"))?;
        let parsed: Value =
            serde_json::from_str(v).unwrap_or_else(|_| Value::String(v.to_string()));
        out.insert(k.to_string(), parsed);
    }
    Ok(out)
}

/// Merge `extra` on top of `base` (shallow merge — later wins).
pub fn merge(base: &mut Map<String, Value>, extra: Map<String, Value>) {
    for (k, v) in extra {
        base.insert(k, v);
    }
}

/// Convert repeated `Vec<String>` flag occurrences into a JSON array of strings.
pub fn vec_to_array(items: &[String]) -> Value {
    Value::Array(items.iter().cloned().map(Value::String).collect())
}

/// Insert `value` at `key` if `value` is `Some(_)`.
pub fn insert_opt<T: Into<Value>>(map: &mut Map<String, Value>, key: &str, value: Option<T>) {
    if let Some(v) = value {
        map.insert(key.to_string(), v.into());
    }
}

/// Insert `value` at `key` if the vector is non-empty.
pub fn insert_vec(map: &mut Map<String, Value>, key: &str, value: &[String]) {
    if !value.is_empty() {
        map.insert(key.to_string(), vec_to_array(value));
    }
}

/// Build a multipart-style attachment object from `@path` (auto-detects MIME and base64-encodes
/// the bytes) or a literal `name:mime:base64` string.
pub fn attachment_from_spec(spec: &str) -> Result<Value> {
    if let Some(path) = spec.strip_prefix('@') {
        let p = Path::new(path);
        let filename = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        ensure_file_within_cap(path)?;
        let bytes = std::fs::read(p).with_context(|| format!("reading attachment {path}"))?;
        let mime = mime_guess::from_path(p)
            .first_or_octet_stream()
            .essence_str()
            .to_string();
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        Ok(serde_json::json!({
            "filename": filename,
            "content_type": mime,
            "content": encoded,
            "encoding": "base64",
        }))
    } else {
        // Allow user to pass a pre-built JSON attachment object.
        let v: Value = serde_json::from_str(spec)
            .with_context(|| format!("attachment must be `@path` or JSON object, got `{spec}`"))?;
        Ok(v)
    }
}

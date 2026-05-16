use anyhow::Result;
use comfy_table::{presets::UTF8_BORDERS_ONLY, Cell, Color, ContentArrangement, Table};
use serde::Serialize;
use serde_json::Value;
use std::io::IsTerminal;

#[derive(Copy, Clone, Debug)]
pub struct OutputOpts {
    /// User explicitly requested JSON output.
    pub json: bool,
    /// Compact (single-line) JSON.
    pub raw: bool,
    /// Suppress spinners and non-essential chatter.
    pub quiet: bool,
}

impl OutputOpts {
    /// True if we should render structured output as a TTY-friendly table or
    /// formatted block. False if we should emit machine-readable JSON.
    pub fn is_tty_view(&self) -> bool {
        if self.json {
            return false;
        }
        std::io::stdout().is_terminal()
    }

    /// True if we should render spinners, progress bars, status lines.
    pub fn show_chrome(&self) -> bool {
        !self.quiet && std::io::stderr().is_terminal()
    }
}

pub fn print_json<T: Serialize>(value: &T, opts: OutputOpts) -> Result<()> {
    if opts.raw {
        let text = serde_json::to_string(value)?;
        println!("{text}");
    } else {
        let text = serde_json::to_string_pretty(value)?;
        println!("{text}");
    }
    Ok(())
}

pub fn print_value(value: &Value, opts: OutputOpts) -> Result<()> {
    print_json(value, opts)
}

pub fn print_text(text: &str) {
    print!("{text}");
}

/// Render a list-of-rows response as a table when in TTY view, or fall back to
/// JSON. `rows_from(&Value)` extracts the row array from the response (handles
/// envelopes like `{ "data": [...] }`). `columns` defines what to show.
pub fn print_table<F>(
    value: &Value,
    opts: OutputOpts,
    columns: &[Column],
    rows_from: F,
) -> Result<()>
where
    F: Fn(&Value) -> Vec<Value>,
{
    if !opts.is_tty_view() {
        return print_value(value, opts);
    }

    let rows = rows_from(value);
    if rows.is_empty() {
        eprintln!("  (no results)");
        return Ok(());
    }

    let mut table = Table::new();
    table
        .load_preset(UTF8_BORDERS_ONLY)
        .set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(columns.iter().map(|c| Cell::new(c.header).fg(Color::Cyan)));

    for row in &rows {
        let cells = columns.iter().map(|col| {
            let raw = lookup_path(row, col.path);
            let display = match &col.transform {
                Some(f) => f(&raw),
                None => render_scalar(&raw),
            };
            let mut cell = Cell::new(display);
            if let Some(color_fn) = col.color {
                if let Some(c) = color_fn(&raw) {
                    cell = cell.fg(c);
                }
            }
            cell
        });
        table.add_row(cells);
    }

    println!("{table}");
    Ok(())
}

/// Find row arrays in common response envelopes. JetEmail uses
/// `{ "result": [...], "success": true }`, but we also accept Resend-style
/// `data`, generic `items`/`rows`, etc. As a final fallback, if exactly one
/// top-level value is an array, treat that as the rows — this handles
/// resource-named envelopes like `{ smarthosts: [...] }` without needing the
/// resource name baked in here.
pub fn extract_rows(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.clone(),
        Value::Object(m) => {
            for key in [
                "result", "results", "data", "items", "rows", "entries", "records", "list",
            ] {
                if let Some(Value::Array(a)) = m.get(key) {
                    return a.clone();
                }
            }
            // One more level for `{ data: { items: [...] } }`-style.
            for key in ["result", "data"] {
                if let Some(Value::Object(inner)) = m.get(key) {
                    for k in ["items", "rows", "results", "entries", "list"] {
                        if let Some(Value::Array(a)) = inner.get(k) {
                            return a.clone();
                        }
                    }
                }
            }
            // Resource-named envelope: `{ success: true, <name>: [...] }`.
            let arrays: Vec<&Value> = m.values().filter(|v| v.is_array()).collect();
            if arrays.len() == 1 {
                if let Value::Array(a) = arrays[0] {
                    return a.clone();
                }
            }
            // Last resort: treat the object itself as a single row.
            vec![Value::Object(m.clone())]
        }
        _ => vec![],
    }
}

type CellTransform = Box<dyn Fn(&Value) -> String>;

pub struct Column {
    pub header: &'static str,
    /// Dot-separated path into the row (`"id"`, `"settings.enabled"`).
    pub path: &'static str,
    pub transform: Option<CellTransform>,
    pub color: Option<fn(&Value) -> Option<Color>>,
}

impl Column {
    pub fn new(header: &'static str, path: &'static str) -> Self {
        Self {
            header,
            path,
            transform: None,
            color: None,
        }
    }

    pub fn with_transform(mut self, f: impl Fn(&Value) -> String + 'static) -> Self {
        self.transform = Some(Box::new(f));
        self
    }

    pub fn with_color(mut self, f: fn(&Value) -> Option<Color>) -> Self {
        self.color = Some(f);
        self
    }
}

fn lookup_path(v: &Value, path: &str) -> Value {
    let mut cur = v;
    for part in path.split('.') {
        match cur {
            Value::Object(m) => match m.get(part) {
                Some(next) => cur = next,
                None => return Value::Null,
            },
            _ => return Value::Null,
        }
    }
    cur.clone()
}

fn render_scalar(v: &Value) -> String {
    match v {
        Value::Null => "—".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(_) | Value::Object(_) => {
            serde_json::to_string(v).unwrap_or_else(|_| v.to_string())
        }
    }
}

/// Render a quota value: `0` (or null) → "unlimited", everything else → the
/// raw number truncated to 12 chars.
pub fn quota_label(v: &Value) -> String {
    match v {
        Value::Number(n) => match n.as_i64() {
            Some(0) => "unlimited".into(),
            _ => truncate_value(v, 12),
        },
        Value::Null => "unlimited".into(),
        _ => truncate_value(v, 12),
    }
}

/// Render a `string[]`-shaped value as a truncated comma-joined list.
/// Accepts either a real JSON array or a JSON-encoded string (e.g. `"[\"a\"]"`),
/// which is how the API returns array columns persisted as TEXT in D1.
pub fn render_string_list(v: &Value, max: usize) -> String {
    let arr = match v {
        Value::Array(a) => a.clone(),
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(Value::Array(a)) => a,
            _ => return "—".into(),
        },
        _ => return "—".into(),
    };
    if arr.is_empty() {
        return "—".into();
    }
    let items: Vec<String> = arr
        .iter()
        .filter_map(|d| d.as_str().map(String::from))
        .collect();
    truncate_value(&Value::String(items.join(", ")), max)
}

/// Truncate a JSON scalar to `max` chars (UTF-8 safe), substituting `—` for null.
pub fn truncate_value(v: &Value, max: usize) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Null => return "—".to_string(),
        other => render_scalar(other),
    };
    if s.chars().count() <= max {
        s
    } else {
        let head: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

/// Coerce JSON bools, 0/1 numbers, and "0"/"1"/"true"/"false" strings to a bool.
fn coerce_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_i64().and_then(|i| match i {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }),
        Value::String(s) => match s.as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// Render a boolean-ish value as ✓/✗ in green/red.
pub fn bool_check(v: &Value) -> String {
    match coerce_bool(v) {
        Some(true) => "✓".to_string(),
        Some(false) => "✗".to_string(),
        None => render_scalar(v),
    }
}

pub fn bool_check_color(v: &Value) -> Option<Color> {
    match coerce_bool(v) {
        Some(true) => Some(Color::Green),
        Some(false) => Some(Color::Red),
        None => None,
    }
}

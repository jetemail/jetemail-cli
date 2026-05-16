use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};
use reqwest::Method;
use serde_json::{Map, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use crate::client::{ApiClient, Auth};
use crate::tui::{restore_terminal, setup_terminal, TerminalT};

/// Which logs endpoint we're tailing — drives the labels and accent color only.
#[derive(Clone, Copy)]
pub enum LogKind {
    Inbound,
    Outbound,
    Account,
}

impl LogKind {
    fn label(self) -> &'static str {
        match self {
            LogKind::Inbound => "inbound",
            LogKind::Outbound => "outbound",
            LogKind::Account => "account",
        }
    }
    fn accent(self) -> Color {
        match self {
            LogKind::Inbound => palette::get().accent_inbound,
            LogKind::Outbound => palette::get().accent_outbound,
            LogKind::Account => palette::get().accent_account,
        }
    }
}

/// All the inputs needed to keep polling one logs endpoint.
///
/// `query` is a pre-resolved list of key=value pairs (None Options already dropped)
/// so reqwest can serialize it directly via serde_urlencoded — round-tripping through
/// `serde_json::Value` fails because `null` has no urlencoded representation.
pub struct LogSource {
    pub client: ApiClient,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub auth: Auth,
    pub kind: LogKind,
}

pub struct TailOpts {
    pub poll_secs: u64,
    pub buffer: usize,
}

#[derive(Clone)]
struct Entry {
    id: String,
    raw: Value,
}

struct App {
    kind: LogKind,
    entries: Vec<Entry>,
    seen: std::collections::HashSet<String>,
    /// IDs returned by the very first poll. We treat these as "history" and
    /// hide them — the user only wants entries that appear *after* launching
    /// `--tail`, regardless of how (or whether) the endpoint honours
    /// `date_from`. Updates to baseline ids are also suppressed, matching
    /// strict `tail -f` semantics.
    baseline: std::collections::HashSet<String>,
    baseline_collected: bool,
    filtered: Vec<usize>,
    list_state: ListState,
    detail_scroll: u16,
    detail_view: DetailView,
    paused: Arc<AtomicBool>,
    poll_secs: u64,
    buffer: usize,
    search_mode: bool,
    search_query: String,
    last_fetch: Option<Instant>,
    fetch_count: u64,
    last_error: Option<String>,
    last_raw: Option<Value>,
    last_entry_count: usize,
    // Map ListState selection y-coordinate to item index — populated each draw,
    // used for mouse hit-testing.
    list_inner: Rect,
    list_offset: usize,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum DetailView {
    Closed,
    Entry,
    Raw,
}

impl App {
    fn new(kind: LogKind, paused: Arc<AtomicBool>, opts: &TailOpts) -> Self {
        Self {
            kind,
            entries: Vec::new(),
            seen: Default::default(),
            baseline: Default::default(),
            baseline_collected: false,
            filtered: Vec::new(),
            list_state: ListState::default(),
            detail_scroll: 0,
            detail_view: DetailView::Closed,
            paused,
            poll_secs: opts.poll_secs,
            buffer: opts.buffer,
            search_mode: false,
            search_query: String::new(),
            last_fetch: None,
            fetch_count: 0,
            last_error: None,
            last_raw: None,
            last_entry_count: 0,
            list_inner: Rect::default(),
            list_offset: 0,
        }
    }

    fn ingest(&mut self, batch: Vec<Map<String, Value>>) {
        // First successful poll: every id returned counts as "history" and is
        // permanently hidden, so subsequent polls only surface genuinely new
        // entries. This is more robust than trusting the server's `date_from`
        // filter, which behaves inconsistently across the three logs endpoints.
        if !self.baseline_collected {
            for obj in &batch {
                self.baseline.insert(entry_id(obj));
            }
            self.baseline_collected = true;
            return;
        }

        let mut added = 0usize;
        let mut updated = 0usize;
        for obj in batch {
            let id = entry_id(&obj);
            if self.baseline.contains(&id) {
                // Pre-existing entry — strict `tail -f` semantics hide it even
                // when its `last_updated` changes.
                continue;
            }
            let raw = Value::Object(obj);
            // Update in place if we've seen this id — preserves position and lets
            // status transitions (queued_pending → delivered / bounced / …) show live.
            if let Some(existing) = self.entries.iter_mut().find(|e| e.id == id) {
                existing.raw = raw;
                updated += 1;
            } else {
                self.seen.insert(id.clone());
                self.entries.insert(0, Entry { id, raw });
                added += 1;
            }
        }
        if self.entries.len() > self.buffer {
            for e in self.entries.drain(self.buffer..) {
                self.seen.remove(&e.id);
            }
        }
        if added > 0 {
            self.refilter(true);
        } else if updated > 0 && !self.search_query.is_empty() {
            // Updated content may now match/unmatch the filter.
            self.refilter(false);
        }
    }

    fn refilter(&mut self, prepended: bool) {
        let prev_selected_id = self.selected().map(|e| e.id.clone());

        let q = self.search_query.to_lowercase();
        if q.is_empty() {
            self.filtered = (0..self.entries.len()).collect();
        } else {
            self.filtered = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| entry_search_string(e).to_lowercase().contains(&q))
                .map(|(i, _)| i)
                .collect();
        }

        if let Some(prev) = prev_selected_id {
            if let Some(pos) = self
                .filtered
                .iter()
                .position(|&i| self.entries[i].id == prev)
            {
                self.list_state.select(Some(pos));
                return;
            }
        }
        // If we prepended new rows and no prior selection, keep at top.
        // Otherwise also default to top.
        if self.filtered.is_empty() {
            self.list_state.select(None);
        } else if prepended || self.list_state.selected().is_none() {
            self.list_state.select(Some(0));
        }
    }

    fn selected(&self) -> Option<&Entry> {
        self.list_state
            .selected()
            .and_then(|i| self.filtered.get(i))
            .map(|&idx| &self.entries[idx])
    }

    fn move_sel(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            return;
        }
        let cur = self.list_state.selected().unwrap_or(0) as i32;
        let next = (cur + delta).clamp(0, self.filtered.len() as i32 - 1);
        self.list_state.select(Some(next as usize));
    }

    fn select_at(&mut self, idx: usize) {
        if idx < self.filtered.len() {
            self.list_state.select(Some(idx));
        }
    }
}

pub async fn run(source: LogSource, opts: TailOpts) -> Result<()> {
    let (tx, rx) = std::sync::mpsc::channel::<FetchResult>();
    let paused = Arc::new(AtomicBool::new(false));

    // Background poller.
    let p_flag = paused.clone();
    let poll_secs = opts.poll_secs;
    let LogSource {
        client,
        path,
        mut query,
        auth,
        kind,
    } = source;

    // `tail -f` semantics: only show entries that appear after we start watching,
    // unless the user explicitly asked for an earlier window via --date-from.
    //
    // The `date_from` query param is in Unix **seconds** (even though entries'
    // `last_updated` field is stored in milliseconds — these aren't the same
    // unit). Subtract a small grace window so an email submitted ~1s before our
    // "now" isn't missed due to clock skew or pipeline latency.
    if !query.iter().any(|(k, _)| k == "date_from") {
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        query.push((
            "date_from".to_string(),
            now_secs.saturating_sub(5).to_string(),
        ));
    }
    let poller = tokio::spawn(async move {
        loop {
            if !p_flag.load(Ordering::Relaxed) {
                let result = client
                    .request_json::<_, ()>(Method::GET, &path, auth, Some(&query), None, &[])
                    .await;
                let msg = match result {
                    Ok(v) => FetchResult::Ok {
                        entries: entries_from_response(&v),
                        raw: v,
                    },
                    Err(e) => FetchResult::Err(format!("{e:#}")),
                };
                if tx.send(msg).is_err() {
                    return; // receiver gone — UI exited
                }
            }
            tokio::time::sleep(Duration::from_secs(poll_secs)).await;
        }
    });

    let app_paused = paused.clone();
    let res = tokio::task::spawn_blocking(move || {
        let mut terminal = setup_terminal()?;
        let mut app = App::new(kind, app_paused, &opts);
        let result = event_loop(&mut terminal, &mut app, rx);
        let _ = restore_terminal(&mut terminal);
        result
    })
    .await?;

    poller.abort();
    res
}

enum FetchResult {
    Ok {
        entries: Vec<Map<String, Value>>,
        raw: Value,
    },
    Err(String),
}

fn event_loop(
    terminal: &mut TerminalT,
    app: &mut App,
    rx: std::sync::mpsc::Receiver<FetchResult>,
) -> Result<()> {
    loop {
        // Drain any pending fetches.
        while let Ok(msg) = rx.try_recv() {
            match msg {
                FetchResult::Ok { entries, raw } => {
                    app.last_error = None;
                    app.last_fetch = Some(Instant::now());
                    app.fetch_count += 1;
                    app.last_entry_count = entries.len();
                    app.last_raw = Some(raw);
                    app.ingest(entries);
                }
                FetchResult::Err(e) => {
                    app.last_error = Some(e);
                    app.last_fetch = Some(Instant::now());
                }
            }
        }

        terminal.draw(|f| draw(f, app))?;

        if event::poll(Duration::from_millis(150))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    if handle_key(app, key.code, key.modifiers) {
                        return Ok(());
                    }
                }
                Event::Mouse(me) => handle_mouse(app, me),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
}

/// Returns `true` if the caller should exit.
fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) -> bool {
    if app.search_mode {
        match code {
            KeyCode::Esc => {
                app.search_mode = false;
                app.search_query.clear();
                app.refilter(false);
            }
            KeyCode::Enter => {
                app.search_mode = false;
            }
            KeyCode::Backspace => {
                app.search_query.pop();
                app.refilter(false);
            }
            KeyCode::Char(c) if !mods.contains(KeyModifiers::CONTROL) => {
                app.search_query.push(c);
                app.refilter(false);
            }
            _ => {}
        }
        return false;
    }

    if app.detail_view != DetailView::Closed {
        match code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => {
                app.detail_view = DetailView::Closed;
                app.detail_scroll = 0;
            }
            KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => return true,
            KeyCode::Char('r') => {
                app.detail_view = match app.detail_view {
                    DetailView::Entry if app.last_raw.is_some() => DetailView::Raw,
                    DetailView::Raw => DetailView::Entry,
                    other => other,
                };
                app.detail_scroll = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.detail_scroll = app.detail_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.detail_scroll = app.detail_scroll.saturating_add(1);
            }
            KeyCode::PageUp => {
                app.detail_scroll = app.detail_scroll.saturating_sub(10);
            }
            KeyCode::PageDown => {
                app.detail_scroll = app.detail_scroll.saturating_add(10);
            }
            KeyCode::Home => app.detail_scroll = 0,
            _ => {}
        }
        return false;
    }

    match code {
        KeyCode::Char('q') | KeyCode::Esc => return true,
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => return true,
        KeyCode::Char(' ') => {
            let cur = app.paused.load(Ordering::Relaxed);
            app.paused.store(!cur, Ordering::Relaxed);
        }
        KeyCode::Char('/') => {
            app.search_mode = true;
        }
        KeyCode::Char('c') if !mods.contains(KeyModifiers::CONTROL) => {
            // Clear buffer AND re-baseline from the next poll so we don't keep
            // hiding ids that the user has now forgotten about.
            app.entries.clear();
            app.seen.clear();
            app.baseline.clear();
            app.baseline_collected = false;
            app.refilter(false);
        }
        KeyCode::Char('r') if app.last_raw.is_some() => {
            app.detail_view = DetailView::Raw;
            app.detail_scroll = 0;
        }
        KeyCode::Up | KeyCode::Char('k') => app.move_sel(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_sel(1),
        KeyCode::PageUp => app.move_sel(-10),
        KeyCode::PageDown => app.move_sel(10),
        KeyCode::Home | KeyCode::Char('g') => app.move_sel(-(app.filtered.len() as i32)),
        KeyCode::End | KeyCode::Char('G') => app.move_sel(app.filtered.len() as i32),
        KeyCode::Enter | KeyCode::Char('l') if app.selected().is_some() => {
            app.detail_view = DetailView::Entry;
            app.detail_scroll = 0;
        }
        _ => {}
    }
    false
}

fn handle_mouse(app: &mut App, me: MouseEvent) {
    if app.detail_view != DetailView::Closed {
        match me.kind {
            MouseEventKind::ScrollDown => {
                app.detail_scroll = app.detail_scroll.saturating_add(3);
            }
            MouseEventKind::ScrollUp => {
                app.detail_scroll = app.detail_scroll.saturating_sub(3);
            }
            _ => {}
        }
        return;
    }
    match me.kind {
        MouseEventKind::Down(_) => {
            let row = me.row;
            let col = me.column;
            // Only register clicks inside the list area.
            if row >= app.list_inner.y
                && row < app.list_inner.y + app.list_inner.height
                && col >= app.list_inner.x
                && col < app.list_inner.x + app.list_inner.width
            {
                let idx = app.list_offset + (row - app.list_inner.y) as usize;
                app.select_at(idx);
                if me.kind == MouseEventKind::Down(crossterm::event::MouseButton::Left) {
                    app.detail_view = DetailView::Entry;
                    app.detail_scroll = 0;
                }
            }
        }
        MouseEventKind::ScrollDown => app.move_sel(3),
        MouseEventKind::ScrollUp => app.move_sel(-3),
        _ => {}
    }
}

fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title bar / status
            Constraint::Min(1),    // list
            Constraint::Length(1), // help
        ])
        .split(f.area());

    draw_header(f, chunks[0], app);
    draw_list(f, chunks[1], app);
    draw_help(f, chunks[2], app);

    match app.detail_view {
        DetailView::Closed => {}
        DetailView::Entry => draw_entry_modal(f, app),
        DetailView::Raw => draw_raw_modal(f, app),
    }
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let paused = app.paused.load(Ordering::Relaxed);
    let status_text = if paused { "PAUSED" } else { "LIVE" };
    let status_color = if paused {
        palette::get().paused
    } else {
        palette::get().live
    };

    let fetched = match app.last_fetch {
        Some(t) => format!("{}s ago", t.elapsed().as_secs()),
        None => "—".to_string(),
    };

    let title = vec![
        Span::styled(" ● ", Style::default().fg(status_color)),
        Span::styled(
            format!("jetemail logs ({})", app.kind.label()),
            Style::default()
                .fg(app.kind.accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("[{status_text}]"),
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("poll={}s", app.poll_secs),
            Style::default().fg(palette::get().header_meta),
        ),
        Span::raw("  "),
        Span::styled(
            format!("entries={}/{}", app.filtered.len(), app.entries.len()),
            Style::default().fg(palette::get().header_meta),
        ),
        Span::raw("  "),
        Span::styled(
            format!("last fetch={fetched}"),
            Style::default().fg(palette::get().header_meta),
        ),
        Span::raw("  "),
        Span::styled(
            format!("#{}", app.fetch_count),
            Style::default().fg(palette::get().header_meta),
        ),
    ];

    let mut lines = vec![Line::from(title)];

    if app.search_mode || !app.search_query.is_empty() {
        let prefix = if app.search_mode { "/" } else { "filter: " };
        lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(Color::LightCyan)),
            Span::raw(app.search_query.clone()),
            if app.search_mode {
                Span::styled("_", Style::default().fg(Color::LightCyan))
            } else {
                Span::raw("")
            },
        ]));
    } else if let Some(err) = &app.last_error {
        lines.push(Line::from(vec![
            Span::styled("error: ", Style::default().fg(palette::get().error)),
            Span::raw(truncate(err, area.width.saturating_sub(10) as usize)),
        ]));
    } else {
        lines.push(Line::from(""));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.kind.accent()));
    let p = Paragraph::new(lines).block(block);
    f.render_widget(p, area);
}

fn draw_list(f: &mut Frame, area: Rect, app: &mut App) {
    let p = palette::get();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue))
        .title(format!(" {} entries ", app.filtered.len()));

    // Capture inner area for mouse hit-testing.
    app.list_inner = block.inner(area);
    app.list_offset = app.list_state.offset();

    // Empty-state placeholder — tail starts from now, so an empty list is the
    // expected initial state. Render a centered hint instead of just whitespace.
    if app.entries.is_empty() {
        let lines = if app.last_fetch.is_some() {
            vec![
                Line::from(""),
                Line::from(""),
                Line::from(Span::styled(
                    "Waiting for new log entries…",
                    Style::default()
                        .fg(p.value_text)
                        .add_modifier(Modifier::BOLD),
                ))
                .alignment(ratatui::layout::Alignment::Center),
                Line::from(""),
                Line::from(Span::styled(
                    format!(
                        "polling every {}s — send some traffic or wait",
                        app.poll_secs
                    ),
                    Style::default().fg(p.subtext),
                ))
                .alignment(ratatui::layout::Alignment::Center),
                Line::from(""),
                Line::from(Span::styled(
                    "tail shows entries from when you started watching;",
                    Style::default().fg(p.muted),
                ))
                .alignment(ratatui::layout::Alignment::Center),
                Line::from(Span::styled(
                    "pass --date-from to include earlier entries",
                    Style::default().fg(p.muted),
                ))
                .alignment(ratatui::layout::Alignment::Center),
            ]
        } else {
            vec![
                Line::from(""),
                Line::from(""),
                Line::from(Span::styled("Connecting…", Style::default().fg(p.subtext)))
                    .alignment(ratatui::layout::Alignment::Center),
            ]
        };
        let p_widget = Paragraph::new(lines).block(block);
        f.render_widget(p_widget, area);
        return;
    }

    let items: Vec<ListItem> = app
        .filtered
        .iter()
        .map(|&idx| render_entry_row(&app.entries[idx], app.kind))
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(palette::get().selection_bg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn draw_help(f: &mut Frame, area: Rect, app: &App) {
    let key = Style::default().fg(palette::get().key_hint);
    let key_action = Style::default().fg(palette::get().live);
    let key_warn = Style::default().fg(palette::get().error);
    let spans = if app.search_mode {
        vec![
            Span::styled(" Enter ", key_action),
            Span::raw("apply  "),
            Span::styled("Esc ", key_warn),
            Span::raw("clear  "),
            Span::styled(
                "(type to filter visible rows)",
                Style::default().fg(palette::get().muted),
            ),
        ]
    } else if app.detail_view != DetailView::Closed {
        vec![
            Span::styled(" ↑↓/jk ", key),
            Span::raw("scroll  "),
            Span::styled("PgUp/PgDn ", key),
            Span::raw("page  "),
            Span::styled("r ", key),
            Span::raw("toggle raw JSON  "),
            Span::styled("Esc/Enter ", key_action),
            Span::raw("close  "),
            Span::styled("q ", key_warn),
            Span::raw("quit"),
        ]
    } else {
        vec![
            Span::styled(" ↑↓ ", key),
            Span::raw("nav  "),
            Span::styled("Enter/click ", key_action),
            Span::raw("detail  "),
            Span::styled("Space ", key),
            Span::raw("pause  "),
            Span::styled("/ ", key),
            Span::raw("filter  "),
            Span::styled("r ", key),
            Span::raw("raw  "),
            Span::styled("c ", key),
            Span::raw("clear  "),
            Span::styled("q ", key_warn),
            Span::raw("quit"),
        ]
    };
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_entry_modal(f: &mut Frame, app: &mut App) {
    let area = centered_rect(80, 80, f.area());
    f.render_widget(ratatui::widgets::Clear, area);

    let entry = match app.selected() {
        Some(e) => e,
        None => return,
    };

    let lines = build_entry_lines(entry, app.kind);

    let title = format!(
        " log entry — {}   (Enter/Esc close · r for raw JSON) ",
        short_id(&entry.id)
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.kind.accent()))
        .title(title);

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.detail_scroll, 0));
    f.render_widget(para, area);
}

#[derive(Copy, Clone)]
#[allow(dead_code)] // Time and Size aren't bound to any current section but the formatting plumbing is kept ready.
enum FieldFmt {
    Text,
    Time,
    Size,
    Count,
    /// Read the named field as a JSON array and display its length.
    ArrayLen,
    Id,
}

/// Sections shown in the entry detail view. Each section's fields are scanned
/// against the entry; an entire section is skipped if none of its fields have
/// a non-empty value. The display label and a list of source-key candidates
/// are paired so the API can rename fields without us losing the row.
struct Section {
    title: &'static str,
    fields: &'static [(&'static str, &'static [&'static str], FieldFmt)],
}

const SECTIONS: &[Section] = &[
    Section {
        title: "Message",
        fields: &[
            (
                "Message-Id",
                &["message_id", "messageId", "msg_id", "msgId", "mid"],
                FieldFmt::Id,
            ),
            (
                "Spam score",
                &["spamscore", "spam_score", "score"],
                FieldFmt::Text,
            ),
        ],
    },
    Section {
        title: "Delivery",
        fields: &[
            (
                "Response",
                &[
                    "final_response",
                    "finalResponse",
                    "response",
                    "reply",
                    "smtp_response",
                    "smtpResponse",
                ],
                FieldFmt::Text,
            ),
            (
                "Server",
                &[
                    "final_host",
                    "finalHost",
                    "final_mx",
                    "finalMx",
                    "host",
                    "server",
                    "mx",
                    "smtp_host",
                    "smtpHost",
                ],
                FieldFmt::Text,
            ),
            (
                "IP",
                &[
                    "final_ip",
                    "finalIp",
                    "ip",
                    "remote_ip",
                    "remoteIp",
                    "client_ip",
                ],
                FieldFmt::Text,
            ),
            (
                "Protocol",
                &[
                    "final_protocol",
                    "finalProtocol",
                    "protocol",
                    "proto",
                    "transtype",
                ],
                FieldFmt::Text,
            ),
        ],
    },
    Section {
        title: "Source",
        fields: &[
            ("Sender", &["sender"], FieldFmt::Text),
            ("Origin host", &["originhost"], FieldFmt::Text),
            (
                "Source IP",
                &["src", "src_address", "srcAddress"],
                FieldFmt::Text,
            ),
            ("User", &["user"], FieldFmt::Text),
            ("Header From", &["headerFrom"], FieldFmt::Text),
            ("Rule", &["rule_id", "ruleId", "rule"], FieldFmt::Id),
            ("Zone", &["zone"], FieldFmt::Text),
        ],
    },
    Section {
        title: "Engagement",
        fields: &[
            ("Opens", &["opens"], FieldFmt::ArrayLen),
            ("Clicks", &["clicks"], FieldFmt::ArrayLen),
            ("Actions", &["action_count", "actionCount"], FieldFmt::Count),
            (
                "Complaints",
                &["complaint_count", "complaints"],
                FieldFmt::Count,
            ),
            ("Bounces", &["bounce_count", "bounces"], FieldFmt::Count),
        ],
    },
    Section {
        title: "Identifiers",
        fields: &[
            ("id", &["id"], FieldFmt::Id),
            ("Receiver", &["receiver"], FieldFmt::Text),
        ],
    },
];

fn build_entry_lines(entry: &Entry, kind: LogKind) -> Vec<Line<'static>> {
    let m = match entry.raw.as_object() {
        Some(m) => m,
        None => {
            // Not an object — fall back to JSON dump.
            let s = serde_json::to_string_pretty(&entry.raw).unwrap_or_default();
            return s.lines().map(|l| Line::from(l.to_string())).collect();
        }
    };

    let p = palette::get();
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut shown: std::collections::HashSet<String> = std::collections::HashSet::new();

    // ─────────── status banner ───────────
    lines.push(Line::from(""));
    let status_keys: &[&str] = match kind {
        LogKind::Outbound => STATUS_KEYS_OUTBOUND,
        _ => STATUS_KEYS_INBOUND,
    };
    if let Some((status_key, status_value)) = field_str_with_key(m, status_keys) {
        shown.insert(status_key.to_string());
        let color = status_color(&status_value);
        let banner = status_value.replace('_', " ").to_uppercase();
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "▌ ",
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                banner,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]));

        // Subtext: relative time + truncated uid (both in subtext color — bright
        // enough to read but clearly secondary to the banner).
        let mut subtext: Vec<Span<'static>> = Vec::new();
        if let Some(secs) = field_unix_secs(m, &mut shown) {
            subtext.push(Span::styled(
                relative_time(secs),
                Style::default().fg(p.subtext),
            ));
        }
        if let Some(uid) = m
            .get("uid")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            shown.insert("uid".to_string());
            if !subtext.is_empty() {
                subtext.push(Span::styled(" · ", Style::default().fg(p.muted)));
            }
            subtext.push(Span::styled(
                format!("uid {}", truncate(uid, 28)),
                Style::default().fg(p.subtext),
            ));
        }
        if !subtext.is_empty() {
            let mut line_spans: Vec<Span<'static>> = vec![Span::raw("    ")];
            line_spans.extend(subtext);
            lines.push(Line::from(line_spans));
        }
        lines.push(Line::from(""));
    }

    // ─────────── quick triple: From / To / Subject ───────────
    let triple: &[(&str, &[&str])] = &[
        ("From", FROM_KEYS),
        ("To", TO_KEYS),
        ("Subject", SUBJECT_KEYS),
    ];
    let triple_label_width = 9; // longest of "Subject" + padding
    let mut any_triple = false;
    for (label, keys) in triple {
        if let Some((key, value)) = field_str_with_key(m, keys) {
            if value.is_empty() {
                continue;
            }
            shown.insert(key.to_string());
            lines.push(prop_line(label, &value, FieldFmt::Text, triple_label_width));
            any_triple = true;
        }
    }
    if any_triple {
        lines.push(Line::from(""));
    }

    // ─────────── sections ───────────
    for section in SECTIONS {
        let mut section_rows: Vec<Line<'static>> = Vec::new();
        for (label, keys, fmt) in section.fields {
            // For Time fields, use the dedicated time-key scanner so we pick the
            // best timestamp field even if it's not in `keys`.
            let resolved = match fmt {
                FieldFmt::Time => {
                    // Try the listed time keys first
                    let mut hit = None;
                    for k in *keys {
                        if let Some(v) = m.get(*k) {
                            if let Some(formatted) = format_time_value(v) {
                                hit = Some((k.to_string(), formatted));
                                break;
                            }
                        }
                    }
                    hit
                }
                FieldFmt::ArrayLen => keys.iter().find_map(|k| {
                    m.get(*k)
                        .and_then(|v| v.as_array())
                        .map(|a| (k.to_string(), a.len().to_string()))
                }),
                _ => field_str_with_key(m, keys).map(|(k, v)| (k.to_string(), v)),
            };
            if let Some((key, value)) = resolved {
                if shown.contains(&key) || value.is_empty() {
                    continue;
                }
                shown.insert(key.clone());
                let raw_val = m.get(key.as_str());
                let formatted = format_field(&value, raw_val, *fmt);
                if !is_trivial_value(&formatted, *fmt) {
                    section_rows.push(prop_line(label, &formatted, *fmt, 16));
                    if matches!(fmt, FieldFmt::ArrayLen) {
                        if let Some(arr) = raw_val.and_then(|v| v.as_array()) {
                            let mut items: Vec<&Value> = arr.iter().collect();
                            items.sort_by(|a, b| {
                                let da = a.get("date").and_then(|v| v.as_i64()).unwrap_or(0);
                                let db = b.get("date").and_then(|v| v.as_i64()).unwrap_or(0);
                                db.cmp(&da)
                            });
                            for item in items {
                                if let Some(line) = engagement_event_line(item) {
                                    section_rows.push(line);
                                }
                            }
                        }
                    }
                }
            }
        }
        if !section_rows.is_empty() {
            lines.push(section_heading(section.title));
            lines.extend(section_rows);
            lines.push(Line::from(""));
        }
    }

    // ─────────── anything left over, alphabetical, non-empty ───────────
    let mut other_keys: Vec<String> = m
        .iter()
        .filter(|(k, v)| !shown.contains(k.as_str()) && !is_empty_json(v))
        .map(|(k, _)| k.clone())
        .collect();
    other_keys.sort();
    if !other_keys.is_empty() {
        lines.push(section_heading("Other"));
        for key in &other_keys {
            let v = &m[key];
            push_other_field(&mut lines, key, v, 16);
        }
        lines.push(Line::from(""));
    }

    lines
}

fn section_heading(title: &str) -> Line<'static> {
    let p = palette::get();
    Line::from(vec![
        Span::raw("  "),
        Span::styled("─ ", Style::default().fg(p.muted)),
        Span::styled(
            title.to_string(),
            Style::default()
                .fg(p.header_meta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {}", "─".repeat(40)), Style::default().fg(p.muted)),
    ])
}

fn prop_line(label: &str, value: &str, fmt: FieldFmt, label_width: usize) -> Line<'static> {
    let p = palette::get();
    let value_style = match fmt {
        FieldFmt::Time => Style::default().fg(p.value_time),
        FieldFmt::Size => Style::default().fg(p.value_size),
        FieldFmt::Count | FieldFmt::ArrayLen => Style::default().fg(p.value_number),
        FieldFmt::Id => Style::default().fg(p.value_id),
        FieldFmt::Text => Style::default().fg(p.value_text),
    };
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("{:<width$}", label, width = label_width),
            Style::default().fg(p.label),
        ),
        Span::styled(value.to_string(), value_style),
    ])
}

/// Render one open/click event as a sub-row under its count line.
/// Returns None if the item has no useful fields.
fn engagement_event_line(item: &Value) -> Option<Line<'static>> {
    let obj = item.as_object()?;
    let p = palette::get();

    let date = obj
        .get("date")
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
        .filter(|n| *n > 0)
        .map(format_unix);
    let email_client = obj
        .get("email_client")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let country = obj
        .get("country")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let ip = obj
        .get("ip")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let url = obj
        .get("original_url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

    if date.is_none()
        && email_client.is_none()
        && country.is_none()
        && ip.is_none()
        && url.is_none()
    {
        return None;
    }

    // Sub-rows align under the count value (col 18 = 2 indent + 16 label width).
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ".repeat(18))];
    if let Some(d) = date {
        spans.push(Span::styled(d, Style::default().fg(p.value_time)));
    }
    let mut meta_parts: Vec<String> = Vec::new();
    if let Some(ec) = email_client {
        meta_parts.push(ec.to_string());
    }
    if let Some(co) = country {
        meta_parts.push(co.to_string());
    }
    if let Some(i) = ip {
        meta_parts.push(i.to_string());
    }
    if !meta_parts.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            meta_parts.join(" · "),
            Style::default().fg(p.subtext),
        ));
    }
    if let Some(u) = url {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            truncate(u, 80),
            Style::default().fg(p.value_text),
        ));
    }
    Some(Line::from(spans))
}

fn push_other_field(lines: &mut Vec<Line<'static>>, key: &str, v: &Value, label_width: usize) {
    let p = palette::get();
    let label_span = Span::styled(
        format!(
            "{:<width$}",
            truncate(key, label_width.saturating_sub(1)),
            width = label_width
        ),
        Style::default().fg(p.label),
    );
    match v {
        Value::Null => {}
        Value::Bool(b) => lines.push(Line::from(vec![
            Span::raw("  "),
            label_span,
            Span::styled(
                b.to_string(),
                Style::default().fg(if *b {
                    p.value_bool_true
                } else {
                    p.value_bool_false
                }),
            ),
        ])),
        Value::Number(n) => lines.push(Line::from(vec![
            Span::raw("  "),
            label_span,
            Span::styled(n.to_string(), Style::default().fg(p.value_number)),
        ])),
        Value::String(s) => {
            let trimmed = if s.len() > 200 {
                format!("{}…", &s[..200])
            } else {
                s.clone()
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                label_span,
                Span::styled(trimmed, Style::default().fg(p.value_text)),
            ]));
        }
        Value::Array(_) | Value::Object(_) => {
            let pretty = serde_json::to_string(v).unwrap_or_default();
            if pretty.len() <= 160 {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    label_span,
                    Span::styled(pretty, Style::default().fg(p.value_json)),
                ]));
            } else {
                lines.push(Line::from(vec![Span::raw("  "), label_span]));
                let multi = serde_json::to_string_pretty(v).unwrap_or_default();
                for line in multi.lines() {
                    let pad = " ".repeat(label_width + 4);
                    lines.push(Line::from(vec![Span::styled(
                        format!("{pad}{line}"),
                        Style::default().fg(p.value_json),
                    )]));
                }
            }
        }
    }
}

/// Extract the entry's unix seconds (using the same heuristic as `field_time`),
/// recording the key as already-shown so the section loop won't re-print it.
fn field_unix_secs(
    m: &Map<String, Value>,
    shown: &mut std::collections::HashSet<String>,
) -> Option<i64> {
    fn extract(v: &Value) -> Option<i64> {
        let raw = if let Some(n) = v.as_i64() {
            n
        } else if let Some(f) = v.as_f64() {
            f as i64
        } else if let Some(s) = v.as_str() {
            s.parse::<i64>().ok()?
        } else {
            return None;
        };
        if raw <= 0 {
            return None;
        }
        Some(if raw > 10_000_000_000 {
            raw / 1000
        } else {
            raw
        })
    }
    for k in TIME_KEYS {
        if let Some(v) = m.get(*k) {
            if let Some(n) = extract(v) {
                shown.insert((*k).to_string());
                return Some(n);
            }
        }
    }
    // Heuristic fallback: scan numeric fields for plausible timestamps.
    for (k, v) in m {
        if let Value::Number(_) = v {
            if let Some(n) = extract(v) {
                if (1_000_000_000..4_000_000_000).contains(&n)
                    || (1_000_000_000_000..4_000_000_000_000).contains(&n)
                {
                    shown.insert(k.clone());
                    return Some(if n > 10_000_000_000 { n / 1000 } else { n });
                }
            }
        }
    }
    None
}

/// "12 seconds ago" / "3 minutes ago" / "May 12 04:49 UTC" for older.
fn relative_time(secs: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let diff = now.saturating_sub(secs);
    if diff < 0 {
        return format_unix(secs);
    }
    match diff {
        0..=1 => "just now".to_string(),
        2..=59 => format!("{diff} seconds ago"),
        60..=119 => "1 minute ago".to_string(),
        120..=3599 => format!("{} minutes ago", diff / 60),
        3600..=7199 => "1 hour ago".to_string(),
        7200..=86399 => format!("{} hours ago", diff / 3600),
        86400..=172_799 => "1 day ago".to_string(),
        _ => format_unix(secs),
    }
}

fn format_field(value: &str, raw: Option<&Value>, fmt: FieldFmt) -> String {
    match fmt {
        FieldFmt::Size => {
            let n = raw
                .and_then(|v| v.as_i64())
                .or_else(|| value.parse::<i64>().ok());
            match n {
                Some(n) => human_bytes(n),
                None => value.to_string(),
            }
        }
        FieldFmt::Count | FieldFmt::ArrayLen => {
            // Bare integer — leave as-is.
            value.to_string()
        }
        _ => value.to_string(),
    }
}

/// Suppress trivial values (e.g. Count=0, Size=0) that just clutter the view.
fn is_trivial_value(value: &str, fmt: FieldFmt) -> bool {
    match fmt {
        FieldFmt::Count | FieldFmt::ArrayLen => matches!(value, "0" | ""),
        FieldFmt::Size => matches!(value, "0 B" | "0"),
        _ => false,
    }
}

fn is_empty_json(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

fn human_bytes(n: i64) -> String {
    let abs = n.unsigned_abs() as f64;
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let formatted = if abs >= GB {
        format!("{:.2} GB", abs / GB)
    } else if abs >= MB {
        format!("{:.2} MB", abs / MB)
    } else if abs >= KB {
        format!("{:.1} KB", abs / KB)
    } else {
        format!("{abs} B")
    };
    // Also show raw bytes with separators for medium sizes.
    if abs >= KB {
        format!("{formatted} ({} bytes)", with_commas(n))
    } else {
        formatted
    }
}

fn with_commas(n: i64) -> String {
    let s = n.abs().to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

fn draw_raw_modal(f: &mut Frame, app: &mut App) {
    let area = centered_rect(80, 80, f.area());
    f.render_widget(ratatui::widgets::Clear, area);

    let raw = match &app.last_raw {
        Some(v) => v,
        None => return,
    };
    let pretty = serde_json::to_string_pretty(raw).unwrap_or_else(|_| raw.to_string());

    let title = format!(
        " raw last response — extracted {} entries ",
        app.last_entry_count
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(title);

    let para = Paragraph::new(pretty)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.detail_scroll, 0));
    f.render_widget(para, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vert[1])[1]
}

fn render_entry_row(e: &Entry, kind: LogKind) -> ListItem<'static> {
    let m = e.raw.as_object();
    let when = m.and_then(field_time).unwrap_or_else(|| "—".into());
    let status = m.and_then(|m| field_status(m, kind));
    let from = m.and_then(|m| field_str(m, FROM_KEYS));
    let to = m.and_then(|m| field_str(m, TO_KEYS));
    let subject = m.and_then(|m| field_str(m, SUBJECT_KEYS));

    let status_span = match &status {
        Some(s) => Span::styled(
            format!("{:<14}", truncate(s, 14)),
            Style::default()
                .fg(status_color(s))
                .add_modifier(Modifier::BOLD),
        ),
        None => Span::raw("              "),
    };

    let mut spans = vec![
        Span::styled(
            format!(" {:<19} ", when),
            Style::default().fg(palette::get().date),
        ),
        status_span,
        Span::raw("  "),
    ];
    if let Some(f) = from {
        spans.push(Span::styled(
            truncate(&f, 30),
            Style::default().fg(palette::get().from),
        ));
        spans.push(Span::styled(
            " → ",
            Style::default().fg(palette::get().arrow),
        ));
    }
    if let Some(t) = to {
        spans.push(Span::styled(
            truncate(&t, 30),
            Style::default().fg(palette::get().to),
        ));
        spans.push(Span::raw("  "));
    }
    if let Some(s) = subject {
        spans.push(Span::styled(
            truncate(&s, 60),
            Style::default().fg(palette::get().subject),
        ));
    }
    let _ = kind;
    ListItem::new(Line::from(spans))
}

/// Centralized palette. Colors are resolved once at startup based on terminal
/// capability:
///
/// - **Truecolor terminals** (Ghostty, iTerm2, Wezterm, Alacritty, kitty, modern
///   Windows Terminal — anything that sets `COLORTERM=truecolor`): we emit the
///   exact 24-bit RGB so the design renders pixel-perfect.
/// - **256-color terminals** (macOS Terminal.app and the rest): we map each RGB
///   to its nearest entry in the 6×6×6 cube (indices 16-231) of the standard
///   xterm-256 palette. That part of the palette is fixed by spec — Terminal.app
///   doesn't let users theme it — so colors stay consistent.
/// - 16-color terminals fall back to crossterm's own degradation.
///
/// The aesthetic is "cool slate base + status-only color pops" — most of the row
/// stays in muted blue-grays, with strong saturated color reserved for status
/// and live/error state.
pub mod palette {
    use ratatui::style::Color;
    use std::sync::OnceLock;

    pub struct Palette {
        // --- list row ---
        pub selection_bg: Color,
        pub date: Color,
        pub arrow: Color,
        pub from: Color,
        pub to: Color,
        pub subject: Color,
        pub key_hint: Color,

        // --- detail values ---
        pub label: Color,
        pub value_text: Color,
        pub value_time: Color,
        pub value_size: Color,
        pub value_number: Color,
        pub value_id: Color, // identifiers — readable but de-emphasized
        pub subtext: Color,  // secondary content (banner subtext, etc.)
        pub value_bool_true: Color,
        pub value_bool_false: Color,
        pub value_json: Color,

        // --- chrome ---
        pub muted: Color,
        pub header_meta: Color,
        pub live: Color,
        pub paused: Color,
        pub error: Color,

        // --- per-LogKind accents ---
        pub accent_inbound: Color,
        pub accent_outbound: Color,
        pub accent_account: Color,

        // --- status pills ---
        pub status_good: Color,
        pub status_bad: Color,
        pub status_warn: Color,
        pub status_info: Color,
        pub status_spam: Color,
        pub status_drop: Color,
        pub status_other: Color,
    }

    pub fn get() -> &'static Palette {
        static P: OnceLock<Palette> = OnceLock::new();
        P.get_or_init(init)
    }

    fn init() -> Palette {
        let truecolor = std::env::var("COLORTERM")
            .map(|v| {
                let v = v.to_ascii_lowercase();
                v == "truecolor" || v == "24bit"
            })
            .unwrap_or(false);

        let c = |r: u8, g: u8, b: u8| -> Color {
            if truecolor {
                Color::Rgb(r, g, b)
            } else {
                Color::Indexed(rgb_to_cube_index(r, g, b))
            }
        };

        // Tailwind hues, picked for legibility on both black and the selection bg.
        let status_good = c(34, 197, 94); // green-500
        let status_bad = c(239, 68, 68); // red-500
        let status_warn = c(234, 179, 8); // yellow-500
        let slate_300 = c(203, 213, 225);
        let slate_400 = c(148, 163, 184);
        let slate_500 = c(100, 116, 139);
        let slate_600 = c(71, 85, 105);
        let yellow_400 = c(250, 204, 21);

        Palette {
            selection_bg: c(40, 45, 60),
            date: slate_400,
            arrow: slate_600,
            from: slate_300,
            to: c(186, 230, 253), // sky-200
            subject: Color::White,
            key_hint: yellow_400,

            label: slate_400,
            value_text: Color::White,
            value_time: slate_300,
            value_size: yellow_400,
            value_number: yellow_400,
            // slate-300 is dim enough to read as "metadata" but readable on any
            // dark bg (slate-600 was too low-contrast on Ghostty's themed bg).
            value_id: slate_300,
            // Same idea: secondary content should be visibly less prominent than
            // the main value text without disappearing into the background.
            subtext: slate_400,
            value_bool_true: status_good,
            value_bool_false: status_bad,
            value_json: slate_400,

            muted: slate_600,
            header_meta: slate_400,
            live: status_good,
            paused: status_warn,
            error: status_bad,

            accent_inbound: c(56, 189, 248),   // sky-400
            accent_outbound: c(129, 140, 248), // indigo-400 (not pink)
            accent_account: c(251, 191, 36),   // amber-400

            status_good,
            status_bad,
            status_warn,
            status_info: c(96, 165, 250), // blue-400
            status_spam: c(168, 85, 247), // purple-500
            status_drop: slate_500,
            status_other: c(125, 211, 252), // sky-300
        }
    }

    /// Map an 8-bit-per-channel RGB triple to the closest xterm-256 cube index.
    /// The cube uses levels 0, 95, 135, 175, 215, 255 (so midpoints fall at ~47,
    /// 115, 155, 195, 235). Index 16 is (0,0,0); each `+1` to red costs 36,
    /// green costs 6, blue costs 1.
    fn rgb_to_cube_index(r: u8, g: u8, b: u8) -> u8 {
        let q = |v: u8| -> u8 {
            if v < 48 {
                0
            } else if v < 115 {
                1
            } else if v < 155 {
                2
            } else if v < 195 {
                3
            } else if v < 235 {
                4
            } else {
                5
            }
        };
        16 + 36 * q(r) + 6 * q(g) + q(b)
    }
}

fn status_color(s: &str) -> Color {
    let up = s.to_ascii_uppercase();
    // Order matters — check the more-specific variants first.
    if up.contains("DELIVER")
        || up.contains("ACCEPT")
        || up.contains("SENT")
        || up.contains("SUCCESS")
        || up.contains("COMPLETE")
        || up.contains("PASSED")
        || up == "OK"
        || up == "DONE"
    {
        palette::get().status_good
    } else if up.contains("HARD")
        || up.contains("REJECT")
        || up.contains("VIRUS")
        || up.contains("FAIL")
    {
        palette::get().status_bad
    } else if up.contains("SOFT") || up.contains("DEFER") || up.contains("RETRY") {
        palette::get().status_warn
    } else if up.contains("BOUNCE") {
        palette::get().status_bad
    } else if up.contains("SPAM") || up.contains("BLOCK") || up.contains("COMPLAINT") {
        palette::get().status_spam
    } else if up.contains("QUEUE") || up.contains("PENDING") || up.contains("WAIT") {
        palette::get().status_info
    } else if up.contains("DROP") {
        palette::get().status_drop
    } else {
        palette::get().status_other
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn short_id(id: &str) -> String {
    if id.len() <= 24 {
        id.to_string()
    } else {
        format!("{}…", &id[..23])
    }
}

fn entry_search_string(e: &Entry) -> String {
    serde_json::to_string(&e.raw).unwrap_or_default()
}

const TIME_KEYS: &[&str] = &[
    "last_updated",
    "lastUpdated",
    "start_time",
    "startTime",
    "date",
    "timestamp",
    "time",
    "created_at",
    "createdAt",
    "received_at",
    "receivedAt",
    "sent_at",
    "sentAt",
    "delivered_at",
    "deliveredAt",
    "logged_at",
    "loggedAt",
    "event_time",
    "eventTime",
    "event_at",
    "eventAt",
    "ts",
    "dt",
    "at",
    "when",
];

const STATUS_KEYS_OUTBOUND: &[&str] = &[
    "delivery_status",
    "deliveryStatus",
    "final_status",
    "finalStatus",
    "action",
    "status",
    "state",
    "result",
    "outcome",
    "event",
    "disposition",
    "logtype",
    "type",
];

const STATUS_KEYS_INBOUND: &[&str] = &[
    "logtype",
    "status",
    "delivery_status",
    "deliveryStatus",
    "state",
    "action",
    "result",
    "event",
    "type",
    "disposition",
];

const FROM_KEYS: &[&str] = &[
    "from_address",
    "fromAddress",
    "from",
    "sender",
    "src",
    "src_address",
    "srcAddress",
    "mail_from",
    "mailFrom",
    "header_from",
    "headerFrom",
];

const TO_KEYS: &[&str] = &[
    "to_address",
    "toAddress",
    "to",
    "recipient",
    "rcpt",
    "rcpt_to",
    "rcptTo",
    "dest",
    "destination",
];

const SUBJECT_KEYS: &[&str] = &["subject", "subj"];

fn field_str(m: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(v) = m.get(*k) {
            match v {
                Value::String(s) if !s.is_empty() => return Some(s.clone()),
                Value::Number(n) => return Some(n.to_string()),
                _ => {}
            }
        }
    }
    None
}

/// Look up the first key from `keys` that resolves to a usable scalar in `m`,
/// returning both the matched key and the formatted value.
fn field_str_with_key<'a>(
    m: &'a Map<String, Value>,
    keys: &[&'a str],
) -> Option<(&'a str, String)> {
    for k in keys {
        if let Some(v) = m.get(*k) {
            match v {
                Value::String(s) if !s.is_empty() => return Some((*k, s.clone())),
                Value::Number(n) => return Some((*k, n.to_string())),
                _ => {}
            }
        }
    }
    None
}

fn field_time(m: &Map<String, Value>) -> Option<String> {
    if let Some((_, formatted)) = field_time_with_key(m) {
        return Some(formatted);
    }
    None
}

/// Try every known timestamp key; if none match, scan every numeric field for a
/// plausible Unix timestamp (seconds or milliseconds). Returns `(matched_key, formatted)`.
fn field_time_with_key(m: &Map<String, Value>) -> Option<(&'static str, String)> {
    for k in TIME_KEYS {
        if let Some(v) = m.get(*k) {
            if let Some(formatted) = format_time_value(v) {
                return Some((*k, formatted));
            }
        }
    }
    // Heuristic fallback: any field whose value looks like a Unix timestamp.
    // Seconds: 1_000_000_000 (2001) .. 4_000_000_000 (2096).
    // Milliseconds: 1_000_000_000_000 .. 4_000_000_000_000.
    for (key, v) in m {
        let plausible = match v {
            Value::Number(n) => n
                .as_i64()
                .filter(|n| {
                    (1_000_000_000..4_000_000_000).contains(n)
                        || (1_000_000_000_000..4_000_000_000_000).contains(n)
                })
                .is_some(),
            _ => false,
        };
        if plausible {
            // Leak the key — accepted cost for ergonomics; only a handful per session.
            let leaked: &'static str = Box::leak(key.clone().into_boxed_str());
            return format_time_value(v).map(|s| (leaked, s));
        }
    }
    None
}

fn format_time_value(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        if s.is_empty() {
            return None;
        }
        return Some(format_time_string(s));
    }
    if let Some(n) = v.as_i64() {
        if n <= 0 {
            return None;
        }
        return Some(format_unix(n));
    }
    if let Some(f) = v.as_f64() {
        if f <= 0.0 {
            return None;
        }
        return Some(format_unix(f as i64));
    }
    None
}

fn field_status(m: &Map<String, Value>, kind: LogKind) -> Option<String> {
    let keys: &[&str] = match kind {
        LogKind::Outbound => STATUS_KEYS_OUTBOUND,
        LogKind::Inbound | LogKind::Account => STATUS_KEYS_INBOUND,
    };
    field_str(m, keys)
}

fn format_unix(secs: i64) -> String {
    // Heuristic: if it's likely ms, scale.
    let secs = if secs > 10_000_000_000 {
        secs / 1000
    } else {
        secs
    };
    // Manual UTC formatting (avoids pulling in chrono).
    let (h, m, s, day, mon, year) = unix_to_components(secs);
    format!("{year:04}-{mon:02}-{day:02} {h:02}:{m:02}:{s:02}")
}

fn format_time_string(s: &str) -> String {
    // If it parses as integer seconds, format as a date.
    if let Ok(n) = s.parse::<i64>() {
        return format_unix(n);
    }
    // Otherwise just truncate ISO-8601 to "YYYY-MM-DD HH:MM:SS" if recognizable.
    if s.len() >= 19 && s.as_bytes().get(10) == Some(&b'T') {
        let bytes = s.as_bytes();
        return format!(
            "{} {}",
            std::str::from_utf8(&bytes[0..10]).unwrap_or(""),
            std::str::from_utf8(&bytes[11..19]).unwrap_or("")
        );
    }
    s.to_string()
}

/// Convert a Unix timestamp (seconds, UTC) to (h, m, s, day, month, year).
fn unix_to_components(secs: i64) -> (u32, u32, u32, u32, u32, i32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;

    // Days since 1970-01-01 → calendar date (proleptic Gregorian).
    let mut z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i32 + (era as i32) * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    let _ = &mut z;
    (h, m, s, day, month, year)
}

fn entries_from_response(v: &Value) -> Vec<Map<String, Value>> {
    fn unwrap_array(arr: &[Value]) -> Vec<Map<String, Value>> {
        arr.iter()
            .filter_map(|v| match v {
                Value::Object(m) => Some(m.clone()),
                _ => None,
            })
            .collect()
    }
    match v {
        Value::Array(a) => unwrap_array(a),
        Value::Object(m) => {
            // JetEmail wraps array responses in `{ "result": [...], "success": true }`.
            // We also accept the common alternatives so this stays portable.
            for key in [
                "result", "results", "data", "items", "logs", "rows", "entries", "records",
            ] {
                if let Some(Value::Array(a)) = m.get(key) {
                    return unwrap_array(a);
                }
            }
            // Maybe { data: { items: [...] } } or { result: { entries: [...] } }.
            for key in ["result", "data"] {
                if let Some(Value::Object(inner)) = m.get(key) {
                    for k in ["items", "logs", "rows", "results", "entries"] {
                        if let Some(Value::Array(a)) = inner.get(k) {
                            return unwrap_array(a);
                        }
                    }
                }
            }
            vec![m.clone()]
        }
        _ => vec![],
    }
}

fn entry_id(m: &Map<String, Value>) -> String {
    for k in ["uid", "id", "message_id", "event_id", "uuid"] {
        if let Some(v) = m.get(k) {
            if let Some(s) = v.as_str() {
                return s.to_string();
            }
            if let Some(n) = v.as_i64() {
                return n.to_string();
            }
        }
    }
    // Fallback: hash the serialized JSON.
    let mut h = DefaultHasher::new();
    serde_json::to_string(m).unwrap_or_default().hash(&mut h);
    format!("h{:x}", h.finish())
}

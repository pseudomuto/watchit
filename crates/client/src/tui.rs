use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

use ratatui::{
    DefaultTerminal,
    Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Style, Stylize},
    text::Line,
    widgets::{Block, Cell, Clear, Padding, Paragraph, Row, Table, TableState, Tabs},
};
use shared::api::{
    download::{AddRequest, ListRequest, Torrent, torrent::State},
    search::{Result as SearchResult, SearchRequest},
};

use crate::api::Client;

/// Rows moved by PageUp/PageDown.
const PAGE_SIZE: usize = 10;

/// How long to wait for a key press before checking for search results.
const TICK: Duration = Duration::from_millis(50);

/// How often the Downloads tab refreshes while it's open.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Which tab is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Search,
    Downloads,
}

/// Which pane on the Search tab receives key presses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Search,
    Results,
}

/// What the event loop should do after a key press.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Search { id: u64, term: String },
    Add { name: String, uri: String },
    List,
    Quit,
}

#[derive(Debug, PartialEq, Eq)]
enum Status {
    Idle,
    Searching,
    /// The first List since starting is running.
    Fetching,
    Adding(String),
    Added(String),
    Error(String),
    /// The last List failed. Cleared by the next one that succeeds.
    ListFailed(String),
}

#[derive(Debug)]
pub struct App {
    query: String,
    focus: Focus,
    results: Vec<SearchResult>,
    table: TableState,
    status: Status,
    /// Id of the latest search. Responses for older ids are dropped.
    request_id: u64,
    /// The row waiting for confirmation while the confirm modal is open. It's
    /// a copy, so a search finishing behind the modal can't change what gets
    /// added.
    confirming: Option<SearchResult>,
    tab: Tab,
    downloads: Vec<Torrent>,
    downloads_table: TableState,
    /// True while a List request is running, so polls don't pile up.
    listing: bool,
    /// When the last List finished. `None` until the first one does.
    listed_at: Option<Instant>,
}

impl App {
    pub fn new(query: String) -> Self {
        Self {
            query,
            focus: Focus::Search,
            results: Vec::new(),
            table: TableState::default(),
            status: Status::Idle,
            request_id: 0,
            confirming: None,
            tab: Tab::Search,
            downloads: Vec::new(),
            downloads_table: TableState::default(),
            listing: false,
            listed_at: None,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }

        if self.confirming.is_some() {
            return self.handle_confirm_key(key);
        }

        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            return self.switch_tab();
        }

        match (self.tab, self.focus) {
            (Tab::Search, Focus::Search) => self.handle_search_key(key),
            (Tab::Search, Focus::Results) => self.handle_results_key(key),
            (Tab::Downloads, _) => self.handle_downloads_key(key),
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char(c) => self.query.push(c),
            KeyCode::Backspace => {
                self.query.pop();
            }
            KeyCode::Esc => self.focus = Focus::Results,
            KeyCode::Enter => return self.start_search(),
            _ => {}
        }

        Action::None
    }

    fn handle_results_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Action::Quit,
            KeyCode::Char('/') => self.focus = Focus::Search,
            KeyCode::Char('j') | KeyCode::Down => self.select(self.selected() + 1),
            KeyCode::Char('k') | KeyCode::Up => self.select(self.selected().saturating_sub(1)),
            KeyCode::PageDown => self.select(self.selected() + PAGE_SIZE),
            KeyCode::PageUp => self.select(self.selected().saturating_sub(PAGE_SIZE)),
            KeyCode::Char('g') | KeyCode::Home => self.select(0),
            KeyCode::Char('G') | KeyCode::End => self.select(usize::MAX),
            KeyCode::Char('d') => self.confirming = self.table.selected().and_then(|i| self.results.get(i)).cloned(),
            _ => {}
        }

        Action::None
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => return self.start_add(),
            KeyCode::Char('n') | KeyCode::Esc => self.confirming = None,
            _ => {}
        }

        Action::None
    }

    fn handle_downloads_key(&mut self, key: KeyEvent) -> Action {
        let selected = self.downloads_table.selected().unwrap_or(0);

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Action::Quit,
            KeyCode::Char('j') | KeyCode::Down => self.select_download(selected + 1),
            KeyCode::Char('k') | KeyCode::Up => self.select_download(selected.saturating_sub(1)),
            KeyCode::PageDown => self.select_download(selected + PAGE_SIZE),
            KeyCode::PageUp => self.select_download(selected.saturating_sub(PAGE_SIZE)),
            KeyCode::Char('g') | KeyCode::Home => self.select_download(0),
            KeyCode::Char('G') | KeyCode::End => self.select_download(usize::MAX),
            _ => {}
        }

        Action::None
    }

    /// Toggles between the tabs. Opening Downloads lists right away.
    fn switch_tab(&mut self) -> Action {
        self.tab = match self.tab {
            Tab::Search => Tab::Downloads,
            Tab::Downloads => Tab::Search,
        };

        match self.tab {
            Tab::Downloads => self.start_list(),
            Tab::Search => Action::None,
        }
    }

    /// Returns `Action::List` when the Downloads tab is open and the last List
    /// finished at least `POLL_INTERVAL` before `now`.
    pub fn tick(&mut self, now: Instant) -> Action {
        let due = self
            .listed_at
            .is_none_or(|at| now.saturating_duration_since(at) >= POLL_INTERVAL);

        match self.tab == Tab::Downloads && due {
            true => self.start_list(),
            false => Action::None,
        }
    }

    /// Starts a List unless one is already running.
    fn start_list(&mut self) -> Action {
        if self.listing {
            return Action::None;
        }

        self.listing = true;
        if self.listed_at.is_none() && self.list_owns_status() {
            self.status = Status::Fetching;
        }

        Action::List
    }

    /// Applies a List response that finished at `now`. The selection stays on
    /// the same torrent when rows move. Errors keep the previous rows.
    pub fn finish_list(&mut self, now: Instant, result: Result<Vec<Torrent>, String>) {
        self.listing = false;
        self.listed_at = Some(now);

        match result {
            Ok(torrents) => {
                let selected = self.downloads_table.selected();
                let id = selected.and_then(|i| self.downloads.get(i)).map(|t| t.id.clone());
                let row = id
                    .and_then(|id| torrents.iter().position(|t| t.id == id))
                    .or(selected)
                    .unwrap_or(0);

                self.downloads = torrents;
                self.select_download(row);

                if matches!(self.status, Status::Fetching | Status::ListFailed(_)) {
                    self.status = Status::Idle;
                }
            }
            Err(msg) if self.list_owns_status() => self.status = Status::ListFailed(msg),
            Err(_) => {}
        }
    }

    /// Whether List progress may replace the current status. Searches, adds,
    /// and their errors stay on screen until they're done or replaced.
    fn list_owns_status(&self) -> bool {
        matches!(
            self.status,
            Status::Idle | Status::Fetching | Status::ListFailed(_) | Status::Added(_)
        )
    }

    /// Selects download row `i`, clamped to the last row. Selects nothing when
    /// there are no torrents.
    fn select_download(&mut self, i: usize) {
        let row = self.downloads.len().checked_sub(1).map(|last| i.min(last));
        self.downloads_table.select(row);
    }

    fn start_add(&mut self) -> Action {
        let Some(r) = self.confirming.take() else {
            return Action::None;
        };

        self.status = Status::Adding(r.name.clone());
        Action::Add {
            name: r.name,
            uri: r.uri,
        }
    }

    /// Applies the response for adding the result named `name`.
    pub fn finish_add(&mut self, name: String, result: Result<(), String>) {
        self.status = match result {
            Ok(()) => Status::Added(name),
            Err(msg) => Status::Error(msg),
        };
    }

    /// Applies the response for search `id`, unless a newer search has
    /// started since. Errors keep the previous results on screen.
    pub fn finish_search(&mut self, id: u64, result: Result<Vec<SearchResult>, String>) {
        if id != self.request_id {
            return;
        }

        match result {
            Ok(results) => {
                self.results = results;
                self.status = Status::Idle;
                self.table.select(None);
                self.select(0);
            }
            Err(msg) => self.status = Status::Error(msg),
        }
    }

    /// Selects row `i`, clamped to the last row. Selects nothing when there
    /// are no results.
    fn select(&mut self, i: usize) {
        let row = self.results.len().checked_sub(1).map(|last| i.min(last));
        self.table.select(row);
    }

    fn selected(&self) -> usize {
        self.table.selected().unwrap_or(0)
    }

    fn start_search(&mut self) -> Action {
        let term = self.query.trim();
        if term.is_empty() {
            return Action::None;
        }

        self.request_id += 1;
        self.status = Status::Searching;
        self.focus = Focus::Results;

        Action::Search {
            id: self.request_id,
            term: term.to_string(),
        }
    }
}

/// A response from a request running in the background.
enum Response {
    Search {
        id: u64,
        result: Result<Vec<SearchResult>, String>,
    },
    Add {
        name: String,
        result: Result<(), String>,
    },
    List {
        result: Result<Vec<Torrent>, String>,
    },
}

/// Runs the TUI until the user quits. When `term` is given, it's prefilled
/// and searched right away.
pub fn run(terminal: &mut DefaultTerminal, client: Client, term: Option<String>) -> color_eyre::Result<()> {
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(term.unwrap_or_default());
    let mut action = app.start_search();

    loop {
        match action {
            Action::Quit => return Ok(()),
            Action::Search { id, term } => {
                let mut search = client.search.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = search
                        .search(SearchRequest { term })
                        .await
                        .map(|resp| resp.into_inner().results)
                        .map_err(|status| format!("search failed: {}", status.message()));

                    // Only fails when the receiver is gone, i.e. the user quit.
                    let _ = tx.send(Response::Search { id, result });
                });
            }
            Action::Add { name, uri } => {
                let mut download = client.download.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = download
                        .add(AddRequest { uri })
                        .await
                        .map(|_| ())
                        .map_err(|status| format!("add failed: {}", status.message()));

                    let _ = tx.send(Response::Add { name, result });
                });
            }
            Action::List => {
                let mut download = client.download.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = download
                        .list(ListRequest {})
                        .await
                        .map(|resp| resp.into_inner().torrents)
                        .map_err(|status| format!("list failed: {}", status.message()));

                    let _ = tx.send(Response::List { result });
                });
            }
            Action::None => {}
        }

        while let Ok(resp) = rx.try_recv() {
            match resp {
                Response::Search { id, result } => app.finish_search(id, result),
                Response::Add { name, result } => app.finish_add(name, result),
                Response::List { result } => app.finish_list(Instant::now(), result),
            }
        }

        terminal.draw(|f| render(&mut app, f))?;

        action = Action::None;
        if event::poll(TICK)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            action = app.handle_key(key);
        }

        if action == Action::None {
            action = app.tick(Instant::now());
        }
    }
}

pub fn render(app: &mut App, frame: &mut Frame) {
    let [tabs_area, body_area, status_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());

    render_tabs(app, frame, tabs_area);
    match app.tab {
        Tab::Search => {
            let [search_area, results_area] =
                Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(body_area);
            render_search(app, frame, search_area);
            render_results(app, frame, results_area);
        }
        Tab::Downloads => render_downloads(app, frame, body_area),
    }
    render_status(app, frame, status_area);

    if let Some(r) = &app.confirming {
        render_confirm(r, frame, frame.area());
    }
}

fn render_tabs(app: &App, frame: &mut Frame, area: Rect) {
    let selected = match app.tab {
        Tab::Search => 0,
        Tab::Downloads => 1,
    };

    let tabs = Tabs::new(["Search", "Downloads"])
        .select(selected)
        .style(Style::new().fg(Color::DarkGray))
        .highlight_style(Style::new().fg(Color::Yellow).bold());
    frame.render_widget(tabs, area);
}

fn pane(title: String, focused: bool) -> Block<'static> {
    let style = match focused {
        true => Style::new().fg(Color::Yellow),
        false => Style::new().fg(Color::DarkGray),
    };

    Block::bordered()
        .title(title)
        .border_style(style)
        .padding(Padding::horizontal(1))
}

fn render_search(app: &App, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Focus::Search;
    frame.render_widget(
        Paragraph::new(app.query.as_str()).block(pane(" Search ".into(), focused)),
        area,
    );

    if focused {
        // Skip the border and the padding.
        let x = area.x + 2 + app.query.chars().count() as u16;
        frame.set_cursor_position((x.min(area.right().saturating_sub(3)), area.y + 1));
    }
}

fn render_results(app: &mut App, frame: &mut Frame, area: Rect) {
    let right = |s: String| Cell::from(Line::from(s).right_aligned());
    let header = Row::new([
        Cell::from("Name"),
        right("Seeders".into()),
        right("Leechers".into()),
        right("Size".into()),
        right("Date".into()),
    ])
    .style(Style::new().bold());

    let rows = app.results.iter().map(|r| {
        Row::new([
            Cell::from(r.name.clone()),
            right(r.seeders.to_string()),
            right(r.leechers.to_string()),
            right(format_size(r.size_bytes)),
            right(format_date(r)),
        ])
    });

    let widths = [
        Constraint::Fill(1),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(10),
    ];

    let title = format!(" Results ({}) ", app.results.len());
    let table = Table::new(rows, widths)
        .header(header)
        .block(pane(title, app.focus == Focus::Results))
        .row_highlight_style(Style::new().reversed());

    frame.render_stateful_widget(table, area, &mut app.table);
}

fn render_downloads(app: &mut App, frame: &mut Frame, area: Rect) {
    let right = |s: String| Cell::from(Line::from(s).right_aligned());
    let header = Row::new([
        Cell::from("Name"),
        Cell::from("State"),
        Cell::from("Progress"),
        right("Size".into()),
        right("Down".into()),
        right("Up".into()),
        right("ETA".into()),
    ])
    .style(Style::new().bold());

    let rows = app.downloads.iter().map(|t| {
        let (label, color) = state_style(t.state());
        Row::new([
            Cell::from(t.name.clone()),
            Cell::from(label).style(Style::new().fg(color)),
            Cell::from(progress_bar(t.progress)),
            right(format_size(t.size_bytes)),
            right(format_rate(t.download_rate)),
            right(format_rate(t.upload_rate)),
            right(format_eta(t.eta_seconds)),
        ])
    });

    let widths = [
        Constraint::Fill(1),
        Constraint::Length(11),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(7),
    ];

    let title = format!(" Downloads ({}) ", app.downloads.len());
    let table = Table::new(rows, widths)
        .header(header)
        .block(pane(title, true))
        .row_highlight_style(Style::new().reversed());

    frame.render_stateful_widget(table, area, &mut app.downloads_table);
}

/// The label and color for a torrent's state.
fn state_style(state: State) -> (&'static str, Color) {
    match state {
        State::Downloading => ("Downloading", Color::Green),
        State::Queued => ("Queued", Color::Yellow),
        State::Checking => ("Checking", Color::Yellow),
        State::Error => ("Error", Color::Red),
        State::Paused => ("Paused", Color::DarkGray),
        State::Seeding => ("Seeding", Color::DarkGray),
        State::Unspecified => ("Unknown", Color::DarkGray),
    }
}

/// Draws the confirm modal centered over `area`. Long names are cut off at the
/// modal's edge.
fn render_confirm(r: &SearchResult, frame: &mut Frame, area: Rect) {
    let [area] = Layout::horizontal([Constraint::Percentage(80)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Length(5)]).flex(Flex::Center).areas(area);

    let text = vec![
        Line::from(r.name.as_str()),
        Line::default(),
        Line::from("[y] yes  [n] no").dark_gray(),
    ];

    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(text).block(pane(" Send to Deluge? ".into(), true)), area);
}

fn render_status(app: &App, frame: &mut Frame, area: Rect) {
    let status = match &app.status {
        Status::Searching => Line::from("Searching...").yellow(),
        Status::Fetching => Line::from("Fetching...").yellow(),
        Status::Adding(name) => Line::from(format!("Adding {name}...")).yellow(),
        Status::Added(name) => Line::from(format!("Added {name}")).green(),
        Status::Error(msg) => Line::from(msg.as_str()).red(),
        Status::ListFailed(msg) => Line::from(msg.as_str()).red(),
        Status::Idle if app.tab == Tab::Search && app.request_id > 0 && app.results.is_empty() => {
            Line::from("No results")
        }
        Status::Idle => Line::default(),
    };

    let hints = match (&app.confirming, app.tab, app.focus) {
        (Some(_), _, _) => "y send  n cancel",
        (None, Tab::Search, Focus::Search) => "enter search  esc results  tab downloads  ctrl-c quit",
        (None, Tab::Search, Focus::Results) => "j/k move  g/G top/bottom  d deluge  / search  tab downloads  q quit",
        (None, Tab::Downloads, _) => "j/k move  g/G top/bottom  tab search  q quit",
    };

    let [status_area, hints_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(hints.len() as u16)]).areas(area);
    frame.render_widget(status, status_area);
    frame.render_widget(Line::from(hints).dark_gray(), hints_area);
}

fn format_size(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    match unit {
        0 => format!("{bytes} B"),
        _ => format!("{size:.1} {}", UNITS[unit]),
    }
}

/// Draws `progress` (0 to 100) as a 5-cell bar plus a percentage. Values
/// outside that range, and NaN, are clamped.
fn progress_bar(progress: f64) -> String {
    let pct = match progress.is_nan() {
        true => 0.0,
        false => progress.clamp(0.0, 100.0),
    };
    let filled = (pct / 20.0).round() as usize;

    format!("{}{} {pct:>3.0}%", "█".repeat(filled), "░".repeat(5 - filled))
}

/// Formats a rate in bytes/sec, or `-` when nothing is moving.
fn format_rate(bytes_per_sec: i64) -> String {
    match bytes_per_sec {
        r if r <= 0 => "-".into(),
        r => format!("{}/s", format_size(r)),
    }
}

/// Formats an ETA in seconds as e.g. `45s`, `3m`, `2h10m` or `1d4h`, or `-`
/// when it's unknown.
fn format_eta(secs: i64) -> String {
    match secs {
        s if s <= 0 => "-".into(),
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h{}m", s / 3600, s % 3600 / 60),
        s => format!("{}d{}h", s / 86_400, s % 86_400 / 3600),
    }
}

/// Formats `created_at` as a UTC date, or `-` when it's missing or invalid.
fn format_date(r: &SearchResult) -> String {
    r.created_at
        .as_ref()
        .and_then(|ts| jiff::Timestamp::new(ts.seconds, ts.nanos).ok())
        .map_or_else(|| "-".into(), |ts| ts.strftime("%Y-%m-%d").to_string())
}

#[cfg(test)]
mod tests {
    use prost_types::Timestamp;
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    #[test]
    fn format_size_uses_binary_units() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KiB");
        assert_eq!(format_size(6_114_656_256), "5.7 GiB");
    }

    #[test]
    fn progress_bar_clamps() {
        assert_eq!(progress_bar(0.0), "░░░░░   0%");
        assert_eq!(progress_bar(62.0), "███░░  62%");
        assert_eq!(progress_bar(100.0), "█████ 100%");
        assert_eq!(progress_bar(100.00001), "█████ 100%");
        assert_eq!(progress_bar(150.0), "█████ 100%");
        assert_eq!(progress_bar(-5.0), "░░░░░   0%");
        assert_eq!(progress_bar(f64::NAN), "░░░░░   0%");
    }

    #[test]
    fn format_rate_uses_binary_units_per_second() {
        assert_eq!(format_rate(0), "-");
        assert_eq!(format_rate(-1), "-");
        assert_eq!(format_rate(1536), "1.5 KiB/s");
        assert_eq!(format_rate(4_404_019), "4.2 MiB/s");
    }

    #[test]
    fn format_eta_is_compact() {
        assert_eq!(format_eta(0), "-");
        assert_eq!(format_eta(-1), "-");
        assert_eq!(format_eta(45), "45s");
        assert_eq!(format_eta(180), "3m");
        assert_eq!(format_eta(7800), "2h10m");
        assert_eq!(format_eta(100_800), "1d4h");
    }

    #[test]
    fn format_date_uses_utc_date() {
        let r = SearchResult {
            created_at: Some(Timestamp {
                seconds: 1_777_075_200, // 2026-04-25T00:00:00Z
                nanos: 0,
            }),
            ..Default::default()
        };

        assert_eq!(format_date(&r), "2026-04-25");
    }

    #[test]
    fn format_date_without_timestamp_is_a_dash() {
        assert_eq!(format_date(&SearchResult::default()), "-");
    }

    fn draw(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 10)).unwrap();
        terminal.draw(|f| render(app, f)).unwrap();

        let buf = terminal.backend().buffer();
        buf.content()
            .chunks(buf.area.width as usize)
            .fold(String::new(), |mut out, row| {
                row.iter().for_each(|c| out.push_str(c.symbol()));
                out.push('\n');
                out
            })
    }

    #[test]
    fn render_shows_results_without_uris() {
        let mut app = App::new("ubuntu".into());
        let Action::Search { id, .. } = app.start_search() else {
            panic!("expected a search");
        };
        app.finish_search(
            id,
            Ok(vec![SearchResult {
                name: "ubuntu-24.04-desktop-amd64.iso".into(),
                uri: "magnet:?xt=urn:btih:abc".into(),
                seeders: 812,
                leechers: 33,
                size_bytes: 6_114_656_256,
                created_at: Some(Timestamp {
                    seconds: 1_777_075_200,
                    nanos: 0,
                }),
            }]),
        );

        let screen = draw(&mut app);
        for want in [
            "ubuntu",
            "Name",
            "Seeders",
            "Leechers",
            "Size",
            "Date",
            "812",
            "33",
            "5.7 GiB",
            "2026-04-25",
        ] {
            assert!(screen.contains(want), "missing {want:?} in:\n{screen}");
        }
        assert!(
            screen.contains("ubuntu-24.04-desktop-amd64.iso"),
            "missing name in:\n{screen}"
        );
        assert!(!screen.contains("magnet:"), "uri shown in:\n{screen}");
    }

    #[test]
    fn render_shows_downloads() {
        let mut t = Torrent {
            id: "a".into(),
            name: "ubuntu-24.04.iso".into(),
            progress: 62.0,
            size_bytes: 6_114_656_256,
            download_rate: 4_404_019,
            upload_rate: 122_880,
            eta_seconds: 180,
            ..Default::default()
        };
        t.set_state(State::Downloading);
        let mut app = downloads_app(vec![t], Instant::now());

        let screen = draw(&mut app);
        for want in [
            "Search",
            "Downloads (1)",
            "Name",
            "State",
            "Progress",
            "ETA",
            "ubuntu-24.04.iso",
            "Downloading",
            "███░░  62%",
            "5.7 GiB",
            "4.2 MiB/s",
            "120.0 KiB/s",
            "3m",
            "tab search",
        ] {
            assert!(screen.contains(want), "missing {want:?} in:\n{screen}");
        }
        assert!(!screen.contains("Results"), "search body shown in:\n{screen}");
    }

    #[test]
    fn render_pads_panels() {
        let mut app = App::new("ubuntu".into());

        let screen = draw(&mut app);
        assert!(screen.contains("│ ubuntu"), "missing padding in:\n{screen}");
    }

    #[test]
    fn render_puts_the_cursor_after_the_query() {
        let mut app = App::new("ubuntu".into());
        let mut terminal = Terminal::new(TestBackend::new(100, 10)).unwrap();
        terminal.draw(|f| render(&mut app, f)).unwrap();

        // 1 for the border, 1 for the padding, then the query.
        let pos = terminal.get_cursor_position().unwrap();
        assert_eq!((pos.x, pos.y), (2 + 6, 2));
    }

    #[test]
    fn render_shows_tabs_on_the_search_tab() {
        let mut app = App::new(String::new());

        let screen = draw(&mut app);
        assert!(screen.contains("Search"), "missing tab in:\n{screen}");
        assert!(screen.contains("Downloads"), "missing tab in:\n{screen}");
        assert!(screen.contains("tab downloads"), "missing hint in:\n{screen}");
    }

    #[test]
    fn render_shows_fetching_until_the_first_list() {
        let mut app = App::new(String::new());
        press(&mut app, KeyCode::Tab);
        assert!(draw(&mut app).contains("Fetching..."));

        app.finish_list(Instant::now(), Ok(vec![torrent("a")]));
        assert!(!draw(&mut app).contains("Fetching..."));
    }

    #[test]
    fn render_hides_no_results_on_downloads_tab() {
        let mut app = app_with_results(0);
        assert!(draw(&mut app).contains("No results"));

        press(&mut app, KeyCode::Tab);
        app.finish_list(Instant::now(), Ok(vec![]));

        let screen = draw(&mut app);
        assert!(!screen.contains("No results"), "search status leaked in:\n{screen}");
        assert!(screen.contains("Downloads (0)"), "missing title in:\n{screen}");
    }

    #[test]
    fn render_shows_list_errors() {
        let now = Instant::now();
        let mut app = downloads_app(vec![torrent("a")], now);
        app.finish_list(now, Err("list failed: deluge is down".into()));

        let screen = draw(&mut app);
        assert!(
            screen.contains("list failed: deluge is down"),
            "missing error in:\n{screen}"
        );
        assert!(screen.contains("a.iso"), "rows dropped in:\n{screen}");
    }

    #[test]
    fn render_shows_status() {
        let mut app = App::new("ubuntu".into());
        let Action::Search { id, .. } = app.start_search() else {
            panic!("expected a search");
        };
        assert!(draw(&mut app).contains("Searching..."));

        app.finish_search(id, Ok(vec![]));
        assert!(draw(&mut app).contains("No results"));

        app.finish_search(id, Err("jackett is down".into()));
        assert!(draw(&mut app).contains("jackett is down"));
    }

    fn press(app: &mut App, code: KeyCode) -> Action {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    #[test]
    fn typing_edits_the_query_when_search_is_focused() {
        let mut app = App::new(String::new());
        type_str(&mut app, "ubuntx");
        press(&mut app, KeyCode::Backspace);
        type_str(&mut app, "u");

        assert_eq!(app.query, "ubuntu");
    }

    #[test]
    fn enter_starts_a_search_and_focuses_results() {
        let mut app = App::new("ubuntu".into());

        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Action::Search {
                id: 1,
                term: "ubuntu".into()
            }
        );
        assert_eq!(app.focus, Focus::Results);
        assert_eq!(app.status, Status::Searching);
    }

    #[test]
    fn enter_with_blank_query_does_nothing() {
        let mut app = App::new("  ".into());

        assert_eq!(press(&mut app, KeyCode::Enter), Action::None);
        assert_eq!(app.focus, Focus::Search);
    }

    #[test]
    fn tab_switches_tabs_and_keeps_the_query() {
        let mut app = App::new("ubu".into());

        assert_eq!(press(&mut app, KeyCode::Tab), Action::List);
        assert_eq!(app.tab, Tab::Downloads);

        assert_eq!(press(&mut app, KeyCode::BackTab), Action::None);
        assert_eq!(app.tab, Tab::Search);
        assert_eq!(app.query, "ubu");
        assert_eq!(app.focus, Focus::Search);

        // Also from the results pane.
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.tab, Tab::Downloads);
    }

    #[test]
    fn esc_and_slash_switch_focus() {
        let mut app = App::new(String::new());
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.focus, Focus::Results);

        press(&mut app, KeyCode::Char('/'));
        assert_eq!(app.focus, Focus::Search);
    }

    #[test]
    fn q_quits_from_results_but_types_in_search() {
        let mut app = App::new(String::new());
        assert_eq!(press(&mut app, KeyCode::Char('q')), Action::None);
        assert_eq!(app.query, "q");

        press(&mut app, KeyCode::Esc);
        assert_eq!(press(&mut app, KeyCode::Char('q')), Action::Quit);
    }

    fn torrent(id: &str) -> Torrent {
        Torrent {
            id: id.into(),
            name: format!("{id}.iso"),
            ..Default::default()
        }
    }

    /// An app on the Downloads tab whose first List finished at `now`.
    fn downloads_app(torrents: Vec<Torrent>, now: Instant) -> App {
        let mut app = App::new(String::new());
        assert_eq!(press(&mut app, KeyCode::Tab), Action::List);
        app.finish_list(now, Ok(torrents));
        app
    }

    #[test]
    fn tick_lists_only_on_the_downloads_tab_and_never_overlaps() {
        let mut app = App::new(String::new());
        let now = Instant::now();
        assert_eq!(app.tick(now), Action::None);

        assert_eq!(press(&mut app, KeyCode::Tab), Action::List);
        assert_eq!(app.status, Status::Fetching);
        assert_eq!(app.tick(now + Duration::from_secs(10)), Action::None);
    }

    #[test]
    fn tick_waits_for_the_poll_interval_after_a_list() {
        let now = Instant::now();
        let mut app = downloads_app(vec![torrent("a")], now);
        assert_eq!(app.status, Status::Idle);

        assert_eq!(app.tick(now + Duration::from_millis(1999)), Action::None);
        assert_eq!(app.tick(now + POLL_INTERVAL), Action::List);
        assert_eq!(app.status, Status::Idle, "later polls must not show Fetching");
    }

    #[test]
    fn switching_back_to_downloads_lists_right_away() {
        let now = Instant::now();
        let mut app = downloads_app(vec![torrent("a")], now);

        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.tick(now + POLL_INTERVAL),
            Action::None,
            "no polling on the Search tab"
        );
        assert_eq!(press(&mut app, KeyCode::Tab), Action::List);
    }

    #[test]
    fn finish_list_keeps_selection_on_the_same_torrent() {
        let now = Instant::now();
        let mut app = downloads_app(vec![torrent("a"), torrent("b"), torrent("c")], now);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.downloads_table.selected(), Some(1));

        app.finish_list(now, Ok(vec![torrent("b"), torrent("c"), torrent("a")]));
        assert_eq!(app.downloads_table.selected(), Some(0));
    }

    #[test]
    fn finish_list_clamps_when_the_selected_torrent_is_gone() {
        let now = Instant::now();
        let mut app = downloads_app(vec![torrent("a"), torrent("b"), torrent("c")], now);
        press(&mut app, KeyCode::Char('G'));

        app.finish_list(now, Ok(vec![torrent("a")]));
        assert_eq!(app.downloads_table.selected(), Some(0));
    }

    #[test]
    fn finish_list_error_keeps_rows_and_recovers() {
        let now = Instant::now();
        let mut app = downloads_app(vec![torrent("a"), torrent("b")], now);

        app.finish_list(now, Err("list failed: deluge is down".into()));
        assert_eq!(app.downloads.len(), 2);
        assert_eq!(app.status, Status::ListFailed("list failed: deluge is down".into()));
        assert_eq!(
            app.tick(now + POLL_INTERVAL),
            Action::List,
            "keeps polling after an error"
        );

        app.finish_list(now, Ok(vec![torrent("a")]));
        assert_eq!(app.status, Status::Idle);
    }

    #[test]
    fn finish_list_leaves_other_statuses_alone() {
        let now = Instant::now();
        let mut app = downloads_app(vec![], now);

        for status in [Status::Searching, Status::Error("jackett is down".into())] {
            app.status = status;
            let want = format!("{:?}", app.status);
            app.finish_list(now, Ok(vec![torrent("a")]));
            assert_eq!(format!("{:?}", app.status), want);
        }
    }

    #[test]
    fn failed_list_leaves_search_and_add_statuses_alone() {
        let now = Instant::now();
        let mut app = downloads_app(vec![], now);

        for status in [
            Status::Searching,
            Status::Adding("ubuntu.iso".into()),
            Status::Error("add failed: boom".into()),
        ] {
            app.status = status;
            let want = format!("{:?}", app.status);
            app.finish_list(now, Err("list failed: deluge is down".into()));
            assert_eq!(format!("{:?}", app.status), want);
        }
    }

    #[test]
    fn first_list_does_not_hide_a_running_search() {
        let mut app = App::new("ubuntu".into());
        press(&mut app, KeyCode::Enter);

        assert_eq!(press(&mut app, KeyCode::Tab), Action::List);
        assert_eq!(app.status, Status::Searching);
    }

    #[test]
    fn downloads_navigation_moves_selection_within_bounds() {
        let mut app = downloads_app(vec![torrent("a"), torrent("b"), torrent("c")], Instant::now());
        assert_eq!(app.downloads_table.selected(), Some(0));

        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.downloads_table.selected(), Some(0));

        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.downloads_table.selected(), Some(2));

        press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.downloads_table.selected(), Some(0));

        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.downloads_table.selected(), Some(2));
    }

    #[test]
    fn downloads_navigation_with_no_torrents_selects_nothing() {
        let mut app = downloads_app(vec![], Instant::now());

        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.downloads_table.selected(), None);
    }

    #[test]
    fn q_and_esc_quit_from_downloads() {
        for key in [KeyCode::Char('q'), KeyCode::Esc] {
            let mut app = downloads_app(vec![], Instant::now());
            assert_eq!(press(&mut app, key), Action::Quit, "{key:?}");
        }
    }

    fn result(name: &str) -> SearchResult {
        SearchResult {
            name: name.into(),
            uri: format!("magnet:?dn={name}"),
            ..Default::default()
        }
    }

    fn app_with_results(n: usize) -> App {
        let mut app = App::new("x".into());
        let Action::Search { id, .. } = press(&mut app, KeyCode::Enter) else {
            panic!("expected a search");
        };
        app.finish_search(id, Ok((0..n).map(|i| result(&i.to_string())).collect()));
        app
    }

    #[test]
    fn finished_search_replaces_results_and_selects_first_row() {
        let app = app_with_results(3);

        assert_eq!(app.results.len(), 3);
        assert_eq!(app.table.selected(), Some(0));
        assert_eq!(app.status, Status::Idle);
    }

    #[test]
    fn finished_search_with_no_results_selects_nothing() {
        let app = app_with_results(0);

        assert_eq!(app.table.selected(), None);
    }

    #[test]
    fn stale_search_results_are_dropped() {
        let mut app = app_with_results(1);
        let Action::Search { id, .. } = app.start_search() else {
            panic!("expected a search");
        };

        app.finish_search(id - 1, Ok(vec![result("old"), result("old")]));
        assert_eq!(app.results.len(), 1);
        assert_eq!(app.status, Status::Searching);
    }

    #[test]
    fn failed_search_keeps_previous_results() {
        let mut app = app_with_results(2);
        let Action::Search { id, .. } = app.start_search() else {
            panic!("expected a search");
        };

        app.finish_search(id, Err("jackett is down".into()));
        assert_eq!(app.results.len(), 2);
        assert_eq!(app.status, Status::Error("jackett is down".into()));
    }

    #[test]
    fn navigation_moves_selection_within_bounds() {
        let mut app = app_with_results(3);

        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.table.selected(), Some(0));

        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.table.selected(), Some(2));

        press(&mut app, KeyCode::Up);
        assert_eq!(app.table.selected(), Some(1));

        press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.table.selected(), Some(0));

        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.table.selected(), Some(2));

        press(&mut app, KeyCode::PageUp);
        assert_eq!(app.table.selected(), Some(0));

        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.table.selected(), Some(2));
    }

    #[test]
    fn navigation_with_no_results_selects_nothing() {
        let mut app = app_with_results(0);

        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.table.selected(), None);
    }

    #[test]
    fn ctrl_c_quits_from_anywhere() {
        let mut app = App::new(String::new());
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert_eq!(app.handle_key(ctrl_c), Action::Quit);
        app.focus = Focus::Results;
        assert_eq!(app.handle_key(ctrl_c), Action::Quit);
    }

    fn app_confirming_row(n: usize, row: usize) -> App {
        let mut app = app_with_results(n);
        app.select(row);
        press(&mut app, KeyCode::Char('d'));
        app
    }

    #[test]
    fn d_opens_confirm_for_the_selected_row() {
        let app = app_confirming_row(3, 1);

        assert_eq!(app.confirming, Some(result("1")));
    }

    #[test]
    fn d_with_no_results_does_nothing() {
        let mut app = app_with_results(0);

        assert_eq!(press(&mut app, KeyCode::Char('d')), Action::None);
        assert_eq!(app.confirming, None);
    }

    #[test]
    fn y_and_enter_confirm_the_add() {
        for key in [KeyCode::Char('y'), KeyCode::Enter] {
            let mut app = app_confirming_row(3, 2);

            assert_eq!(
                press(&mut app, key),
                Action::Add {
                    name: "2".into(),
                    uri: "magnet:?dn=2".into(),
                }
            );
            assert_eq!(app.confirming, None);
            assert_eq!(app.status, Status::Adding("2".into()));
            assert_eq!(app.focus, Focus::Results);
        }
    }

    #[test]
    fn n_and_esc_cancel_without_quitting() {
        for key in [KeyCode::Char('n'), KeyCode::Esc] {
            let mut app = app_confirming_row(3, 1);

            assert_eq!(press(&mut app, key), Action::None);
            assert_eq!(app.confirming, None);
            assert_eq!(app.focus, Focus::Results);
            assert_eq!(app.status, Status::Idle);
        }
    }

    #[test]
    fn other_keys_are_ignored_while_confirming() {
        let mut app = app_confirming_row(3, 1);

        for key in [
            KeyCode::Char('q'),
            KeyCode::Char('j'),
            KeyCode::Char('/'),
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Char('d'),
        ] {
            assert_eq!(press(&mut app, key), Action::None, "{key:?}");
        }
        assert_eq!(app.confirming, Some(result("1")));
        assert_eq!(app.table.selected(), Some(1));
        assert_eq!(app.focus, Focus::Results);
        assert_eq!(app.tab, Tab::Search);
    }

    #[test]
    fn ctrl_c_quits_while_confirming() {
        let mut app = app_confirming_row(3, 1);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert_eq!(app.handle_key(ctrl_c), Action::Quit);
    }

    #[test]
    fn search_finishing_while_confirming_keeps_the_picked_row() {
        let mut app = app_confirming_row(3, 1);
        let id = app.request_id;

        app.finish_search(id, Ok(vec![result("new")]));
        assert_eq!(app.confirming, Some(result("1")));

        assert_eq!(
            press(&mut app, KeyCode::Char('y')),
            Action::Add {
                name: "1".into(),
                uri: "magnet:?dn=1".into(),
            }
        );
    }

    #[test]
    fn finish_add_reports_the_outcome() {
        let mut app = app_with_results(1);

        app.finish_add("ubuntu".into(), Ok(()));
        assert_eq!(app.status, Status::Added("ubuntu".into()));

        app.finish_add("ubuntu".into(), Err("add failed: already in session".into()));
        assert_eq!(app.status, Status::Error("add failed: already in session".into()));
    }

    #[test]
    fn render_shows_add_status() {
        let mut app = app_with_results(1);

        app.status = Status::Adding("ubuntu.iso".into());
        assert!(draw(&mut app).contains("Adding ubuntu.iso..."));

        app.finish_add("ubuntu.iso".into(), Ok(()));
        assert!(draw(&mut app).contains("Added ubuntu.iso"));
    }

    #[test]
    fn render_shows_confirm_modal_with_the_name() {
        let mut app = app_confirming_row(3, 1);
        app.confirming = Some(result("ubuntu-24.04-desktop-amd64.iso"));

        let screen = draw(&mut app);
        for want in [
            "Send to Deluge?",
            "ubuntu-24.04-desktop-amd64.iso",
            "[y] yes  [n] no",
            "y send  n cancel",
        ] {
            assert!(screen.contains(want), "missing {want:?} in:\n{screen}");
        }
    }

    #[test]
    fn render_hides_confirm_modal_after_cancel() {
        let mut app = app_confirming_row(3, 1);
        press(&mut app, KeyCode::Esc);

        let screen = draw(&mut app);
        assert!(!screen.contains("Send to Deluge?"), "modal still shown:\n{screen}");
        assert!(screen.contains("d deluge"), "missing results hint:\n{screen}");
    }

    #[test]
    fn render_truncates_long_names_in_confirm_modal() {
        let mut app = app_confirming_row(3, 1);
        app.confirming = Some(result(&"x".repeat(500)));

        let screen = draw(&mut app);
        assert!(screen.contains("Send to Deluge?"), "missing modal in:\n{screen}");
        assert!(screen.contains(&"x".repeat(40)), "missing name in:\n{screen}");
        assert!(
            screen.contains("[y] yes  [n] no"),
            "name wrapped over the hints in:\n{screen}"
        );
    }
}

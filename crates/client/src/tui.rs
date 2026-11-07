use std::{sync::mpsc, time::Duration};

use ratatui::{
    DefaultTerminal,
    Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::Line,
    widgets::{Block, Cell, Paragraph, Row, Table, TableState},
};
use shared::api::search::{Result as SearchResult, SearchRequest, SearchServiceClient};
use tonic::transport::Channel;

/// Rows moved by PageUp/PageDown.
const PAGE_SIZE: usize = 10;

/// How long to wait for a key press before checking for search results.
const TICK: Duration = Duration::from_millis(50);

/// Which pane receives key presses.
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
    Quit,
}

#[derive(Debug, PartialEq, Eq)]
enum Status {
    Idle,
    Searching,
    Error(String),
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
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }

        match self.focus {
            Focus::Search => self.handle_search_key(key),
            Focus::Results => self.handle_results_key(key),
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char(c) => self.query.push(c),
            KeyCode::Backspace => {
                self.query.pop();
            }
            KeyCode::Tab | KeyCode::Esc => self.focus = Focus::Results,
            KeyCode::Enter => return self.start_search(),
            _ => {}
        }

        Action::None
    }

    fn handle_results_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Action::Quit,
            KeyCode::Char('/') | KeyCode::Tab => self.focus = Focus::Search,
            KeyCode::Char('j') | KeyCode::Down => self.select(self.selected() + 1),
            KeyCode::Char('k') | KeyCode::Up => self.select(self.selected().saturating_sub(1)),
            KeyCode::PageDown => self.select(self.selected() + PAGE_SIZE),
            KeyCode::PageUp => self.select(self.selected().saturating_sub(PAGE_SIZE)),
            KeyCode::Char('g') | KeyCode::Home => self.select(0),
            KeyCode::Char('G') | KeyCode::End => self.select(usize::MAX),
            _ => {}
        }

        Action::None
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

/// Runs the TUI until the user quits. When `term` is given, it's prefilled
/// and searched right away.
pub fn run(
    terminal: &mut DefaultTerminal,
    search: SearchServiceClient<Channel>,
    term: Option<String>,
) -> color_eyre::Result<()> {
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(term.unwrap_or_default());
    let mut action = app.start_search();

    loop {
        match action {
            Action::Quit => return Ok(()),
            Action::Search { id, term } => {
                let mut search = search.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = search
                        .search(SearchRequest { term })
                        .await
                        .map(|resp| resp.into_inner().results)
                        .map_err(|status| format!("search failed: {}", status.message()));

                    // Only fails when the receiver is gone, i.e. the user quit.
                    let _ = tx.send((id, result));
                });
            }
            Action::None => {}
        }

        while let Ok((id, result)) = rx.try_recv() {
            app.finish_search(id, result);
        }

        terminal.draw(|f| render(&mut app, f))?;

        action = Action::None;
        if event::poll(TICK)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            action = app.handle_key(key);
        }
    }
}

pub fn render(app: &mut App, frame: &mut Frame) {
    let [search_area, results_area, status_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());

    render_search(app, frame, search_area);
    render_results(app, frame, results_area);
    render_status(app, frame, status_area);
}

fn pane(title: String, focused: bool) -> Block<'static> {
    let style = match focused {
        true => Style::new().fg(Color::Yellow),
        false => Style::new().fg(Color::DarkGray),
    };

    Block::bordered().title(title).border_style(style)
}

fn render_search(app: &App, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Focus::Search;
    frame.render_widget(
        Paragraph::new(app.query.as_str()).block(pane(" Search ".into(), focused)),
        area,
    );

    if focused {
        let x = area.x + 1 + app.query.chars().count() as u16;
        frame.set_cursor_position((x.min(area.right().saturating_sub(2)), area.y + 1));
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

fn render_status(app: &App, frame: &mut Frame, area: Rect) {
    let status = match &app.status {
        Status::Searching => Line::from("Searching...").yellow(),
        Status::Error(msg) => Line::from(msg.as_str()).red(),
        Status::Idle if app.request_id > 0 && app.results.is_empty() => Line::from("No results"),
        Status::Idle => Line::default(),
    };

    let hints = match app.focus {
        Focus::Search => "enter search  tab results  ctrl-c quit",
        Focus::Results => "j/k move  g/G top/bottom  / search  q quit",
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
    fn tab_and_slash_switch_focus() {
        let mut app = App::new(String::new());
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Results);

        press(&mut app, KeyCode::Char('/'));
        assert_eq!(app.focus, Focus::Search);

        press(&mut app, KeyCode::Esc);
        assert_eq!(app.focus, Focus::Results);
    }

    #[test]
    fn q_quits_from_results_but_types_in_search() {
        let mut app = App::new(String::new());
        assert_eq!(press(&mut app, KeyCode::Char('q')), Action::None);
        assert_eq!(app.query, "q");

        press(&mut app, KeyCode::Tab);
        assert_eq!(press(&mut app, KeyCode::Char('q')), Action::Quit);
    }

    fn result(name: &str) -> SearchResult {
        SearchResult {
            name: name.into(),
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
}

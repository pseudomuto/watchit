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

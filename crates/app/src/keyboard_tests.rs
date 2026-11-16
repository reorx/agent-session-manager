//! Behavior: the keyboard paths of a search. The whole app runs in a headless window on the core
//! fixture tree, and keys go through GPUI's own dispatch: the bindings of `bind_keys`, the key
//! contexts of the focused element and the action handlers of `AppRoot`. No OS event is sent,
//! so nothing can land in another application.
//!
//! No `use gpui_kit::*` here: with `test-support` on, the glob carries GPUI's `test` attribute.

use std::time::Duration;

use agent_session_core::claude::paths::ClaudeDir;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, App, AppContext as _, Entity, Focusable as _, TestAppContext, Window};

use crate::toolbar::SEARCH_DEBOUNCE;
use crate::{AppPaths, AppRoot, IndexStatus, bind_keys, style};

struct Harness {
    _dir: tempfile::TempDir,
    app: Entity<AppRoot>,
    window: AnyWindowHandle,
}

/// The app on the fixture sessions, with a throw-away index and view state, once the start-up
/// scan is over.
fn launch(cx: &mut TestAppContext) -> Harness {
    launch_with(cx, |_| {})
}

/// `launch`, after `prepare` put whatever it likes into the throw-away directory (`state.json`).
fn launch_with(cx: &mut TestAppContext, prepare: impl FnOnce(&std::path::Path)) -> Harness {
    let h = start(cx, prepare, |dir| dir.join("index.sqlite"));
    wait_until(cx, &h, "the start-up scan", |app, cx| matches!(app.index_status, IndexStatus::Idle(_)) && shown_rows(app, cx) > 0);
    h
}

/// The app with its window, without waiting for anything; `db` says where the index goes.
fn start(cx: &mut TestAppContext, prepare: impl FnOnce(&std::path::Path), db: impl FnOnce(&std::path::Path) -> std::path::PathBuf) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    prepare(dir.path());
    let claude = dir.path().join("claude");
    copy_fixture(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../core/tests/fixtures/claude")), &claude);
    let paths = AppPaths {
        claude: ClaudeDir::new(claude),
        db: db(dir.path()),
        state: dir.path().join("state.json"),
        favorites: agent_session_core::favorites::FavoritesDir { root: dir.path().join("favorites"), trash: agent_session_core::favorites::Trash::Dir(dir.path().join("trash")) },
    };
    // the store and search threads are real ones; the test scheduler must put up with their wake-ups
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        style::install_theme(cx);
        bind_keys(cx);
    });
    let mut app = None;
    let window = cx.add_window(|window, cx| {
        let view = cx.new(|cx| AppRoot::new(paths, window, cx));
        app = Some(view.clone());
        Root::new(view, window, cx)
    });
    Harness { _dir: dir, app: app.expect("the window builds the app"), window: window.into() }
}

/// The store and search threads answer in real time, while the test executor only runs what
/// is ready: look again every few milliseconds.
fn wait_until(cx: &mut TestAppContext, h: &Harness, what: &str, ready: impl Fn(&AppRoot, &App) -> bool) {
    for _ in 0..500 {
        cx.run_until_parked();
        if h.app.read_with(cx, |app, cx| ready(app, cx)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

fn in_window<R>(cx: &mut TestAppContext, h: &Harness, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
    cx.update_window(h.window, |_, window, cx| f(window, cx)).expect("the test window is open")
}

fn press(cx: &mut TestAppContext, h: &Harness, key: &str) {
    in_window(cx, h, |window, cx| window.press(key, cx));
}

fn type_text(cx: &mut TestAppContext, h: &Harness, text: &str) {
    in_window(cx, h, |window, cx| window.input(text, cx));
}

fn shown_rows(app: &AppRoot, cx: &App) -> usize {
    app.table.read(cx).delegate().ix.len()
}

fn search_box(cx: &mut TestAppContext, h: &Harness) -> String {
    h.app.read_with(cx, |app, cx| app.search.read(cx).value().to_string())
}

/// (the search box has the focus, the table has the focus)
fn focus(cx: &mut TestAppContext, h: &Harness) -> (bool, bool) {
    let app = h.app.clone();
    in_window(cx, h, |window, cx| {
        let app = app.read(cx);
        (app.search.read(cx).focus_handle(cx).is_focused(window), app.table.read(cx).focus_handle(cx).is_focused(window))
    })
}

fn search_done(app: &AppRoot, _: &App) -> bool {
    app.search_summary.is_some()
}

fn sort_like_header(cx: &mut TestAppContext, h: &Harness, key: &str) {
    use gpui_kit::component::table::{ColumnSort, TableDelegate};
    let (table, col, sort) = h.app.read_with(cx, |app, cx| {
        let delegate = app.table.read(cx).delegate();
        let col = (0..delegate.columns_count(cx)).find(|ix| delegate.column(*ix, cx).key == key).unwrap();
        let sort = match delegate.column(col, cx).sort.unwrap() {
            ColumnSort::Default => ColumnSort::Descending,
            ColumnSort::Descending => ColumnSort::Ascending,
            ColumnSort::Ascending => ColumnSort::Default,
        };
        (app.table.clone(), col, sort)
    });
    // TableState::perform_sort is private. Use its delegate entry point with the same
    // direction cycle and notification, without calling AppRoot's selection repair.
    // Native icon hit testing and TableState's header arrows are verified on the desktop.
    in_window(cx, h, |window, cx| {
        table.update(cx, |table, cx| {
            table.delegate_mut().perform_sort(col, sort, window, cx);
            cx.notify();
        });
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_scan_refreshes_catalog_context_in_the_table_and_selected_metadata(cx: &mut TestAppContext) {
    let h = launch(cx);
    let id = "11111111-aaaa-4bbb-8ccc-000000000001";
    h.app.update(cx, |app, cx| {
        let ix = app.table.read(cx).delegate().ix.iter().position(|&i| app.table.read(cx).delegate().rows[i].id == id).unwrap();
        app.select_row(ix, cx);
        assert_eq!(app.selected.as_ref().unwrap().context_fmt, "1%");
    });
    let dir = h._dir.path().join("claude/cache/model-catalog");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("test-cc.json"), r#"{"catalog":{"config":{"models":[{"id":"claude-opus-5","runtime":{"max_input_tokens":100000}}]}}}"#).unwrap();
    h.app.update(cx, |app, _| app.store.request(crate::store::StoreRequest::Scan { quiet: false }));
    wait_until(cx, &h, "catalog context refresh", |app, _| app.catalog.window("claude-opus-5") == Some(100_000));
    h.app.read_with(cx, |app, cx| {
        let row = app.table.read(cx).delegate().rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!(row.context_fmt, "14%");
        assert_eq!(app.selected.as_ref().unwrap().id, id);
        assert_eq!(app.selected.as_ref().unwrap().context_detail, "14k / 100k · peak 40k");
        assert!((row.context_pct.unwrap() - 0.14305).abs() < 1e-6);
    });
}

#[gpui_kit::test]
fn header_sort_keeps_the_selected_session_in_all_three_directions(cx: &mut TestAppContext) {
    use gpui_kit::component::table::ColumnSort;
    let h = launch(cx);
    let id = h.app.update(cx, |app, cx| {
        app.selection_event_count = Some(0);
        app.select_row(0, cx);
        app.selected.as_ref().unwrap().id.clone()
    });
    for sort in [Some(ColumnSort::Descending), Some(ColumnSort::Ascending), None] {
        sort_like_header(cx, &h, "prompts");
        let position = h.app.read_with(cx, |app, cx| {
            let delegate = app.table.read(cx).delegate();
            assert_eq!(delegate.sort_state().map(|(_, dir)| dir), sort);
            assert_eq!(app.selected.as_ref().unwrap().id, id, "detail must keep its session");
            delegate.position_of(&id).unwrap()
        });
        assert_selection_settled(cx, &h, position);
    }
}

#[gpui_kit::test]
fn header_sort_on_an_empty_table_does_not_create_a_selection(cx: &mut TestAppContext) {
    let h = launch(cx);
    h.app.update(cx, |app, cx| {
        app.selection_event_count = Some(0);
        app.set_project(Some("no-such-project".into()), cx);
    });
    for _ in 0..3 {
        sort_like_header(cx, &h, "prompts");
        h.app.read_with(cx, |app, cx| {
            assert!(app.selected.is_none());
            assert!(app.saved.selected_id.is_none());
            assert_eq!(app.table.read(cx).selected_row(), None);
            assert_eq!(app.selection_event_count, Some(0));
        });
    }
}

#[gpui_kit::test]
fn matches_header_sort_keeps_the_selected_session_and_detail_hit(cx: &mut TestAppContext) {
    use gpui_kit::component::table::ColumnSort;
    let h = launch(cx);
    press(cx, &h, "cmd-f");
    type_text(cx, &h, "e");
    press(cx, &h, "enter");
    wait_until(cx, &h, "multiple search results", |app, cx| search_done(app, cx) && shown_rows(app, cx) > 1);
    wait_until(cx, &h, "selected result messages", |app, _| app.detail.hit_position().is_some());
    let (id, hit) = h.app.update(cx, |app, _| {
        app.selection_event_count = Some(0);
        (app.selected.as_ref().unwrap().id.clone(), app.detail.hit_position())
    });
    for sort in [Some(ColumnSort::Descending), Some(ColumnSort::Ascending), None] {
        sort_like_header(cx, &h, "matches");
        let position = h.app.read_with(cx, |app, cx| {
            let delegate = app.table.read(cx).delegate();
            assert_eq!(delegate.sort_state().map(|(_, dir)| dir), sort);
            assert_eq!(app.selected.as_ref().unwrap().id, id);
            assert_eq!(app.detail.hit_position(), hit, "sorting must not clear or reload the detail");
            delegate.position_of(&id).unwrap()
        });
        assert_selection_settled(cx, &h, position);
    }
}

fn assert_selection_settled(cx: &mut TestAppContext, h: &Harness, row: usize) {
    cx.run_until_parked();
    h.app.read_with(cx, |app, cx| {
        assert!(app.selection_event_count.unwrap() <= 32, "selection feedback loop: {:?} events", app.selection_event_count);
        let table = app.table.read(cx);
        assert_eq!(table.selected_row(), Some(row));
        assert_eq!(app.selected.as_ref().unwrap().id, table.delegate().row(row).id);
        assert_eq!(app.saved.selected_id.as_deref(), Some(table.delegate().row(row).id.as_ref()));
    });
}

#[gpui_kit::test]
fn multiple_selections_in_one_update_settle_on_the_last_row(cx: &mut TestAppContext) {
    let h = launch(cx);
    h.app.update(cx, |app, cx| {
        app.selection_event_count = Some(0);
        app.select_row(1, cx);
        app.select_row(0, cx);
    });
    assert_selection_settled(cx, &h, 0);
}

#[gpui_kit::test]
fn table_click_notifications_and_keyboard_navigation_still_select_rows(cx: &mut TestAppContext) {
    let h = launch(cx);
    h.app.update(cx, |app, cx| {
        app.selection_event_count = Some(0);
        // This is the same TableState entry point used by mouse clicks, including its events.
        app.table.update(cx, |table, cx| {
            table.set_selected_row(0, cx);
            table.set_selected_row(1, cx);
        });
    });
    assert_selection_settled(cx, &h, 1);
    press(cx, &h, "down");
    assert_selection_settled(cx, &h, 2);
}

#[gpui_kit::test]
fn filtering_after_a_selection_and_clearing_the_table_discard_old_events(cx: &mut TestAppContext) {
    let h = launch(cx);
    h.app.update(cx, |app, cx| {
        app.selection_event_count = Some(0);
        app.select_row(1, cx);
        app.set_project(Some("-Users-me-Code-beta".into()), cx);
        app.select_row(0, cx);
    });
    assert_selection_settled(cx, &h, 0);
    h.app.update(cx, |app, cx| {
        app.select_row(1, cx);
        app.set_project(Some("no-such-project".into()), cx);
    });
    cx.run_until_parked();
    h.app.read_with(cx, |app, cx| {
        assert!(app.selection_event_count.unwrap() <= 32);
        assert!(app.selected.is_none());
        assert_eq!(app.table.read(cx).selected_row(), None);
    });
}

#[gpui_kit::test]
fn autoplay_filter_sort_and_explicit_selection_settle_together(cx: &mut TestAppContext) {
    use gpui_kit::component::table::ColumnSort;
    use crate::model::{RangePreset, project_entries};

    let h = launch(cx);
    h.app.update(cx, |app, cx| app.select_row(2, cx));
    h.app.update(cx, |app, cx| {
        app.selection_event_count = Some(0);
        app.set_project(None, cx);
        app.set_range(RangePreset::Days30, cx);
        let delegate = app.table.read(cx).delegate();
        let busiest = project_entries((0..delegate.ix.len()).map(|i| delegate.row(i)))
            .into_iter().max_by_key(|p| p.count).expect("fixture has recent transcripts");
        app.set_project(Some(busiest.key), cx);
        app.table.update(cx, |table, cx| {
            table.delegate_mut().set_sort("cost", ColumnSort::Descending);
            table.refresh(cx);
        });
        app.select_row(0, cx);
    });
    assert_selection_settled(cx, &h, 0);
}

#[gpui_kit::test]
fn the_table_has_the_focus_after_launch_so_shortcuts_work_before_any_click(cx: &mut TestAppContext) {
    let h = launch(cx);
    assert_eq!(focus(cx, &h), (false, true), "a window without a focus drops every key binding and menu action");
    assert!(h.app.read_with(cx, |app, _| app.detail_visible));
    press(cx, &h, "cmd-\\");
    assert!(!h.app.read_with(cx, |app, _| app.detail_visible), "cmd-\\ hides the detail panel");
}

#[gpui_kit::test]
fn escape_in_the_search_box_drops_the_search_and_hands_the_focus_to_the_table(cx: &mut TestAppContext) {
    let h = launch(cx);
    let all = h.app.read_with(cx, shown_rows);

    press(cx, &h, "cmd-f");
    assert_eq!(focus(cx, &h), (true, false), "cmd-f puts the cursor into the search box");
    type_text(cx, &h, "parser");
    assert_eq!(search_box(cx, &h), "parser");
    cx.executor().advance_clock(SEARCH_DEBOUNCE + Duration::from_millis(10));
    wait_until(cx, &h, "the search for a typed query", search_done);
    h.app.read_with(cx, |app, cx| {
        assert_eq!(app.query, "parser");
        assert!(app.table.read(cx).delegate().search_active());
        assert_eq!(shown_rows(app, cx), 1, "only the session with hits is listed");
    });

    press(cx, &h, "escape");
    assert_eq!(search_box(cx, &h), "", "the box is emptied");
    assert_eq!(focus(cx, &h), (false, true), "the focus goes back to the table");
    h.app.read_with(cx, |app, cx| {
        assert!(!app.searching() && app.search_summary.is_none());
        assert!(!app.table.read(cx).delegate().search_active(), "the Matches column is gone");
        assert_eq!(shown_rows(app, cx), all, "every session is listed again");
    });
}

#[gpui_kit::test]
fn enter_searches_at_once_and_n_steps_through_the_hits_while_the_table_has_the_focus(cx: &mut TestAppContext) {
    let h = launch(cx);
    let hit = |app: &AppRoot, _: &App| app.detail.hit_position();

    press(cx, &h, "cmd-f");
    type_text(cx, &h, "parser");
    press(cx, &h, "enter");
    assert_eq!(focus(cx, &h), (false, true), "enter hands the focus to the table");
    // no clock moved: the search did not wait for the typing pause
    wait_until(cx, &h, "the search started by enter", search_done);
    // hits count by place (a prompt, a step, a reply, a summary): one in each of turns 1, 2 and 4
    wait_until(cx, &h, "the conversation of the session with hits", |app, cx| hit(app, cx) == Some((1, 3)));

    press(cx, &h, "n");
    assert_eq!(h.app.read_with(cx, hit), Some((2, 3)));
    press(cx, &h, "shift-n");
    assert_eq!(h.app.read_with(cx, hit), Some((1, 3)));
    press(cx, &h, "shift-n");
    assert_eq!(h.app.read_with(cx, hit), Some((3, 3)), "stepping wraps around");
    press(cx, &h, "cmd-g");
    assert_eq!(h.app.read_with(cx, hit), Some((1, 3)));

    press(cx, &h, "cmd-f");
    type_text(cx, &h, "n");
    assert_eq!(search_box(cx, &h), "parsern", "in the search box n is a letter");
    assert_eq!(h.app.read_with(cx, hit), Some((1, 3)));
    press(cx, &h, "cmd-g");
    assert_eq!(h.app.read_with(cx, hit), Some((2, 3)), "cmd-g steps from anywhere");
}

#[gpui_kit::test]
fn rerunning_the_same_search_does_not_reread_the_conversation(cx: &mut TestAppContext) {
    let h = launch(cx);
    press(cx, &h, "cmd-f");
    type_text(cx, &h, "parser");
    press(cx, &h, "enter");
    wait_until(cx, &h, "the search started by enter", search_done);
    wait_until(cx, &h, "the conversation marked for it", |app, _| app.detail.hit_position().is_some());
    let (seq, id) = h.app.read_with(cx, |app, _| (app.conversation_seq, app.selected.as_ref().unwrap().id.clone()));

    // a filter change searches again, with the same query
    h.app.update(cx, |app, cx| app.toggle_empty_sessions(cx));
    wait_until(cx, &h, "the search after the filter change", search_done);
    h.app.read_with(cx, |app, _| {
        assert_eq!(app.selected.as_ref().unwrap().id, id);
        assert_eq!(app.conversation_seq, seq, "the same search marks the same places: nothing to read again");
        assert!(app.detail.hit_position().is_some());
    });

    in_window(cx, &h, |window, cx| h.app.update(cx, |app, cx| app.set_query("parse", window, cx)));
    assert_ne!(h.app.read_with(cx, |app, _| app.conversation_seq), seq, "another query reads the transcript again");
}

#[gpui_kit::test]
fn a_picked_custom_range_filters_the_table_and_hands_the_focus_back_to_it(cx: &mut TestAppContext) {
    use chrono::NaiveDate;
    use gpui_kit::component::calendar::Date;
    use gpui_kit::component::date_picker::DatePickerEvent;

    use crate::model::RangePreset;

    let h = launch(cx);
    h.app.update(cx, |app, cx| app.set_range(RangePreset::Custom, cx));
    // while the user clicks through the calendar, the picker has the focus
    let picker = h.app.read_with(cx, |app, _| app.date_picker.clone());
    in_window(cx, &h, |window, cx| picker.read(cx).focus_handle(cx).focus(window, cx));
    assert_eq!(focus(cx, &h), (false, false));

    // what the picker sends once both ends are picked; fixture transcripts carry checkout mtimes,
    // so the range picks a history-only session with a fixed date (S4, 2026-08-20)
    let day = |d: u32| NaiveDate::from_ymd_opt(2026, 8, d).unwrap();
    picker.update(cx, |_, cx| cx.emit(DatePickerEvent::Change(Date::Range(Some(day(1)), Some(day(31))))));
    cx.run_until_parked();
    h.app.read_with(cx, |app, cx| {
        assert_eq!((app.range, app.custom_range), (RangePreset::Custom, Some((day(1), day(31)))));
        let delegate = app.table.read(cx).delegate();
        let shown: Vec<&str> = (0..delegate.ix.len()).map(|i| delegate.row(i).short_id.as_ref()).collect();
        assert_eq!(shown, ["44444444"]);
    });
    assert_eq!(focus(cx, &h), (false, true), "the table has the focus again");
    press(cx, &h, "cmd-\\");
    assert!(!h.app.read_with(cx, |app, _| app.detail_visible), "so its shortcuts work right away");

    // half a range (the first click) changes nothing yet
    picker.update(cx, |_, cx| cx.emit(DatePickerEvent::Change(Date::Range(Some(day(5)), None))));
    cx.run_until_parked();
    assert_eq!(h.app.read_with(cx, |app, _| app.custom_range), Some((day(1), day(31))));
}

#[gpui_kit::test]
fn command_d_favorites_and_unfavorites_and_filter_composes(cx: &mut TestAppContext) {
    let h = launch(cx);
    h.app.update(cx, |app, cx| app.select_row(0, cx));
    press(cx, &h, "cmd-d");
    wait_until(cx, &h, "favorite", |app, _| app.selected.as_ref().is_some_and(|r| r.favorite));
    h.app.update(cx, |app, cx| app.toggle_favorites_only(cx));
    assert_eq!(h.app.read_with(cx, shown_rows), 1);
    press(cx, &h, "cmd-d");
    wait_until(cx, &h, "unfavorite", |app, cx| shown_rows(app, cx) == 0);
    assert!(!cx.has_pending_prompt());
}
#[gpui_kit::test]
fn changing_favorites_directory_moves_files_and_restores_focus(cx: &mut TestAppContext) {
    let h = launch(cx); h.app.update(cx, |app, cx| app.select_row(0, cx)); press(cx, &h, "cmd-d");
    wait_until(cx, &h, "favorite", |app, _| app.selected.as_ref().is_some_and(|r| r.favorite));
    in_window(cx, &h, |window, cx| h.app.update(cx, |app, cx| app.change_favorites_folder(window, cx)));
    let new = agent_session_core::config::canonical_root(&h._dir.path().join("new-favorites")).unwrap();
    cx.simulate_path_prompt_response(|_| Some(vec![new.clone()]));
    wait_until(cx, &h, "migration", |app, _| app.selected.as_ref().is_some_and(|r| r.transcript_path.as_ref().is_some_and(|p| p.starts_with(&new))));
    assert_eq!(focus(cx, &h), (false, true));
}

pub(crate) fn copy_fixture(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap(); let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() { copy_fixture(&entry.path(), &target); }
        else { std::fs::copy(entry.path(), target).unwrap(); }
    }
}

/// Tokens of the response `append_response` writes.
pub(crate) const APPENDED: agent_session_core::model::Tokens =
    agent_session_core::model::Tokens { input: 10, cache_write_5m: 0, cache_write_1h: 3_000, cache_read: 50_000, output: 1_200, web_search: 0 };

/// Append one assistant response of `model` at `ts_ms` (ms since epoch) to the transcript of
/// `session_id` under `claude/projects/-Users-me-Code-alpha`, as Claude Code would.
pub(crate) fn append_response(claude: &std::path::Path, session_id: &str, msg_id: &str, model: &str, ts_ms: i64) {
    use std::io::Write as _;
    let ts = chrono::DateTime::from_timestamp_millis(ts_ms).unwrap().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let t = APPENDED;
    let line = serde_json::json!({
        "parentUuid": null, "isSidechain": false, "userType": "external", "entrypoint": "cli", "cwd": "/Users/me/Code/alpha",
        "sessionId": session_id, "version": "2.1.260", "gitBranch": "main", "uuid": format!("appended-{msg_id}"), "timestamp": ts,
        "requestId": format!("req_{msg_id}"), "type": "assistant",
        "message": {"model": model, "id": msg_id, "type": "message", "role": "assistant", "content": [{"type": "text", "text": "Appended."}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": t.input, "cache_creation_input_tokens": t.cache_write_1h, "cache_read_input_tokens": t.cache_read, "output_tokens": t.output,
                "server_tool_use": {"web_search_requests": 0, "web_fetch_requests": 0}, "service_tier": "standard",
                "cache_creation": {"ephemeral_1h_input_tokens": t.cache_write_1h, "ephemeral_5m_input_tokens": 0}, "speed": "standard"}},
    });
    let path = claude.join("projects/-Users-me-Code-alpha").join(format!("{session_id}.jsonl"));
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(file, "{line}").unwrap();
}

#[gpui_kit::test]
fn last_copy_requires_confirmation_and_cancel_keeps_it(cx: &mut TestAppContext) {
    let h = launch(cx);
    let original = h.app.update(cx, |app, cx| { app.select_row(0, cx); app.selected.as_ref().unwrap().transcript_path.clone().unwrap() });
    press(cx, &h, "cmd-d");
    wait_until(cx, &h, "favorite", |app, _| app.selected.as_ref().is_some_and(|r| r.favorite));
    std::fs::remove_file(original).unwrap();
    press(cx, &h, "cmd-d"); cx.run_until_parked(); assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("取消"); cx.run_until_parked();
    assert!(h.app.read_with(cx, |app, _| app.selected.as_ref().unwrap().transcript_path.as_ref().unwrap().exists()));
    assert_eq!(focus(cx, &h), (false, true));
}

const S1: &str = "11111111-aaaa-4bbb-8ccc-000000000001";
const S4: &str = "44444444-aaaa-4bbb-8ccc-000000000004";

fn select(cx: &mut TestAppContext, h: &Harness, id: &str) {
    h.app.update(cx, |app, cx| {
        let row = app.table.read(cx).delegate().position_of(id).expect("the session is listed");
        app.select_row(row, cx);
    });
}

/// The test platform's clipboard: the system one is never touched.
fn clipboard(cx: &mut TestAppContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

#[gpui_kit::test]
fn command_c_copies_the_session_id_and_shift_command_c_the_resume_command(cx: &mut TestAppContext) {
    let h = launch(cx);
    select(cx, &h, S1);
    assert_eq!(focus(cx, &h), (false, true));
    press(cx, &h, "cmd-c");
    assert_eq!(clipboard(cx).as_deref(), Some(S1));
    press(cx, &h, "cmd-shift-c");
    // without a cd when the fixture's project folder is not on this machine
    let project = "/Users/me/Code/alpha";
    assert_eq!(clipboard(cx), crate::actions::resume_command(S1, project, std::path::Path::new(project).is_dir()));
    assert!(clipboard(cx).unwrap().ends_with(&format!("claude --resume {S1}")));
}

#[gpui_kit::test]
fn command_c_in_the_search_box_does_not_copy_the_session_id(cx: &mut TestAppContext) {
    let h = launch(cx);
    select(cx, &h, S1);
    press(cx, &h, "cmd-f");
    type_text(cx, &h, "parser");
    press(cx, &h, "cmd-a");
    press(cx, &h, "cmd-c");
    assert_eq!(clipboard(cx).as_deref(), Some("parser"), "the box copies its own text");
    press(cx, &h, "escape");
    press(cx, &h, "cmd-c");
    assert_eq!(clipboard(cx).as_deref(), Some(S1), "back in the table cmd-c copies the id again");
}

#[gpui_kit::test]
fn copying_the_resume_command_of_a_history_session_copies_nothing(cx: &mut TestAppContext) {
    let h = launch(cx);
    select(cx, &h, S4);
    press(cx, &h, "cmd-shift-c");
    assert_eq!(clipboard(cx), None, "the transcript is gone: there is nothing to resume");
    press(cx, &h, "cmd-c");
    assert_eq!(clipboard(cx).as_deref(), Some(S4), "its id can still be copied");
}

/// The row at the top of the conversation list.
fn top_row(app: &AppRoot, _: &App) -> Option<crate::conversation::Row> {
    app.detail.rows.get(app.detail.list.logical_scroll_top().item_ix).copied()
}

fn conversation_shown(app: &AppRoot, _: &App) -> bool {
    app.detail.load == crate::detail::Load::Ready && app.detail.has_conversation()
}

#[gpui_kit::test]
fn brackets_jump_between_turns_and_stop_at_the_ends(cx: &mut TestAppContext) {
    use crate::conversation::Row::Head;
    let h = launch(cx);
    select(cx, &h, S1);
    wait_until(cx, &h, "the conversation of S1", conversation_shown);
    // a short session fits the window, and the list pulls a scroll past its end back: a small one
    cx.simulate_window_resize(h.window, gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(300.)));
    h.app.update(cx, |app, _| app.detail.toggle_turn(1));
    let open = h.app.read_with(cx, |app, _| app.detail.open_turns.clone());
    assert_eq!(h.app.read_with(cx, top_row), Some(Head(0)));

    for expected in [Head(1), Head(2), Head(3), Head(3)] {
        press(cx, &h, "]");
        assert_eq!(h.app.read_with(cx, top_row), Some(expected));
    }
    for expected in [Head(2), Head(1), Head(0), Head(0)] {
        press(cx, &h, "[");
        assert_eq!(h.app.read_with(cx, top_row), Some(expected), "and back, to the first card");
    }
    h.app.read_with(cx, |app, _| assert_eq!(app.detail.open_turns, open, "jumping opens and closes nothing"));
}

#[gpui_kit::test]
fn brackets_do_nothing_on_the_metadata_tab(cx: &mut TestAppContext) {
    use crate::conversation::Row::Head;
    use crate::detail::DetailTab;
    let h = launch(cx);
    select(cx, &h, S1);
    wait_until(cx, &h, "the conversation of S1", conversation_shown);
    cx.simulate_window_resize(h.window, gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(300.)));
    h.app.update(cx, |app, cx| app.set_detail_tab(DetailTab::Metadata, cx));
    press(cx, &h, "]");
    press(cx, &h, "]");
    h.app.read_with(cx, |app, cx| {
        assert!(app.detail.tab == DetailTab::Metadata);
        assert_eq!(top_row(app, cx), Some(Head(0)), "the hidden conversation stays where it was");
    });
    h.app.update(cx, |app, cx| app.set_detail_tab(DetailTab::Conversation, cx));
    press(cx, &h, "]");
    assert_eq!(h.app.read_with(cx, top_row), Some(Head(1)), "back on Conversation the keys work");
}

#[gpui_kit::test]
fn a_history_session_shows_its_prompts_as_cards(cx: &mut TestAppContext) {
    use crate::conversation::{Card, Row};
    let h = launch(cx);
    select(cx, &h, S4);
    wait_until(cx, &h, "the prompts of S4", conversation_shown);
    h.app.read_with(cx, |app, _| {
        let view = app.detail.view.as_ref().unwrap();
        assert!(view.history);
        assert_eq!(view.summary, "4 prompts from history.jsonl");
        assert!(matches!(&view.cards[1], Card::Turn(t) if t.prompt_only && t.prompt.text.contains("line1\nline2\nline3")), "pasted text restored");
        assert_eq!(app.detail.rows, [Row::Head(0), Row::Head(1), Row::Head(2), Row::Head(3)]);
    });
    // (a search never lists a history-only session: search mode reads transcripts only)
    cx.simulate_window_resize(h.window, gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(300.)));
    press(cx, &h, "]");
    assert_eq!(h.app.read_with(cx, top_row), Some(Row::Head(1)), "] steps from prompt to prompt");
}

#[gpui_kit::test]
fn hiding_the_sorted_column_from_the_header_menu_keeps_the_selection_and_is_remembered(cx: &mut TestAppContext) {
    use crate::table::{ShowAllColumns, ToggleColumn};
    let h = launch(cx);
    let position = |cx: &mut TestAppContext| h.app.read_with(cx, |app, cx| app.table.read(cx).delegate().position_of(S4).unwrap());
    let by_default = position(cx);
    sort_like_header(cx, &h, "prompts");
    // otherwise the old row index would still point at the session and the test would prove nothing
    assert_ne!(position(cx), by_default, "S4 sits on another row when sorted by prompts");
    select(cx, &h, S4);
    // what a click on a menu item does: the action goes to the focused table and up to AppRoot
    in_window(cx, &h, |window, cx| window.dispatch_action(Box::new(ToggleColumn { key: "prompts".into() }), cx));
    cx.run_until_parked();
    let state = || crate::settings::ViewState::load(&h._dir.path().join("state.json"));
    h.app.read_with(cx, |app, cx| {
        let table = app.table.read(cx);
        assert_eq!(table.delegate().hidden_columns(), ["prompts"]);
        assert_eq!(table.delegate().sort_state(), None, "the order went back to the default");
        let selected = table.selected_row().map(|ix| table.delegate().row(ix).id.to_string());
        assert_eq!(selected.as_deref(), Some(S4), "the same session stays selected where it moved");
    });
    assert_eq!(state().hidden_columns, ["prompts"]);
    assert_eq!(state().sort, None);
    in_window(cx, &h, |window, cx| window.dispatch_action(Box::new(ShowAllColumns), cx));
    cx.run_until_parked();
    assert!(state().hidden_columns.is_empty());
}

// ---- today's usage and automatic scans ----------------------------------------------------

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn usage_loaded(app: &AppRoot, _: &App) -> bool {
    !app.today_label.is_empty()
}

/// What `append_response`'s call costs on `model`, as the status bar writes it.
fn today_of_one_appended(model: &str) -> String {
    use agent_session_core::pricing::{price, price_tokens};
    let cost = price_tokens(&price(model).expect("a model with a price"), &APPENDED, false, false).total();
    format!("Today {}", agent_session_core::format::fmt_usd(cost))
}

fn updated_at(app: &AppRoot, cx: &App, id: &str) -> i64 {
    app.table.read(cx).delegate().rows.iter().find(|r| r.id == id).expect("the session is listed").updated_at
}

#[gpui_kit::test]
fn the_status_bar_shows_todays_cost_and_follows_a_scan(cx: &mut TestAppContext) {
    let h = launch(cx);
    wait_until(cx, &h, "today's usage", usage_loaded);
    h.app.read_with(cx, |app, cx| {
        assert_eq!(app.today_label, "Today $0.00", "the fixture's calls were all made on other days");
        assert!(app.status_left(cx).ends_with(" · Today $0.00"), "{}", app.status_left(cx));
    });

    append_response(&h._dir.path().join("claude"), S1, "msg_today", "claude-opus-5", now_ms());
    h.app.update(cx, |app, cx| app.rescan(false, cx));
    let expected = today_of_one_appended("claude-opus-5");
    wait_until(cx, &h, "today's cost after the scan", |app, _| app.today_label != "Today $0.00");
    h.app.read_with(cx, |app, cx| {
        assert_eq!(app.today_label, expected.as_str(), "the price of the one call made today");
        assert!(app.status_left(cx).ends_with(&format!(" · {expected}")), "{}", app.status_left(cx));
    });

    // the filter does not change what today cost
    h.app.update(cx, |app, cx| app.set_project(Some("no-such-project".into()), cx));
    assert_eq!(h.app.read_with(cx, |app, _| app.today_label.clone()), expected.as_str());

    // a model without a price costs nothing and is counted
    append_response(&h._dir.path().join("claude"), S1, "msg_unpriced", "claude-unknown-9", now_ms());
    h.app.update(cx, |app, cx| app.rescan(false, cx));
    wait_until(cx, &h, "the unpriced call", |app, _| app.today_label.ends_with("(1 unpriced)"));
    assert_eq!(h.app.read_with(cx, |app, _| app.today_label.clone()), format!("{expected} (1 unpriced)").as_str());
}

#[gpui_kit::test]
fn ten_minutes_later_an_appended_transcript_shows_up_without_any_input(cx: &mut TestAppContext) {
    let h = launch(cx);
    wait_until(cx, &h, "today's usage", usage_loaded);
    let before = h.app.read_with(cx, |app, cx| updated_at(app, cx, S1));
    let at = now_ms();
    append_response(&h._dir.path().join("claude"), S1, "msg_idle", "claude-opus-5", at);
    cx.executor().advance_clock(crate::AUTO_SCAN_MIN_GAP);
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(300));
    cx.run_until_parked();
    assert_eq!(h.app.read_with(cx, |app, cx| updated_at(app, cx, S1)), before, "nothing scans before the timer is due");

    cx.executor().advance_clock(crate::AUTO_SCAN_IDLE);
    wait_until(cx, &h, "S1 updated by the automatic scan", |app, cx| updated_at(app, cx, S1) != before);
    h.app.read_with(cx, |app, cx| {
        assert_eq!(updated_at(app, cx, S1) / 1000, at / 1000);
        assert_eq!(app.table.read(cx).delegate().row(0).id, S1, "the newest session sorts first");
        assert_eq!(app.today_label, today_of_one_appended("claude-opus-5"));
    });
}

#[gpui_kit::test]
fn scans_triggered_within_thirty_seconds_run_once(cx: &mut TestAppContext) {
    let h = launch(cx);
    let done = |cx: &mut TestAppContext| h.app.read_with(cx, |app, _| app.scans_done);
    let base = done(cx);
    assert!(!h.app.update(cx, |app, cx| app.auto_scan("test", cx)), "the start-up scan just ended");

    cx.executor().advance_clock(crate::AUTO_SCAN_MIN_GAP);
    let sent = h.app.update(cx, |app, cx| [app.auto_scan("test", cx), app.auto_scan("test", cx)]);
    assert_eq!(sent, [true, false], "the second waits for the first to end");
    wait_until(cx, &h, "the automatic scan", |app, _| !app.scan_in_flight);
    std::thread::sleep(Duration::from_millis(100));
    cx.run_until_parked();
    assert_eq!(done(cx), base + 1, "one request, one scan");

    cx.executor().advance_clock(crate::AUTO_SCAN_MIN_GAP - Duration::from_secs(1));
    assert!(!h.app.update(cx, |app, cx| app.auto_scan("test", cx)), "29 s after the last one ended");
    cx.executor().advance_clock(Duration::from_secs(1));
    assert!(h.app.update(cx, |app, cx| app.auto_scan("test", cx)), "30 s after");
    wait_until(cx, &h, "the next automatic scan", |app, _| app.scans_done == base + 2);
}

#[gpui_kit::test]
fn an_automatic_scan_that_finds_nothing_leaves_the_status_bar_alone(cx: &mut TestAppContext) {
    let h = launch(cx);
    h.app.update(cx, |app, _| app.index_status = IndexStatus::Idle("indexed 4 of 4 files in 0.10s".into()));
    cx.executor().advance_clock(crate::AUTO_SCAN_MIN_GAP);
    let base = h.app.update(cx, |app, cx| {
        assert!(app.auto_scan("test", cx));
        assert!(matches!(app.index_status, IndexStatus::Idle(_)), "no 'loading index…' for an automatic scan");
        app.scans_done
    });
    wait_until(cx, &h, "the automatic scan", |app, _| app.scans_done == base + 1);
    h.app.read_with(cx, |app, _| {
        assert!(matches!(&app.index_status, IndexStatus::Idle(s) if s == "indexed 4 of 4 files in 0.10s"), "the summary of the last scan stays");
    });
    // one the user asks for reports even when nothing changed
    h.app.update(cx, |app, cx| app.rescan(false, cx));
    wait_until(cx, &h, "the manual scan", |app, _| app.scans_done == base + 2);
    h.app.read_with(cx, |app, _| assert!(matches!(&app.index_status, IndexStatus::Idle(s) if s.starts_with("index up to date"))));
}

#[gpui_kit::test]
fn crossing_four_moves_today_and_the_range_filter(cx: &mut TestAppContext) {
    use agent_session_core::usage::day_start_ms;
    use chrono::{Local, NaiveDate};

    use crate::model::RangePreset;

    let h = launch(cx);
    wait_until(cx, &h, "today's usage", usage_loaded);
    append_response(&h._dir.path().join("claude"), S1, "msg_today", "claude-opus-5", now_ms());
    h.app.update(cx, |app, cx| {
        app.set_range(RangePreset::Today, cx);
        app.rescan(false, cx);
    });
    let paid = today_of_one_appended("claude-opus-5");
    wait_until(cx, &h, "today's cost", |app, _| app.today_label == paid.as_str());
    let today = h.app.read_with(cx, |app, _| app.usage_day);
    let tomorrow = today.succ_opt().unwrap();
    let at = |day: NaiveDate, h: u32, m: u32| day.and_hms_opt(h, m, 0).unwrap().and_local_timezone(Local).earliest().unwrap().timestamp_millis();
    let since = |app: &AppRoot, cx: &App| app.current_filter(cx).since_ms;

    h.app.update(cx, |app, cx| app.roll_day(at(tomorrow, 3, 59), cx));
    h.app.read_with(cx, |app, cx| {
        assert_eq!((app.usage_day, since(app, cx)), (today, Some(day_start_ms(today, &Local))), "3:59 is still today");
        assert_eq!(app.today_label, paid.as_str());
    });

    h.app.update(cx, |app, cx| app.roll_day(at(tomorrow, 4, 0), cx));
    h.app.read_with(cx, |app, cx| {
        assert_eq!(app.usage_day, tomorrow);
        assert_eq!(since(app, cx), Some(day_start_ms(tomorrow, &Local)), "Today starts at 4:00 of the new day");
        assert_eq!(app.today_label, "Today $0.00", "and counts from zero again");
        assert_eq!(app.range, RangePreset::Today);
    });
}

#[gpui_kit::test]
fn coming_back_to_the_window_scans_at_once(cx: &mut TestAppContext) {
    let h = launch(cx);
    wait_until(cx, &h, "today's usage", usage_loaded);
    let before = h.app.read_with(cx, |app, cx| updated_at(app, cx, S1));
    append_response(&h._dir.path().join("claude"), S1, "msg_back", "claude-opus-5", now_ms());
    cx.executor().advance_clock(crate::AUTO_SCAN_MIN_GAP);
    in_window(cx, &h, |window, _| window.activate_window());
    wait_until(cx, &h, "S1 updated by the scan on activation", |app, cx| updated_at(app, cx, S1) != before);

    // the window loses the focus to another one: nothing scans; it comes back: a scan
    cx.executor().advance_clock(crate::AUTO_SCAN_MIN_GAP);
    let base = h.app.read_with(cx, |app, _| app.scans_done);
    let other = cx.add_window(|_, _| gpui_kit::EmptyView);
    other.update(cx, |_, window, _| window.activate_window()).unwrap();
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(100));
    cx.run_until_parked();
    assert_eq!(h.app.read_with(cx, |app, _| (app.scans_done, app.scan_in_flight)), (base, false), "losing the focus scans nothing");
    in_window(cx, &h, |window, _| window.activate_window());
    wait_until(cx, &h, "the scan on coming back", |app, _| app.scans_done == base + 1);
}

#[gpui_kit::test]
fn an_automatic_scan_does_not_scroll_the_table_back_to_the_selected_row(cx: &mut TestAppContext) {
    let h = launch(cx);
    // a few rows tall: the table scrolls
    cx.simulate_window_resize(h.window, gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(260.)));
    let (last, mover) = h.app.update(cx, |app, cx| {
        app.select_row(0, cx);
        let delegate = app.table.read(cx).delegate();
        let last = delegate.ix.len() - 1;
        // the transcript listed lowest: once written to, it moves to the top and row 0 moves down
        let mover = [S1, "22222222-aaaa-4bbb-8ccc-000000000002", "55555555-aaaa-4bbb-8ccc-000000000005"]
            .into_iter()
            .max_by_key(|id| delegate.position_of(id))
            .unwrap();
        (last, mover)
    });
    let selected = h.app.read_with(cx, |app, _| app.selected.as_ref().unwrap().id.clone());
    assert_ne!(selected, mover);
    h.app.update(cx, |app, cx| app.table.update(cx, |table, cx| table.scroll_to_row(last, cx)));
    cx.run_until_parked();
    let offset = |cx: &mut TestAppContext| h.app.read_with(cx, |app, cx| app.table.read(cx).vertical_scroll_handle.0.borrow().base_handle.offset());
    let scrolled = offset(cx);
    assert!(scrolled.y < gpui_kit::px(0.), "the table scrolled down, away from the selected first row: {scrolled:?}");

    append_response(&h._dir.path().join("claude"), mover, "msg_mover", "claude-opus-5", now_ms());
    cx.executor().advance_clock(crate::AUTO_SCAN_MIN_GAP);
    h.app.update(cx, |app, cx| assert!(app.auto_scan("test", cx)));
    wait_until(cx, &h, "the written session on top", |app, cx| app.table.read(cx).delegate().row(0).id == mover);
    cx.run_until_parked();
    h.app.read_with(cx, |app, cx| {
        let table = app.table.read(cx);
        assert_eq!(app.selected.as_ref().unwrap().id, selected, "the same session stays selected");
        assert_eq!(table.selected_row(), table.delegate().position_of(&selected), "on its new row");
    });
    assert_eq!(offset(cx), scrolled, "the table stays where the user scrolled to");
}

#[gpui_kit::test]
fn an_automatic_scan_that_finds_the_index_busy_changes_nothing_on_screen(cx: &mut TestAppContext) {
    let h = launch(cx);
    // asm scanning the same index, with something for this scan to write
    let writer = agent_session_core::index::Db::open(&h._dir.path().join("index.sqlite")).unwrap();
    append_response(&h._dir.path().join("claude"), S1, "msg_busy", "claude-opus-5", now_ms());
    writer.conn().execute_batch("BEGIN IMMEDIATE").unwrap();
    cx.executor().advance_clock(crate::AUTO_SCAN_MIN_GAP);
    let before = h.app.update(cx, |app, cx| {
        app.index_status = IndexStatus::Idle("previous summary".into());
        assert!(app.auto_scan("test", cx));
        updated_at(app, cx, S1)
    });
    let unchanged = |app: &AppRoot, cx: &App| {
        assert!(matches!(&app.index_status, IndexStatus::Idle(s) if s == "previous summary"), "status: {:?}", app.index_status);
        assert_eq!(updated_at(app, cx, S1), before, "nothing of the skipped scan reaches the table");
    };
    for _ in 0..800 {
        cx.run_until_parked();
        if h.app.read_with(cx, |app, cx| {
            unchanged(app, cx);
            !app.scan_in_flight
        }) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    writer.conn().execute_batch("ROLLBACK").unwrap();
    h.app.read_with(cx, |app, cx| {
        assert!(!app.scan_in_flight, "the scan gave up on the lock");
        unchanged(app, cx);
    });
    // the next one writes it
    cx.executor().advance_clock(crate::AUTO_SCAN_MIN_GAP);
    h.app.update(cx, |app, cx| assert!(app.auto_scan("test", cx)));
    wait_until(cx, &h, "S1 written by the next scan", |app, cx| updated_at(app, cx, S1) != before);
}

#[gpui_kit::test]
fn an_automatic_scan_does_not_scroll_search_results_back_to_the_top(cx: &mut TestAppContext) {
    let h = launch(cx);
    // enough matching sessions for the results to scroll
    let project = h._dir.path().join("claude/projects/-Users-me-Code-alpha");
    let template = std::fs::read_to_string(project.join(format!("{S1}.jsonl"))).unwrap();
    for n in 10..22 {
        let id = format!("aaaaaaaa-aaaa-4bbb-8ccc-{n:012}");
        std::fs::write(project.join(format!("{id}.jsonl")), template.replace(S1, &id)).unwrap();
    }
    let base = h.app.update(cx, |app, cx| {
        app.rescan(false, cx);
        app.scans_done
    });
    wait_until(cx, &h, "the extra sessions", |app, cx| app.scans_done > base && shown_rows(app, cx) > 12);

    press(cx, &h, "cmd-f");
    type_text(cx, &h, "e");
    // The typed query is searched once its debounce timer is due, and that timer starts only when
    // the executor first runs the task. Left pending, it would fire when the test moves the clock
    // for the automatic scan, and a new search scrolls to the top on its own.
    cx.run_until_parked();
    cx.executor().advance_clock(SEARCH_DEBOUNCE);
    wait_until(cx, &h, "search results", |app, cx| search_done(app, cx) && shown_rows(app, cx) > 12);
    cx.simulate_window_resize(h.window, gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(220.)));
    let selected = h.app.update(cx, |app, cx| {
        app.select_row(0, cx);
        let last = app.table.read(cx).delegate().ix.len() - 1;
        app.table.update(cx, |table, cx| table.scroll_to_row(last, cx));
        app.selected.as_ref().unwrap().id.clone()
    });
    cx.run_until_parked();
    let offset = |cx: &mut TestAppContext| h.app.read_with(cx, |app, cx| app.table.read(cx).vertical_scroll_handle.0.borrow().base_handle.offset());
    let before = offset(cx);
    assert!(before.y < gpui_kit::px(0.), "the results scrolled down: {before:?}");

    let (seq, summary) = h.app.read_with(cx, |app, _| (app.search_seq, app.search_summary.clone().unwrap()));
    append_response(&h._dir.path().join("claude"), S1, "msg_search", "claude-opus-5", now_ms());
    cx.executor().advance_clock(crate::AUTO_SCAN_MIN_GAP);
    cx.run_until_parked();
    assert_eq!(offset(cx), before, "nothing pending moves the results before the scan");
    h.app.update(cx, |app, cx| assert!(app.auto_scan("test", cx)));
    // the scan, its reload and the same search run again, all of it on screen
    wait_until(cx, &h, "the scan and the repeated search", |app, cx| {
        !app.scan_in_flight && app.search_seq > seq && !app.search_pending && search_done(app, cx)
    });
    cx.run_until_parked();
    h.app.read_with(cx, |app, cx| {
        let table = app.table.read(cx);
        assert_eq!(table.delegate().row(0).id, S1, "the written session sorts first in the new results");
        assert_eq!(app.search_summary.as_ref().unwrap().matches, summary.matches + 1, "the new results are on screen");
        assert_eq!(app.selected.as_ref().unwrap().id, selected, "the same session stays selected");
        assert_eq!(table.selected_row(), table.delegate().position_of(&selected), "on its row");
    });
    assert_eq!(offset(cx), before, "the results stay where the user scrolled to");
}

#[gpui_kit::test]
fn a_selected_session_gone_from_refreshed_results_hands_the_selection_to_the_first_row(cx: &mut TestAppContext) {
    let h = launch(cx);
    press(cx, &h, "cmd-f");
    type_text(cx, &h, "e");
    cx.run_until_parked();
    cx.executor().advance_clock(SEARCH_DEBOUNCE);
    wait_until(cx, &h, "search results", |app, cx| search_done(app, cx) && shown_rows(app, cx) > 1);
    // the second result's transcript goes away: the next search cannot find it
    let (gone, path) = h.app.update(cx, |app, cx| {
        app.select_row(1, cx);
        let row = app.selected.clone().unwrap();
        (row.id.clone(), row.transcript_path.clone().unwrap())
    });
    let removed = h._dir.path().join("removed");
    std::fs::create_dir_all(&removed).unwrap();
    std::fs::rename(&path, removed.join(path.file_name().unwrap())).unwrap();

    let seq = h.app.read_with(cx, |app, _| app.search_seq);
    cx.executor().advance_clock(crate::AUTO_SCAN_MIN_GAP);
    h.app.update(cx, |app, cx| assert!(app.auto_scan("test", cx)));
    wait_until(cx, &h, "the scan and the repeated search", |app, cx| {
        !app.scan_in_flight && app.search_seq > seq && !app.search_pending && search_done(app, cx)
    });
    h.app.read_with(cx, |app, cx| {
        let table = app.table.read(cx);
        assert_eq!(table.delegate().position_of(&gone), None, "no longer found");
        assert_eq!(table.selected_row(), Some(0));
        assert_eq!(app.selected.as_ref().unwrap().id, table.delegate().row(0).id, "the highlight and the detail panel agree");
    });
}

// ---- closing the window into the menu bar ---------------------------------------------------

/// The app as on every launch after the first close: the menu bar notice was shown before.
fn launch_noticed(cx: &mut TestAppContext) -> Harness {
    launch_with(cx, |dir| crate::settings::ViewState { resident_notice_shown: true, ..Default::default() }.save(&dir.join("state.json")).unwrap())
}

/// What a click on the red button does: AppKit asks the window whether it may close. True when
/// it was closed.
fn close_like_the_red_button(cx: &mut TestAppContext, h: &Harness) -> bool {
    let closed = gpui_kit::VisualTestContext::from_window(h.window, cx).simulate_close();
    cx.run_until_parked();
    closed
}

fn hidden(cx: &mut TestAppContext, h: &Harness) -> bool {
    h.app.read_with(cx, |app, _| app.window_hidden)
}

fn menu_command(cx: &mut TestAppContext, h: &Harness, command: crate::resident::ResidentCommand) {
    let app = h.app.clone();
    in_window(cx, h, |window, cx| app.update(cx, |app, cx| app.on_resident_command(command, window, cx)));
    cx.run_until_parked();
}

fn window_active(cx: &mut TestAppContext, h: &Harness) -> bool {
    in_window(cx, h, |window, _| window.is_window_active())
}

fn saved_state(h: &Harness) -> crate::settings::ViewState {
    crate::settings::ViewState::load(&h._dir.path().join("state.json"))
}

#[gpui_kit::test]
fn closing_the_window_hides_it_and_keeps_the_app_running(cx: &mut TestAppContext) {
    let h = launch_noticed(cx);
    wait_until(cx, &h, "today's usage", usage_loaded);
    assert!(!close_like_the_red_button(cx, &h), "the window is not closed, only hidden");
    assert!(hidden(cx, &h));
    assert!(!cx.has_pending_prompt(), "the notice was shown on an earlier run");

    // the app, its store thread and its timer are all still there
    let before = h.app.read_with(cx, |app, cx| updated_at(app, cx, S1));
    append_response(&h._dir.path().join("claude"), S1, "msg_hidden", "claude-opus-5", now_ms());
    cx.executor().advance_clock(crate::AUTO_SCAN_IDLE);
    wait_until(cx, &h, "S1 updated by the timer while hidden", |app, cx| updated_at(app, cx, S1) != before);
    let paid = today_of_one_appended("claude-opus-5");
    h.app.read_with(cx, |app, _| {
        assert_eq!(app.today_label, paid.as_str());
        let menu = app.resident_view.as_ref().expect("the menu follows today");
        assert_eq!(menu.today, paid.replace("Today ", "Today  "));
        assert_eq!(menu.title, "$0");
        assert!(app.window_hidden, "a scan does not bring the window back");
    });
}

#[gpui_kit::test]
fn command_w_closes_like_the_red_button(cx: &mut TestAppContext) {
    use crate::resident::ResidentCommand::OpenSessions;
    let h = launch_noticed(cx);
    press(cx, &h, "cmd-w");
    assert!(hidden(cx, &h));
    menu_command(cx, &h, OpenSessions);
    assert!(!hidden(cx, &h));
    // from the search box too
    press(cx, &h, "cmd-f");
    assert_eq!(focus(cx, &h), (true, false));
    press(cx, &h, "cmd-w");
    assert!(hidden(cx, &h));
}

#[gpui_kit::test]
fn reopening_restores_the_view_and_gives_the_table_the_focus(cx: &mut TestAppContext) {
    use crate::model::RangePreset;
    use crate::resident::ResidentCommand::OpenSessions;
    let h = launch_noticed(cx);
    let hit = |app: &AppRoot, _: &App| app.detail.hit_position();
    h.app.update(cx, |app, cx| app.set_range(RangePreset::Days30, cx));
    sort_like_header(cx, &h, "prompts");
    press(cx, &h, "cmd-f");
    type_text(cx, &h, "parser");
    press(cx, &h, "enter");
    wait_until(cx, &h, "the search", search_done);
    wait_until(cx, &h, "the first hit", |app, cx| hit(app, cx) == Some((1, 3)));
    let view = |cx: &mut TestAppContext| h.app.read_with(cx, |app, cx| (app.view_state(cx), app.query.clone(), app.selected.as_ref().map(|r| r.id.clone())));
    let before = view(cx);
    // closed while typing in the search box
    press(cx, &h, "cmd-f");
    press(cx, &h, "cmd-w");
    assert!(hidden(cx, &h));

    menu_command(cx, &h, OpenSessions);
    assert!(!hidden(cx, &h));
    assert!(window_active(cx, &h), "the window is in front");
    assert_eq!(view(cx), before, "range, sort, selection and query as they were");
    assert_eq!(search_box(cx, &h), "parser");
    assert_eq!(focus(cx, &h), (false, true), "the table has the focus");
    press(cx, &h, "n");
    assert_eq!(h.app.read_with(cx, hit), Some((2, 3)), "its keys work without a click");
}

#[gpui_kit::test]
fn the_first_close_asks_once(cx: &mut TestAppContext) {
    use crate::resident::ResidentCommand::OpenSessions;
    let h = launch(cx);
    assert!(!close_like_the_red_button(cx, &h));
    assert!(cx.has_pending_prompt(), "the first close explains the menu bar item");
    assert!(!hidden(cx, &h), "the window waits for the answer");
    // a second close while the notice is up changes nothing
    press(cx, &h, "cmd-w");
    assert!(!hidden(cx, &h));

    cx.simulate_prompt_answer("好");
    cx.run_until_parked();
    assert!(hidden(cx, &h));
    assert!(saved_state(&h).resident_notice_shown, "remembered for the next launch");

    menu_command(cx, &h, OpenSessions);
    assert!(!close_like_the_red_button(cx, &h));
    assert!(!cx.has_pending_prompt(), "only once");
    assert!(hidden(cx, &h));
    assert!(h.app.read_with(cx, |app, _| !app.quitting));
}

#[gpui_kit::test]
fn quit_now_in_the_first_close_notice_quits(cx: &mut TestAppContext) {
    let h = launch(cx);
    press(cx, &h, "cmd-w");
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("立即退出");
    cx.run_until_parked();
    // the test platform's quit does nothing: what the app asked for is all there is to see
    assert!(h.app.read_with(cx, |app, _| app.quitting));
    assert!(saved_state(&h).resident_notice_shown, "the notice was read");
}

#[gpui_kit::test]
fn the_menu_commands_come_over_a_channel(cx: &mut TestAppContext) {
    use crate::resident::ResidentCommand::{OpenSessions, ScanNow};
    let h = launch_noticed(cx);
    // as the menu bar item's target (and Finder's reopen) send them
    let (menu, commands) = async_channel::unbounded();
    let app = h.app.clone();
    in_window(cx, &h, |window, cx| app.update(cx, |app, cx| app.listen_to_menu(commands, window, cx)));
    press(cx, &h, "cmd-w");
    menu.try_send(OpenSessions).unwrap();
    cx.run_until_parked();
    assert!(!hidden(cx, &h));
    let base = h.app.read_with(cx, |app, _| app.scans_done);
    cx.executor().advance_clock(crate::AUTO_SCAN_MIN_GAP);
    menu.try_send(ScanNow).unwrap();
    wait_until(cx, &h, "the scan from the menu", |app, _| app.scans_done > base);
}

#[gpui_kit::test]
fn scans_fail_at_once_when_the_index_never_opened(cx: &mut TestAppContext) {
    use crate::resident::ResidentCommand::ScanNow;
    // a file where the index's directory should be: the store thread reports it and stops
    let h = start(
        cx,
        |dir| {
            std::fs::write(dir.join("not-a-dir"), "x").unwrap();
            crate::settings::ViewState { resident_notice_shown: true, ..Default::default() }.save(&dir.join("state.json")).unwrap();
        },
        |dir| dir.join("not-a-dir").join("index.sqlite"),
    );
    let failed = |app: &AppRoot| matches!(&app.index_status, IndexStatus::Failed(m) if m.starts_with("open index"));
    wait_until(cx, &h, "the open failure", |app, _| failed(app) && !app.scan_in_flight);
    let menu = |cx: &mut TestAppContext| h.app.read_with(cx, |app, _| app.resident_view.clone().expect("the menu follows the scans"));
    assert!(menu(cx).scanned.starts_with("Scan failed "), "{:?}", menu(cx));

    // Scan Now, with the window hidden
    press(cx, &h, "cmd-w");
    menu_command(cx, &h, ScanNow);
    h.app.read_with(cx, |app, _| {
        assert!(!app.scan_in_flight, "no scan is running: nobody would ever end it");
        assert!(failed(app), "the status bar keeps saying why: {:?}", app.index_status);
    });
    let after = menu(cx);
    assert!(!after.scanning && after.scanned.starts_with("Scan failed "), "not Scanning… for ever: {after:?}");

    // the idle timer's scan, too
    cx.executor().advance_clock(crate::AUTO_SCAN_IDLE);
    cx.run_until_parked();
    h.app.read_with(cx, |app, _| assert!(!app.scan_in_flight));
    assert!(menu(cx).scanned.starts_with("Scan failed "));
}

#[gpui_kit::test]
fn scan_now_from_the_menu_scans_while_hidden(cx: &mut TestAppContext) {
    use crate::resident::ResidentCommand::ScanNow;
    let h = launch_noticed(cx);
    wait_until(cx, &h, "today's usage", usage_loaded);
    press(cx, &h, "cmd-w");
    assert!(hidden(cx, &h));
    h.app.read_with(cx, |app, _| {
        let menu = app.resident_view.as_ref().unwrap();
        assert!(!menu.scanning && menu.scanned.starts_with("Scanned "), "{menu:?}");
    });

    append_response(&h._dir.path().join("claude"), S1, "msg_scan_now", "claude-opus-5", now_ms());
    let base = h.app.read_with(cx, |app, _| app.scans_done);
    // the fixture scans in a few milliseconds: look in the same update
    let app = h.app.clone();
    in_window(cx, &h, |window, cx| {
        app.update(cx, |app, cx| {
            app.on_resident_command(ScanNow, window, cx);
            let menu = app.resident_view.as_ref().unwrap();
            assert!(menu.scanning, "Scan Now is greyed out while it scans");
            assert_eq!(menu.scanned, "Scanning…");
        })
    });
    wait_until(cx, &h, "the scan from the menu", |app, _| app.scans_done > base);
    let paid = today_of_one_appended("claude-opus-5");
    h.app.read_with(cx, |app, _| {
        let menu = app.resident_view.as_ref().unwrap();
        assert!(!menu.scanning && menu.scanned.starts_with("Scanned "), "{menu:?}");
        assert_eq!(menu.today, paid.replace("Today ", "Today  "), "the new call is counted");
        assert!(app.window_hidden);
    });
}

// ---- the Usage view ---------------------------------------------------------------------------

/// (the Usage view has the focus, the table has the focus)
fn view_focus(cx: &mut TestAppContext, h: &Harness) -> (bool, bool) {
    let app = h.app.clone();
    in_window(cx, h, |window, cx| {
        let app = app.read(cx);
        (app.usage_view.focus.is_focused(window), app.table.read(cx).focus_handle(cx).is_focused(window))
    })
}

fn view(cx: &mut TestAppContext, h: &Harness) -> crate::model::MainView {
    h.app.read_with(cx, |app, _| app.view)
}

/// The fixture's calls were all made in September 2026.
fn september() -> (chrono::NaiveDate, chrono::NaiveDate) {
    (chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(), chrono::NaiveDate::from_ymd_opt(2026, 9, 30).unwrap())
}

#[gpui_kit::test]
fn control_2_shows_usage_and_control_1_goes_back_with_the_focus_following(cx: &mut TestAppContext) {
    use crate::model::MainView;
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    press(cx, &h, "ctrl-2");
    assert_eq!(view(cx, &h), MainView::Usage);
    assert_eq!(view_focus(cx, &h), (true, false), "the keys go to the Usage view");
    assert_eq!(saved_state(&h).view, MainView::Usage, "remembered for the next launch");
    h.app.read_with(cx, |app, _| {
        let cards: Vec<&str> = app.usage_view.snapshot.cards.iter().map(|c| c.label.as_ref()).collect();
        assert_eq!(cards, ["Cost", "In", "Out", "Calls", "Sessions"], "the view was built on the way in");
    });

    press(cx, &h, "ctrl-1");
    assert_eq!(view(cx, &h), MainView::Sessions);
    assert_eq!(view_focus(cx, &h), (false, true), "the table has the focus again");
    let selected = h.app.read_with(cx, |app, cx| app.table.read(cx).selected_row());
    press(cx, &h, "down");
    assert_ne!(h.app.read_with(cx, |app, cx| app.table.read(cx).selected_row()), selected, "its keys work without a click");
    assert_eq!(saved_state(&h).view, MainView::Sessions);

    // the View menu's items are the same actions
    in_window(cx, &h, |window, cx| window.dispatch_action(Box::new(crate::ShowUsage), cx));
    cx.run_until_parked();
    assert_eq!((view(cx, &h), view_focus(cx, &h)), (MainView::Usage, (true, false)));
}

#[gpui_kit::test]
fn the_view_shortcuts_work_from_the_search_box(cx: &mut TestAppContext) {
    use crate::model::MainView;
    let h = launch(cx);
    press(cx, &h, "cmd-f");
    type_text(cx, &h, "parser");
    press(cx, &h, "ctrl-2");
    assert_eq!(view(cx, &h), MainView::Usage);
    assert_eq!(view_focus(cx, &h), (true, false));
    assert_eq!(search_box(cx, &h), "parser", "typing in the box did not eat the shortcut, nor the shortcut the text");
}

#[gpui_kit::test]
fn command_1_to_4_still_switch_the_preview_mode(cx: &mut TestAppContext) {
    use crate::model::{DisplayMode, MainView};
    let h = launch(cx);
    let mode = |cx: &mut TestAppContext| h.app.read_with(cx, |app, cx| app.table.read(cx).delegate().mode);
    for (key, expected) in [("cmd-1", DisplayMode::Title), ("cmd-3", DisplayMode::LastPrompt), ("cmd-4", DisplayMode::LastMessage), ("cmd-2", DisplayMode::FirstPrompt)] {
        press(cx, &h, key);
        assert_eq!(mode(cx), expected, "{key}");
    }
    assert_eq!(view(cx, &h), MainView::Sessions, "⌘1–⌘4 are not the view shortcuts");
}

#[gpui_kit::test]
fn command_f_in_usage_goes_to_sessions_and_focuses_the_search(cx: &mut TestAppContext) {
    use crate::model::MainView;
    let h = launch(cx);
    press(cx, &h, "ctrl-2");
    press(cx, &h, "cmd-f");
    assert_eq!(view(cx, &h), MainView::Sessions);
    assert_eq!(focus(cx, &h), (true, false), "the cursor is in the search box");
    type_text(cx, &h, "parser");
    assert_eq!(search_box(cx, &h), "parser");
}

#[gpui_kit::test]
fn session_only_shortcuts_do_nothing_in_usage(cx: &mut TestAppContext) {
    use crate::actions::HideProject;
    use crate::{NextHit, ToggleEmptySessions, ToggleFavorite};
    let h = launch(cx);
    select(cx, &h, S1);
    let state = |cx: &mut TestAppContext| {
        h.app.read_with(cx, |app, cx| {
            let f = app.current_filter(cx);
            (app.table.read(cx).delegate().mode, app.detail_visible, f.hidden_projects.clone(), f.show_empty, app.selected.as_ref().map(|r| (r.id.clone(), r.favorite)))
        })
    };
    let before = state(cx);
    press(cx, &h, "ctrl-2");
    for key in ["cmd-d", "cmd-\\", "cmd-1", "cmd-4", "cmd-g", "alt-cmd-r"] {
        press(cx, &h, key);
    }
    // the menu items of the same actions
    for action in [Box::new(ToggleFavorite) as Box<dyn gpui_kit::Action>, Box::new(HideProject { id: None }), Box::new(ToggleEmptySessions), Box::new(NextHit)] {
        in_window(cx, &h, |window, cx| window.dispatch_action(action, cx));
    }
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(100));
    cx.run_until_parked();
    assert_eq!(state(cx), before, "preview mode, detail panel, hidden projects and favorites as they were");
    assert_eq!(h.app.read_with(cx, |app, _| app.search_opts.mode), crate::search::SearchMode::Text);
    assert_eq!(view_focus(cx, &h), (true, false));
}

#[gpui_kit::test]
fn escape_leaves_a_drilled_day(cx: &mut TestAppContext) {
    use agent_session_core::usage::UsageRange;
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    append_response(&h._dir.path().join("claude"), S1, "msg_today", "claude-opus-5", now_ms());
    h.app.update(cx, |app, cx| app.rescan(false, cx));
    wait_until(cx, &h, "today's call", |app, _| app.today.responses == 1);
    press(cx, &h, "ctrl-2");
    h.app.update(cx, |app, cx| app.set_usage_range(UsageRange::Days7, cx));
    let today = h.app.read_with(cx, |app, _| app.usage_day);
    let empty_day = today.pred_opt().unwrap();
    assert!(!h.app.update(cx, |app, cx| app.drill_into_day(empty_day, cx)), "a day without calls does not open");
    assert!(h.app.update(cx, |app, cx| app.drill_into_day(today, cx)));
    h.app.read_with(cx, |app, _| {
        assert_eq!((app.usage_view.range, app.usage_view.day), (UsageRange::Day, Some(today)));
        assert!(app.usage_view.snapshot.chart.hourly, "the day by the hour");
        assert_eq!(app.usage_view.snapshot.cards[3].value, "1");
    });
    assert_eq!(saved_state(&h).usage_range, UsageRange::Days7, "a clicked day is not saved: the range it came from is");

    press(cx, &h, "escape");
    h.app.read_with(cx, |app, _| {
        assert_eq!((app.usage_view.range, app.usage_view.day), (UsageRange::Days7, None), "back to the 7 days");
        assert!(!app.usage_view.snapshot.chart.hourly);
    });
    press(cx, &h, "escape");
    assert_eq!(h.app.read_with(cx, |app, _| app.usage_view.range), UsageRange::Days7, "nothing more to leave");
}

/// The chart on screen in the test window: where it was painted and its layout.
fn chart_on_screen(cx: &mut TestAppContext, h: &Harness) -> (gpui_kit::Bounds<gpui_kit::Pixels>, crate::usage_chart::Layout) {
    cx.run_until_parked();
    h.app.read_with(cx, |app, _| {
        let bounds = app.usage_view.chart_bounds.get();
        (bounds, crate::usage_chart::Layout::of(bounds, &app.usage_view.snapshot.chart))
    })
}

#[gpui_kit::test]
fn clicking_a_day_s_bar_opens_its_hours_and_other_clicks_do_nothing(cx: &mut TestAppContext) {
    use agent_session_core::usage::UsageRange;
    use gpui_kit::{Modifiers, VisualTestContext, point, px};
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    append_response(&h._dir.path().join("claude"), S1, "msg_today", "claude-opus-5", now_ms());
    h.app.update(cx, |app, cx| app.rescan(false, cx));
    wait_until(cx, &h, "today's call", |app, _| app.today.responses == 1);
    press(cx, &h, "ctrl-2");
    h.app.update(cx, |app, cx| app.set_usage_range(UsageRange::Days7, cx));
    let (bounds, l) = chart_on_screen(cx, &h);
    assert!(bounds.size.width > px(100.) && l.slots == 7, "the week was painted");
    let at = |x: f32, y: f32| point(bounds.origin.x + px(x), bounds.origin.y + px(y));
    let range = |cx: &mut TestAppContext| h.app.read_with(cx, |app, _| (app.usage_view.range, app.usage_view.day));

    VisualTestContext::from_window(h.window, cx).simulate_click(at(l.center(5), l.bottom() - 2.), Modifiers::none());
    assert_eq!(range(cx), (UsageRange::Days7, None), "yesterday has no calls: nothing opens");
    VisualTestContext::from_window(h.window, cx).simulate_click(at(l.left - 20., l.bottom() - 2.), Modifiers::none());
    assert_eq!(range(cx), (UsageRange::Days7, None), "over the value labels");

    // anywhere in today's column, its bar is short
    VisualTestContext::from_window(h.window, cx).simulate_click(at(l.center(6), l.top + 4.), Modifiers::none());
    let today = h.app.read_with(cx, |app, _| app.usage_day);
    assert_eq!(range(cx), (UsageRange::Day, Some(today)), "today by the hour");
    let (_, hours) = chart_on_screen(cx, &h);
    assert_eq!(hours.slots, 24);
    VisualTestContext::from_window(h.window, cx).simulate_click(at(hours.center(10), hours.bottom() - 2.), Modifiers::none());
    assert_eq!(range(cx), (UsageRange::Day, Some(today)), "an hour does not open anything");
    press(cx, &h, "escape");
    assert_eq!(range(cx), (UsageRange::Days7, None));
}

#[gpui_kit::test]
fn a_bar_shows_its_tooltip_under_the_mouse_or_when_pinned(cx: &mut TestAppContext) {
    use gpui_kit::{Modifiers, VisualTestContext, point, px};
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    press(cx, &h, "ctrl-2");
    let (first, last) = september();
    h.app.update(cx, |app, cx| app.set_usage_custom_range(first, last, cx));
    let (bounds, l) = chart_on_screen(cx, &h);
    let mut window = VisualTestContext::from_window(h.window, cx);
    let tooltip = |window: &mut VisualTestContext| {
        window.run_until_parked();
        window.debug_bounds(crate::usage_chart::TOOLTIP_SELECTOR)
    };
    assert_eq!(tooltip(&mut window), None, "no tooltip until asked");

    // autoplay's hover step: the mouse is elsewhere
    h.app.update(&mut window, |app, cx| {
        app.usage_view.hover = Some(3);
        cx.notify();
    });
    assert!(tooltip(&mut window).is_some(), "the pinned bar's tooltip");
    h.app.update(&mut window, |app, cx| {
        app.usage_view.hover = None;
        cx.notify();
    });
    assert_eq!(tooltip(&mut window), None);

    // the mouse over a bar, then off the chart
    window.simulate_mouse_move(point(bounds.origin.x + px(l.center(10)), bounds.origin.y + px(l.bottom() - 4.)), None, Modifiers::none());
    let shown = tooltip(&mut window).expect("the tooltip under the mouse");
    assert!(shown.size.width > px(150.));
    window.simulate_mouse_move(point(bounds.origin.x + px(l.center(10)), bounds.origin.y + bounds.size.height + px(80.)), None, Modifiers::none());
    assert_eq!(tooltip(&mut window), None, "gone with the mouse");
}

#[gpui_kit::test]
fn a_filter_set_in_usage_is_still_set_in_sessions(cx: &mut TestAppContext) {
    use agent_session_core::usage::UsageRange;
    use crate::model::{Kind, MainView};
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    press(cx, &h, "ctrl-2");
    let (first, last) = september();
    h.app.update(cx, |app, cx| app.set_usage_custom_range(first, last, cx));
    let all = h.app.read_with(cx, |app, _| app.usage_view.snapshot.totals);
    assert!(all.responses > 0, "the fixture's calls are in September");

    // hidden projects still cost money: Usage counts them
    h.app.update(cx, |app, cx| app.hide_project("-Users-me-Code-beta".into(), cx));
    assert_eq!(h.app.read_with(cx, |app, _| app.usage_view.snapshot.totals), all);

    h.app.update(cx, |app, cx| {
        app.show_project("-Users-me-Code-beta", cx);
        app.set_project(Some("-Users-me-Code-alpha".into()), cx);
        app.toggle_kind(Kind::Sdk, cx);
    });
    let alpha = h.app.read_with(cx, |app, _| {
        let snap = &app.usage_view.snapshot;
        assert!(snap.projects.iter().all(|p| p.key.as_deref() == Some("-Users-me-Code-alpha")));
        assert!(snap.projects[0].selected);
        assert_eq!(snap.cards[0].value, snap.projects[0].cost, "Cost is the picked project's cost");
        snap.totals
    });
    assert!(alpha.responses > 0 && alpha.responses < all.responses);
    assert_eq!(h.app.read_with(cx, |app, _| app.usage_view.range), UsageRange::Custom);

    press(cx, &h, "ctrl-1");
    assert_eq!(view(cx, &h), MainView::Sessions);
    h.app.read_with(cx, |app, cx| {
        let filter = app.current_filter(cx);
        assert_eq!(filter.project.as_deref(), Some("-Users-me-Code-alpha"));
        assert!(!filter.kinds.sdk);
        let delegate = app.table.read(cx).delegate();
        assert!((0..delegate.ix.len()).all(|i| delegate.row(i).project_key == "-Users-me-Code-alpha"), "the table shows the project picked in Usage");
    });
}

#[gpui_kit::test]
fn picking_a_custom_range_in_usage_hands_the_focus_back_to_usage(cx: &mut TestAppContext) {
    use agent_session_core::usage::UsageRange;
    use gpui_kit::component::calendar::Date;
    use gpui_kit::component::date_picker::DatePickerEvent;
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    press(cx, &h, "ctrl-2");
    h.app.update(cx, |app, cx| app.set_usage_range(UsageRange::Custom, cx));
    let picker = h.app.read_with(cx, |app, _| app.usage_view.date_picker.clone());
    in_window(cx, &h, |window, cx| picker.read(cx).focus_handle(cx).focus(window, cx));
    assert_eq!(view_focus(cx, &h), (false, false), "the picker has the focus while the user clicks through it");

    let (first, last) = september();
    picker.update(cx, |_, cx| cx.emit(DatePickerEvent::Change(Date::Range(Some(first), Some(last)))));
    cx.run_until_parked();
    h.app.read_with(cx, |app, _| {
        assert_eq!((app.usage_view.range, app.usage_view.custom), (UsageRange::Custom, Some((first, last))));
        assert!(app.usage_view.snapshot.totals.responses > 0);
        // the fixture's September is in 2026: other years write it out
        let label = if chrono::Datelike::year(&app.usage_day) == 2026 { "Sep 1 – Sep 30" } else { "Sep 1 – Sep 30, 2026" };
        assert_eq!(app.usage_view.snapshot.range_label, label);
    });
    assert_eq!(view_focus(cx, &h), (true, false), "the Usage view has the focus again");
    assert_eq!(saved_state(&h).usage_custom_range, Some((first, last)));
    press(cx, &h, "ctrl-1");
    assert_eq!(view(cx, &h), crate::model::MainView::Sessions, "so its shortcuts work right away");
    assert_eq!(h.app.read_with(cx, |app, _| app.range), crate::model::RangePreset::Any, "the table's own range is another one");
}

#[gpui_kit::test]
fn usage_scans_every_minute_and_sessions_every_ten(cx: &mut TestAppContext) {
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    let done = |cx: &mut TestAppContext| h.app.read_with(cx, |app, _| app.scans_done);
    let base = done(cx);
    cx.executor().advance_clock(crate::AUTO_SCAN_MIN_GAP);
    press(cx, &h, "ctrl-2");
    wait_until(cx, &h, "the scan on showing Usage", |app, _| app.scans_done == base + 1);

    let quiet = |cx: &mut TestAppContext| {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(150));
        cx.run_until_parked();
    };
    cx.executor().advance_clock(crate::USAGE_SCAN_IDLE - Duration::from_secs(1));
    quiet(cx);
    assert_eq!(done(cx), base + 1, "not before the minute is up");
    cx.executor().advance_clock(Duration::from_secs(1));
    wait_until(cx, &h, "the scan a minute later", |app, _| app.scans_done == base + 2);
    cx.executor().advance_clock(crate::USAGE_SCAN_IDLE);
    wait_until(cx, &h, "and the next minute", |app, _| app.scans_done == base + 3);

    press(cx, &h, "ctrl-1");
    cx.executor().advance_clock(crate::USAGE_SCAN_IDLE);
    quiet(cx);
    assert_eq!(done(cx), base + 3, "the session table scans every ten minutes");
    cx.executor().advance_clock(crate::AUTO_SCAN_IDLE - crate::USAGE_SCAN_IDLE);
    wait_until(cx, &h, "the scan ten minutes later", |app, _| app.scans_done == base + 4);
}

#[gpui_kit::test]
fn a_reload_in_usage_defers_the_search_until_sessions_is_shown(cx: &mut TestAppContext) {
    let h = launch(cx);
    press(cx, &h, "cmd-f");
    type_text(cx, &h, "parser");
    press(cx, &h, "enter");
    wait_until(cx, &h, "the search", search_done);
    let (seq, matches) = h.app.read_with(cx, |app, _| (app.search_seq, app.search_summary.as_ref().unwrap().matches));

    press(cx, &h, "ctrl-2");
    let base = h.app.read_with(cx, |app, _| app.scans_done);
    append_response(&h._dir.path().join("claude"), S1, "msg_search", "claude-opus-5", now_ms());
    h.app.update(cx, |app, cx| app.rescan(false, cx));
    wait_until(cx, &h, "the scan and its reload", |app, _| app.scans_done > base && !app.scan_in_flight);
    cx.run_until_parked();
    assert_eq!(h.app.read_with(cx, |app, _| app.search_seq), seq, "no search while Usage is shown");

    press(cx, &h, "ctrl-1");
    assert!(h.app.read_with(cx, |app, _| app.search_seq) > seq, "searched again on the way back");
    wait_until(cx, &h, "the repeated search", |app, cx| search_done(app, cx) && !app.search_pending);
    h.app.read_with(cx, |app, _| assert_eq!(app.search_summary.as_ref().unwrap().matches, matches, "the same hits (the appended reply says no 'parser')"));
}

#[gpui_kit::test]
fn the_view_its_range_and_the_chart_metric_come_back_after_a_restart(cx: &mut TestAppContext) {
    use agent_session_core::usage::{Metric, UsageRange};
    use crate::model::MainView;
    let (first, last) = september();
    let h = launch_with(cx, |dir| {
        crate::settings::ViewState {
            view: MainView::Usage,
            usage_range: UsageRange::Custom,
            usage_custom_range: Some((first, last)),
            usage_metric: Metric::HitRate,
            ..Default::default()
        }
        .save(&dir.join("state.json"))
        .unwrap()
    });
    wait_until(cx, &h, "the ledger in the Usage view", |app, _| app.usage_view.snapshot.totals.responses > 0);
    assert_eq!(view(cx, &h), MainView::Usage);
    assert_eq!(view_focus(cx, &h), (true, false), "the keys go to the Usage view from the start");
    h.app.read_with(cx, |app, _| {
        assert_eq!((app.usage_view.range, app.usage_view.custom, app.usage_view.metric), (UsageRange::Custom, Some((first, last)), Metric::HitRate));
        assert_eq!(app.usage_view.snapshot.chart.metric, Metric::HitRate);
    });
    h.app.update(cx, |app, cx| app.set_usage_metric(Metric::Out, cx));
    assert_eq!(saved_state(&h).usage_metric, Metric::Out);
}

#[gpui_kit::test]
fn todays_cost_in_the_status_bar_opens_todays_usage_and_matches_it(cx: &mut TestAppContext) {
    use agent_session_core::usage::UsageRange;
    use crate::model::MainView;
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    append_response(&h._dir.path().join("claude"), S1, "msg_today", "claude-opus-5", now_ms());
    h.app.update(cx, |app, cx| app.rescan(false, cx));
    let paid = today_of_one_appended("claude-opus-5");
    wait_until(cx, &h, "today's cost", |app, _| app.today_label == paid.as_str());
    h.app.update(cx, |app, cx| app.set_usage_range(UsageRange::Days30, cx));

    // what a click on `Today $…` does
    in_window(cx, &h, |window, cx| h.app.update(cx, |app, cx| app.show_todays_usage(window, cx)));
    assert_eq!(view(cx, &h), MainView::Usage);
    assert_eq!(view_focus(cx, &h), (true, false));
    h.app.read_with(cx, |app, cx| {
        assert_eq!(app.usage_view.range, UsageRange::Today);
        assert_eq!(format!("Today {}", app.usage_view.snapshot.cards[0].value), paid, "the Cost card is the status bar's Today");
        assert_eq!(app.status_left(cx), paid, "in Usage the status bar's left is only today");
    });
}

#[gpui_kit::test]
fn open_usage_in_the_menu_bar_menu_shows_the_window_on_usage(cx: &mut TestAppContext) {
    use crate::model::MainView;
    use crate::resident::ResidentCommand::{OpenSessions, OpenUsage, Reopen};
    let h = launch_noticed(cx);
    press(cx, &h, "cmd-w");
    assert!(hidden(cx, &h));
    menu_command(cx, &h, OpenUsage);
    assert!(!hidden(cx, &h));
    assert_eq!((view(cx, &h), view_focus(cx, &h)), (MainView::Usage, (true, false)));

    // opened again from Finder: the window as it was
    press(cx, &h, "cmd-w");
    menu_command(cx, &h, Reopen);
    assert_eq!((view(cx, &h), view_focus(cx, &h)), (MainView::Usage, (true, false)));

    press(cx, &h, "cmd-w");
    menu_command(cx, &h, OpenSessions);
    assert_eq!((view(cx, &h), view_focus(cx, &h)), (MainView::Sessions, (false, true)));
}

#[gpui_kit::test]
fn a_hidden_usage_view_scans_every_ten_minutes(cx: &mut TestAppContext) {
    let h = launch_noticed(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    press(cx, &h, "ctrl-2");
    press(cx, &h, "cmd-w");
    assert!(hidden(cx, &h));
    wait_until(cx, &h, "no scan under way", |app, _| !app.scan_in_flight);
    let base = h.app.read_with(cx, |app, _| app.scans_done);
    cx.executor().advance_clock(crate::USAGE_SCAN_IDLE);
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(150));
    cx.run_until_parked();
    assert_eq!(h.app.read_with(cx, |app, _| app.scans_done), base, "no scan a minute after the window went away");
    cx.executor().advance_clock(crate::AUTO_SCAN_IDLE - crate::USAGE_SCAN_IDLE);
    wait_until(cx, &h, "the scan ten minutes later", |app, _| app.scans_done == base + 1);
}

// ---- the Usage view, rework R1 ----------------------------------------------------------------

#[gpui_kit::test]
fn changing_the_favorites_folder_from_usage_gives_the_keys_back_to_usage(cx: &mut TestAppContext) {
    use crate::model::MainView;
    let h = launch(cx);
    press(cx, &h, "ctrl-2");
    // the Favorites menu works in either view
    in_window(cx, &h, |window, cx| window.dispatch_action(Box::new(crate::ChangeFavoritesFolder), cx));
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths(), "the folder dialog is up");
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    assert_eq!(view(cx, &h), MainView::Usage);
    assert_eq!(view_focus(cx, &h), (true, false), "back to Usage, not to the table it hides");
    press(cx, &h, "ctrl-1");
    assert_eq!((view(cx, &h), view_focus(cx, &h)), (MainView::Sessions, (false, true)), "so its keys work");
}

const S2: &str = "22222222-aaaa-4bbb-8ccc-000000000002";

/// Usage lists models without their date or `[1m]`; the session table knows its sessions by
/// the main model as written in the transcript (`claude-haiku-4-5-20251001`).
#[gpui_kit::test]
fn a_model_picked_in_usage_keeps_its_sessions_in_sessions(cx: &mut TestAppContext) {
    use crate::model::MainView;
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    press(cx, &h, "ctrl-2");
    let pick = h.app.read_with(cx, |app, _| app.usage_view.models.iter().map(|(m, _)| m.clone()).find(|m| m.starts_with("claude-haiku")).expect("the fixture's Haiku call"));
    assert_eq!(pick, "claude-haiku-4-5", "Usage lists it without its date");
    h.app.update(cx, |app, cx| app.set_model_filter(Some(pick.into()), cx));
    press(cx, &h, "ctrl-1");
    assert_eq!(view(cx, &h), MainView::Sessions);
    h.app.read_with(cx, |app, cx| {
        let delegate = app.table.read(cx).delegate();
        let ids: Vec<String> = (0..delegate.ix.len()).map(|i| delegate.row(i).id.to_string()).collect();
        assert_eq!(ids, [S2], "the session whose main model is Haiku 4.5");
        let haiku: Vec<&str> = app.models.iter().filter(|e| e.model.starts_with("claude-haiku")).map(|e| e.model.as_ref()).collect();
        assert_eq!(haiku, ["claude-haiku-4-5"], "one entry in the Sessions menu, the same as Usage's");
    });

    // a pick saved by an older version (the dated name) still finds it, and Usage still counts it
    h.app.update(cx, |app, cx| app.set_model_filter(Some("claude-haiku-4-5-20251001".into()), cx));
    assert_eq!(h.app.read_with(cx, |app, cx| app.current_filter(cx).model), Some("claude-haiku-4-5".into()), "kept without its date");
    assert_eq!(h.app.read_with(cx, shown_rows), 1);
    press(cx, &h, "ctrl-2");
    let (first, last) = september();
    h.app.update(cx, |app, cx| app.set_usage_custom_range(first, last, cx));
    let haiku = h.app.read_with(cx, |app, _| app.usage_view.snapshot.totals);
    assert_eq!((haiku.responses, haiku.unpriced_responses), (1, 0), "Usage still has its one call");
}

/// 900 px is narrower than the Usage toolbar: it wraps onto a second row instead of pushing
/// Model, `as of` and the refresh button out of the window (acceptance, 12-narrow.png).
#[gpui_kit::test]
fn the_usage_toolbar_wraps_in_a_narrow_window_instead_of_running_off_it(cx: &mut TestAppContext) {
    use agent_session_core::usage::UsageRange;
    use gpui_kit::{VisualTestContext, px, size};
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    press(cx, &h, "ctrl-2");
    h.app.update(cx, |app, cx| app.set_project(Some("-Users-me-Code-alpha".into()), cx));
    let mut window = VisualTestContext::from_window(h.window, cx);
    let check = |window: &mut VisualTestContext, what: &str, width: f32| {
        window.simulate_resize(size(px(width), px(800.)));
        window.run_until_parked();
        for selector in [crate::toolbar::USAGE_MODEL_SELECTOR, crate::toolbar::USAGE_END_SELECTOR] {
            let b = window.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} on screen"));
            assert!(b.origin.x >= px(0.) && b.right() <= px(width), "{what} at {width} px: {selector} at {b:?}");
        }
    };
    for width in [1400., 900., 700.] {
        check(&mut window, "Today", width);
    }
    // Custom's date picker takes room too
    h.app.update(&mut window, |app, cx| app.set_usage_range(UsageRange::Custom, cx));
    for width in [1400., 900., 700.] {
        check(&mut window, "Custom", width);
    }
}

/// The tooltip box stays in the plot area, under the chart's title row and legend, wherever the
/// mouse is on the bar (acceptance, 04-hover-bar.png: it covered the legend).
#[gpui_kit::test]
fn a_bar_s_tooltip_never_covers_the_chart_s_legend(cx: &mut TestAppContext) {
    use gpui_kit::{Modifiers, VisualTestContext, point, px};
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    press(cx, &h, "ctrl-2");
    let (first, last) = september();
    h.app.update(cx, |app, cx| app.set_usage_custom_range(first, last, cx));
    let (bounds, l) = chart_on_screen(cx, &h);
    // the busiest day has the tallest tooltip
    let busiest = h.app.read_with(cx, |app, _| {
        let slots = &app.usage_view.snapshot.chart.slots;
        (0..slots.len()).max_by_key(|ix| slots[*ix].totals.responses).unwrap()
    });
    let mut window = VisualTestContext::from_window(h.window, cx);
    let plot_top = bounds.origin.y + px(l.top);
    // just under the middle the box used to open upwards from the mouse, over the title row
    let middle = bounds.size.height.as_f32() / 2.;
    for (ix, y) in [(busiest, l.bottom() - 4.), (busiest, middle + 4.), (busiest, middle + 30.), (l.slots - 1, l.top + l.height * 0.7), (0, l.top + 2.)] {
        window.simulate_mouse_move(point(bounds.origin.x + px(l.center(ix)), bounds.origin.y + px(y)), None, Modifiers::none());
        window.run_until_parked();
        let tip = window.debug_bounds(crate::usage_chart::TOOLTIP_SELECTOR).expect("a tooltip");
        // the box starts at the plot area's top, its content a padding below
        assert!(tip.origin.y >= plot_top, "bar {ix}, mouse {y} px down: the tooltip starts at {:?}, above the plot area at {plot_top:?}", tip.origin.y);
    }
}

// ---- from Usage back to the session table (usage-drilldown) -----------------------------------

const S3: &str = "33333333-aaaa-4bbb-8ccc-000000000003";
const ALPHA: &str = "-Users-me-Code-alpha";
const BETA: &str = "-Users-me-Code-beta";
/// A session the ledger kept after its transcript was gone, with no row in the table.
const GONE: &str = "99999999-aaaa-4bbb-8ccc-000000000009";

/// The usage day (from 4:00 here) of a fixture time.
fn local_day(at: &str) -> chrono::NaiveDate {
    agent_session_core::usage::usage_day(chrono::DateTime::parse_from_rfc3339(at).unwrap().timestamp_millis(), &chrono::Local)
}

/// S1's first calls, S2's only one, S3's.
fn s1_day() -> chrono::NaiveDate {
    local_day("2026-09-01T08:00:05Z")
}
fn s2_day() -> chrono::NaiveDate {
    local_day("2026-09-05T12:00:02Z")
}
fn s3_day() -> chrono::NaiveDate {
    local_day("2026-09-10T02:00:03Z")
}

/// The Usage view on these days.
fn usage_on(cx: &mut TestAppContext, h: &Harness, (first, last): (chrono::NaiveDate, chrono::NaiveDate)) {
    press(cx, h, "ctrl-2");
    h.app.update(cx, |app, cx| app.set_usage_custom_range(first, last, cx));
}

fn notices(cx: &mut TestAppContext, h: &Harness) -> Vec<String> {
    h.app.read_with(cx, |app, _| app.notices.iter().map(|n| n.to_string()).collect())
}

/// The ids the table lists, sorted.
fn listed(app: &AppRoot, cx: &App) -> Vec<String> {
    let delegate = app.table.read(cx).delegate();
    let mut ids: Vec<String> = (0..delegate.ix.len()).map(|i| delegate.row(i).id.to_string()).collect();
    ids.sort();
    ids
}

/// What a click on a row of the most expensive sessions does.
fn reveal(cx: &mut TestAppContext, h: &Harness, id: &str) {
    let app = h.app.clone();
    in_window(cx, h, |window, cx| app.update(cx, |app, cx| app.reveal_session(id, window, cx)));
    cx.run_until_parked();
}

/// What a click on a project's `↗` (`Some`) or the Sessions card's does.
fn jump(cx: &mut TestAppContext, h: &Harness, project: Option<&str>) {
    let app = h.app.clone();
    in_window(cx, h, |window, cx| app.update(cx, |app, cx| app.show_active_sessions(project.map(|p| p.to_string().into()), window, cx)));
    cx.run_until_parked();
}

fn clear_active(cx: &mut TestAppContext, h: &Harness) {
    let app = h.app.clone();
    in_window(cx, h, |window, cx| app.update(cx, |app, cx| app.clear_active(window, cx)));
    cx.run_until_parked();
}

/// The ledger hands over a session with one call at `at` whose transcript is long gone.
fn ledger_only(cx: &mut TestAppContext, h: &Harness, id: &str, project: &str, at: &str) {
    use agent_session_core::model::{HourlyUsage, LedgerSession, SessionUsage, Tokens};
    let hour = chrono::DateTime::parse_from_rfc3339(at).unwrap().timestamp_millis() / 3_600_000;
    let session = SessionUsage {
        session: LedgerSession {
            id: id.into(),
            project_key: format!("-Users-me-Code-{project}"),
            project_path: Some(format!("/Users/me/Code/{project}")),
            title: "cleaned up long ago".into(),
            entrypoint: Some("cli".into()),
            ..Default::default()
        },
        hours: vec![HourlyUsage { hour, model: "claude-opus-5".into(), responses: 1, tokens: Tokens { input: 10, output: 10, ..Default::default() }, ..Default::default() }],
    };
    let app = h.app.clone();
    in_window(cx, h, |window, cx| app.update(cx, |app, cx| app.on_store_events(vec![crate::store::StoreEvent::Usage { sessions: vec![session], full: false }], window, cx)));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn revealing_a_session_from_usage_selects_it_and_focuses_the_table(cx: &mut TestAppContext) {
    use crate::model::MainView;
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    select(cx, &h, S1);
    h.app.update(cx, |app, _| app.detail_visible = false);
    usage_on(cx, &h, september());
    assert!(h.app.read_with(cx, |app, _| app.usage_view.snapshot.sessions.iter().any(|s| s.id == S2 && s.listed)), "S2 is in the list, with a row");

    reveal(cx, &h, S2);
    assert_eq!(view(cx, &h), MainView::Sessions);
    h.app.read_with(cx, |app, cx| {
        assert_eq!(app.selected.as_ref().unwrap().id, S2);
        let table = app.table.read(cx);
        assert_eq!(table.selected_row(), table.delegate().position_of(S2), "the table's own selection is on it");
        assert!(app.detail_visible, "the detail panel shows it");
    });
    assert_eq!(view_focus(cx, &h), (false, true), "the keys go to the table");
    assert!(notices(cx, &h).is_empty(), "nothing was in its way");
    press(cx, &h, "cmd-c");
    assert_eq!(clipboard(cx).as_deref(), Some(S2), "⌘C works at once");
}

#[gpui_kit::test]
fn revealing_a_filtered_out_session_relaxes_only_what_hides_it(cx: &mut TestAppContext) {
    use crate::model::{MainView, RangePreset};
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    h.app.update(cx, |app, cx| {
        app.set_project(Some(ALPHA.into()), cx);
        let day = chrono::NaiveDate::from_ymd_opt(2000, 1, 3).unwrap();
        app.set_custom_range(day, day, cx);
    });
    assert_eq!(h.app.read_with(cx, shown_rows), 0, "nothing was updated in 2000");
    usage_on(cx, &h, september());

    reveal(cx, &h, S2);
    assert_eq!(view(cx, &h), MainView::Sessions);
    h.app.read_with(cx, |app, cx| {
        assert_eq!(app.selected.as_ref().unwrap().id, S2);
        let filter = app.current_filter(cx);
        assert_eq!((filter.since_ms, filter.until_ms, app.range), (None, None, RangePreset::Any), "Updated is any time");
        assert_eq!(filter.project.as_deref(), Some(ALPHA), "the project was S2's: it stays");
    });
    assert_eq!(notices(cx, &h), ["已清除过滤：Updated"]);
    assert_eq!(saved_state(&h).range, RangePreset::Any, "saved like a pick in the Updated menu");
}

#[gpui_kit::test]
fn revealing_a_session_hidden_by_a_search_clears_the_search(cx: &mut TestAppContext) {
    use crate::model::MainView;
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    press(cx, &h, "cmd-f");
    type_text(cx, &h, "parser");
    press(cx, &h, "enter");
    wait_until(cx, &h, "the search", |app, cx| search_done(app, cx) && !app.search_pending);
    assert_eq!(h.app.read_with(cx, listed), [S1], "S2 says no 'parser'");
    usage_on(cx, &h, september());

    reveal(cx, &h, S2);
    assert_eq!(view(cx, &h), MainView::Sessions);
    assert_eq!(search_box(cx, &h), "", "the search box is emptied");
    h.app.read_with(cx, |app, cx| {
        assert!(!app.searching() && !app.table.read(cx).delegate().search_active(), "and the search is over");
        assert_eq!(app.selected.as_ref().unwrap().id, S2);
    });
    assert_eq!(view_focus(cx, &h), (false, true));
    assert_eq!(notices(cx, &h), ["已清除过滤：Search"]);

    // a session among the hits keeps the search
    press(cx, &h, "cmd-f");
    type_text(cx, &h, "parser");
    press(cx, &h, "enter");
    wait_until(cx, &h, "the search", |app, cx| search_done(app, cx) && !app.search_pending);
    usage_on(cx, &h, september());
    reveal(cx, &h, S1);
    assert_eq!(search_box(cx, &h), "parser");
    assert_eq!(h.app.read_with(cx, |app, _| app.selected.as_ref().unwrap().id.to_string()), S1);
    assert_eq!(notices(cx, &h).len(), 1, "nothing new to say");
}

#[gpui_kit::test]
fn a_session_without_a_row_stays_in_usage_and_explains(cx: &mut TestAppContext) {
    use crate::model::MainView;
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    usage_on(cx, &h, september());
    ledger_only(cx, &h, GONE, "alpha", "2026-09-03T08:00:00Z");
    h.app.read_with(cx, |app, _| {
        let lines = &app.usage_view.snapshot.sessions;
        assert!(lines.iter().any(|s| s.id == GONE && !s.listed), "listed in Usage, greyed: the table has no row for it");
        assert!(lines.iter().filter(|s| s.id != GONE).all(|s| s.listed));
    });

    reveal(cx, &h, GONE);
    assert_eq!(view(cx, &h), MainView::Usage, "nothing to show in the table");
    assert_eq!(view_focus(cx, &h), (true, false));
    assert_eq!(notices(cx, &h), [crate::drilldown::GONE_NOTICE]);
}

#[gpui_kit::test]
fn a_session_left_only_in_history_is_revealed_as_its_history_row(cx: &mut TestAppContext) {
    use crate::model::{Kind, MainView};
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    usage_on(cx, &h, september());
    // the ledger kept S4's calls; its transcript is gone, history.jsonl still has its prompts
    ledger_only(cx, &h, S4, "alpha", "2026-09-03T08:00:00Z");
    assert!(h.app.read_with(cx, |app, _| app.usage_view.snapshot.sessions.iter().any(|s| s.id == S4 && s.listed)), "the table has its History row");

    reveal(cx, &h, S4);
    assert_eq!(view(cx, &h), MainView::Sessions);
    h.app.read_with(cx, |app, _| {
        let row = app.selected.as_ref().expect("a selected row");
        assert_eq!((row.id.as_ref(), row.kind), (S4, Kind::History));
        assert!(app.detail_visible, "the detail panel shows its prompts");
    });
    assert_eq!(view_focus(cx, &h), (false, true));
}

#[gpui_kit::test]
fn a_session_of_a_hidden_project_is_not_revealed(cx: &mut TestAppContext) {
    use crate::model::MainView;
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    h.app.update(cx, |app, cx| app.hide_project(BETA.into(), cx));
    usage_on(cx, &h, september());
    reveal(cx, &h, S3);
    assert_eq!(view(cx, &h), MainView::Usage);
    assert_eq!(notices(cx, &h), [crate::drilldown::HIDDEN_NOTICE]);
    assert!(h.app.read_with(cx, |app, cx| app.current_filter(cx).hidden_projects.contains(BETA)), "still hidden: that is the user's to undo");
}

#[gpui_kit::test]
fn jumping_from_a_project_sets_project_any_time_and_active(cx: &mut TestAppContext) {
    use crate::model::{ActiveRange, MainView, RangePreset};
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    h.app.update(cx, |app, cx| app.set_range(RangePreset::Today, cx));
    let (first, last) = september();
    usage_on(cx, &h, (first, last));

    jump(cx, &h, Some(ALPHA));
    assert_eq!(view(cx, &h), MainView::Sessions);
    assert_eq!(view_focus(cx, &h), (false, true));
    h.app.read_with(cx, |app, cx| {
        let filter = app.current_filter(cx);
        assert_eq!(filter.project.as_deref(), Some(ALPHA));
        assert_eq!((filter.since_ms, filter.until_ms, app.range), (None, None, RangePreset::Any));
        assert_eq!(filter.active, Some(ActiveRange { first, last }), "the days Usage showed");
        assert_eq!(listed(app, cx), [S1, S2], "alpha's sessions with calls in September");
        assert!(app.table.read(cx).delegate().search_targets().iter().all(|t| t.project_key == ALPHA));
    });
    assert!(notices(cx, &h).is_empty(), "every session Usage counts is listed");
}

#[gpui_kit::test]
fn jumping_from_the_sessions_card_keeps_project_kind_and_model(cx: &mut TestAppContext) {
    use crate::model::{ActiveRange, Kind, MainView};
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    h.app.update(cx, |app, cx| {
        app.set_project(Some(BETA.into()), cx);
        app.toggle_kind(Kind::Sdk, cx);
        app.set_model_filter(Some("claude-opus-5".into()), cx);
    });
    let before = h.app.read_with(cx, |app, cx| app.current_filter(cx));
    usage_on(cx, &h, (s3_day(), s3_day()));
    jump(cx, &h, None);
    assert_eq!(view(cx, &h), MainView::Sessions);
    h.app.read_with(cx, |app, cx| {
        let filter = app.current_filter(cx);
        assert_eq!((filter.project, filter.kinds, filter.model), (before.project, before.kinds, before.model), "Project, Kind and Model as they were");
        assert_eq!(filter.active, Some(ActiveRange { first: s3_day(), last: s3_day() }));
        assert_eq!(listed(app, cx), [S3]);
    });
}

#[gpui_kit::test]
fn the_notice_counts_the_sessions_the_table_cannot_list(cx: &mut TestAppContext) {
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    usage_on(cx, &h, september());
    ledger_only(cx, &h, GONE, "alpha", "2026-09-03T08:00:00Z");
    ledger_only(cx, &h, "88888888-aaaa-4bbb-8ccc-000000000008", "beta", "2026-09-03T08:00:00Z");
    let alpha_sessions = h.app.update(cx, |app, cx| {
        app.set_project(Some(ALPHA.into()), cx);
        app.usage_view.snapshot.cards[4].value.to_string()
    });
    assert_eq!(alpha_sessions, "3", "Usage counts S1, S2 and the one without a transcript");

    jump(cx, &h, Some(ALPHA));
    assert_eq!(h.app.read_with(cx, shown_rows), 2);
    assert_eq!(notices(cx, &h), ["这段时间还有 1 个 session 的记录已被清理，未列出"], "3 in Usage − 2 listed; the beta one is another project");

    // without such a session nothing is said
    usage_on(cx, &h, (s1_day(), s1_day()));
    jump(cx, &h, Some(ALPHA));
    assert_eq!(h.app.read_with(cx, listed), [S1]);
    assert_eq!(notices(cx, &h).len(), 1);
}

#[gpui_kit::test]
fn clearing_the_active_tag_keeps_the_other_filters_and_the_focus(cx: &mut TestAppContext) {
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    usage_on(cx, &h, (s1_day(), s1_day()));
    jump(cx, &h, Some(ALPHA));
    assert_eq!(h.app.read_with(cx, listed), [S1], "S2's call was on another day");
    let with_active = h.app.read_with(cx, |app, cx| app.current_filter(cx));
    // the focus somewhere else first (set directly: after `press("cmd-f")` the test window now
    // and then hands the focus back to the box once the clear's effects run, whoever took it)
    let app = h.app.clone();
    in_window(cx, &h, |window, cx| app.update(cx, |app, cx| app.focus_search(window, cx)));
    assert_eq!(focus(cx, &h), (true, false));

    clear_active(cx, &h);
    h.app.read_with(cx, |app, cx| {
        let filter = app.current_filter(cx);
        assert_eq!(filter, crate::model::Filter { active: None, ..with_active.clone() }, "only the Active filter went");
        assert!(listed(app, cx).contains(&S2.to_string()), "alpha's other sessions are back");
    });
    assert_eq!(focus(cx, &h), (false, true), "the keys go to the table");
}

#[gpui_kit::test]
fn searching_under_the_active_filter_and_clearing_it_reruns_the_search(cx: &mut TestAppContext) {
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    usage_on(cx, &h, (s1_day(), s1_day()));
    jump(cx, &h, None);
    h.app.update(cx, |app, cx| app.toggle_regex(cx));
    press(cx, &h, "cmd-f");
    type_text(cx, &h, "parser|hello");
    press(cx, &h, "enter");
    wait_until(cx, &h, "the search", |app, cx| search_done(app, cx) && !app.search_pending);
    h.app.read_with(cx, |app, cx| {
        assert_eq!(listed(app, cx), [S1], "S2 says 'hello', but had no call that day");
        assert_eq!(app.search_summary.as_ref().unwrap().sessions, 1, "it was not even searched");
    });
    let seq = h.app.read_with(cx, |app, _| app.search_seq);

    clear_active(cx, &h);
    assert!(h.app.read_with(cx, |app, _| app.search_seq) > seq, "searched again");
    wait_until(cx, &h, "the search over all sessions", |app, cx| search_done(app, cx) && !app.search_pending);
    assert_eq!(h.app.read_with(cx, listed), [S1, S2]);
    assert_eq!(search_box(cx, &h), "parser|hello");
}

#[gpui_kit::test]
fn the_active_filter_is_not_restored_after_a_restart(cx: &mut TestAppContext) {
    use crate::model::RangePreset;
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    h.app.update(cx, |app, cx| app.set_range(RangePreset::Today, cx));
    usage_on(cx, &h, september());
    jump(cx, &h, Some(ALPHA));
    let state = saved_state(&h);
    assert_eq!((state.range, state.project_key.as_deref()), (RangePreset::Any, Some(ALPHA)), "what the jump set is saved like any pick");

    let again = launch_with(cx, |dir| state.save(&dir.join("state.json")).unwrap());
    again.app.read_with(cx, |app, cx| {
        let filter = app.current_filter(cx);
        assert_eq!(filter.active, None, "the Active filter is not");
        assert_eq!((app.range, filter.since_ms, filter.project.as_deref()), (RangePreset::Any, None, Some(ALPHA)));
        assert!(listed(app, cx).contains(&S2.to_string()));
    });
}

#[gpui_kit::test]
fn a_scan_that_adds_calls_in_range_adds_the_session_to_the_list(cx: &mut TestAppContext) {
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    usage_on(cx, &h, (s2_day(), s2_day()));
    jump(cx, &h, Some(ALPHA));
    assert_eq!(h.app.read_with(cx, listed), [S2]);

    let at = chrono::DateTime::parse_from_rfc3339("2026-09-05T12:00:30Z").unwrap().timestamp_millis();
    append_response(&h._dir.path().join("claude"), S1, "msg_same_day", "claude-opus-5", at);
    // an automatic scan, as the timer or coming back to the window starts it
    cx.executor().advance_clock(crate::AUTO_SCAN_MIN_GAP);
    assert!(h.app.update(cx, |app, cx| app.auto_scan("test", cx)));
    wait_until(cx, &h, "S1 with its call that day", |app, cx| listed(app, cx).len() == 2);
    assert_eq!(h.app.read_with(cx, listed), [S1, S2]);
}

// ---- the same with the mouse ------------------------------------------------------------------

/// A window tall enough for the Usage view's tables to be on screen, and a click on `selector`.
fn click_on(cx: &mut TestAppContext, h: &Harness, selector: &'static str) {
    use gpui_kit::{Modifiers, VisualTestContext, px, size};
    let mut window = VisualTestContext::from_window(h.window, cx);
    window.simulate_resize(size(px(1440.), px(1600.)));
    window.run_until_parked();
    let b = window.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} on screen"));
    window.simulate_click(b.center(), Modifiers::none());
    window.run_until_parked();
}

#[gpui_kit::test]
fn a_click_on_a_session_line_in_usage_reveals_it(cx: &mut TestAppContext) {
    use crate::model::MainView;
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    usage_on(cx, &h, september());
    let top = h.app.read_with(cx, |app, _| app.usage_view.snapshot.sessions[0].id.to_string());
    click_on(cx, &h, crate::usage_view::SESSION_LINE_SELECTOR);
    assert_eq!(view(cx, &h), MainView::Sessions);
    assert_eq!(h.app.read_with(cx, |app, _| app.selected.as_ref().unwrap().id.to_string()), top);
}

/// The arrow sits on the project's row, whose own click sets (or unsets) the Project filter: a
/// click on the arrow is the arrow's alone.
#[gpui_kit::test]
fn a_click_on_a_project_s_arrow_jumps_and_does_not_also_click_its_row(cx: &mut TestAppContext) {
    use crate::model::MainView;
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    usage_on(cx, &h, september());
    let top = h.app.update(cx, |app, cx| {
        let key = app.usage_view.snapshot.projects[0].key.clone().unwrap();
        // picked: a click on its row would unset it
        app.set_project(Some(key.clone()), cx);
        key
    });
    click_on(cx, &h, crate::usage_view::PROJECT_JUMP_SELECTOR);
    assert_eq!(view(cx, &h), MainView::Sessions);
    h.app.read_with(cx, |app, cx| {
        let filter = app.current_filter(cx);
        assert_eq!(filter.project, Some(top), "the row's click did not run too");
        assert!(filter.active.is_some());
    });
}

#[gpui_kit::test]
fn the_sessions_card_s_arrow_and_the_tag_s_cross_work_with_the_mouse(cx: &mut TestAppContext) {
    use crate::model::MainView;
    let h = launch(cx);
    wait_until(cx, &h, "the ledger", usage_loaded);
    usage_on(cx, &h, (s1_day(), s1_day()));
    click_on(cx, &h, crate::usage_view::SESSIONS_JUMP_SELECTOR);
    assert_eq!(view(cx, &h), MainView::Sessions);
    assert_eq!(h.app.read_with(cx, listed), [S1]);
    click_on(cx, &h, crate::toolbar::ACTIVE_TAG_SELECTOR);
    assert!(h.app.read_with(cx, |app, cx| app.current_filter(cx).active.is_none()), "the tag's ✕ cleared it");
    assert_eq!(focus(cx, &h), (false, true));
}

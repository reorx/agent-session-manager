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
    let dir = tempfile::tempdir().unwrap();
    prepare(dir.path());
    let claude = dir.path().join("claude");
    copy_fixture(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../core/tests/fixtures/claude")), &claude);
    let paths = AppPaths {
        claude: ClaudeDir::new(claude),
        db: dir.path().join("index.sqlite"),
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
    let h = Harness { _dir: dir, app: app.expect("the window builds the app"), window: window.into() };
    wait_until(cx, &h, "the start-up scan", |app, cx| matches!(app.index_status, IndexStatus::Idle(_)) && shown_rows(app, cx) > 0);
    h
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

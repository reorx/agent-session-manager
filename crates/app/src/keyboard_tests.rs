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
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths {
        claude: ClaudeDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../core/tests/fixtures/claude")),
        db: dir.path().join("index.sqlite"),
        state: dir.path().join("state.json"),
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
    wait_until(cx, &h, "the messages of the session with hits", |app, cx| hit(app, cx) == Some((1, 3)));

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

//! From the Usage view back to the session table: a session of "Most expensive sessions" shown
//! and selected there (`reveal_session`), or the sessions with a call in Usage's days
//! (`show_active_sessions`, the Active filter).
//!
//! Which sessions had a call in the Active filter's days comes from the usage data, not from
//! the rows: `sync_active_ids` works them out and hands them to the table's delegate, and
//! `Filter::matches` never looks at usage.

use std::collections::HashSet;
use std::sync::Arc;

use agent_session_core::usage::{Span, day_start_ms, range_span, usage_day};
use chrono::{Days, Local};
use gpui_kit::*;

use crate::AppRoot;
use crate::model::{ActiveRange, CallsInRange, Filter, MainView, RangePreset};
use crate::usage_view::usage_filter;

/// A click on a session the table has no row for: its transcript and its history are gone.
pub const GONE_NOTICE: &str = "这个 session 的记录已被清理，只保留了用量";
/// A click on a session of a project hidden in the table: hiding is the user's, so is undoing it.
pub const HIDDEN_NOTICE: &str = "这个 session 的项目在会话表里被隐藏，可在 Project 菜单里恢复显示";

/// What a jump to the Active filter says about the sessions Usage counted but the table does not
/// list: `unlisted` of them, `gone` of which have no row at all. `None` when all are listed.
pub fn unlisted_notice(unlisted: usize, gone: usize) -> Option<String> {
    match (unlisted, gone) {
        (0, _) => None,
        (n, gone) if gone >= n => Some(format!("这段时间还有 {n} 个 session 的记录已被清理，未列出")),
        (n, 0) => Some(format!("这段时间还有 {n} 个 session 被会话表的其他过滤挡住，未列出")),
        (n, gone) => Some(format!("这段时间还有 {n} 个 session 未列出：{gone} 个的记录已被清理，其余被会话表的其他过滤挡住")),
    }
}

/// Whether a reveal keeps the search on screen: only when the results on screen are sure to
/// stay as they are, with the session among them (`hit`). They are the box's only when
/// `settled`: the query typed is the one searched and its results are on screen (more may come,
/// they only add). Results still on their way, or a query still waiting for the typing to
/// pause, may not have the session and would take it away when they come (usage-p4-rv R1).
/// And no search may run again (`again`): one
/// that went stale while Usage was shown (a filter changed there, or a scan reloaded the rows)
/// runs once the table is back, and so does one over a filter this reveal relaxes. A search
/// run again over other or newer sessions keeps only the newest 200 with a hit, so the session
/// may drop out, however complete the old results were (R1.1).
pub fn keeps_search(settled: bool, hit: bool, again: bool) -> bool {
    settled && hit && !again
}

/// `[from, until)` of the Active filter's days (each from 4:00 local).
fn span_of(range: ActiveRange) -> Span {
    let after = range.last.checked_add_days(Days::new(1)).unwrap_or(range.last);
    Span { from_ms: day_start_ms(range.first, &Local), until_ms: day_start_ms(after, &Local) }
}

impl AppRoot {
    /// A notification, also in the log (the screenshot tours check it there).
    pub(crate) fn say(&mut self, message: impl Into<SharedString>, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::WindowExt;
        let message = message.into();
        println!("notice: {message}");
        #[cfg(test)]
        self.notices.push(message.clone());
        window.push_notification(message, cx);
    }

    /// The sessions with a call in `range`, of `model` when one is given, by the usage data.
    pub(crate) fn active_ids_in(&self, range: ActiveRange, model: Option<&str>) -> Arc<HashSet<SharedString>> {
        Arc::new(self.usage.active_sessions(span_of(range), model).into_iter().map(|id| SharedString::from(id.to_string())).collect())
    }

    /// The Active set `filter` asks for: its days, its model.
    pub(crate) fn active_ids_for(&self, filter: &Filter) -> Option<Arc<HashSet<SharedString>>> {
        filter.active.map(|range| self.active_ids_in(range, filter.model.as_deref()))
    }

    /// New usage came in: the sessions active in the Active filter's days again. True when they
    /// changed (the table then lists others).
    pub(crate) fn sync_active_ids(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(ids) = self.active_ids_for(&self.current_filter(cx)) else { return false };
        if self.table.read(cx).delegate().active_ids().is_some_and(|old| *old == ids) {
            return false;
        }
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_active_ids(Some(ids));
            cx.notify();
        });
        true
    }

    /// A click on a row of "Most expensive sessions": the session table with this session
    /// selected, scrolled to, in the detail panel, the keys on the table. Whatever hides it is
    /// relaxed (and said); a session without a row, or of a hidden project, stays in Usage with
    /// a notice instead.
    pub(crate) fn reveal_session(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.table.read(cx).delegate().row_by_id(id).cloned() else {
            self.say(GONE_NOTICE, window, cx);
            return;
        };
        let filter = self.current_filter(cx);
        if filter.hidden_projects.contains(&row.project_key) {
            self.say(HIDDEN_NOTICE, window, cx);
            return;
        }
        println!("reveal: {}", row.short_id);
        let calls = self.calls_in_active_days(&filter, id, cx);
        let (filter, names) = filter.relaxed_for(&row, calls);
        let mut relaxed = Vec::new();
        let typed = self.search.read(cx).value().trim().to_string();
        if self.searching() || !typed.is_empty() {
            // not the end of the search: its first results clear `search_pending` (a refresh's end
            // does), and what still comes only adds hits (the search thread keeps to the newest
            // 200), so a hit on screen stays until another search
            let settled = typed == self.query.as_ref() && !self.search_pending;
            let again = self.search_stale.is_some() || !names.is_empty();
            if !keeps_search(settled, self.table.read(cx).delegate().hit(id).is_some(), again) {
                // empty the box and end the search (a query still waiting for the typing to
                // pause then finds the box empty), the focus stays where it is (unlike Escape)
                self.search.update(cx, |input, cx| input.set_value("", window, cx));
                self.run_search(cx);
                relaxed.push("Search");
            }
        }
        if names.contains(&"Updated") {
            self.range = RangePreset::Any;
        }
        relaxed.extend(names);
        // whatever the filter change selects on its way (it may be another row) gives way to it,
        // and a selection still awaited from the saved state is dropped
        self.wanted_id = Some(row.id.clone());
        // a search still on is one nothing runs again: the filter changes only when it went
        self.apply_filter(filter, cx);
        if !relaxed.is_empty() {
            self.say(format!("已清除过滤：{}", relaxed.join("、")), window, cx);
        }
        self.detail_visible = true;
        self.set_view(MainView::Sessions, window, cx);
        if let Some(ix) = self.table.read(cx).delegate().position_of(id) {
            self.select_row(ix, cx);
            // selecting the row already selected does not scroll
            self.table.update(cx, |table, cx| table.scroll_to_row(ix, cx));
        }
        self.sync_projection(cx);
        self.focus_table(window, cx);
        self.persist(cx);
        cx.notify();
    }

    /// What `id` called in the days of `filter.active` (`Picked` without that filter).
    fn calls_in_active_days(&self, filter: &Filter, id: &str, cx: &App) -> CallsInRange {
        let Some(range) = filter.active else { return CallsInRange::Picked };
        if self.table.read(cx).delegate().is_active(id) {
            CallsInRange::Picked
        } else if filter.model.is_some() && self.active_ids_in(range, None).contains(id) {
            CallsInRange::OtherModels
        } else {
            CallsInRange::None
        }
    }

    /// A project's `↗` (`Some(key)`) or the Sessions card's: the session table with only the
    /// sessions that had a call in the days Usage shows, Updated any time, no search; the project
    /// set when one is given, the other filters as they were. Says how many sessions Usage
    /// counts that the table cannot list.
    pub(crate) fn show_active_sessions(&mut self, project: Option<SharedString>, window: &mut Window, cx: &mut Context<Self>) {
        let u = &self.usage_view;
        let span = range_span(u.range, self.usage_day, u.custom, u.day, &Local);
        let range = ActiveRange { first: usage_day(span.from_ms, &Local), last: usage_day(span.until_ms - 1, &Local) };
        let mut filter = self.current_filter(cx);
        (filter.since_ms, filter.until_ms) = (None, None);
        filter.active = Some(range);
        if project.is_some() {
            filter.project = project;
        }
        println!("active: {} – {} · project {}", range.first, range.last, filter.project.as_deref().unwrap_or("all"));
        self.range = RangePreset::Any;
        if self.searching() {
            self.search.update(cx, |input, cx| input.set_value("", window, cx));
            self.run_search(cx);
        }
        self.apply_filter(filter.clone(), cx);
        self.set_view(MainView::Sessions, window, cx);
        self.focus_table(window, cx);
        self.persist(cx);

        let counted = self.usage.summary(span, &usage_filter(&filter));
        let delegate = self.table.read(cx).delegate();
        let shown: HashSet<&str> = delegate.ix.iter().map(|i| delegate.rows[*i].id.as_ref()).collect();
        let unlisted: Vec<&str> = counted.by_session.iter().map(|s| s.id.as_str()).filter(|id| !shown.contains(id)).collect();
        let gone = unlisted.iter().filter(|id| delegate.row_by_id(id).is_none()).count();
        if let Some(notice) = unlisted_notice(unlisted.len(), gone) {
            self.say(notice, window, cx);
        }
        cx.notify();
    }

    /// The ✕ on the toolbar's `Active …` tag: that filter goes, the others stay, the keys go to
    /// the table.
    pub(crate) fn clear_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut filter = self.current_filter(cx);
        filter.active = None;
        println!("active: cleared");
        self.apply_filter(filter, cx);
        self.focus_table(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;

    #[test]
    fn the_notice_says_why_sessions_are_not_listed() {
        assert_eq!(unlisted_notice(0, 0), None);
        assert_eq!(unlisted_notice(3, 3).as_deref(), Some("这段时间还有 3 个 session 的记录已被清理，未列出"));
        assert_eq!(unlisted_notice(2, 0).as_deref(), Some("这段时间还有 2 个 session 被会话表的其他过滤挡住，未列出"));
        assert_eq!(unlisted_notice(5, 2).as_deref(), Some("这段时间还有 5 个 session 未列出：2 个的记录已被清理，其余被会话表的其他过滤挡住"));
    }

    #[test]
    fn a_reveal_keeps_only_a_search_that_is_sure_to_still_show_the_session() {
        // (settled, hit, again)
        assert!(keeps_search(true, true, false), "the results on screen stay, the session among them");
        assert!(!keeps_search(true, true, true), "searched again, the newest 200 may not have it, complete or cut before");
        assert!(!keeps_search(true, false, false));
        assert!(!keeps_search(false, true, false), "old hits: the results on their way decide");
    }

    #[test]
    fn the_active_days_run_from_four_to_four() {
        let (first, last) = ("2026-11-10".parse().unwrap(), "2026-11-12".parse().unwrap());
        let span = span_of(ActiveRange { first, last });
        assert_eq!(span.from_ms, day_start_ms(first, &Local));
        assert_eq!(span.until_ms, day_start_ms("2026-11-13".parse().unwrap(), &Local));
    }
}

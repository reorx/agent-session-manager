//! Session table: `DataTable` delegate holding the canonical rows plus a filtered + sorted
//! index vector. Filtering and sorting run on the main thread (10k rows ≈ 7ms).

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::model::{DisplayMode, Filter, SessionRow};
use crate::search::{RowHit, SearchTarget};
use crate::style::{self, FluentBuilder};

/// Fixed widths from the mockup; Preview takes whatever is left.
pub const PROJECT_W: f32 = 110.;
pub const STARTED_W: f32 = 88.;
pub const UPDATED_W: f32 = 88.;
pub const PROMPTS_W: f32 = 58.;
pub const COST_W: f32 = 62.;
pub const CTX_W: f32 = 44.;
pub const SIZE_W: f32 = 60.;
pub const KIND_W: f32 = 46.;
/// Only there while a search is active.
pub const MATCHES_W: f32 = 64.;
const FIXED_COLS_W: f32 = PROJECT_W + STARTED_W + UPDATED_W + PROMPTS_W + COST_W + CTX_W + SIZE_W + KIND_W;
pub const PREVIEW_MIN_W: f32 = 200.;
pub const ROW_H: f32 = 36.;

const PREVIEW_COL: usize = 1;
const MATCHES_COL: usize = 2;
const MATCHES_KEY: &str = "matches";
const DEFAULT_SORT_KEY: &str = "updated";

pub struct SessionTableDelegate {
    pub rows: Vec<Arc<SessionRow>>,
    /// Session id → position in `rows`.
    by_id: HashMap<SharedString, usize>,
    /// Indices into `rows` after filter + sort.
    pub ix: Vec<usize>,
    columns: Vec<Column>,
    pub mode: DisplayMode,
    filter: Filter,
    /// Hits of the active search by session id; only these sessions are shown while it is `Some`.
    search: Option<HashMap<SharedString, Arc<RowHit>>>,
    sort: Option<(SharedString, ColumnSort)>,
    /// Cost of the rows in `ix` (the status bar shows it).
    pub shown_cost: f64,
    pub last_filter_ms: u128,
}

fn col(key: &'static str, name: &'static str, width: f32) -> Column {
    Column::new(key, name).width(px(width)).paddings(Edges { top: px(0.), bottom: px(0.), left: px(6.), right: px(6.) })
}

impl SessionTableDelegate {
    pub fn new() -> Self {
        let columns = vec![
            col("project", "Project", PROJECT_W).sortable(),
            col("preview", "Preview", 460.).resizable(false),
            col("started", "Started", STARTED_W).sortable(),
            col("updated", "Updated", UPDATED_W).sortable().descending(),
            col("prompts", "Prompts", PROMPTS_W).text_right().sortable(),
            col("cost", "Cost", COST_W).text_right().sortable(),
            col("context", "Ctx", CTX_W).text_right().sortable(),
            col("size", "Size", SIZE_W).text_right().sortable(),
            col("kind", "Kind", KIND_W).sortable(),
        ];
        Self {
            rows: Vec::new(),
            by_id: HashMap::new(),
            ix: Vec::new(),
            columns,
            mode: DisplayMode::FirstPrompt,
            filter: Filter::default(),
            search: None,
            sort: None,
            shown_cost: 0.0,
            last_filter_ms: 0,
        }
    }

    pub fn row(&self, row_ix: usize) -> &Arc<SessionRow> {
        &self.rows[self.ix[row_ix]]
    }

    /// Replace every row (a load from the index).
    pub fn set_rows(&mut self, rows: Vec<Arc<SessionRow>>) {
        self.by_id = rows.iter().enumerate().map(|(i, r)| (r.id.clone(), i)).collect();
        self.rows = rows;
        self.recompute();
    }

    /// Add new sessions and replace changed ones by id (rows streamed by a scan).
    pub fn upsert(&mut self, rows: Vec<Arc<SessionRow>>) {
        for row in rows {
            match self.by_id.get(&row.id) {
                Some(&i) => self.rows[i] = row,
                None => {
                    self.by_id.insert(row.id.clone(), self.rows.len());
                    self.rows.push(row);
                }
            }
        }
        self.recompute();
    }

    /// Where a session currently sits in the filtered + sorted view.
    pub fn position_of(&self, id: &str) -> Option<usize> {
        let i = *self.by_id.get(id)?;
        self.ix.iter().position(|x| *x == i)
    }

    /// Current sort as (column key, direction); `None` is the default order.
    pub fn sort_state(&self) -> Option<(SharedString, ColumnSort)> {
        self.sort.clone()
    }

    /// Sort programmatically (restoring a saved state). The caller must `TableState::refresh`
    /// afterwards so the header arrows follow.
    pub fn set_sort(&mut self, key: &str, sort: ColumnSort) {
        if !self.columns.iter().any(|c| c.key == key && c.sort.is_some()) {
            return;
        }
        self.sort(key, sort);
    }

    pub fn search_active(&self) -> bool {
        self.search.is_some()
    }

    pub fn hit(&self, id: &str) -> Option<&Arc<RowHit>> {
        self.search.as_ref()?.get(id)
    }

    /// Enter search mode with no hits yet (`true`), or leave it (`false`). The Matches column
    /// comes and goes with it, so the caller must `TableState::refresh` afterwards.
    pub fn set_search(&mut self, active: bool) {
        if active != self.columns.iter().any(|c| c.key == MATCHES_KEY) {
            match active {
                true => self.columns.insert(MATCHES_COL, col(MATCHES_KEY, "Matches", MATCHES_W).text_right().sortable()),
                false => drop(self.columns.remove(MATCHES_COL)),
            }
        }
        if !active && self.sort.as_ref().is_some_and(|(key, _)| key == MATCHES_KEY) {
            self.sort = None;
        }
        self.search = active.then(HashMap::new);
        self.sync_column_arrows();
        self.recompute();
    }

    /// Hits of the running search, batch by batch.
    pub fn add_hits(&mut self, hits: Vec<(SharedString, Arc<RowHit>)>) {
        if let Some(search) = &mut self.search {
            search.extend(hits);
            self.recompute();
        }
    }

    /// The sessions a search should look into: whatever passes the filters, newest first.
    pub fn search_targets(&self) -> Vec<SearchTarget> {
        let mut rows: Vec<&Arc<SessionRow>> = self.rows.iter().filter(|r| self.filter.matches(r)).collect();
        rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        rows.iter()
            .map(|r| SearchTarget {
                session_id: r.id.to_string(),
                project_key: r.project_key.to_string(),
                path: r.transcript_path.clone(),
                updated_at: r.updated_at,
            })
            .collect()
    }

    /// Width of every column but Preview.
    pub fn fixed_cols_width(&self) -> f32 {
        if self.search_active() { FIXED_COLS_W + MATCHES_W } else { FIXED_COLS_W }
    }

    fn shows(&self, row: &SessionRow) -> bool {
        self.filter.matches(row) && self.search.as_ref().is_none_or(|hits| hits.contains_key(&row.id))
    }

    fn recompute(&mut self) {
        let t = Instant::now();
        self.ix = (0..self.rows.len()).filter(|i| self.shows(&self.rows[*i])).collect();
        self.shown_cost = self.ix.iter().filter_map(|i| self.rows[*i].cost).fold(0.0, |a, b| a + b);
        match self.sort.clone() {
            Some((key, sort)) => self.sort(&key, sort),
            None => self.sort_by_updated_desc(),
        }
        self.last_filter_ms = t.elapsed().as_millis();
    }

    pub fn preview_width(&self) -> Pixels {
        self.columns[PREVIEW_COL].width
    }

    pub fn set_preview_width(&mut self, width: Pixels) {
        self.columns[PREVIEW_COL].width = width;
    }

    pub fn filter(&self) -> &Filter {
        &self.filter
    }

    /// Re-run filter + current sort. Returns true if the filter changed.
    pub fn set_filter(&mut self, filter: Filter) -> bool {
        if self.filter == filter {
            return false;
        }
        self.filter = filter;
        self.recompute();
        true
    }

    fn sort_by_updated_desc(&mut self) {
        let rows = &self.rows;
        self.ix.sort_by(|a, b| rows[*b].updated_at.cmp(&rows[*a].updated_at));
    }

    fn matches_of(&self, row: &SessionRow) -> u32 {
        self.hit(&row.id).map_or(0, |hit| hit.count)
    }

    fn compare(&self, a: &SessionRow, b: &SessionRow, key: &str) -> Ordering {
        match key {
            MATCHES_KEY => self.matches_of(a).cmp(&self.matches_of(b)),
            "project" => a.project.cmp(&b.project),
            "started" => a.started_at.cmp(&b.started_at),
            "updated" => a.updated_at.cmp(&b.updated_at),
            "prompts" => a.prompts.cmp(&b.prompts),
            "cost" => cmp_missing_low(a.cost, b.cost, f64::total_cmp),
            "context" => cmp_missing_low(a.context_pct, b.context_pct, f32::total_cmp),
            "size" => a.size.cmp(&b.size),
            "kind" => (a.kind as u8).cmp(&(b.kind as u8)),
            _ => Ordering::Equal,
        }
    }

    /// Header arrows live in `Column::sort`; keep them in step with `self.sort` so a later
    /// `TableState::refresh` (the Preview column is re-measured on every resize) does not reset them.
    fn sync_column_arrows(&mut self) {
        let (active, dir) = match &self.sort {
            Some((key, dir)) => (key.clone(), *dir),
            None => (DEFAULT_SORT_KEY.into(), ColumnSort::Descending),
        };
        for column in self.columns.iter_mut().filter(|c| c.sort.is_some()) {
            column.sort = Some(if column.key == active { dir } else { ColumnSort::Default });
        }
    }

    fn sort(&mut self, key: &str, sort: ColumnSort) {
        if sort == ColumnSort::Default {
            self.sort = None;
            self.sync_column_arrows();
            self.sort_by_updated_desc();
            return;
        }
        self.sort = Some((key.to_string().into(), sort));
        self.sync_column_arrows();
        let mut ix = std::mem::take(&mut self.ix);
        ix.sort_by(|a, b| {
            let (ra, rb) = (&self.rows[*a], &self.rows[*b]);
            let ord = self.compare(ra, rb, key);
            let ord = if sort == ColumnSort::Descending { ord.reverse() } else { ord };
            ord.then_with(|| rb.updated_at.cmp(&ra.updated_at))
        });
        self.ix = ix;
    }

    /// Second Preview line while searching: where the first match is, then its snippet.
    fn hit_line(&self, hit: &RowHit, cx: &App) -> Div {
        let marks = hit.ranges.iter().map(|r| (r.clone(), style::hit_highlight()));
        h_flex()
            .gap(px(5.))
            .text_size(px(11.))
            .line_height(px(14.))
            .child(div().flex_none().font_family(style::mono(cx)).text_size(px(10.)).text_color(style::muted_light()).child(hit.label.clone()))
            .child(div().flex_1().min_w_0().truncate().text_color(style::text_secondary()).child(StyledText::new(hit.snippet.clone()).with_highlights(marks)))
    }

    /// `right`: the column is declared `text_right` (numbers); `DataTable` leaves cell alignment to the delegate.
    fn mono_cell(&self, text: SharedString, color: Hsla, right: bool, cx: &App) -> AnyElement {
        div()
            .font_family(style::mono(cx))
            .text_size(px(11.))
            .whitespace_nowrap()
            .text_color(color)
            .when(right, |el| el.w_full().text_right())
            .child(text)
            .into_any_element()
    }
}

/// A missing value sorts below any present one.
fn cmp_missing_low<T: Copy>(a: Option<T>, b: Option<T>, cmp: impl Fn(&T, &T) -> Ordering) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => cmp(&a, &b),
        (a, b) => a.is_some().cmp(&b.is_some()),
    }
}

impl TableDelegate for SessionTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.ix.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        self.columns[col_ix].clone()
    }

    fn perform_sort(&mut self, col_ix: usize, sort: ColumnSort, _: &mut Window, cx: &mut Context<TableState<Self>>) {
        let key = self.columns[col_ix].key.clone();
        self.sort(&key, sort);
        cx.notify();
    }

    fn render_th(&mut self, col_ix: usize, _: &mut Window, _: &mut Context<TableState<Self>>) -> impl IntoElement {
        let column = &self.columns[col_ix];
        let is_sorted = self.sort.as_ref().map(|(k, _)| *k == column.key).unwrap_or(column.key == DEFAULT_SORT_KEY);
        div()
            .size_full()
            .flex()
            .items_center()
            .text_size(px(11.))
            .when(column.align == TextAlign::Right, |el| el.justify_end())
            .when(is_sorted, |el| el.text_color(style::text()).font_weight(FontWeight::SEMIBOLD))
            .child(column.name.clone())
    }

    fn context_menu(&mut self, row_ix: usize, menu: gpui_kit::component::menu::PopupMenu, _: &mut Window, _: &mut Context<TableState<Self>>) -> gpui_kit::component::menu::PopupMenu {
        use gpui_kit::component::menu::PopupMenuItem;
        let row = self.row(row_ix).clone();
        let path = row.transcript_path.clone();
        menu.menu(if row.favorite { "取消收藏" } else { "收藏" }, Box::new(crate::favorites::FavoriteSession { id: row.id.to_string() }))
            .item(PopupMenuItem::new("在 Finder 中显示").on_click(move |_, _, cx| { if let Some(path) = &path { cx.reveal_path(path); } }))
    }

    fn render_td(&mut self, row_ix: usize, col_ix: usize, _: &mut Window, cx: &mut Context<TableState<Self>>) -> impl IntoElement {
        let row = self.row(row_ix).clone();
        let right = self.columns[col_ix].align == TextAlign::Right;
        match self.columns[col_ix].key.as_ref() {
            "project" => div().truncate().child(row.project.clone()).into_any_element(),
            "preview" => {
                let sub = match self.hit(&row.id) {
                    Some(hit) => self.hit_line(hit, cx),
                    None => div().truncate().text_size(px(11.)).line_height(px(14.)).text_color(style::muted()).child(self.mode.preview(&row)),
                };
                v_flex()
                    .w_full()
                    .h(px(ROW_H))
                    .justify_center()
                    .overflow_hidden()
                    .child(div().flex().gap(px(4.)).line_height(px(15.)).when(row.favorite, |el| el.child(div().text_color(style::favorite()).child("★"))).child(div().truncate().child(row.title.clone())))
                    .child(sub)
                    .into_any_element()
            }
            MATCHES_KEY => {
                let count = self.matches_of(&row).to_string();
                self.mono_cell(count.into(), style::text(), right, cx)
            }
            "started" => self.mono_cell(row.started_fmt.clone(), style::text_secondary(), right, cx),
            "updated" => self.mono_cell(row.updated_fmt.clone(), style::text(), right, cx),
            "prompts" => self.mono_cell(row.prompts_fmt.clone(), style::text(), right, cx),
            "cost" => self.mono_cell(row.cost_fmt.clone(), style::text(), right, cx),
            "context" => self.mono_cell(row.context_fmt.clone(), style::text_secondary(), right, cx),
            "size" => self.mono_cell(row.size_fmt.clone(), style::text_secondary(), right, cx),
            "kind" => match style::kind_badge(row.kind, cx) {
                Some(badge) => div().child(badge).into_any_element(),
                None => div().into_any_element(),
            },
            _ => div().into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    // `gpui_kit::*` exports gpui's own `test` attribute; name the standard one explicitly.
    use core::prelude::v1::test;

    use super::*;
    use crate::model::tests::{row, summary};
    use crate::model::{Kind, KindSet};

    fn ids(d: &SessionTableDelegate) -> Vec<&str> {
        (0..d.ix.len()).map(|i| d.row(i).id.as_ref()).collect()
    }

    fn with_prompts(id: &str, project: &str, updated_at: i64, prompts: u32) -> Arc<SessionRow> {
        let mut s = summary(id, project, updated_at);
        s.user_msg_count = prompts;
        Arc::new(SessionRow::from_summary(&s))
    }

    #[test]
    fn rows_show_newest_first_by_default() {
        let mut d = SessionTableDelegate::new();
        d.set_rows(vec![row("a", "/p/x", 100), row("c", "/p/x", 300), row("b", "/p/y", 200)]);
        assert_eq!(ids(&d), ["c", "b", "a"]);
        assert_eq!(d.position_of("b"), Some(1));
        assert_eq!(d.position_of("zzz"), None);
    }

    #[test]
    fn streamed_rows_are_inserted_in_order_and_changed_rows_replace_the_old_ones() {
        let mut d = SessionTableDelegate::new();
        d.set_rows(vec![row("a", "/p/x", 100), row("b", "/p/x", 200)]);
        d.upsert(vec![row("c", "/p/x", 150), row("a", "/p/x", 400)]);
        assert_eq!(ids(&d), ["a", "b", "c"], "a moved to the top after its update, the new c is the oldest");
        assert_eq!(d.rows.len(), 3, "a was replaced, not duplicated");
    }

    #[test]
    fn the_filter_and_the_sort_survive_new_data() {
        let mut d = SessionTableDelegate::new();
        d.set_rows(vec![with_prompts("a", "/p/x", 100, 5), with_prompts("b", "/p/y", 200, 9), with_prompts("c", "/p/x", 300, 1)]);
        d.set_filter(Filter { project: Some("-p-x".into()), ..Default::default() });
        d.set_sort("prompts", ColumnSort::Descending);
        assert_eq!(ids(&d), ["a", "c"]);
        d.upsert(vec![with_prompts("d", "/p/x", 50, 7), with_prompts("e", "/p/y", 500, 99)]);
        assert_eq!(ids(&d), ["d", "a", "c"], "still project x, still by prompts");
        assert_eq!(d.position_of("e"), None, "filtered out rows have no position");
        d.set_rows(vec![with_prompts("z", "/p/x", 1, 3)]);
        assert_eq!(ids(&d), ["z"]);
    }

    fn history(id: &str, project: &str, updated_at: i64) -> Arc<SessionRow> {
        let mut s = summary(id, project, updated_at);
        s.transcript_missing = true;
        Arc::new(SessionRow::from_summary(&s))
    }

    #[test]
    fn the_kind_filter_survives_new_data() {
        let mut d = SessionTableDelegate::new();
        d.set_rows(vec![row("a", "/p/x", 100), history("h1", "/p/x", 200), row("b", "/p/y", 300)]);
        let mut kinds = KindSet::default();
        kinds.toggle(Kind::History);
        assert!(d.set_filter(Filter { kinds, ..Default::default() }));
        assert_eq!(ids(&d), ["b", "a"]);
        d.upsert(vec![history("h2", "/p/y", 900), row("c", "/p/x", 400), history("a", "/p/x", 500)]);
        assert_eq!(ids(&d), ["c", "b"], "new history rows stay hidden, and a turned into history so it left");
        assert_eq!(d.position_of("h2"), None);
        assert_eq!(d.rows.len(), 5, "hidden rows are still kept");
        assert!(d.set_filter(Filter::default()));
        assert_eq!(ids(&d), ["h2", "a", "c", "b", "h1"]);
    }

    fn hit(count: u32) -> Arc<RowHit> {
        Arc::new(RowHit { count, label: "user".into(), snippet: "…".into(), ranges: Vec::new() })
    }

    fn keys(d: &SessionTableDelegate) -> Vec<String> {
        d.columns.iter().map(|c| c.key.to_string()).collect()
    }

    #[test]
    fn a_search_shows_only_the_sessions_with_hits_and_stacks_with_the_filters() {
        let mut d = SessionTableDelegate::new();
        d.set_rows(vec![row("a", "/p/x", 100), row("b", "/p/y", 200), row("c", "/p/x", 300)]);
        d.set_search(true);
        assert!(ids(&d).is_empty(), "a search that found nothing (yet) shows nothing");
        d.add_hits(vec![("a".into(), hit(2)), ("b".into(), hit(7))]);
        d.add_hits(vec![("c".into(), hit(1))]);
        assert_eq!(ids(&d), ["c", "b", "a"], "batches add up; newest first as always");
        d.set_filter(Filter { project: Some("-p-x".into()), ..Default::default() });
        assert_eq!(ids(&d), ["c", "a"], "the project filter still applies");
        d.set_search(false);
        d.set_filter(Filter::default());
        assert_eq!(ids(&d), ["c", "b", "a"]);
        assert!(d.hit("a").is_none());
    }

    #[test]
    fn the_matches_column_exists_only_while_searching_and_sorts_by_hit_count() {
        let mut d = SessionTableDelegate::new();
        let before = keys(&d);
        d.set_rows(vec![row("a", "/p/x", 100), row("b", "/p/y", 200), row("c", "/p/x", 300)]);
        d.set_search(true);
        assert_eq!(keys(&d)[..3], ["project", "preview", "matches"]);
        assert_eq!(d.fixed_cols_width(), FIXED_COLS_W + MATCHES_W, "Preview gives up the room");
        d.add_hits(vec![("a".into(), hit(9)), ("b".into(), hit(1)), ("c".into(), hit(4))]);
        d.set_sort("matches", ColumnSort::Descending);
        assert_eq!(ids(&d), ["a", "c", "b"]);
        d.set_search(true);
        assert_eq!(keys(&d).iter().filter(|k| *k == "matches").count(), 1, "a new search keeps the one column");
        assert_eq!(d.sort_state(), Some(("matches".into(), ColumnSort::Descending)), "and the sort");

        d.set_search(false);
        assert_eq!(keys(&d), before);
        assert_eq!(d.sort_state(), None, "without the column the table is back to newest first");
        assert_eq!(ids(&d), ["c", "b", "a"]);
        assert_eq!(d.fixed_cols_width(), FIXED_COLS_W);
    }

    #[test]
    fn search_targets_are_the_filtered_sessions_newest_first() {
        let mut d = SessionTableDelegate::new();
        d.set_rows(vec![row("a", "/p/x", 100), row("b", "/p/y", 200), row("c", "/p/x", 300), history("h", "/p/x", 400)]);
        d.set_filter(Filter { project: Some("-p-x".into()), ..Default::default() });
        d.set_search(true);
        let targets = d.search_targets();
        let got: Vec<(&str, bool)> = targets.iter().map(|t| (t.session_id.as_str(), t.path.is_some())).collect();
        assert_eq!(got, [("h", false), ("c", true), ("a", true)], "hits do not narrow the targets; history has no transcript");
    }

    #[test]
    fn a_hidden_project_stays_hidden_when_new_sessions_of_it_arrive() {
        let mut d = SessionTableDelegate::new();
        d.set_rows(vec![row("a", "/p/x", 100), row("p1", "/p/probe", 200)]);
        let mut filter = Filter::default();
        filter.hide_project("-p-probe".into());
        assert!(d.set_filter(filter));
        assert_eq!(ids(&d), ["a"]);
        d.upsert(vec![row("p2", "/p/probe", 900), row("b", "/p/y", 300)]);
        assert_eq!(ids(&d), ["b", "a"]);
        assert_eq!(d.position_of("p2"), None);
    }

    #[test]
    fn equal_sort_values_fall_back_to_newest_first() {
        let mut d = SessionTableDelegate::new();
        d.set_rows(vec![with_prompts("old", "/p/x", 100, 2), with_prompts("new", "/p/x", 300, 2), with_prompts("top", "/p/x", 200, 8)]);
        d.set_sort("prompts", ColumnSort::Ascending);
        assert_eq!(ids(&d), ["new", "old", "top"]);
    }

    #[test]
    fn header_arrows_follow_the_sort_state() {
        let arrows = |d: &SessionTableDelegate| -> Vec<(String, ColumnSort)> {
            d.columns.iter().filter_map(|c| c.sort.map(|s| (c.key.to_string(), s))).filter(|(_, s)| *s != ColumnSort::Default).collect()
        };
        let mut d = SessionTableDelegate::new();
        assert_eq!(arrows(&d), [("updated".to_string(), ColumnSort::Descending)]);
        d.set_sort("size", ColumnSort::Ascending);
        assert_eq!(arrows(&d), [("size".to_string(), ColumnSort::Ascending)]);
        assert_eq!(d.sort_state(), Some(("size".into(), ColumnSort::Ascending)));
        d.set_sort("size", ColumnSort::Default);
        assert_eq!(arrows(&d), [("updated".to_string(), ColumnSort::Descending)], "back to the default order");
        assert_eq!(d.sort_state(), None);
        d.set_sort("preview", ColumnSort::Ascending);
        assert_eq!(d.sort_state(), None, "Preview is not sortable");
    }

    fn priced(id: &str, project: &str, updated_at: i64, cost: Option<f64>, context: Option<u64>) -> Arc<SessionRow> {
        let mut s = summary(id, project, updated_at);
        s.cost_usd = cost;
        (s.context_tokens, s.context_model) = (context, Some("claude-opus-5".into()));
        Arc::new(SessionRow::from_summary(&s))
    }

    #[test]
    fn cost_and_context_columns_sit_after_prompts() {
        let d = SessionTableDelegate::new();
        assert_eq!(keys(&d), ["project", "preview", "started", "updated", "prompts", "cost", "context", "size", "kind"]);
        assert_eq!(FIXED_COLS_W, PROJECT_W + STARTED_W + UPDATED_W + PROMPTS_W + COST_W + CTX_W + SIZE_W + KIND_W);
    }

    #[test]
    fn cost_and_context_sort_the_most_first_and_rows_without_data_last() {
        let mut d = SessionTableDelegate::new();
        d.set_rows(vec![
            priced("a", "/p/x", 100, Some(1.0), Some(100_000)),
            priced("b", "/p/x", 200, None, None),
            priced("c", "/p/x", 300, Some(5.0), Some(50_000)),
            priced("d", "/p/x", 400, Some(0.5), Some(900_000)),
        ]);
        d.set_sort("cost", ColumnSort::Descending);
        assert_eq!(ids(&d), ["c", "a", "d", "b"]);
        d.set_sort("context", ColumnSort::Descending);
        assert_eq!(ids(&d), ["d", "a", "c", "b"]);
        d.set_sort("cost", ColumnSort::Ascending);
        assert_eq!(ids(&d), ["b", "d", "a", "c"], "no cost sorts below any cost");
    }

    #[test]
    fn the_shown_cost_adds_up_the_rows_on_screen() {
        let mut d = SessionTableDelegate::new();
        d.set_rows(vec![priced("a", "/p/x", 100, Some(1.0), None), priced("b", "/p/y", 200, Some(2.5), None), priced("c", "/p/x", 300, None, None)]);
        assert_eq!(d.shown_cost, 3.5);
        d.set_filter(Filter { project: Some("-p-x".into()), ..Default::default() });
        assert_eq!(d.shown_cost, 1.0, "the filter narrows it");
        d.set_search(true);
        d.add_hits(vec![("c".into(), hit(1))]);
        assert_eq!(d.shown_cost, 0.0, "while searching only the sessions with hits count");
        d.add_hits(vec![("a".into(), hit(3))]);
        assert_eq!(d.shown_cost, 1.0);
        d.upsert(vec![priced("a", "/p/x", 400, Some(4.0), None)]);
        assert_eq!(d.shown_cost, 4.0, "a row streamed in again brings its new cost");
    }
}

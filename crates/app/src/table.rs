//! Session table: `DataTable` delegate holding the canonical rows plus a filtered + sorted
//! index vector. Filtering and sorting run on the main thread (10k rows ≈ 7ms).

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::component::v_flex;
use gpui_kit::*;

use crate::model::{DisplayMode, Filter, SessionRow};
use crate::style::{self, FluentBuilder};

/// Fixed widths from the mockup; Preview takes whatever is left.
pub const PROJECT_W: f32 = 110.;
pub const STARTED_W: f32 = 88.;
pub const UPDATED_W: f32 = 88.;
pub const PROMPTS_W: f32 = 58.;
pub const SIZE_W: f32 = 60.;
pub const KIND_W: f32 = 46.;
pub const FIXED_COLS_W: f32 = PROJECT_W + STARTED_W + UPDATED_W + PROMPTS_W + SIZE_W + KIND_W;
pub const PREVIEW_MIN_W: f32 = 240.;
pub const ROW_H: f32 = 36.;

const PREVIEW_COL: usize = 1;
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
    sort: Option<(SharedString, ColumnSort)>,
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
            sort: None,
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

    fn recompute(&mut self) {
        let t = Instant::now();
        self.ix = (0..self.rows.len()).filter(|i| self.filter.matches(&self.rows[*i])).collect();
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

    fn compare(a: &SessionRow, b: &SessionRow, key: &str) -> Ordering {
        match key {
            "project" => a.project.cmp(&b.project),
            "started" => a.started_at.cmp(&b.started_at),
            "updated" => a.updated_at.cmp(&b.updated_at),
            "prompts" => a.prompts.cmp(&b.prompts),
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
        let rows = &self.rows;
        self.ix.sort_by(|a, b| {
            let (ra, rb) = (&rows[*a], &rows[*b]);
            let ord = Self::compare(ra, rb, key);
            let ord = if sort == ColumnSort::Descending { ord.reverse() } else { ord };
            ord.then_with(|| rb.updated_at.cmp(&ra.updated_at))
        });
    }

    fn mono_cell(&self, text: SharedString, color: Hsla, cx: &App) -> AnyElement {
        div().font_family(style::mono(cx)).text_size(px(11.)).whitespace_nowrap().text_color(color).child(text).into_any_element()
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
            .when(is_sorted, |el| el.text_color(style::text()).font_weight(FontWeight::SEMIBOLD))
            .child(column.name.clone())
    }

    fn render_td(&mut self, row_ix: usize, col_ix: usize, _: &mut Window, cx: &mut Context<TableState<Self>>) -> impl IntoElement {
        let row = self.row(row_ix).clone();
        match self.columns[col_ix].key.as_ref() {
            "project" => div().truncate().child(row.project.clone()).into_any_element(),
            "preview" => {
                let sub = self.mode.preview(&row);
                v_flex()
                    .w_full()
                    .h(px(ROW_H))
                    .justify_center()
                    .overflow_hidden()
                    .child(div().truncate().line_height(px(15.)).child(row.title.clone()))
                    .child(div().truncate().text_size(px(11.)).line_height(px(14.)).text_color(style::muted()).child(sub))
                    .into_any_element()
            }
            "started" => self.mono_cell(row.started_fmt.clone(), style::text_secondary(), cx),
            "updated" => self.mono_cell(row.updated_fmt.clone(), style::text(), cx),
            "prompts" => self.mono_cell(row.prompts_fmt.clone(), style::text(), cx),
            "size" => self.mono_cell(row.size_fmt.clone(), style::text_secondary(), cx),
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
}

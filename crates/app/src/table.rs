//! Session table: `DataTable` delegate holding the canonical rows plus a filtered + sorted
//! index vector. Filtering and sorting run on the main thread (10k rows ≈ 7ms).

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Instant;

use agent_session_core::claude::paths::ClaudeDir;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::actions::{self, CopyResume, CopySessionId, ExportTranscript, HideProject, OpenProjectFolder, RevealTranscript};
use crate::favorites::FavoriteSession;
use crate::model::{DisplayMode, Filter, SessionRow};
use agent_session_core::cost::compare;
use crate::search::{RowHit, SearchTarget};
use crate::style::{self, FluentBuilder};

/// Fixed widths from the mockup; Preview takes whatever is left.
pub const PROJECT_W: f32 = 110.;
pub const STARTED_W: f32 = 88.;
pub const UPDATED_W: f32 = 88.;
pub const PROMPTS_W: f32 = 58.;
pub const COST_W: f32 = 62.;
pub const CTX_W: f32 = 44.;
pub const IN_W: f32 = 52.;
/// Cache-read tokens and their share of In.
pub const HIT_W: f32 = 52.;
pub const HIT_PCT_W: f32 = 48.;
pub const OUT_W: f32 = 48.;
pub const SIZE_W: f32 = 60.;
pub const KIND_W: f32 = 46.;
/// Only there while a search is active.
pub const MATCHES_W: f32 = 64.;
/// Every column but Preview and Matches.
#[cfg(test)]
const FIXED_COLS_W: f32 = PROJECT_W + STARTED_W + UPDATED_W + PROMPTS_W + COST_W + CTX_W + IN_W + HIT_W + HIT_PCT_W + OUT_W + SIZE_W + KIND_W;
pub const PREVIEW_MIN_W: f32 = 160.;
pub const ROW_H: f32 = 36.;
/// The row context menu (mockup 6), wide enough for the start of a resume command.
const MENU_W: f32 = 330.;
/// What a menu item leaves for its content: the menu's 4px edge and the item's 8px padding,
/// on both sides.
const MENU_ITEM_W: f32 = MENU_W - 24.;

const PREVIEW_KEY: &str = "preview";
const MATCHES_KEY: &str = "matches";
const DEFAULT_SORT_KEY: &str = "updated";

pub struct SessionTableDelegate {
    pub rows: Vec<Arc<SessionRow>>,
    /// Session id → position in `rows`.
    by_id: HashMap<SharedString, usize>,
    /// Indices into `rows` after filter + sort.
    pub ix: Vec<usize>,
    /// Every column in table order, Matches aside; the header menu hides any but Preview.
    all_columns: Vec<Column>,
    hidden: BTreeSet<SharedString>,
    /// What shows: `all_columns` minus the hidden ones, Matches after Preview while searching.
    columns: Vec<Column>,
    pub mode: DisplayMode,
    filter: Filter,
    /// Hits of the active search by session id; only these sessions are shown while it is `Some`.
    search: Option<HashMap<SharedString, Arc<RowHit>>>,
    sort: Option<(SharedString, ColumnSort)>,
    /// Cost of the rows in `ix` (the status bar shows it).
    pub shown_cost: f64,
    /// Model the Cost tab prices sessions as, while it is open: the status bar then also shows
    /// what the rows in `ix` would cost with it as their main model.
    compare_model: Option<SharedString>,
    projection: Option<Projection>,
    pub last_filter_ms: u128,
    /// The context menu checks whether a favorite's original transcript is still there.
    claude: ClaudeDir,
}

fn col(key: &'static str, name: &'static str, width: f32) -> Column {
    Column::new(key, name).width(px(width)).paddings(Edges { top: px(0.), bottom: px(0.), left: px(6.), right: px(6.) })
}

fn hideable(key: &str) -> bool {
    key != PREVIEW_KEY && key != MATCHES_KEY
}

/// Show or hide a column (the header menu); handled by `AppRoot`, which saves the choice.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, gpui_kit::Action)]
#[action(namespace = app, no_json)]
pub struct ToggleColumn {
    pub key: String,
}

#[derive(Clone, Debug, PartialEq, Default, serde::Deserialize, gpui_kit::Action)]
#[action(namespace = app, no_json)]
pub struct ShowAllColumns;

impl SessionTableDelegate {
    pub fn new(claude: ClaudeDir) -> Self {
        let all_columns = vec![
            col("project", "Project", PROJECT_W).sortable(),
            col("preview", "Preview", 460.).resizable(false),
            col("started", "Started", STARTED_W).sortable(),
            col("updated", "Updated", UPDATED_W).sortable().descending(),
            col("prompts", "Prompts", PROMPTS_W).text_right().sortable(),
            col("cost", "Cost", COST_W).text_right().sortable(),
            col("context", "Ctx", CTX_W).text_right().sortable(),
            col("input", "In", IN_W).text_right().sortable(),
            col("cache_read", "Hit", HIT_W).text_right().sortable(),
            col("cache_hit", "Hit%", HIT_PCT_W).text_right().sortable(),
            col("output", "Out", OUT_W).text_right().sortable(),
            col("size", "Size", SIZE_W).text_right().sortable(),
            col("kind", "Kind", KIND_W).sortable(),
        ];
        Self {
            rows: Vec::new(),
            by_id: HashMap::new(),
            ix: Vec::new(),
            columns: all_columns.clone(),
            all_columns,
            hidden: BTreeSet::new(),
            mode: DisplayMode::FirstPrompt,
            filter: Filter::default(),
            search: None,
            sort: None,
            shown_cost: 0.0,
            compare_model: None,
            projection: None,
            last_filter_ms: 0,
            claude,
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
        if !active && self.sort.as_ref().is_some_and(|(key, _)| key == MATCHES_KEY) {
            self.sort = None;
        }
        self.search = active.then(HashMap::new);
        self.rebuild_columns();
        self.recompute();
    }

    /// Rebuild `columns` after the hidden set or the search changed. The caller must
    /// `TableState::refresh` afterwards.
    fn rebuild_columns(&mut self) {
        let mut columns: Vec<Column> = self.all_columns.iter().filter(|c| !self.hidden.contains(&c.key)).cloned().collect();
        if self.search.is_some() {
            let at = columns.iter().position(|c| c.key == PREVIEW_KEY).map_or(0, |i| i + 1);
            columns.insert(at, col(MATCHES_KEY, "Matches", MATCHES_W).text_right().sortable());
        }
        self.columns = columns;
        self.sync_column_arrows();
    }

    /// Hide these columns and show the rest; Preview, Matches and unknown keys are ignored. A
    /// hidden column cannot stay the sort, so the order goes back to the default. The caller
    /// must `TableState::refresh` afterwards.
    pub fn set_hidden_columns(&mut self, keys: impl IntoIterator<Item = String>) {
        let known = |key: &str| hideable(key) && self.all_columns.iter().any(|c| c.key == key);
        let hidden: BTreeSet<SharedString> = keys.into_iter().filter(|k| known(k)).map(Into::into).collect();
        self.hidden = hidden;
        if self.sort.as_ref().is_some_and(|(key, _)| self.hidden.contains(key)) {
            self.sort = None;
        }
        self.rebuild_columns();
        self.recompute();
    }

    pub fn toggle_column(&mut self, key: &str) {
        let mut hidden = self.hidden_columns();
        match hidden.iter().position(|k| k == key) {
            Some(i) => drop(hidden.remove(i)),
            None => hidden.push(key.to_string()),
        }
        self.set_hidden_columns(hidden);
    }

    /// Sorted keys of the hidden columns.
    pub fn hidden_columns(&self) -> Vec<String> {
        self.hidden.iter().map(|k| k.to_string()).collect()
    }

    /// The header menu: (key, name, shown) of every column it can hide, in table order.
    pub fn column_menu(&self) -> Vec<(SharedString, SharedString, bool)> {
        self.all_columns.iter().filter(|c| hideable(&c.key)).map(|c| (c.key.clone(), c.name.clone(), !self.hidden.contains(&c.key))).collect()
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
        self.columns.iter().filter(|c| c.key != PREVIEW_KEY).map(|c| c.width.as_f32()).sum()
    }

    fn shows(&self, row: &SessionRow) -> bool {
        self.filter.matches(row) && self.search.as_ref().is_none_or(|hits| hits.contains_key(&row.id))
    }

    fn recompute(&mut self) {
        let t = Instant::now();
        self.ix = (0..self.rows.len()).filter(|i| self.shows(&self.rows[*i])).collect();
        self.shown_cost = self.ix.iter().filter_map(|i| self.rows[*i].cost).fold(0.0, |a, b| a + b);
        self.project();
        match self.sort.clone() {
            Some((key, sort)) => self.sort(&key, sort),
            None => self.sort_by_updated_desc(),
        }
        self.last_filter_ms = t.elapsed().as_millis();
    }

    /// Price the shown rows as `model` too (the Cost tab is open), or stop (`None`).
    pub fn set_compare_model(&mut self, model: Option<SharedString>) {
        if self.compare_model != model {
            self.compare_model = model;
            self.project();
        }
    }

    pub fn projection(&self) -> Option<&Projection> {
        self.projection.as_ref()
    }

    /// Sums over the shown rows whose main model has a price, so both sides cover the same calls.
    fn project(&mut self) {
        self.projection = self.compare_model.as_ref().map(|target| {
            let mut p = Projection { target: target.clone(), actual: 0.0, predicted: 0.0 };
            for c in self.ix.iter().filter_map(|i| compare(&self.rows[*i].model_usage, target)) {
                if let Some(actual) = c.actual_total() {
                    p.actual += actual;
                    p.predicted += c.predicted_total();
                }
            }
            p
        });
    }

    pub fn preview_width(&self) -> Pixels {
        self.all_columns.iter().find(|c| c.key == PREVIEW_KEY).map_or(px(PREVIEW_MIN_W), |c| c.width)
    }

    pub fn set_preview_width(&mut self, width: Pixels) {
        for column in self.all_columns.iter_mut().chain(self.columns.iter_mut()).filter(|c| c.key == PREVIEW_KEY) {
            column.width = width;
        }
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
            "input" => a.input_tokens.cmp(&b.input_tokens),
            "cache_read" => a.cache_read_tokens.cmp(&b.cache_read_tokens),
            "cache_hit" => cmp_missing_low(a.cache_hit, b.cache_hit, f32::total_cmp),
            "output" => a.output_tokens.cmp(&b.output_tokens),
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

fn column_menu(mut menu: PopupMenu, entries: &[(SharedString, SharedString, bool)]) -> PopupMenu {
    for (key, name, shown) in entries {
        menu = menu.item(PopupMenuItem::new(name.clone()).checked(*shown).action(Box::new(ToggleColumn { key: key.to_string() })));
    }
    let all_shown = entries.iter().all(|(_, _, shown)| *shown);
    menu.separator().item(PopupMenuItem::new("Show All Columns").action(Box::new(ShowAllColumns)).disabled(all_shown))
}

/// What the shown rows cost, and would cost with `target` as their main model.
#[derive(Clone, Debug, PartialEq)]
pub struct Projection {
    pub target: SharedString,
    pub actual: f64,
    pub predicted: f64,
}

/// A context menu label with its shortcut on the right.
fn menu_line(label: &'static str, shortcut: &'static str) -> Div {
    h_flex().w_full().gap(px(12.)).items_center().justify_between().child(label).child(div().text_size(px(11.)).text_color(style::muted()).child(shortcut))
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

    /// The column name; a right click opens the menu that shows and hides columns.
    fn render_th(&mut self, col_ix: usize, _: &mut Window, cx: &mut Context<TableState<Self>>) -> impl IntoElement {
        let column = &self.columns[col_ix];
        let is_sorted = self.sort.as_ref().map(|(k, _)| *k == column.key).unwrap_or(column.key == DEFAULT_SORT_KEY);
        let entries = self.column_menu();
        div()
            .id(SharedString::from(format!("th-{}", column.key)))
            .size_full()
            .flex()
            .items_center()
            .text_size(px(11.))
            .when(column.align == TextAlign::Right, |el| el.justify_end())
            .when(is_sorted, |el| el.text_color(style::text()).font_weight(FontWeight::SEMIBOLD))
            // the table's own menu (for rows) would otherwise open on the row right-clicked last
            .on_mouse_down(MouseButton::Right, cx.listener(|table, _: &MouseDownEvent, _, cx| table.set_right_clicked_row(None, cx)))
            .child(column.name.clone())
            .context_menu(move |menu, _, _| column_menu(menu, &entries))
    }

    /// The actions of the right-clicked row, which need not be the selected one; greyed out as
    /// in the detail header. Items with an id find no key binding by themselves (a binding is
    /// for `id: None`), so the ones with a shortcut draw its hint.
    fn context_menu(&mut self, row_ix: usize, menu: PopupMenu, _: &mut Window, cx: &mut Context<TableState<Self>>) -> PopupMenu {
        let row = self.row(row_ix).clone();
        let a = actions::check(&row, &self.claude);
        let id = || Some(row.id.to_string());
        let favorite = if row.favorite { "Unfavorite" } else { "Favorite" };
        // what Copy Resume Command copies, or why it cannot
        let preview: SharedString = a.resume.clone().map(Into::into).or(a.copy_resume.err().map(Into::into)).unwrap_or_default();
        let mono = style::mono(cx);
        menu.min_w(px(MENU_W))
            .max_w(px(MENU_W))
            .item(PopupMenuItem::element(move |_, _| menu_line(favorite, "⌘D")).action(Box::new(FavoriteSession { id: row.id.to_string() })).disabled(a.favorite.is_err()))
            .separator()
            .item(PopupMenuItem::element(|_, _| menu_line("Copy Session ID", "⌘C")).action(Box::new(CopySessionId { id: id() })).disabled(a.copy_id.is_err()))
            .item(
                PopupMenuItem::element(move |_, _| {
                    // a fixed width: the item's own wrapper would grow to the whole command
                    v_flex()
                        .w(px(MENU_ITEM_W))
                        .overflow_hidden()
                        .py(px(3.))
                        .child(menu_line("Copy Resume Command", "⇧⌘C"))
                        .child(div().truncate().font_family(mono.clone()).text_size(px(10.)).line_height(px(13.)).text_color(style::muted()).child(preview.clone()))
                })
                .action(Box::new(CopyResume { id: id() }))
                .disabled(a.copy_resume.is_err()),
            )
            .separator()
            .menu_with_disabled("Reveal Transcript in Finder", Box::new(RevealTranscript { id: id() }), a.reveal.is_err())
            .menu_with_disabled("Open Project Folder", Box::new(OpenProjectFolder { id: id() }), a.open_folder.is_err())
            .menu_with_disabled("Export Transcript as TXT…", Box::new(ExportTranscript { id: id() }), a.export.is_err())
            .separator()
            .menu(format!("Hide Project “{}”", row.project), Box::new(HideProject { id: id() }))
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
            "input" => self.mono_cell(row.input_fmt.clone(), style::text_secondary(), right, cx),
            "cache_read" => self.mono_cell(row.cache_read_fmt.clone(), style::text_secondary(), right, cx),
            "cache_hit" => self.mono_cell(row.cache_hit_fmt.clone(), style::text_secondary(), right, cx),
            "output" => self.mono_cell(row.output_fmt.clone(), style::text_secondary(), right, cx),
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

    use crate::model::tests::{builtin_row, row, summary};
    use crate::model::{Kind, KindSet};

    fn delegate() -> SessionTableDelegate {
        SessionTableDelegate::new(ClaudeDir::new("/nonexistent"))
    }

    fn ids(d: &SessionTableDelegate) -> Vec<&str> {
        (0..d.ix.len()).map(|i| d.row(i).id.as_ref()).collect()
    }

    fn with_prompts(id: &str, project: &str, updated_at: i64, prompts: u32) -> Arc<SessionRow> {
        let mut s = summary(id, project, updated_at);
        s.user_msg_count = prompts;
        Arc::new(builtin_row(&s))
    }

    #[test]
    fn rows_show_newest_first_by_default() {
        let mut d = delegate();
        d.set_rows(vec![row("a", "/p/x", 100), row("c", "/p/x", 300), row("b", "/p/y", 200)]);
        assert_eq!(ids(&d), ["c", "b", "a"]);
        assert_eq!(d.position_of("b"), Some(1));
        assert_eq!(d.position_of("zzz"), None);
    }

    #[test]
    fn streamed_rows_are_inserted_in_order_and_changed_rows_replace_the_old_ones() {
        let mut d = delegate();
        d.set_rows(vec![row("a", "/p/x", 100), row("b", "/p/x", 200)]);
        d.upsert(vec![row("c", "/p/x", 150), row("a", "/p/x", 400)]);
        assert_eq!(ids(&d), ["a", "b", "c"], "a moved to the top after its update, the new c is the oldest");
        assert_eq!(d.rows.len(), 3, "a was replaced, not duplicated");
    }

    #[test]
    fn the_filter_and_the_sort_survive_new_data() {
        let mut d = delegate();
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
        Arc::new(builtin_row(&s))
    }

    #[test]
    fn the_kind_filter_survives_new_data() {
        let mut d = delegate();
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
        let mut d = delegate();
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
        let mut d = delegate();
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
        let mut d = delegate();
        d.set_rows(vec![row("a", "/p/x", 100), row("b", "/p/y", 200), row("c", "/p/x", 300), history("h", "/p/x", 400)]);
        d.set_filter(Filter { project: Some("-p-x".into()), ..Default::default() });
        d.set_search(true);
        let targets = d.search_targets();
        let got: Vec<(&str, bool)> = targets.iter().map(|t| (t.session_id.as_str(), t.path.is_some())).collect();
        assert_eq!(got, [("h", false), ("c", true), ("a", true)], "hits do not narrow the targets; history has no transcript");
    }

    #[test]
    fn a_hidden_project_stays_hidden_when_new_sessions_of_it_arrive() {
        let mut d = delegate();
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
        let mut d = delegate();
        d.set_rows(vec![with_prompts("old", "/p/x", 100, 2), with_prompts("new", "/p/x", 300, 2), with_prompts("top", "/p/x", 200, 8)]);
        d.set_sort("prompts", ColumnSort::Ascending);
        assert_eq!(ids(&d), ["new", "old", "top"]);
    }

    #[test]
    fn header_arrows_follow_the_sort_state() {
        let arrows = |d: &SessionTableDelegate| -> Vec<(String, ColumnSort)> {
            d.columns.iter().filter_map(|c| c.sort.map(|s| (c.key.to_string(), s))).filter(|(_, s)| *s != ColumnSort::Default).collect()
        };
        let mut d = delegate();
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
        Arc::new(builtin_row(&s))
    }

    #[test]
    fn cost_context_and_token_columns_sit_after_prompts() {
        let d = delegate();
        assert_eq!(keys(&d), ["project", "preview", "started", "updated", "prompts", "cost", "context", "input", "cache_read", "cache_hit", "output", "size", "kind"]);
        assert_eq!(FIXED_COLS_W, PROJECT_W + STARTED_W + UPDATED_W + PROMPTS_W + COST_W + CTX_W + IN_W + HIT_W + HIT_PCT_W + OUT_W + SIZE_W + KIND_W);
    }

    fn hide(d: &mut SessionTableDelegate, keys: &[&str]) {
        d.set_hidden_columns(keys.iter().map(|k| k.to_string()));
    }

    #[test]
    fn hidden_columns_leave_the_table_and_give_preview_their_width() {
        let mut d = delegate();
        hide(&mut d, &["started", "kind"]);
        assert_eq!(keys(&d), ["project", "preview", "updated", "prompts", "cost", "context", "input", "cache_read", "cache_hit", "output", "size"]);
        assert_eq!(d.fixed_cols_width(), FIXED_COLS_W - STARTED_W - KIND_W);
        assert_eq!(d.hidden_columns(), ["kind", "started"]);
        d.toggle_column("started");
        assert_eq!(keys(&d)[2], "started", "shown again in its own place");
        assert_eq!(d.hidden_columns(), ["kind"]);
    }

    #[test]
    fn preview_and_matches_stay_and_unknown_keys_are_ignored() {
        let mut d = delegate();
        hide(&mut d, &["preview", "matches", "nope"]);
        assert!(d.hidden_columns().is_empty());
        hide(&mut d, &["project"]);
        d.set_search(true);
        assert_eq!(keys(&d)[..2], ["preview", "matches"], "Matches still follows Preview");
        assert_eq!(d.fixed_cols_width(), FIXED_COLS_W - PROJECT_W + MATCHES_W);
        d.set_search(false);
        assert_eq!(keys(&d)[..2], ["preview", "started"]);
        d.set_search(true);
        hide(&mut d, &[]);
        assert_eq!(keys(&d)[..3], ["project", "preview", "matches"], "showing a column keeps the Matches column");
    }

    #[test]
    fn hiding_the_sorted_column_goes_back_to_the_default_order() {
        let mut d = delegate();
        d.set_rows(vec![with_prompts("a", "/p/x", 100, 9), with_prompts("b", "/p/x", 200, 1), with_prompts("c", "/p/x", 300, 5)]);
        d.set_sort("prompts", ColumnSort::Descending);
        assert_eq!(ids(&d), ["a", "c", "b"]);
        d.toggle_column("prompts");
        assert_eq!(d.sort_state(), None);
        assert_eq!(ids(&d), ["c", "b", "a"], "Updated, newest first");
        d.set_sort("prompts", ColumnSort::Ascending);
        assert_eq!(d.sort_state(), None, "a hidden column cannot be sorted by");
        d.set_sort("cost", ColumnSort::Ascending);
        d.toggle_column("size");
        assert_eq!(d.sort_state(), Some(("cost".into(), ColumnSort::Ascending)), "hiding another column keeps the sort");
    }

    #[test]
    fn the_column_menu_lists_every_column_but_preview_with_whether_it_shows() {
        let mut d = delegate();
        d.toggle_column("size");
        let menu: Vec<(String, bool)> = d.column_menu().into_iter().map(|(_, name, shown)| (name.to_string(), shown)).collect();
        let names: Vec<&str> = menu.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Project", "Started", "Updated", "Prompts", "Cost", "Ctx", "In", "Hit", "Hit%", "Out", "Size", "Kind"]);
        assert_eq!(menu.iter().filter(|(_, shown)| !shown).map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["Size"]);
    }

    #[test]
    fn the_preview_width_survives_hiding_and_showing_columns() {
        let mut d = delegate();
        d.set_preview_width(px(300.));
        d.toggle_column("size");
        d.set_search(true);
        d.toggle_column("size");
        assert_eq!(d.preview_width(), px(300.));
    }

    fn cached(id: &str, updated_at: i64, read: u64, other_input: u64) -> Arc<SessionRow> {
        let mut s = summary(id, "/p/x", updated_at);
        (s.tokens_cache_read, s.tokens_cache_write) = (read, other_input);
        Arc::new(builtin_row(&s))
    }

    #[test]
    fn hit_sorts_by_cache_reads_and_hit_percent_by_share_with_rows_without_input_last() {
        let mut d = delegate();
        d.set_rows(vec![cached("a", 100, 900, 100), cached("b", 200, 0, 0), cached("c", 300, 5_000, 5_000), cached("d", 400, 0, 10)]);
        d.set_sort("cache_read", ColumnSort::Descending);
        assert_eq!(ids(&d), ["c", "a", "d", "b"]);
        d.set_sort("cache_hit", ColumnSort::Descending);
        assert_eq!(ids(&d), ["a", "c", "d", "b"], "90%, 50%, 0%, then the session without input");
        d.set_sort("cache_hit", ColumnSort::Ascending);
        assert_eq!(ids(&d), ["b", "d", "c", "a"], "a missing share sorts below 0%");
    }

    fn tokens(id: &str, updated_at: i64, input: u64, output: u64) -> Arc<SessionRow> {
        let mut s = summary(id, "/p/x", updated_at);
        (s.tokens_cache_read, s.tokens_out) = (input, output);
        Arc::new(builtin_row(&s))
    }

    #[test]
    fn in_and_out_sort_by_token_counts() {
        let mut d = delegate();
        d.set_rows(vec![tokens("a", 100, 5_000_000, 10), tokens("b", 200, 0, 0), tokens("c", 300, 9_000_000, 5)]);
        d.set_sort("input", ColumnSort::Descending);
        assert_eq!(ids(&d), ["c", "a", "b"]);
        d.set_sort("output", ColumnSort::Descending);
        assert_eq!(ids(&d), ["a", "c", "b"]);
    }

    fn on_model(id: &str, project: &str, model: &str, output: u64) -> Arc<SessionRow> {
        use agent_session_core::model::{ModelUsage, Tokens};
        let mut s = summary(id, project, 100);
        let tokens = Tokens { output, ..Tokens::default() };
        s.model_usage = vec![ModelUsage { model: model.into(), fast: false, long_prompt: false, responses: 1, tokens, cost_usd: None }];
        Arc::new(builtin_row(&s))
    }

    #[test]
    fn the_projection_prices_the_shown_rows_as_the_compared_model() {
        let mut d = delegate();
        d.set_rows(vec![on_model("a", "/p/x", "claude-opus-5-5", 1_000_000), on_model("b", "/p/y", "claude-opus-5-5", 2_000_000), on_model("c", "/p/x", "claude-opus-9", 1)]);
        assert_eq!(d.projection(), None, "only while the Cost tab is open");
        d.set_compare_model(Some("claude-fable-5-2".into()));
        let p = d.projection().unwrap();
        assert_eq!((p.target.as_ref(), p.actual, p.predicted), ("claude-fable-5-2", 60.0, 120.0), "an unpriced main model is left out of both");
        d.set_filter(Filter { project: Some("-p-x".into()), ..Default::default() });
        assert_eq!(d.projection().map(|p| (p.actual, p.predicted)), Some((20.0, 40.0)), "the filter narrows it");
        d.set_compare_model(None);
        assert_eq!(d.projection(), None);
    }

    #[test]
    fn cost_and_context_sort_the_most_first_and_rows_without_data_last() {
        let mut d = delegate();
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
        let mut d = delegate();
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

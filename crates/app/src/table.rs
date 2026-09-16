//! Session table: `DataTable` delegate holding the canonical rows plus a filtered + sorted
//! index vector. Filtering and sorting run on the main thread (10k rows ≈ 7ms).

use std::cmp::Ordering;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::component::{h_flex, v_flex};
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

pub struct SessionTableDelegate {
    pub rows: Vec<Arc<SessionRow>>,
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
    pub fn new(rows: Vec<Arc<SessionRow>>) -> Self {
        let columns = vec![
            col("project", "Project", PROJECT_W).sortable(),
            col("preview", "Preview", 460.).resizable(false),
            col("started", "Started", STARTED_W).sortable(),
            col("updated", "Updated", UPDATED_W).sortable().descending(),
            col("prompts", "Prompts", PROMPTS_W).text_right().sortable(),
            col("size", "Size", SIZE_W).text_right().sortable(),
            col("kind", "Kind", KIND_W).sortable(),
        ];
        let mut this = Self {
            ix: (0..rows.len()).collect(),
            rows,
            columns,
            mode: DisplayMode::FirstPrompt,
            filter: Filter::default(),
            sort: None,
            last_filter_ms: 0,
        };
        this.sort_by_updated_desc();
        this
    }

    pub fn row(&self, row_ix: usize) -> &Arc<SessionRow> {
        &self.rows[self.ix[row_ix]]
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
        let t = Instant::now();
        self.ix = (0..self.rows.len()).filter(|i| self.filter.matches(&self.rows[*i])).collect();
        match self.sort.clone() {
            Some((key, sort)) => self.sort(&key, sort),
            None => self.sort_by_updated_desc(),
        }
        self.last_filter_ms = t.elapsed().as_millis();
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

    fn sort(&mut self, key: &str, sort: ColumnSort) {
        if sort == ColumnSort::Default {
            self.sort = None;
            self.sort_by_updated_desc();
            return;
        }
        self.sort = Some((key.to_string().into(), sort));
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
        let t = Instant::now();
        self.sort(&key, sort);
        println!("sort {key} {sort:?}: {:?}", t.elapsed());
        cx.notify();
    }

    fn render_th(&mut self, col_ix: usize, _: &mut Window, _: &mut Context<TableState<Self>>) -> impl IntoElement {
        let column = &self.columns[col_ix];
        let is_sorted = self.sort.as_ref().map(|(k, _)| *k == column.key).unwrap_or(col_ix == 3 && self.sort.is_none());
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
                    .child(
                        h_flex()
                            .gap(px(4.))
                            .line_height(px(15.))
                            .overflow_hidden()
                            .child(
                                div()
                                    .flex_none()
                                    .font_family(style::mono(cx))
                                    .text_size(px(11.))
                                    .text_color(style::muted())
                                    .child(row.num_fmt.clone()),
                            )
                            .child(div().truncate().child(row.title.clone())),
                    )
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

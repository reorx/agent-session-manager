//! M0 spike: prove the gpui-kit stack works for this app.
//!
//! - `DataTable` with 10,000 fake session rows, sortable headers, two-line preview cell
//! - `h_resizable` main/detail split, `TabBar::segmented`, range `DatePicker`, `StatusBar`
//! - Set `ASM_SPIKE_AUTOPLAY=1` to scroll / select / sort automatically after launch
//!   (used to take screenshots without GUI automation).

use std::cmp::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Days, Local, TimeZone};
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState, DateRangePreset};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::table::{Column, ColumnSort, DataTable, TableDelegate, TableEvent, TableState};
use gpui_kit::component::{ActiveTheme, Root, Sizable, h_flex, h_resizable, resizable_panel, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

gpui_kit::actions!(app, [Quit, ToggleDetail]);

const ROW_COUNT: usize = 10_000;
const PROJECTS: [&str; 12] = [
    "agent-session-manager",
    "scripts",
    "agent-sessions",
    "ideachat",
    "dotfiles",
    "blog",
    "herdr",
    "zed",
    "gpui-kit",
    "notes",
    "infra",
    "playground",
];
const BRANCHES: [&str; 6] = ["main", "master", "feat/table", "fix/parser", "wip", ""];
const PROMPTS: [&str; 8] = [
    "开始进行 PRD 的 M0：工具链准备和 gpui-kit spike",
    "Fix the JSONL parser so tool_result blocks with nested arrays are handled",
    "Why does the FTS5 trigram tokenizer miss two-character CJK queries?",
    "Refactor SessionStore so filtering runs on the main thread in under a frame",
    "帮我看看 history.jsonl 里没有 sessionId 的旧记录应该怎么分组",
    "Add a Kind column with SDK / BG badges and make it sortable",
    "Write BDD tests for asm list against the Python script output",
    "Set up cargo-bundle and produce a .app with an icon",
];
const REPLIES: [&str; 4] = [
    "Done. The parser now borrows the raw line and only allocates for text blocks.",
    "I found the cause: the trigram tokenizer needs at least three code points.",
    "Applied the change and ran the tests; 42 passed.",
    "Here is the plan: first the delegate, then the store, then the toolbar.",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Cli,
    Sdk,
    Background,
    History,
}

impl Kind {
    fn badge(self) -> Option<&'static str> {
        match self {
            Kind::Cli => None,
            Kind::Sdk => Some("SDK"),
            Kind::Background => Some("BG"),
            Kind::History => Some("History"),
        }
    }
}

/// One fake session. Display strings are pre-formatted so `render_td` does no work.
struct SessionRow {
    short_id: SharedString,
    project: SharedString,
    title: SharedString,
    first_prompt: SharedString,
    last_prompt: SharedString,
    last_message: SharedString,
    started_at: i64,
    updated_at: i64,
    started_fmt: SharedString,
    updated_fmt: SharedString,
    prompts: u32,
    prompts_fmt: SharedString,
    size: u64,
    size_fmt: SharedString,
    cost: f64,
    cost_fmt: SharedString,
    branch: SharedString,
    tools: u32,
    tools_fmt: SharedString,
    kind: Kind,
}

/// Tiny deterministic PRNG so the fake data is stable between runs.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn format_ts(ms: i64) -> SharedString {
    let dt = Local.timestamp_millis_opt(ms).single().unwrap_or_else(Local::now);
    dt.format("%m-%d %H:%M").to_string().into()
}

fn format_size(bytes: u64) -> SharedString {
    let kb = bytes as f64 / 1024.0;
    if kb < 1024.0 {
        format!("{kb:.0} KB").into()
    } else {
        format!("{:.1} MB", kb / 1024.0).into()
    }
}

fn generate_rows(n: usize) -> Vec<Arc<SessionRow>> {
    let mut rng = Lcg(0x5eed);
    let now = Local::now().timestamp_millis();
    let ninety_days: u64 = 90 * 24 * 3600 * 1000;
    (0..n)
        .map(|i| {
            let updated_at = now - rng.below(ninety_days) as i64;
            let started_at = updated_at - rng.below(6 * 3600 * 1000) as i64;
            let prompts = rng.below(60) as u32 + 1;
            let size = rng.below(30 * 1024 * 1024) + 2048;
            let cost = rng.below(4000) as f64 / 100.0;
            let tools = rng.below(400) as u32;
            let kind = match rng.below(20) {
                0..=13 => Kind::Cli,
                14..=16 => Kind::Sdk,
                17..=18 => Kind::Background,
                _ => Kind::History,
            };
            let prompt = PROMPTS[rng.below(PROMPTS.len() as u64) as usize];
            let last_prompt = PROMPTS[rng.below(PROMPTS.len() as u64) as usize];
            let reply = REPLIES[rng.below(REPLIES.len() as u64) as usize];
            let id = format!("{:08x}-{:04x}-4{:03x}", rng.next() as u32, rng.below(0xffff), rng.below(0xfff));
            Arc::new(SessionRow {
                short_id: id[..8].to_string().into(),
                project: PROJECTS[rng.below(PROJECTS.len() as u64) as usize].into(),
                title: format!("#{i} {prompt}").into(),
                first_prompt: prompt.into(),
                last_prompt: last_prompt.into(),
                last_message: reply.into(),
                started_at,
                updated_at,
                started_fmt: format_ts(started_at),
                updated_fmt: format_ts(updated_at),
                prompts,
                prompts_fmt: prompts.to_string().into(),
                size,
                size_fmt: format_size(size),
                cost,
                cost_fmt: if cost > 0.0 { format!("${cost:.2}").into() } else { "".into() },
                branch: BRANCHES[rng.below(BRANCHES.len() as u64) as usize].into(),
                tools,
                tools_fmt: tools.to_string().into(),
                kind,
            })
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DisplayMode {
    Title,
    FirstPrompt,
    LastPrompt,
    LastMessage,
}

impl DisplayMode {
    const ALL: [DisplayMode; 4] = [
        DisplayMode::Title,
        DisplayMode::FirstPrompt,
        DisplayMode::LastPrompt,
        DisplayMode::LastMessage,
    ];

    fn label(self) -> &'static str {
        match self {
            DisplayMode::Title => "Title",
            DisplayMode::FirstPrompt => "First prompt",
            DisplayMode::LastPrompt => "Last prompt",
            DisplayMode::LastMessage => "Last message",
        }
    }
}

/// Table delegate: owns the canonical rows plus a filtered/sorted index vector.
struct SessionTableDelegate {
    rows: Vec<Arc<SessionRow>>,
    ix: Vec<usize>,
    columns: Vec<Column>,
    mode: DisplayMode,
}

impl SessionTableDelegate {
    fn new(rows: Vec<Arc<SessionRow>>) -> Self {
        let columns = vec![
            Column::new("project", "Project").width(px(170.)).sortable(),
            Column::new("preview", "Preview").width(px(460.)),
            Column::new("started", "Started").width(px(100.)).sortable(),
            Column::new("updated", "Updated").width(px(100.)).sortable().descending(),
            Column::new("prompts", "Prompts").width(px(80.)).text_right().sortable(),
            Column::new("size", "Size").width(px(90.)).text_right().sortable(),
            Column::new("cost", "Cost").width(px(80.)).text_right().sortable(),
            Column::new("branch", "Branch").width(px(120.)).sortable(),
            Column::new("tools", "Tools").width(px(70.)).text_right().sortable(),
            Column::new("kind", "Kind").width(px(90.)).sortable(),
            Column::new("id", "ID").width(px(100.)),
        ];
        let mut this = Self { ix: (0..rows.len()).collect(), rows, columns, mode: DisplayMode::FirstPrompt };
        this.sort_by_updated_desc();
        this
    }

    fn sort_by_updated_desc(&mut self) {
        let rows = &self.rows;
        self.ix.sort_by(|a, b| rows[*b].updated_at.cmp(&rows[*a].updated_at));
    }

    fn row(&self, row_ix: usize) -> &Arc<SessionRow> {
        &self.rows[self.ix[row_ix]]
    }

    fn compare(a: &SessionRow, b: &SessionRow, key: &str) -> Ordering {
        match key {
            "project" => a.project.cmp(&b.project),
            "started" => a.started_at.cmp(&b.started_at),
            "updated" => a.updated_at.cmp(&b.updated_at),
            "prompts" => a.prompts.cmp(&b.prompts),
            "size" => a.size.cmp(&b.size),
            "cost" => a.cost.total_cmp(&b.cost),
            "branch" => a.branch.cmp(&b.branch),
            "tools" => a.tools.cmp(&b.tools),
            "kind" => (a.kind as u8).cmp(&(b.kind as u8)),
            _ => Ordering::Equal,
        }
    }

    fn sort(&mut self, key: &str, sort: ColumnSort) {
        if sort == ColumnSort::Default {
            self.sort_by_updated_desc();
            return;
        }
        let rows = &self.rows;
        self.ix.sort_by(|a, b| {
            let (ra, rb) = (&rows[*a], &rows[*b]);
            let ord = Self::compare(ra, rb, key);
            let ord = if sort == ColumnSort::Descending { ord.reverse() } else { ord };
            ord.then_with(|| rb.updated_at.cmp(&ra.updated_at))
        });
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

    fn render_td(&mut self, row_ix: usize, col_ix: usize, _: &mut Window, cx: &mut Context<TableState<Self>>) -> impl IntoElement {
        let row = self.row(row_ix).clone();
        let muted = cx.theme().muted_foreground;
        match self.columns[col_ix].key.as_ref() {
            "project" => div().truncate().child(row.project.clone()).into_any_element(),
            "preview" => {
                let preview = match self.mode {
                    DisplayMode::Title => row.title.clone(),
                    DisplayMode::FirstPrompt => row.first_prompt.clone(),
                    DisplayMode::LastPrompt => row.last_prompt.clone(),
                    DisplayMode::LastMessage => row.last_message.clone(),
                };
                v_flex()
                    .w_full()
                    .overflow_hidden()
                    .child(div().truncate().child(row.title.clone()))
                    .child(div().truncate().text_xs().text_color(muted).child(preview))
                    .into_any_element()
            }
            "started" => div().child(row.started_fmt.clone()).into_any_element(),
            "updated" => div().child(row.updated_fmt.clone()).into_any_element(),
            "prompts" => div().child(row.prompts_fmt.clone()).into_any_element(),
            "size" => div().child(row.size_fmt.clone()).into_any_element(),
            "cost" => div().child(row.cost_fmt.clone()).into_any_element(),
            "branch" => div().truncate().text_color(muted).child(row.branch.clone()).into_any_element(),
            "tools" => div().child(row.tools_fmt.clone()).into_any_element(),
            "kind" => match row.kind.badge() {
                Some(label) => div()
                    .px_1p5()
                    .rounded_sm()
                    .text_xs()
                    .bg(cx.theme().secondary)
                    .text_color(cx.theme().secondary_foreground)
                    .child(label)
                    .into_any_element(),
                None => div().into_any_element(),
            },
            "id" => div().font_family("Menlo").text_xs().text_color(muted).child(row.short_id.clone()).into_any_element(),
            _ => div().into_any_element(),
        }
    }
}

struct AppRoot {
    table: Entity<TableState<SessionTableDelegate>>,
    date_picker: Entity<DatePickerState>,
    scope_ix: usize,
    mode_ix: usize,
    selected: Option<Arc<SessionRow>>,
    detail_visible: bool,
    status: SharedString,
    build_ms: u128,
    _subscriptions: Vec<Subscription>,
    _autoplay: Option<Task<()>>,
}

impl AppRoot {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let t = Instant::now();
        let rows = generate_rows(ROW_COUNT);
        let gen_ms = t.elapsed().as_millis();
        let table = cx.new(|cx| TableState::new(SessionTableDelegate::new(rows), window, cx));
        let build_ms = t.elapsed().as_millis();
        println!("generated {ROW_COUNT} rows in {gen_ms}ms, table ready in {build_ms}ms");

        let date_picker = cx.new(|cx| DatePickerState::range(window, cx));

        let mut subscriptions = vec![
            cx.subscribe_in(&table, window, Self::on_table_event),
            cx.subscribe_in(&date_picker, window, |this, _, ev: &DatePickerEvent, _, cx| {
                let DatePickerEvent::Change(date) = ev;
                this.status = format!("date range: {date}").into();
                cx.notify();
            }),
        ];
        subscriptions.shrink_to_fit();

        let mut this = Self {
            table,
            date_picker,
            scope_ix: 2,
            mode_ix: 1,
            selected: None,
            detail_visible: true,
            status: "idle".into(),
            build_ms,
            _subscriptions: subscriptions,
            _autoplay: None,
        };
        if std::env::var_os("ASM_SPIKE_AUTOPLAY").is_some() {
            this._autoplay = Some(this.autoplay(window, cx));
        }
        this
    }

    fn on_table_event(&mut self, table: &Entity<TableState<SessionTableDelegate>>, ev: &TableEvent, _: &mut Window, cx: &mut Context<Self>) {
        match ev {
            TableEvent::SelectRow(ix) => {
                self.selected = Some(table.read(cx).delegate().row(*ix).clone());
                self.status = format!("selected row {ix}").into();
                cx.notify();
            }
            TableEvent::DoubleClickedRow(ix) => {
                self.status = format!("double-clicked row {ix}").into();
                self.detail_visible = true;
                cx.notify();
            }
            TableEvent::ColumnWidthsChanged(widths) => {
                self.status = format!("column widths: {} cols", widths.len()).into();
                cx.notify();
            }
            _ => {}
        }
    }

    /// Scripted interaction for screenshots: scroll deep, select a row, then sort by Prompts.
    fn autoplay(&self, window: &Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            this.update_in(cx, |this, _, cx| {
                this.table.update(cx, |table, cx| {
                    table.scroll_to_row(5_000, cx);
                    table.set_selected_row(5_000, cx);
                });
                this.status = "autoplay: scrolled to row 5000".into();
                cx.notify();
            })
            .ok();
            cx.background_executor().timer(Duration::from_secs(3)).await;
            this.update_in(cx, |this, window, cx| {
                this.table.update(cx, |table, cx| {
                    table.delegate_mut().perform_sort(4, ColumnSort::Descending, window, cx);
                    table.scroll_to_row(0, cx);
                    table.set_selected_row(0, cx);
                });
                this.status = "autoplay: sorted by Prompts desc".into();
                cx.notify();
            })
            .ok();
        })
    }

    fn set_mode(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.mode_ix = ix;
        self.table.update(cx, |table, cx| {
            table.delegate_mut().mode = DisplayMode::ALL[ix];
            cx.notify();
        });
        cx.notify();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let today = Local::now().date_naive();
        let presets = vec![
            DateRangePreset::range("Today", today, today),
            DateRangePreset::range("3 days", today.checked_sub_days(Days::new(2)).unwrap_or(today), today),
            DateRangePreset::range("7 days", today.checked_sub_days(Days::new(6)).unwrap_or(today), today),
            DateRangePreset::range("30 days", today.checked_sub_days(Days::new(29)).unwrap_or(today), today),
        ];
        h_flex()
            .w_full()
            .gap_3()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                TabBar::new("scope")
                    .segmented()
                    .small()
                    .selected_index(self.scope_ix)
                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                        this.scope_ix = *ix;
                        cx.notify();
                    }))
                    .children(["User", "Assistant", "Both", "Tools"].map(|l| Tab::new().label(l))),
            )
            .child(DatePicker::new(&self.date_picker).placeholder("Updated range").cleanable(true).presets(presets).small().w(px(240.)))
            .child(div().flex_1())
            .child(
                TabBar::new("mode")
                    .segmented()
                    .small()
                    .selected_index(self.mode_ix)
                    .on_click(cx.listener(|this, ix: &usize, _, cx| this.set_mode(*ix, cx)))
                    .children(DisplayMode::ALL.map(|m| Tab::new().label(m.label()))),
            )
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let field = |label: &'static str, value: SharedString| {
            h_flex()
                .gap_2()
                .text_sm()
                .child(div().w(px(72.)).flex_shrink_0().text_color(muted).child(label))
                .child(div().flex_1().truncate().child(value))
        };
        v_flex()
            .size_full()
            .p_3()
            .gap_2()
            .border_l_1()
            .border_color(cx.theme().border)
            .when_some(self.selected.clone(), |el, row| {
                el.child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(row.title.clone()))
                    .child(field("Project", row.project.clone()))
                    .child(field("ID", row.short_id.clone()))
                    .child(field("Started", row.started_fmt.clone()))
                    .child(field("Updated", row.updated_fmt.clone()))
                    .child(field("Prompts", row.prompts_fmt.clone()))
                    .child(field("Size", row.size_fmt.clone()))
                    .child(field("Cost", row.cost_fmt.clone()))
                    .child(field("Branch", row.branch.clone()))
                    .child(field("Tools", row.tools_fmt.clone()))
                    .child(div().mt_2().text_sm().text_color(muted).child("First prompt"))
                    .child(div().text_sm().child(row.first_prompt.clone()))
                    .child(div().mt_2().text_sm().text_color(muted).child("Last message"))
                    .child(div().text_sm().child(row.last_message.clone()))
            })
            .when(self.selected.is_none(), |el| {
                el.items_center().justify_center().text_color(muted).child("Select a session")
            })
    }
}

impl Render for AppRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let total = self.table.read(cx).delegate().rows.len();
        let shown = self.table.read(cx).delegate().ix.len();
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &ToggleDetail, _, cx| {
                this.detail_visible = !this.detail_visible;
                cx.notify();
            }))
            .child(self.render_toolbar(cx))
            .child(
                div().flex_1().min_h_0().w_full().child(
                    h_resizable("main")
                        .child(
                            resizable_panel()
                                .size(px(960.))
                                .size_range(px(480.)..px(2400.))
                                .child(DataTable::new(&self.table).stripe(true).bordered(false).with_size(px(48.))),
                        )
                        .child(
                            resizable_panel()
                                .size_range(px(240.)..px(1200.))
                                .visible(self.detail_visible)
                                .child(self.render_detail(cx)),
                        ),
                ),
            )
            .child(
                StatusBar::new()
                    .left(format!("{shown} / {total} sessions"))
                    .child(self.status.clone())
                    .right(format!("table built in {}ms", self.build_ms)),
            )
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}

fn main() {
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    app.run(move |cx| {
        gpui_kit::init(cx);
        cx.activate(true);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None), KeyBinding::new("cmd-\\", ToggleDetail, None)]);
        cx.set_menus([
            Menu::new("Agent Session Manager").items(vec![MenuItem::action("Quit", Quit)]),
            Menu::new("View").items(vec![MenuItem::action("Toggle Detail Panel", ToggleDetail)]),
        ]);

        let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("Agent Session Manager".into()), ..Default::default() }),
            ..Default::default()
        };
        cx.spawn(async move |cx| {
            cx.open_window(options, |window, cx| {
                let view = cx.new(|cx| AppRoot::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");
        })
        .detach();
    });
}

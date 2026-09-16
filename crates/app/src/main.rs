//! Agent Session Manager — macOS desktop app listing Claude Code sessions.
//!
//! Layout follows the Claude Design mockup: toolbar (dropdown filters + search), session table,
//! detail panel with Metadata / Conversation tabs, status bar. Data is still fake (PRD M1 adds
//! the real parser); set `ASM_SPIKE_AUTOPLAY=1` to scroll / select / filter / sort automatically
//! after launch (used by `make screenshot`).

mod detail;
mod fake;
mod model;
mod style;
mod table;
mod toolbar;

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::table::{ColumnSort, DataTable, TableDelegate, TableEvent, TableState};
use gpui_kit::component::{ResizableState, Root, Sizable, h_resizable, resizable_panel, v_flex};
use gpui_kit::*;

use crate::detail::{DETAIL_W, DetailState, DetailTab};
use crate::model::{DisplayMode, RangePreset, ScopeSet, SessionRow};
use crate::table::{FIXED_COLS_W, PREVIEW_MIN_W, ROW_H, SessionTableDelegate};

gpui_kit::actions!(app, [Quit, ToggleDetail, FocusSearch, Preview1, Preview2, Preview3, Preview4]);

const WINDOW_W: f32 = 1440.;
const WINDOW_H: f32 = 900.;
/// Space the table needs besides its columns: trailing empty column + vertical scrollbar.
const TABLE_CHROME_W: f32 = 16.;

pub struct AppRoot {
    table: Entity<TableState<SessionTableDelegate>>,
    search: Entity<InputState>,
    resizable: Entity<ResizableState>,
    projects: Vec<(SharedString, usize)>,
    scope: ScopeSet,
    range: RangePreset,
    selected: Option<Arc<SessionRow>>,
    detail: DetailState,
    detail_visible: bool,
    status: SharedString,
    build_ms: u128,
    _subscriptions: Vec<Subscription>,
    _autoplay: Option<Task<()>>,
}

impl AppRoot {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let t = Instant::now();
        let rows = fake::generate_rows(fake::ROW_COUNT);
        let projects = fake::project_counts(&rows);
        let gen_ms = t.elapsed().as_millis();
        let table = cx.new(|cx| TableState::new(SessionTableDelegate::new(rows), window, cx));
        let build_ms = t.elapsed().as_millis();
        println!("generated {} rows in {gen_ms}ms, table ready in {build_ms}ms", fake::ROW_COUNT);

        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search sessions").clean_on_escape());
        let resizable = cx.new(|_| ResizableState::default());

        let subscriptions = vec![
            cx.subscribe_in(&table, window, Self::on_table_event),
            cx.subscribe_in(&search, window, |this, state, ev: &InputEvent, _, cx| {
                if let InputEvent::Change = ev {
                    let query = state.read(cx).value();
                    this.status = if query.is_empty() { "idle".into() } else { format!("search: {query}").into() };
                    cx.notify();
                }
            }),
        ];

        let mut this = Self {
            table,
            search,
            resizable,
            projects,
            scope: ScopeSet::default(),
            range: RangePreset::Any,
            selected: None,
            detail: DetailState::default(),
            detail_visible: true,
            status: "idle".into(),
            build_ms,
            _subscriptions: subscriptions,
            _autoplay: None,
        };
        this.select_row(0, cx);
        if std::env::var_os("ASM_SPIKE_AUTOPLAY").is_some() {
            this._autoplay = Some(this.autoplay(window, cx));
        }
        this
    }

    fn select_row(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        let row = self.table.update(cx, |table, cx| {
            let row = table.delegate().ix.get(row_ix).map(|_| table.delegate().row(row_ix).clone());
            if row.is_some() && table.selected_row() != Some(row_ix) {
                table.set_selected_row(row_ix, cx);
            }
            row
        });
        if let Some(row) = row {
            self.detail.set_conversation(fake::conversation_for(&row));
            self.status = format!("selected #{} · {}", row.num, row.project).into();
            self.selected = Some(row);
        }
        cx.notify();
    }

    fn on_table_event(&mut self, _: &Entity<TableState<SessionTableDelegate>>, ev: &TableEvent, _: &mut Window, cx: &mut Context<Self>) {
        match ev {
            TableEvent::SelectRow(ix) => self.select_row(*ix, cx),
            TableEvent::DoubleClickedRow(ix) => {
                self.detail_visible = true;
                self.select_row(*ix, cx);
            }
            _ => {}
        }
    }

    /// Scripted interaction for screenshots: scroll deep + select, then filter / sort / switch tab.
    fn autoplay(&self, window: &Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            this.update_in(cx, |this, _, cx| {
                this.table.update(cx, |table, cx| table.scroll_to_row(5_000, cx));
                this.select_row(5_000, cx);
                this.status = "autoplay: scrolled to row 5000".into();
                cx.notify();
            })
            .ok();
            cx.background_executor().timer(Duration::from_secs(3)).await;
            this.update_in(cx, |this, window, cx| {
                this.set_project(Some("scripts".into()), cx);
                this.set_range(RangePreset::Days30, cx);
                this.table.update(cx, |table, cx| {
                    table.delegate_mut().perform_sort(4, ColumnSort::Descending, window, cx);
                    table.scroll_to_row(0, cx);
                });
                this.select_row(0, cx);
                this.set_detail_tab(DetailTab::Metadata, cx);
                this.status = "autoplay: scripts · 30 days · sorted by Prompts desc".into();
                cx.notify();
            })
            .ok();
        })
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.search.read(cx).focus_handle(cx).clone();
        handle.focus(window, cx);
    }

    /// Keep the Preview column filling whatever width the table panel has.
    fn sync_preview_width(&mut self, window: &Window, cx: &mut Context<Self>) {
        let viewport_w = window.viewport_size().width;
        let panel_w = self.resizable.read(cx).sizes().first().copied().filter(|w| *w > px(0.));
        let table_w = match panel_w {
            Some(w) => w,
            None if self.detail_visible => viewport_w - px(DETAIL_W),
            None => viewport_w,
        };
        let preview_w = (table_w - px(FIXED_COLS_W) - px(TABLE_CHROME_W)).max(px(PREVIEW_MIN_W));
        let current = self.table.read(cx).delegate().preview_width();
        if (current - preview_w).abs() > px(0.5) {
            self.table.update(cx, |table, cx| {
                table.delegate_mut().set_preview_width(preview_w);
                table.refresh(cx);
            });
        }
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> StatusBar {
        let delegate = self.table.read(cx).delegate();
        let (shown, total) = (delegate.ix.len(), delegate.rows.len());
        StatusBar::new()
            .h(px(22.))
            .py_0()
            .px(px(10.))
            .font_family(style::mono(cx))
            .text_size(px(11.))
            .text_color(style::head_fg())
            .left(format!("{shown} / {total} sessions"))
            .child(self.status.clone())
            .right(format!("table built in {}ms", self.build_ms))
    }
}

impl Render for AppRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_preview_width(window, cx);
        v_flex()
            .size_full()
            .bg(style::white())
            .text_color(style::text())
            .text_size(px(12.))
            .line_height(px(16.))
            .on_action(cx.listener(|this, _: &ToggleDetail, _, cx| {
                this.detail_visible = !this.detail_visible;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)))
            .on_action(cx.listener(|this, _: &Preview1, _, cx| this.set_mode(DisplayMode::Title, cx)))
            .on_action(cx.listener(|this, _: &Preview2, _, cx| this.set_mode(DisplayMode::FirstPrompt, cx)))
            .on_action(cx.listener(|this, _: &Preview3, _, cx| this.set_mode(DisplayMode::LastPrompt, cx)))
            .on_action(cx.listener(|this, _: &Preview4, _, cx| this.set_mode(DisplayMode::LastMessage, cx)))
            .child(self.render_toolbar(cx))
            .child(
                div().flex_1().min_h_0().w_full().child(
                    h_resizable("main")
                        .with_state(&self.resizable)
                        .child(
                            resizable_panel()
                                .size(px(WINDOW_W - DETAIL_W))
                                .size_range(px(480.)..px(2400.))
                                .child(
                                    div()
                                        .size_full()
                                        .border_r_1()
                                        .border_color(style::border())
                                        .child(DataTable::new(&self.table).stripe(false).bordered(false).with_size(px(ROW_H))),
                                ),
                        )
                        .child(
                            resizable_panel()
                                .size(px(DETAIL_W))
                                .size_range(px(360.)..px(1200.))
                                .visible(self.detail_visible)
                                .child(self.render_detail(cx)),
                        ),
                ),
            )
            .child(self.render_status_bar(cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}

fn main() {
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    app.run(move |cx| {
        gpui_kit::init(cx);
        style::install_theme(cx);
        cx.activate(true);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-\\", ToggleDetail, None),
            KeyBinding::new("cmd-f", FocusSearch, None),
            KeyBinding::new("cmd-1", Preview1, None),
            KeyBinding::new("cmd-2", Preview2, None),
            KeyBinding::new("cmd-3", Preview3, None),
            KeyBinding::new("cmd-4", Preview4, None),
        ]);
        cx.set_menus([
            Menu::new("Agent Session Manager").items(vec![MenuItem::action("Quit", Quit)]),
            Menu::new("View").items(vec![
                MenuItem::action("Find", FocusSearch),
                MenuItem::action("Toggle Detail Panel", ToggleDetail),
                MenuItem::separator(),
                MenuItem::action("Preview: Title", Preview1),
                MenuItem::action("Preview: First Prompt", Preview2),
                MenuItem::action("Preview: Last Prompt", Preview3),
                MenuItem::action("Preview: Last Message", Preview4),
            ]),
        ]);

        let bounds = Bounds::centered(None, size(px(WINDOW_W), px(WINDOW_H)), cx);
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

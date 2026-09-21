//! Toolbar: `Label value ▾` dropdowns (Project / Scope / Updated / Preview) and the search box.

use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable, h_flex};
use gpui_kit::*;

use crate::model::{DisplayMode, Filter, RangePreset, ScopeSet};
use crate::style::{self, FluentBuilder};
use crate::AppRoot;

pub const TOOLBAR_H: f32 = 36.;
const CONTROL_H: f32 = 22.;
const MENU_ITEM_H: f32 = 24.;

/// `Label value ▾` trigger button styled like the mockup.
fn trigger(id: &'static str, label: &'static str, value: SharedString, cx: &App) -> Button {
    let variant = ButtonCustomVariant::new(cx)
        .color(style::white())
        .foreground(style::text())
        .hover(style::control_hover())
        .active(style::control_hover());
    Button::new(id)
        .custom(variant)
        .small()
        .h(px(CONTROL_H))
        .pl(px(8.))
        .pr(px(6.))
        .border_1()
        .border_color(style::control_border())
        .rounded(px(5.))
        .dropdown_caret(true)
        .child(
            h_flex()
                .gap(px(4.))
                .text_size(px(12.))
                .child(div().text_color(style::label()).child(label))
                .child(div().font_weight(FontWeight::MEDIUM).child(value)),
        )
}

/// One row inside a dropdown: `name` on the left, an optional right-aligned `hint`.
fn menu_row(name: SharedString, hint: SharedString, hint_mono: bool, cx: &App) -> Div {
    h_flex()
        .h(px(MENU_ITEM_H))
        .w_full()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .child(div().flex_1().truncate().child(name))
        .when(!hint.is_empty(), |el| {
            el.child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .opacity(0.6)
                    .when(hint_mono, |el| el.font_family(style::mono(cx)))
                    .child(hint),
            )
        })
}

fn menu_item(
    name: SharedString,
    hint: SharedString,
    hint_mono: bool,
    checked: bool,
    on_click: impl Fn(&mut App) + 'static,
) -> PopupMenuItem {
    PopupMenuItem::element(move |_, cx| menu_row(name.clone(), hint.clone(), hint_mono, cx))
        .checked(checked)
        .on_click(move |_, _, cx| on_click(cx))
}

impl AppRoot {
    fn current_filter(&self, cx: &App) -> Filter {
        self.table.read(cx).delegate().filter().clone()
    }

    /// `project` is a `project_key`.
    pub fn set_project(&mut self, project: Option<SharedString>, cx: &mut Context<Self>) {
        let mut filter = self.current_filter(cx);
        filter.project = project;
        self.apply_filter(filter, cx);
    }

    pub fn set_range(&mut self, range: RangePreset, cx: &mut Context<Self>) {
        self.range = range;
        let mut filter = self.current_filter(cx);
        filter.since_ms = range.since_ms();
        self.apply_filter(filter, cx);
    }

    pub fn toggle_empty_sessions(&mut self, cx: &mut Context<Self>) {
        let mut filter = self.current_filter(cx);
        filter.show_empty = !filter.show_empty;
        self.apply_filter(filter, cx);
    }

    fn apply_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        let (changed, ms, shown) = self.table.update(cx, |table, cx| {
            let changed = table.delegate_mut().set_filter(filter);
            if changed {
                table.scroll_to_row(0, cx);
                cx.notify();
            }
            (changed, table.delegate().last_filter_ms, table.delegate().ix.len())
        });
        if changed {
            println!("filter + sort: {shown} rows in {ms}ms");
            self.status = format!("filtered {shown} sessions in {ms}ms").into();
            self.after_rows_changed(cx);
            self.persist(cx);
        }
        cx.notify();
    }

    pub fn set_mode(&mut self, mode: DisplayMode, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            table.delegate_mut().mode = mode;
            cx.notify();
        });
        self.persist(cx);
        cx.notify();
    }

    fn project_menu(&self, cx: &Context<Self>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let this = cx.entity().downgrade();
        let current = self.current_filter(cx).project;
        let total: usize = self.projects.iter().map(|p| p.count).sum();
        let projects = self.projects.clone();
        move |menu, _, _| {
            let this_all = this.clone();
            let mut menu = menu.min_w(px(248.)).max_h(px(420.)).scrollable(true).item(menu_item(
                "All projects".into(),
                total.to_string().into(),
                true,
                current.is_none(),
                move |cx| {
                    this_all.update(cx, |app, cx| app.set_project(None, cx)).ok();
                },
            ));
            for project in &projects {
                let this = this.clone();
                let pick = project.key.clone();
                menu = menu.item(menu_item(
                    project.name.clone(),
                    project.count.to_string().into(),
                    true,
                    current.as_ref() == Some(&project.key),
                    move |cx| {
                        let pick = pick.clone();
                        this.update(cx, |app, cx| app.set_project(Some(pick), cx)).ok();
                    },
                ));
            }
            menu
        }
    }

    fn scope_menu(&self, cx: &Context<Self>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let this = cx.entity().downgrade();
        let scope = self.scope;
        move |menu, _, _| {
            let mut menu = menu.min_w(px(200.));
            for (ix, (label, hint)) in ScopeSet::ITEMS.iter().enumerate() {
                let this = this.clone();
                menu = menu.item(menu_item((*label).into(), (*hint).into(), false, scope.get(ix), move |cx| {
                    this.update(cx, |app, cx| {
                        app.scope.toggle(ix);
                        cx.notify();
                    })
                    .ok();
                }));
            }
            menu.separator().label("Applies to search. Tools = tool input/output, regex only.")
        }
    }

    fn range_menu(&self, cx: &Context<Self>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let this = cx.entity().downgrade();
        let current = self.range;
        move |menu, _, _| {
            let mut menu = menu.min_w(px(180.));
            for preset in RangePreset::ALL {
                let this = this.clone();
                menu = menu.item(menu_item(preset.label().into(), "".into(), false, preset == current, move |cx| {
                    this.update(cx, |app, cx| app.set_range(preset, cx)).ok();
                }));
            }
            menu
        }
    }

    fn preview_menu(&self, cx: &Context<Self>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let this = cx.entity().downgrade();
        let current = self.table.read(cx).delegate().mode;
        move |menu, _, _| {
            let mut menu = menu.min_w(px(200.));
            for mode in DisplayMode::ALL {
                let this = this.clone();
                menu = menu.item(menu_item(mode.label().into(), mode.shortcut().into(), true, mode == current, move |cx| {
                    this.update(cx, |app, cx| app.set_mode(mode, cx)).ok();
                }));
            }
            menu
        }
    }

    pub fn render_toolbar(&self, cx: &mut Context<Self>) -> Div {
        let filter = self.current_filter(cx);
        let project_label: SharedString = match &filter.project {
            Some(key) => self.projects.iter().find(|p| p.key == *key).map_or_else(|| key.clone(), |p| p.name.clone()),
            None => "All".into(),
        };
        let mode = self.table.read(cx).delegate().mode;

        h_flex()
            .w_full()
            .h(px(TOOLBAR_H))
            .flex_none()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .bg(style::toolbar_bg())
            .border_b_1()
            .border_color(style::border_soft())
            .child(trigger("project", "Project", project_label, cx).dropdown_menu(self.project_menu(cx)))
            .child(trigger("scope", "Scope", self.scope.label().into(), cx).dropdown_menu(self.scope_menu(cx)))
            .child(trigger("range", "Updated", self.range.label().into(), cx).dropdown_menu(self.range_menu(cx)))
            .child(
                div().ml(px(4.)).w(px(220.)).child(
                    Input::new(&self.search)
                        .small()
                        .h(px(CONTROL_H))
                        .prefix(Icon::new(IconName::Search).xsmall().text_color(style::muted_light()))
                        .suffix(
                            div()
                                .font_family(style::mono(cx))
                                .text_size(px(10.))
                                .text_color(cx.theme().muted_foreground.opacity(0.8))
                                .child("⌘F"),
                        ),
                ),
            )
            .child(div().flex_1())
            .child(
                trigger("preview", "Preview", mode.label().into(), cx)
                    .dropdown_menu_with_anchor(Anchor::TopRight, self.preview_menu(cx)),
            )
    }
}

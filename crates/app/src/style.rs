//! Design tokens from the Claude Design mockup (`Agent Session Manager.dc.html`).
//!
//! Every hex value here is copied from the mockup so the app can be diffed against it.
//! Colors are exposed as `Hsla` constructors so call sites can stay `.bg(TEXT)`-style.

use gpui_kit::component::ActiveTheme;
use gpui_kit::*;

use crate::model::Kind;

fn hex(v: u32) -> Hsla {
    rgb(v).into()
}

// Text
pub fn text() -> Hsla {
    hex(0x1d1d1f)
}
pub fn text_secondary() -> Hsla {
    hex(0x5c5c61)
}
pub fn muted() -> Hsla {
    hex(0x8a8a90)
}
pub fn muted_light() -> Hsla {
    hex(0x9a9aa0)
}
pub fn label() -> Hsla {
    hex(0x7a7a80)
}
pub fn head_fg() -> Hsla {
    hex(0x6e6e73)
}
pub fn accent() -> Hsla {
    hex(0x2563eb)
}

// Surfaces & borders
pub fn border() -> Hsla {
    hex(0xdcdce0)
}
pub fn border_soft() -> Hsla {
    hex(0xe3e3e6)
}
pub fn hairline() -> Hsla {
    hex(0xececef)
}
pub fn row_border() -> Hsla {
    hex(0xf0f0f2)
}
pub fn toolbar_bg() -> Hsla {
    hex(0xf9f9fa)
}
pub fn head_bg() -> Hsla {
    hex(0xfafafb)
}
pub fn chrome_bg() -> Hsla {
    hex(0xf3f3f5)
}
pub fn control_border() -> Hsla {
    hex(0xd3d3d8)
}
pub fn control_hover() -> Hsla {
    hex(0xf2f2f4)
}
pub fn hover_bg() -> Hsla {
    hex(0xf3f5f9)
}
pub fn segment_bg() -> Hsla {
    hex(0xececef)
}

// Conversation
pub fn user_bg() -> Hsla {
    hex(0xf1f4fa)
}
pub fn user_border() -> Hsla {
    hex(0xe3e8f2)
}
pub fn user_time() -> Hsla {
    hex(0x7a8aa8)
}
pub fn asst_text() -> Hsla {
    hex(0x2b2b2e)
}
pub fn tool_name() -> Hsla {
    hex(0x6d4fc2)
}
pub fn expanded_bg() -> Hsla {
    hex(0xfbfbfc)
}
pub fn row_hover() -> Hsla {
    hex(0xf7f7f8)
}
pub fn white() -> Hsla {
    hex(0xffffff)
}
/// The `·` between the session id, project and branch in the detail header.
pub fn dot_sep() -> Hsla {
    hex(0xc4c4c8)
}

// Turn cards (M4 mockups 1–4: `project/{Main,Expanded,Search,Special}.dc.html`)
pub fn conversation_bg() -> Hsla {
    hex(0xf3f3f5)
}
pub fn card_border() -> Hsla {
    hex(0xe3e3e6)
}
/// Between a card's head and its steps or reply.
pub fn step_border() -> Hsla {
    hex(0xf0f0f2)
}
/// Between two step rows.
pub fn step_sep() -> Hsla {
    hex(0xf4f4f6)
}
/// Top edge of an open step's input / output box.
pub fn expanded_border() -> Hsla {
    hex(0xf2f2f4)
}
/// The prompt block of an AUTO card (a turn's is `user_bg`).
pub fn auto_bg() -> Hsla {
    hex(0xf6f6f8)
}
/// The step a tick click opened, and its box.
pub fn lit_bg() -> Hsla {
    hex(0xeaf1fe)
}
pub fn lit_box_bg() -> Hsla {
    hex(0xf5f8fe)
}
/// Status lamps: ended with a reply / interrupted or API error / no reply.
pub fn turn_ok() -> Hsla {
    hex(0x16a34a)
}
pub fn turn_err() -> Hsla {
    hex(0xdc2626)
}
pub fn turn_off() -> Hsla {
    hex(0xc4c4c8)
}
/// Text of errors: `1 error`, `interrupted`, a failed tool's result and output.
pub fn error_text() -> Hsla {
    hex(0xc81e1e)
}
pub fn api_error_fg() -> Hsla {
    hex(0xb91c1c)
}
pub fn api_error_bg() -> Hsla {
    hex(0xfef2f2)
}
pub fn api_error_border() -> Hsla {
    hex(0xfbd5d5)
}
/// A tool call's tick (a failed one is `turn_err`); process text is a `muted` dot.
pub fn tick() -> Hsla {
    hex(0xb4b4bb)
}
/// Ring of a thinking's hollow tick.
pub fn tick_thinking() -> Hsla {
    hex(0xa8a8b0)
}
/// The lines either side of `context compacted`.
pub fn compact_rule() -> Hsla {
    hex(0xd6d6db)
}
/// A fenced code block in a prompt, process text or reply (mockup 2's `.block`), framed in
/// `hairline`.
pub fn code_block_bg() -> Hsla {
    hex(0xf7f7f9)
}
/// Under inline code (mockup 2's `.ic`).
pub fn inline_code_bg() -> Hsla {
    hex(0xededf0)
}

// Search (not in the mockup): hits are marked like a text marker, the hit the detail panel is
// parked on gets the stronger one; toggled toolbar buttons carry the accent. Hit badges and the
// rings of ticks with hits use the same two colors.
pub fn hit_bg() -> Hsla {
    hex(0xfde68a)
}
pub fn hit_current_bg() -> Hsla {
    hex(0xfdba74)
}
pub fn hit_highlight() -> HighlightStyle {
    HighlightStyle { background_color: Some(hit_bg()), color: Some(text()), ..Default::default() }
}
pub fn hit_current_highlight() -> HighlightStyle {
    HighlightStyle { background_color: Some(hit_current_bg()), color: Some(text()), ..Default::default() }
}
pub fn toggle_on_bg() -> Hsla {
    hex(0xe3ecfb)
}
pub fn toggle_on_fg() -> Hsla {
    hex(0x1d4fb8)
}

/// Monospace font family (the theme picks `Menlo` on macOS and falls back if missing).
pub fn mono(cx: &App) -> SharedString {
    cx.theme().mono_font_family.clone()
}

/// The UI font, for text inside a monospace row (thinking rows, links in mono lines).
pub fn sans(cx: &App) -> SharedString {
    cx.theme().font_family.clone()
}

/// Kind badge: 9px uppercase mono, filled for SDK / BG, outlined for History.
pub fn kind_badge(kind: Kind, cx: &App) -> Option<AnyElement> {
    let (label, bg, fg, outlined) = match kind {
        Kind::Cli => return None,
        Kind::Sdk => ("SDK", hex(0xe3ecfb), hex(0x1d4fb8), false),
        Kind::Background => ("BG", hex(0xececef), hex(0x5c5c61), false),
        Kind::History => ("HIST", white(), muted_light(), true),
    };
    Some(
        div()
            .flex_none()
            .font_family(mono(cx))
            .text_size(px(9.))
            .font_weight(FontWeight::SEMIBOLD)
            .line_height(px(14.))
            .h(px(14.))
            .px(px(4.))
            .rounded(px(3.))
            .bg(bg)
            .text_color(fg)
            .when(outlined, |el| el.border_1().border_color(hairline()))
            .child(label)
            .into_any_element(),
    )
}

/// Install the mockup palette into the gpui-kit theme. Must run after `gpui_kit::init`.
pub fn install_theme(cx: &mut App) {
    use gpui_kit::component::{Theme, ThemeTokens};

    {
        let theme = Theme::global_mut(cx);
        theme.font_size = px(12.);
        theme.mono_font_size = px(11.);
        theme.radius = px(5.);
        theme.radius_lg = px(6.);

        let transparent = theme.transparent;
        let c = &mut theme.colors;
        c.background = white();
        c.foreground = text();
        c.border = border();
        c.muted_foreground = muted();
        c.popover = white();
        c.popover_foreground = text();
        c.accent = accent();
        c.accent_foreground = white();
        // Selection is painted as an overlay above the cells, so it must stay translucent
        // (accent at 16% over white ≈ the mockup's #dde8fa).
        c.list_active = accent().opacity(0.16);
        c.list_active_border = transparent;
        c.list_hover = hover_bg();
        c.table = white();
        c.table_even = white();
        c.table_head = head_bg();
        c.table_head_foreground = head_fg();
        c.table_active = accent().opacity(0.16);
        c.table_active_border = transparent;
        c.table_hover = hover_bg();
        c.table_row_border = row_border();
        c.status_bar = chrome_bg();
        c.status_bar_border = border();
        c.title_bar = chrome_bg();
        c.title_bar_border = border();
        c.tab_bar_segmented = segment_bg();
        c.tab_active = white();
        c.tab_active_foreground = text();
        c.tab_foreground = head_fg();
        c.input = control_border();
        c.ring = accent();
        theme.tokens = ThemeTokens::from(&theme.colors);
    }
    Theme::sync_base(cx);
}

/// `FluentBuilder` re-export so callers only need `use crate::style::*`.
pub use gpui_kit::prelude::FluentBuilder;

pub fn favorite() -> gpui_kit::Hsla { gpui_kit::rgb(0xb58116).into() }

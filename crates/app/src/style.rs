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
pub fn chip_border() -> Hsla {
    hex(0xc9c9ce)
}
pub fn white() -> Hsla {
    hex(0xffffff)
}

// Search (not in the mockup): hits are marked like a text marker, the hit the detail panel is
// parked on gets the stronger one; toggled toolbar buttons carry the accent.
pub fn hit_highlight() -> HighlightStyle {
    HighlightStyle { background_color: Some(hex(0xfde68a)), color: Some(text()), ..Default::default() }
}
pub fn hit_current_highlight() -> HighlightStyle {
    HighlightStyle { background_color: Some(hex(0xfdba74)), color: Some(text()), ..Default::default() }
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

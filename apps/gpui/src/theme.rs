use gpui_kit::component::ActiveTheme;
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::*;

pub fn hex(color: u32) -> Hsla {
    rgb(color).into()
}

/// Footer and flyout-section background in dark mode, a step below the window.
pub const DARK_FOOTER: u32 = 0x0f0f14;

/// Applies light or dark mode with the app palette, which matches the Tauri front end.
pub fn apply(window: &mut Window, cx: &mut App, dark: Option<bool>) {
    match dark {
        Some(dark) => Theme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, Some(window), cx),
        None => Theme::sync_system_appearance(Some(window), cx),
    }
    let dark = cx.theme().is_dark();
    Theme::update(cx, |t| {
        let (accent, hover) = if dark { (0x9d8cff, 0xb0a3ff) } else { (0x6d5ef5, 0x7f72f7) };
        t.primary = hex(accent);
        t.primary_hover = hex(hover);
        t.primary_active = hex(accent);
        t.primary_foreground = white();
        t.button_primary = hex(accent);
        t.button_primary_hover = hex(hover);
        t.button_primary_active = hex(accent);
        t.button_primary_foreground = white();
        t.ring = hex(accent);
        t.slider_bar = hex(accent);
        t.slider_thumb = hex(accent);

        // `muted` is the window, `background` the cards.
        let (window_bg, card, hover, border, text, muted) = if dark {
            (0x121218, 0x1c1c25, 0x23232e, 0x2a2a37, 0xececf3, 0x8e8ea3)
        } else {
            (0xf3f3f6, 0xffffff, 0xf7f7fa, 0xe4e4ea, 0x1b1b22, 0x6b6b78)
        };
        t.muted = hex(window_bg);
        t.title_bar = hex(window_bg);
        t.title_bar_border = hex(window_bg);
        t.background = hex(card);
        t.secondary = hex(hover);
        t.secondary_hover = hex(hover);
        t.border = hex(border);
        t.foreground = hex(text);
        t.muted_foreground = hex(muted);
        t.button = hex(card);
        t.button_hover = hex(hover);
        t.button_active = hex(border);
        t.button_foreground = hex(text);
        t.popover = hex(card);
        t.popover_foreground = hex(text);
        if dark {
            t.success = hex(0x4ade80);
            t.switch = hex(border);
        }
    });

}

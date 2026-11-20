use audio_bartender_core::{Accent, AccentColors};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::scroll::ScrollbarMode;
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::*;

pub fn hex(color: u32) -> Hsla {
    rgb(color).into()
}

/// Footer and flyout-section background in dark mode, a step below the window.
pub const DARK_FOOTER: u32 = 0x0f0f14;

/// Applies light or dark mode with the app palette, which matches the Tauri front end.
/// The card color, which `theme.background` is too outside the Windows 11 build.
pub fn card(dark: bool) -> Hsla {
    hex(if dark { 0x1c1c25 } else { 0xffffff })
}

pub fn apply(window: &mut Window, cx: &mut App, dark: Option<bool>, accent: Accent) {
    match dark {
        Some(dark) => Theme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, Some(window), cx),
        None => Theme::sync_system_appearance(Some(window), cx),
    }
    let dark = cx.theme().is_dark();
    Theme::update(cx, |t| {
        let AccentColors { accent, hover, .. } = accent.colors(dark);
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
        // A thin bar that stays visible, so a long page shows there's more below.
        t.scrollbar_mode = ScrollbarMode::Always;
        // Windows 11's own UI font, as in its flyouts.
        if cfg!(feature = "win11") {
            t.font_family = "Segoe UI Variable Text".into();
        }
        // Dimmer in dark mode, where a grey thumb otherwise stands out; matches the Tauri one.
        let (thumb, thumb_hover) = if dark {
            (white().opacity(0.14), white().opacity(0.26))
        } else {
            (hex(0x6b6b78).opacity(0.45), hex(0x6b6b78).opacity(0.7))
        };
        t.scrollbar = transparent_black();
        t.scrollbar_thumb = thumb;
        t.scrollbar_thumb_hover = thumb_hover;

        // `muted` is the window, `background` the cards.
        let (window_bg, card, hover, border, text, muted) = if dark {
            (0x121218, 0x1c1c25, 0x23232e, 0x2a2a37, 0xececf3, 0x8e8ea3)
        } else {
            (0xf3f3f6, 0xffffff, 0xf7f7fa, 0xe4e4ea, 0x1b1b22, 0x6b6b78)
        };
        t.muted = hex(window_bg);
        t.title_bar = hex(window_bg);
        t.title_bar_border = hex(window_bg);
        // The root fills the window with the background. In the Windows 11 build the
        // space around the cards must stay clear for their shadows, so the cards use
        // `card` instead.
        t.background = if cfg!(feature = "win11") { transparent_black() } else { hex(card) };
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

//! Opens the Windows tray icon page named by the argument: `show` or `hide`.

fn main() {
    let page = match std::env::args().nth(1).as_deref() {
        Some("hide") => audio_bartender_core::IconPage::HideWindows,
        _ => audio_bartender_core::IconPage::ShowOurs,
    };
    audio_bartender_core::open_icon_page(page);
}

//! Opens the Windows tray icon page named by the argument: `show` or `hide`.

fn main() {
    let page = match std::env::args().nth(1).as_deref() {
        Some("hide") => mediakeys_core::IconPage::HideWindows,
        _ => mediakeys_core::IconPage::ShowOurs,
    };
    mediakeys_core::open_icon_page(page);
}

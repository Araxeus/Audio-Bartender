#![windows_subsystem = "windows"]

use std::sync::Mutex;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use audio_bartender_core::{
    Accent, Anchor, Core, Event, IconPage, MediaKey, Page, PulseStyle, Setting, State, TrayMiddle, TrayStyle,
};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, RunEvent, Theme, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

const FLYOUT: &str = "flyout";
const FLYOUT_WIDTH: f64 = 360.0;
/// The settings panel beside the flyout, two columns wide.
const PANEL_WIDTH: f64 = 640.0;

/// Each part is a rounded card with a shadow, `MARGIN` inside its box so the shadow
/// fits in the window region. Toward the taskbar the margin is [`edge`], so the cards
/// float `FLOAT` clear of it on Windows 11 and sit flush against it on Windows 10.
const MARGIN: f64 = 8.0;
const FLOAT: f64 = 12.0;
/// How far the panel's box overlaps the flyout's, leaving 8 px between the cards.
const OVERLAP: f64 = 2.0 * MARGIN - 8.0;

/// The width the panel adds to the window.
const PANEL_PART: f64 = PANEL_WIDTH + 2.0 * MARGIN - OVERLAP;
/// The window's fixed size. Its region shows only the flyout and the open panel.
const FRAME_WIDTH: f64 = FLYOUT_WIDTH + 2.0 * MARGIN + PANEL_PART;
/// The settings panel can be taller than the flyout's 640 px, so every setting fits.
const FRAME_HEIGHT: f64 = 720.0 + MARGIN + FLOAT;

/// The margin on the taskbar side of each card.
fn edge(anchor: &Anchor) -> f64 {
    if anchor.floats() { FLOAT } else { 0.0 }
}

/// A tray click that lands right after the flyout closed is the click that closed it.
const REOPEN_GUARD: Duration = Duration::from_millis(300);

/// The hidden flyout keeps its webview for quick reopening, then frees it.
const FLYOUT_KEEPALIVE: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Flyout {
    anchor: Option<Anchor>,
    /// The flyout's height.
    height: Option<f64>,
    /// The settings panel's height while the region shows it.
    panel: Option<f64>,
    closed_at: Option<Instant>,
    /// When the flyout was last shown, for sliding it out from behind the taskbar.
    shown_at: Option<Instant>,
    /// When the flyout started closing, sliding back behind the taskbar.
    closing_at: Option<Instant>,
    /// Stays open when it loses focus.
    pinned: bool,
    /// The user dragged the pinned flyout, so it stays where they put it.
    moved: bool,
}

#[tauri::command]
fn state(core: tauri::State<Core>) -> State {
    core.state()
}

/// Every accent color, for the settings picker.
#[tauri::command]
fn accents() -> Vec<Accent> {
    Accent::ALL.to_vec()
}

#[tauri::command]
fn select(core: tauri::State<Core>, id: Option<String>) {
    core.select(id);
}

#[tauri::command]
fn press(core: tauri::State<Core>, key: MediaKey) {
    core.press(key);
}

/// Sends a key to one app without changing where media keys go.
#[tauri::command]
fn press_for(core: tauri::State<Core>, id: String, key: MediaKey) {
    core.press_for(&id, key);
}

#[tauri::command]
fn check_update(core: tauri::State<Core>) {
    core.check_update();
}

/// Live levels of the given app rows, 0 to 1.
#[tauri::command]
fn levels(core: tauri::State<Core>, ids: Vec<String>) -> Vec<f32> {
    ids.iter().map(|id| core.level(id)).collect()
}

#[tauri::command]
fn set_setting(core: tauri::State<Core>, setting: Setting) {
    core.set(setting);
}

#[tauri::command]
fn adjust_volume(core: tauri::State<Core>, id: String, delta: f32) {
    core.adjust_volume(&id, delta);
}

#[tauri::command]
fn set_volume(core: tauri::State<Core>, id: String, level: f32) {
    core.set_volume(&id, level);
}

#[tauri::command]
fn toggle_mute(core: tauri::State<Core>, id: String) {
    core.toggle_mute(&id);
}

#[tauri::command]
fn adjust_master(core: tauri::State<Core>, delta: f32) {
    core.adjust_master(delta);
}

#[tauri::command]
fn set_master(core: tauri::State<Core>, level: f32) {
    core.set_master(level);
}

#[tauri::command]
fn toggle_master_mute(core: tauri::State<Core>) {
    core.toggle_master_mute();
}

#[tauri::command]
fn set_device(core: tauri::State<Core>, id: String) {
    core.set_device(&id);
}

#[tauri::command]
fn open_icon_page(page: IconPage) {
    audio_bartender_core::open_icon_page(page);
}

#[tauri::command]
fn set_icon_on_taskbar(core: tauri::State<Core>, show: bool) {
    core.set_icon_on_taskbar(show);
}

/// A tray icon look for the settings to pick from, as a `data:` URL.
#[tauri::command]
fn tray_swatch(
    core: tauri::State<Core>,
    style: TrayStyle,
    middle: TrayMiddle,
    pulse: PulseStyle,
    dark: bool,
) -> Option<String> {
    let state = core.state();
    let art = state.now_playing().and_then(|t| t.icon.as_ref()).filter(|i| i.art);
    audio_bartender_core::tray_swatch_url(style, middle, pulse, dark, art)
}

/// Sends an app to an output device, or back to the default with no device.
#[tauri::command]
fn set_route(core: tauri::State<Core>, app: String, device: Option<String>) {
    core.set_route(&app, device.as_deref());
}

#[tauri::command]
fn open_app_volume_settings() {
    audio_bartender_core::open_app_volume_settings();
}

#[tauri::command]
fn quit(app: AppHandle, core: tauri::State<Core>) {
    core.shutdown();
    app.exit(0);
}

/// The header of a pinned flyout drags the window, like a title bar.
#[tauri::command]
fn start_drag(app: AppHandle, window: WebviewWindow) {
    app.state::<Mutex<Flyout>>().lock().unwrap().moved = true;
    let _ = window.start_dragging();
}

#[tauri::command]
fn set_pinned(app: AppHandle, pinned: bool) {
    app.state::<Mutex<Flyout>>().lock().unwrap().pinned = pinned;
}

/// The page reports the flyout's height and the open panel's, so the window region
/// shows exactly those. Returns whether the panel goes left of the flyout, whether
/// both hang down from a taskbar at the top, and whether they float clear of it.
#[tauri::command]
fn flyout_size(app: AppHandle, flyout: f64, panel: Option<f64>) -> (bool, bool, bool) {
    {
        let state = app.state::<Mutex<Flyout>>();
        let mut state = state.lock().unwrap();
        (state.height, state.panel) = (Some(flyout), panel);
        // Only the settings report a panel.
        if state.closing_at.is_none() {
            audio_bartender_core::set_flyout_page(Some(if panel.is_some() { Page::Settings } else { Page::Apps }));
        }
    }
    place_flyout(&app);
    apply_region(&app);
    let Some((anchor, frame)) = frame(&app) else { return (true, false, true) };
    (frame.panel_left(&anchor), anchor.grows_down(), anchor.floats())
}

/// Limits the window to the flyout and the open panel.
fn apply_region(app: &AppHandle) {
    let Some((anchor, frame)) = frame(app) else { return };
    let Some(window) = app.get_webview_window(FLYOUT) else { return };
    let Ok(hwnd) = window.hwnd() else { return };
    let panel = app.state::<Mutex<Flyout>>().lock().unwrap().panel;
    let scale = anchor.scale as f64;
    let overlap = (OVERLAP * scale).round() as i32;
    let panel = panel.map(|h| ((h + MARGIN + edge(&anchor)) * scale).round() as i32);
    let parts = frame.parts(&anchor, panel, overlap);
    audio_bartender_core::set_window_region(hwnd.0 as isize, &parts);
}

/// The anchor and the window in physical pixels, with the flyout at its current height.
fn frame(app: &AppHandle) -> Option<(Anchor, audio_bartender_core::Frame)> {
    let flyout = app.state::<Mutex<Flyout>>();
    let flyout = flyout.lock().unwrap();
    let anchor = flyout.anchor?;
    let physical = |v: f64| (v * anchor.scale as f64).round() as i32;
    let frame = audio_bartender_core::Frame {
        width: physical(FRAME_WIDTH),
        height: physical(FRAME_HEIGHT),
        panel: physical(PANEL_PART),
        visible: physical(flyout.height.unwrap_or(480.0) + MARGIN + edge(&anchor)),
    };
    Some((anchor, frame))
}

/// The page calls this after its first render, so the window never flashes blank.
#[tauri::command]
fn ready(window: WebviewWindow) {
    reveal(window.app_handle(), &window);
}

/// Shows the flyout, sliding it out from behind the taskbar.
fn reveal(app: &AppHandle, window: &WebviewWindow) {
    audio_bartender_core::log("flyout reveal");
    {
        let flyout = app.state::<Mutex<Flyout>>();
        let mut flyout = flyout.lock().unwrap();
        flyout.shown_at = Some(Instant::now());
        flyout.closing_at = None;
        flyout.moved = false;
    }
    place_flyout(app);
    apply_region(app);
    let _ = window.show();
    let _ = window.set_focus();
    let app = app.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(8));
            let shown_at = app.state::<Mutex<Flyout>>().lock().unwrap().shown_at;
            place_flyout(&app);
            if shown_at.is_none_or(|at| audio_bartender_core::slide_progress(at) >= 1.0) {
                break;
            }
        }
    });
}

fn theme(dark: Option<bool>) -> Option<Theme> {
    dark.map(|dark| if dark { Theme::Dark } else { Theme::Light })
}

fn page_name(page: Page) -> &'static str {
    match page {
        Page::Apps => "apps",
        Page::Settings => "settings",
    }
}

/// Sizes the flyout to its content and puts it next to the tray icon.
fn place_flyout(app: &AppHandle) {
    let Some(window) = app.get_webview_window(FLYOUT) else { return };
    let progress = {
        let flyout = app.state::<Mutex<Flyout>>();
        let flyout = flyout.lock().unwrap();
        // A flyout the user dragged stays where they put it.
        if flyout.moved {
            return;
        }
        match (flyout.closing_at, flyout.shown_at) {
            (Some(at), _) => audio_bartender_core::slide_out_progress(at),
            (None, Some(at)) => audio_bartender_core::slide_progress(at),
            (None, None) => 1.0,
        }
    };
    let Some((anchor, frame)) = frame(app) else { return };
    let _ = window.set_size(LogicalSize::new(FRAME_WIDTH, FRAME_HEIGHT));
    match window.hwnd() {
        Ok(hwnd) => audio_bartender_core::place_window(hwnd.0 as isize, &anchor, frame, progress),
        Err(_) => {
            let (x, y) = anchor.place_frame(frame);
            let _ = window.set_position(PhysicalPosition::new(x, y));
        }
    }
}

/// Slides the flyout back behind the taskbar, then hides it.
fn hide_flyout(app: &AppHandle) {
    audio_bartender_core::log("flyout hide");
    let start = Instant::now();
    {
        let flyout = app.state::<Mutex<Flyout>>();
        let mut flyout = flyout.lock().unwrap();
        if flyout.closing_at.is_some() {
            return;
        }
        flyout.closed_at = Some(start);
        flyout.closing_at = Some(start);
    }
    audio_bartender_core::set_flyout_page(None);
    let sliding = app.clone();
    std::thread::spawn(move || {
        loop {
            // Showing it again cancels the slide.
            if sliding.state::<Mutex<Flyout>>().lock().unwrap().closing_at != Some(start) {
                return;
            }
            // A flyout the user moved closes where it is instead of sliding into the taskbar.
            if sliding.state::<Mutex<Flyout>>().lock().unwrap().moved {
                break;
            }
            place_flyout(&sliding);
            if audio_bartender_core::slide_out_progress(start) <= 0.0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(8));
        }
        // Hide on the main thread, where a reveal's show is queued too, so a flyout
        // reopened meanwhile stays open.
        let main = sliding.clone();
        let _ = sliding.run_on_main_thread(move || {
            {
                let flyout = main.state::<Mutex<Flyout>>();
                let mut flyout = flyout.lock().unwrap();
                if flyout.closing_at != Some(start) {
                    return;
                }
                flyout.closing_at = None;
            }
            if let Some(window) = main.get_webview_window(FLYOUT) {
                let _ = window.hide();
            }
        });
    });
    // Free the webview if the flyout stays closed for a while.
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(FLYOUT_KEEPALIVE);
        let closed_at = app.state::<Mutex<Flyout>>().lock().unwrap().closed_at;
        let still_closed = closed_at.is_some_and(|at| at.elapsed() >= FLYOUT_KEEPALIVE);
        if let Some(window) = app.get_webview_window(FLYOUT)
            && still_closed
            && !window.is_visible().unwrap_or(false)
        {
            let _ = window.destroy();
        }
    });
}

/// Shows the flyout on `page` next to `anchor`. With `toggle` (a tray click), an
/// open flyout closes instead.
fn show_flyout(app: &AppHandle, anchor: Anchor, page: Page, toggle: bool) {
    let existing = app.get_webview_window(FLYOUT);
    // A flyout sliding closed counts as closed.
    let closing = app.state::<Mutex<Flyout>>().lock().unwrap().closing_at.is_some();
    let visible = !closing && existing.as_ref().is_some_and(|w| w.is_visible().unwrap_or(false));
    if visible && toggle {
        hide_flyout(app);
        return;
    }
    {
        let flyout = app.state::<Mutex<Flyout>>();
        let mut flyout = flyout.lock().unwrap();
        if toggle && flyout.closed_at.is_some_and(|at| at.elapsed() < REOPEN_GUARD) {
            return;
        }
        flyout.anchor = Some(anchor);
        flyout.closed_at = None;
    }
    audio_bartender_core::set_flyout_page(Some(page));
    if let Some(window) = existing {
        let _ = window.emit_to(FLYOUT, "page", page_name(page));
        if visible {
            place_flyout(app);
            let _ = window.set_focus();
        } else {
            reveal(app, &window);
        }
        return;
    }
    // Must run off the main thread, because WebView2 creation deadlocks inside event-loop callbacks.
    let dark = app.state::<Core>().state().settings.dark;
    let pinned = app.state::<Mutex<Flyout>>().lock().unwrap().pinned;
    let (floats, down) = (anchor.floats(), anchor.grows_down());
    let win11 = audio_bartender_core::windows_11();
    let url = format!("index.html?pinned={pinned}&floats={floats}&down={down}&win11={win11}#{}", page_name(page));
    let built = WebviewWindowBuilder::new(app, FLYOUT, WebviewUrl::App(url.into()))
        .theme(theme(dark))
        .title("Audio Bartender")
        .decorations(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .resizable(false)
        .shadow(true)
        .inner_size(FRAME_WIDTH, FRAME_HEIGHT)
        // The cards have rounded corners and shadows over what's behind.
        .transparent(true)
        .visible(false)
        .build();
    if built.is_ok() {
        place_flyout(app);
    }
}

fn main() {
    audio_bartender_core::set_flavor(audio_bartender_core::Flavor::Web);
    let (tx, rx) = mpsc::channel();
    let Some(core) = Core::start(move |event| {
        let _ = tx.send(event);
    }) else {
        return;
    };
    let hidden = audio_bartender_core::start_hidden();
    let pinned = audio_bartender_core::start_pinned();

    let app = tauri::Builder::default()
        // tao's raw keyboard input registration stops low-level hooks in this process.
        .device_event_filter(tauri::DeviceEventFilter::Always)
        .manage(core)
        .manage(Mutex::new(Flyout { pinned, ..Flyout::default() }))
        .invoke_handler(tauri::generate_handler![
            state,
            accents,
            select,
            press,
            press_for,
            levels,
            check_update,
            set_setting,
            adjust_volume,
            set_volume,
            toggle_mute,
            adjust_master,
            set_master,
            toggle_master_mute,
            set_device,
            open_icon_page,
            set_icon_on_taskbar,
            set_route,
            tray_swatch,
            start_drag,
            open_app_volume_settings,
            quit,
            set_pinned,
            flyout_size,
            ready
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::Focused(focused) = event {
                audio_bartender_core::log(format!("flyout focused {focused}"));
            }
            // Clicking anywhere else closes the flyout, like the Windows one. A new
            // webview gains and loses focus while still hidden, which must not count.
            if let (FLYOUT, WindowEvent::Focused(false)) = (window.label(), event)
                && window.is_visible().unwrap_or(false)
                && !window.state::<Mutex<Flyout>>().lock().unwrap().pinned
            {
                hide_flyout(window.app_handle());
            }
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                if !hidden {
                    let anchor = handle.state::<Core>().default_anchor();
                    show_flyout(&handle, anchor, audio_bartender_core::start_page(), false);
                }
                for event in rx {
                    match event {
                        Event::State(state) => {
                            if let Some(window) = handle.get_webview_window(FLYOUT) {
                                let _ = window.set_theme(theme(state.settings.dark));
                            }
                            let _ = handle.emit("state", state);
                        }
                        Event::Flyout { anchor, page, toggle } => show_flyout(&handle, anchor, page, toggle),
                        Event::DismissFlyout => hide_flyout(&handle),
                        Event::Quit => handle.exit(0),
                    }
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("build tauri app");

    // Closing the flyout keeps the app alive in the tray; Quit exits.
    app.run(|_, event| {
        if let RunEvent::ExitRequested { code: None, api, .. } = event {
            api.prevent_exit();
        }
    });
}

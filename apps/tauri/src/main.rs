#![windows_subsystem = "windows"]

use std::sync::Mutex;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use mediakeys_core::{Accent, Anchor, Core, Event, IconPage, MediaKey, Page, Setting, State};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, RunEvent, Theme, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

const FLYOUT: &str = "flyout";
const FLYOUT_WIDTH: f64 = 360.0;

/// A tray click that lands right after the flyout closed is the click that closed it.
const REOPEN_GUARD: Duration = Duration::from_millis(300);

/// The hidden flyout keeps its webview for quick reopening, then frees it.
const FLYOUT_KEEPALIVE: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Flyout {
    anchor: Option<Anchor>,
    height: Option<f64>,
    closed_at: Option<Instant>,
    /// When the flyout was last shown, for sliding it out from behind the taskbar.
    shown_at: Option<Instant>,
    /// When the flyout started closing, sliding back behind the taskbar.
    closing_at: Option<Instant>,
    /// Stays open when it loses focus.
    pinned: bool,
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
    mediakeys_core::open_icon_page(page);
}

#[tauri::command]
fn quit(app: AppHandle, core: tauri::State<Core>) {
    core.shutdown();
    app.exit(0);
}

#[tauri::command]
fn set_pinned(app: AppHandle, pinned: bool) {
    app.state::<Mutex<Flyout>>().lock().unwrap().pinned = pinned;
}

/// The page reports its content height, so the window fits it exactly.
#[tauri::command]
fn flyout_size(app: AppHandle, height: f64) {
    app.state::<Mutex<Flyout>>().lock().unwrap().height = Some(height);
    place_flyout(&app);
}

/// The page calls this after its first render, so the window never flashes blank.
#[tauri::command]
fn ready(window: WebviewWindow) {
    reveal(window.app_handle(), &window);
}

/// Shows the flyout, sliding it out from behind the taskbar.
fn reveal(app: &AppHandle, window: &WebviewWindow) {
    mediakeys_core::log("flyout reveal");
    {
        let flyout = app.state::<Mutex<Flyout>>();
        let mut flyout = flyout.lock().unwrap();
        flyout.shown_at = Some(Instant::now());
        flyout.closing_at = None;
    }
    place_flyout(app);
    let _ = window.show();
    let _ = window.set_focus();
    let app = app.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(8));
            let shown_at = app.state::<Mutex<Flyout>>().lock().unwrap().shown_at;
            place_flyout(&app);
            if shown_at.is_none_or(|at| mediakeys_core::slide_progress(at) >= 1.0) {
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
    let (anchor, height, progress) = {
        let flyout = app.state::<Mutex<Flyout>>();
        let flyout = flyout.lock().unwrap();
        let progress = match (flyout.closing_at, flyout.shown_at) {
            (Some(at), _) => mediakeys_core::slide_out_progress(at),
            (None, Some(at)) => mediakeys_core::slide_progress(at),
            (None, None) => 1.0,
        };
        (flyout.anchor, flyout.height.unwrap_or(480.0), progress)
    };
    let Some(anchor) = anchor else { return };
    let scale = anchor.scale as f64;
    let _ = window.set_size(LogicalSize::new(FLYOUT_WIDTH, height));
    let (width, height) = ((FLYOUT_WIDTH * scale) as i32, (height * scale) as i32);
    match window.hwnd() {
        Ok(hwnd) => mediakeys_core::place_window_sized(hwnd.0 as isize, &anchor, width, height, progress),
        Err(_) => {
            let (x, y) = anchor.place(width, height);
            let _ = window.set_position(PhysicalPosition::new(x, y));
        }
    }
}

/// Slides the flyout back behind the taskbar, then hides it.
fn hide_flyout(app: &AppHandle) {
    mediakeys_core::log("flyout hide");
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
    let sliding = app.clone();
    std::thread::spawn(move || {
        loop {
            // Showing it again cancels the slide.
            if sliding.state::<Mutex<Flyout>>().lock().unwrap().closing_at != Some(start) {
                return;
            }
            place_flyout(&sliding);
            if mediakeys_core::slide_out_progress(start) <= 0.0 {
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
    let url = format!("index.html?pinned={pinned}#{}", page_name(page));
    let built = WebviewWindowBuilder::new(app, FLYOUT, WebviewUrl::App(url.into()))
        .theme(theme(dark))
        .title("Media Keys Selector")
        .decorations(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .resizable(false)
        .shadow(true)
        .inner_size(FLYOUT_WIDTH, 480.0)
        .visible(false)
        .build();
    if built.is_ok() {
        place_flyout(app);
    }
}

fn main() {
    let (tx, rx) = mpsc::channel();
    let Some(core) = Core::start(move |event| {
        let _ = tx.send(event);
    }) else {
        return;
    };
    let hidden = mediakeys_core::start_hidden();
    let pinned = mediakeys_core::start_pinned();

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
            set_setting,
            adjust_volume,
            set_volume,
            toggle_mute,
            adjust_master,
            set_master,
            toggle_master_mute,
            set_device,
            open_icon_page,
            quit,
            set_pinned,
            flyout_size,
            ready
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::Focused(focused) = event {
                mediakeys_core::log(format!("flyout focused {focused}"));
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
                    show_flyout(&handle, anchor, mediakeys_core::start_page(), false);
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

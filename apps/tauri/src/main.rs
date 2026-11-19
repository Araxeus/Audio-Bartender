#![windows_subsystem = "windows"]

use std::sync::Mutex;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use mediakeys_core::{Anchor, Core, Event, MediaKey, Setting, State};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, RunEvent, Theme, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

const MAIN: &str = "main";
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
}

#[tauri::command]
fn state(core: tauri::State<Core>) -> State {
    core.state()
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
fn quit(app: AppHandle, core: tauri::State<Core>) {
    core.shutdown();
    app.exit(0);
}

#[tauri::command]
fn open_main(app: AppHandle) {
    if let Some(flyout) = app.get_webview_window(FLYOUT) {
        let _ = flyout.hide();
    }
    std::thread::spawn(move || show_main(&app, false));
}

/// The flyout page reports its content height, so the window fits it exactly.
#[tauri::command]
fn flyout_size(app: AppHandle, height: f64) {
    app.state::<Mutex<Flyout>>().lock().unwrap().height = Some(height);
    place_flyout(&app);
}

/// The page calls this after its first render, so a window never flashes blank.
#[tauri::command]
fn ready(window: WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
}

/// Native title bar theme. `None` follows Windows.
fn theme(dark: Option<bool>) -> Option<Theme> {
    dark.map(|dark| if dark { Theme::Dark } else { Theme::Light })
}

/// Focuses the main window, or creates it, optionally on its settings page. Closing
/// it destroys the webview, so the tray-only state costs no WebView2 memory.
fn show_main(app: &AppHandle, settings: bool) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        if settings {
            let _ = window.emit_to(MAIN, "page", "settings");
        }
        return;
    }
    let page = if settings { "index.html#settings" } else { "index.html" };
    // Must run off the main thread, because WebView2 creation deadlocks inside event-loop callbacks.
    let settings = app.state::<Core>().state().settings;
    let _ = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App(page.into()))
        .theme(theme(settings.dark))
        .decorations(!settings.custom_title_bar)
        .title("Media Keys Selector")
        .inner_size(420.0, 640.0)
        .min_inner_size(360.0, 480.0)
        .maximizable(false)
        .center()
        .visible(false)
        .build();
}

/// Sizes the flyout to its content and puts it next to the tray icon.
fn place_flyout(app: &AppHandle) {
    let Some(window) = app.get_webview_window(FLYOUT) else { return };
    let (anchor, height) = {
        let flyout = app.state::<Mutex<Flyout>>();
        let flyout = flyout.lock().unwrap();
        (flyout.anchor, flyout.height.unwrap_or(480.0))
    };
    let Some(anchor) = anchor else { return };
    let scale = anchor.scale as f64;
    let _ = window.set_size(LogicalSize::new(FLYOUT_WIDTH, height));
    let (x, y) = anchor.place((FLYOUT_WIDTH * scale) as i32, (height * scale) as i32);
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

fn hide_flyout(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(FLYOUT) {
        let _ = window.hide();
    }
    app.state::<Mutex<Flyout>>().lock().unwrap().closed_at = Some(Instant::now());
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

/// Opens the flyout next to the tray icon, or closes it if it is open.
fn toggle_flyout(app: &AppHandle, anchor: Anchor) {
    let existing = app.get_webview_window(FLYOUT);
    if existing.as_ref().is_some_and(|w| w.is_visible().unwrap_or(false)) {
        hide_flyout(app);
        return;
    }
    {
        let flyout = app.state::<Mutex<Flyout>>();
        let mut flyout = flyout.lock().unwrap();
        if flyout.closed_at.is_some_and(|at| at.elapsed() < REOPEN_GUARD) {
            return;
        }
        flyout.anchor = Some(anchor);
        flyout.closed_at = None;
    }
    if let Some(window) = existing {
        place_flyout(app);
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let dark = app.state::<Core>().state().settings.dark;
    let built = WebviewWindowBuilder::new(app, FLYOUT, WebviewUrl::App("flyout.html".into()))
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
    let settings = mediakeys_core::start_in_settings();

    let app = tauri::Builder::default()
        // tao's raw keyboard input registration stops low-level hooks in this process.
        .device_event_filter(tauri::DeviceEventFilter::Always)
        .manage(core)
        .manage(Mutex::new(Flyout::default()))
        .invoke_handler(tauri::generate_handler![
            state,
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
            quit,
            open_main,
            flyout_size,
            ready
        ])
        .on_window_event(|window, event| match (window.label(), event) {
            // Clicking anywhere else closes the flyout, like the Windows one.
            (FLYOUT, WindowEvent::Focused(false)) => hide_flyout(window.app_handle()),
            (MAIN, WindowEvent::CloseRequested { .. }) => {
                let core = window.state::<Core>();
                if !core.state().settings.close_to_tray {
                    core.shutdown();
                    window.app_handle().exit(0);
                }
            }
            _ => {}
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                if settings || !hidden {
                    show_main(&handle, settings);
                }
                for event in rx {
                    match event {
                        Event::State(state) => {
                            for label in [MAIN, FLYOUT] {
                                if let Some(window) = handle.get_webview_window(label) {
                                    let _ = window.set_theme(theme(state.settings.dark));
                                }
                            }
                            if let Some(window) = handle.get_webview_window(MAIN) {
                                let _ = window.set_decorations(!state.settings.custom_title_bar);
                            }
                            let _ = handle.emit("state", state);
                        }
                        Event::Show => show_main(&handle, false),
                        Event::ShowSettings => show_main(&handle, true),
                        Event::Flyout(anchor) => toggle_flyout(&handle, anchor),
                        Event::Quit => handle.exit(0),
                    }
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("build tauri app");

    // Closing windows keeps the app alive in the tray; Quit exits.
    app.run(|_, event| {
        if let RunEvent::ExitRequested { code: None, api, .. } = event {
            api.prevent_exit();
        }
    });
}

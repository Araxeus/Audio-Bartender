#![windows_subsystem = "windows"]

use std::sync::mpsc;

use mediakeys_core::{Core, Event, MediaKey, State};
use tauri::{AppHandle, Emitter, Manager, RunEvent, Theme, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const MAIN: &str = "main";

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
fn set_autostart(core: tauri::State<Core>, on: bool) {
    core.set_autostart(on);
}

#[tauri::command]
fn adjust_volume(core: tauri::State<Core>, id: String, delta: f32) {
    core.adjust_volume(id, delta);
}

#[tauri::command]
fn set_custom_title_bar(core: tauri::State<Core>, on: bool) {
    core.set_custom_title_bar(on);
}

#[tauri::command]
fn set_dark(core: tauri::State<Core>, dark: Option<bool>) {
    core.set_dark(dark);
}

/// Native title bar theme. `None` follows Windows.
fn theme(dark: Option<bool>) -> Option<Theme> {
    dark.map(|dark| if dark { Theme::Dark } else { Theme::Light })
}

/// The page calls this after its first render, so the window never flashes blank.
#[tauri::command]
fn ready(window: WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
}

/// Focuses the window, or creates it. Closing the window destroys the webview, so the
/// tray-only state costs no WebView2 memory.
fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    // Must run off the main thread, because WebView2 creation deadlocks inside event-loop callbacks.
    let state = app.state::<Core>().state();
    let _ = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::default())
        .theme(theme(state.dark))
        .decorations(!state.custom_title_bar)
        .title("Media Keys Selector")
        .inner_size(400.0, 540.0)
        .min_inner_size(340.0, 420.0)
        .maximizable(false)
        .center()
        .visible(false)
        .build();
}

fn main() {
    let (tx, rx) = mpsc::channel();
    let Some(core) = Core::start(move |event| {
        let _ = tx.send(event);
    }) else {
        return;
    };
    let hidden = mediakeys_core::start_hidden();

    let app = tauri::Builder::default()
        // tao's raw keyboard input registration stops low-level hooks in this process.
        .device_event_filter(tauri::DeviceEventFilter::Always)
        .manage(core)
        .invoke_handler(tauri::generate_handler![state, select, press, set_autostart, adjust_volume, set_dark, set_custom_title_bar, ready])
        .setup(move |app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                if !hidden {
                    show(&handle);
                }
                for event in rx {
                    match event {
                        Event::State(state) => {
                            if let Some(window) = handle.get_webview_window(MAIN) {
                                let _ = window.set_theme(theme(state.dark));
                                let _ = window.set_decorations(!state.custom_title_bar);
                            }
                            let _ = handle.emit("state", state);
                        }
                        Event::Show => show(&handle),
                        Event::Quit => handle.exit(0),
                    }
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("build tauri app");

    // Closing the window keeps the app alive in the tray; only tray Quit exits.
    app.run(|_, event| {
        if let RunEvent::ExitRequested { code: None, api, .. } = event {
            api.prevent_exit();
        }
    });
}

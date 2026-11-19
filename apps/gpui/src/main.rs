#![windows_subsystem = "windows"]

mod flyout;
mod main_view;
mod theme;
mod ui;

use std::borrow::Cow;
use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui_kit::component::TitleBar;
use gpui_kit::*;
use mediakeys_core::{Anchor, Core, Event, State};

use crate::flyout::Flyout;
use crate::main_view::MainView;

gpui_kit::assets::icon_assets!(
    AppIcons,
    [
        Play, SkipBack, SkipForward, Sparkles, Moon, Sun, AppWindow, Volume, Volume1, Volume2, VolumeX, Settings,
        Power, ChevronDown, ChevronUp, ArrowLeft, Speaker, Check
    ]
);

/// The component defaults plus the app's own icons.
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = AppIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(AppIcons.list(path)?);
        Ok(paths)
    }
}

struct Globals {
    core: Core,
    state: Entity<State>,
    main: Option<AnyWindowHandle>,
    main_view: Option<WeakEntity<MainView>>,
    flyout: Option<AnyWindowHandle>,
    flyout_closed: Option<Instant>,
}

impl Global for Globals {}

/// A tray click that lands right after the flyout closed is the click that closed it.
const REOPEN_GUARD: Duration = Duration::from_millis(300);

fn alive(handle: Option<AnyWindowHandle>, cx: &mut App) -> Option<AnyWindowHandle> {
    let handle = handle?;
    handle.update(cx, |_, _, _| ()).ok().map(|_| handle)
}

/// Focuses the main window, or opens it.
pub fn show_main(cx: &mut App) {
    if let Some(handle) = alive(cx.global::<Globals>().main, cx) {
        let _ = handle.update(cx, |_, window, _| window.activate_window());
        return;
    }
    open_main(cx, None);
}

pub fn open_main(cx: &mut App, bounds: Option<Bounds<Pixels>>) {
    let globals = cx.global::<Globals>();
    let (core, state) = (globals.core.clone(), globals.state.clone());
    let custom = state.read(cx).settings.custom_title_bar;

    // The title also names the taskbar and Alt+Tab entries when the caption is hidden.
    let title = Some("Media Keys Selector".into());
    let base = if custom {
        let options = TitleBar::window_options();
        WindowOptions { titlebar: Some(TitlebarOptions { title, ..TitleBar::title_bar_options() }), ..options }
    } else {
        WindowOptions { titlebar: Some(TitlebarOptions { title, ..Default::default() }), ..Default::default() }
    };
    let bounds = bounds.unwrap_or_else(|| Bounds::centered(None, size(px(420.), px(640.)), cx));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(360.), px(480.))),
        app_id: Some("MediaKeysSelector".into()),
        ..base
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        // Closing quits unless the close button is set to hide to the tray.
        let (state, core) = (state.clone(), core.clone());
        window.on_window_should_close(cx, move |_, cx| {
            if !state.read(cx).settings.close_to_tray {
                core.shutdown();
                cx.quit();
            }
            true
        });
        let (core, state) = (cx.global::<Globals>().core.clone(), cx.global::<Globals>().state.clone());
        cx.new(|cx| MainView::new(core, state, window, cx))
    });
    if let Ok((handle, view)) = opened {
        let globals = cx.global_mut::<Globals>();
        globals.main = Some(handle);
        globals.main_view = Some(view.downgrade());
    }
}

/// Shows the main window on its settings page.
pub fn show_settings(cx: &mut App) {
    show_main(cx);
    if let Some(view) = cx.global::<Globals>().main_view.clone().and_then(|v| v.upgrade()) {
        view.update(cx, |view, cx| view.open_settings(cx));
    }
}

/// Opens the flyout next to the tray icon, or closes it if it is open.
fn toggle_flyout(cx: &mut App, anchor: Anchor) {
    if let Some(handle) = alive(cx.global::<Globals>().flyout, cx) {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
        flyout_closed(cx);
        return;
    }
    if cx.global::<Globals>().flyout_closed.is_some_and(|at| at.elapsed() < REOPEN_GUARD) {
        return;
    }

    let globals = cx.global::<Globals>();
    let (core, state) = (globals.core.clone(), globals.state.clone());
    let height = flyout::height(state.read(cx));
    let scale = anchor.scale;
    let (x, y) = anchor.place((flyout::WIDTH * scale) as i32, (height * scale) as i32);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(x as f32 / scale), px(y as f32 / scale)),
            size: size(px(flyout::WIDTH), px(height)),
        })),
        titlebar: None,
        kind: WindowKind::PopUp,
        focus: true,
        show: true,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        app_id: Some("MediaKeysSelector".into()),
        ..Default::default()
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Flyout::new(core, state, window, cx)));
    if let Ok((handle, _)) = opened {
        let _ = handle.update(cx, |_, window, _| window.activate_window());
        cx.global_mut::<Globals>().flyout = Some(handle);
    }
}

pub fn flyout_closed(cx: &mut App) {
    let globals = cx.global_mut::<Globals>();
    globals.flyout = None;
    globals.flyout_closed = Some(Instant::now());
}

fn main() {
    let (tx, mut rx) = futures::channel::mpsc::unbounded();
    let Some(core) = Core::start(move |event| {
        let _ = tx.unbounded_send(event);
    }) else {
        return;
    };
    let hidden = mediakeys_core::start_hidden();

    gpui_kit::application().with_assets(AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        cx.set_quit_mode(QuitMode::Explicit);
        let state = cx.new(|_| core.state());
        cx.set_global(Globals { core, state, main: None, main_view: None, flyout: None, flyout_closed: None });
        if mediakeys_core::start_in_settings() {
            show_settings(cx);
        } else if !hidden {
            show_main(cx);
        }

        cx.spawn(async move |cx| {
            while let Some(event) = rx.next().await {
                cx.update(|cx| match event {
                    Event::State(next) => {
                        let state = cx.global::<Globals>().state.clone();
                        state.update(cx, |state, cx| {
                            *state = next;
                            cx.notify();
                        });
                    }
                    Event::Show => show_main(cx),
                    Event::ShowSettings => show_settings(cx),
                    Event::Flyout(anchor) => toggle_flyout(cx, anchor),
                    Event::Quit => cx.quit(),
                });
            }
        })
        .detach();
    });
}

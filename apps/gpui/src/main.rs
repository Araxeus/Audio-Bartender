#![windows_subsystem = "windows"]

mod flyout;
mod theme;
mod ui;

use std::borrow::Cow;
use std::time::{Duration, Instant};

use audio_bartender_core::{Anchor, Core, Event, Page, State};
use futures::StreamExt;
use gpui_kit::*;

use crate::flyout::Flyout;

gpui_kit::assets::icon_assets!(
    AppIcons,
    [
        Play,
        SkipBack,
        SkipForward,
        Sparkles,
        Moon,
        Sun,
        Volume,
        Volume1,
        Volume2,
        VolumeX,
        Settings,
        Power,
        ChevronDown,
        ChevronUp,
        Speaker,
        Check,
        Pin,
        PinOff
    ]
);

/// The component defaults plus the app's own icons.
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        // The Back Bar mark, in its small-size cut.
        if path == "brand/glyph.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!("../../../assets/logo/glyph-small.svg"))));
        }
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
    flyout: Option<(AnyWindowHandle, WeakEntity<Flyout>)>,
    flyout_closed: Option<Instant>,
}

impl Global for Globals {}

/// A tray click that lands right after the flyout closed is the click that closed it.
const REOPEN_GUARD: Duration = Duration::from_millis(300);

/// The open flyout, if its window still exists.
fn open_flyout(cx: &mut App) -> Option<(AnyWindowHandle, Entity<Flyout>)> {
    let (handle, view) = cx.global::<Globals>().flyout.clone()?;
    let view = view.upgrade()?;
    handle.update(cx, |_, _, _| ()).ok()?;
    Some((handle, view))
}

/// Shows the flyout on `page`. With `toggle` (a tray click) an open flyout closes.
fn show_flyout(cx: &mut App, anchor: Anchor, page: Page, toggle: bool, pinned: bool) {
    if let Some((handle, view)) = open_flyout(cx) {
        if toggle {
            view.update(cx, |view, cx| view.close(cx));
        } else {
            view.update(cx, |view, cx| view.open_page(page, cx));
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
        return;
    }
    if toggle && cx.global::<Globals>().flyout_closed.is_some_and(|at| at.elapsed() < REOPEN_GUARD) {
        return;
    }

    let globals = cx.global::<Globals>();
    let (core, state) = (globals.core.clone(), globals.state.clone());
    let height = flyout::height(state.read(cx), page);
    let scale = anchor.scale;
    // Starts behind the taskbar, unless Windows animations are off.
    let start = audio_bartender_core::slide_progress(std::time::Instant::now());
    let (x, y) = anchor.place_sliding((flyout::WIDTH * scale) as i32, (height * scale) as i32, start);
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
        app_id: Some("AudioBartender".into()),
        ..Default::default()
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Flyout::new(core, state, anchor, page, pinned, window, cx))
    });
    if let Ok((handle, view)) = opened {
        let _ = handle.update(cx, |_, window, _| window.activate_window());
        cx.global_mut::<Globals>().flyout = Some((handle, view.downgrade()));
    }
}

pub fn flyout_closed(cx: &mut App) {
    let globals = cx.global_mut::<Globals>();
    globals.flyout = None;
    globals.flyout_closed = Some(Instant::now());
}

fn main() {
    audio_bartender_core::set_flavor(audio_bartender_core::Flavor::Native);
    let (tx, mut rx) = futures::channel::mpsc::unbounded();
    let Some(core) = Core::start(move |event| {
        let _ = tx.unbounded_send(event);
    }) else {
        return;
    };
    let hidden = audio_bartender_core::start_hidden();
    let pinned = audio_bartender_core::start_pinned();

    gpui_kit::application().with_assets(AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        cx.set_quit_mode(QuitMode::Explicit);
        let state = cx.new(|_| core.state());
        let anchor = core.default_anchor();
        cx.set_global(Globals { core, state, flyout: None, flyout_closed: None });
        if !hidden {
            show_flyout(cx, anchor, audio_bartender_core::start_page(), false, pinned);
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
                    Event::Flyout { anchor, page, toggle } => show_flyout(cx, anchor, page, toggle, false),
                    Event::Quit => cx.quit(),
                });
            }
        })
        .detach();
    });
}

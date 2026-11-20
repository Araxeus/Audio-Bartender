//! Media key routing and volume control shared by the GUI front ends.
//!
//! The core owns:
//! - a low-level keyboard hook that swallows media keys while a target is chosen,
//! - a low-level mouse hook that turns the wheel over the tray icon into master volume,
//! - a worker that tracks media sessions, audio sessions and output devices,
//! - the tray icon, drawn as a live volume indicator, and its right-click menu.
//!
//! A front end calls [`Core::start`], renders [`State`] snapshots from [`Event::State`],
//! shows its flyout on [`Event::Flyout`] and exits on [`Event::Quit`]. The flyout is
//! the whole UI.

mod audio;
mod config;
mod icon;
mod input;
mod media;
mod names;
mod tooltip;
mod tray_icon;
mod util;
mod volume;

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "camelCase"))]
pub enum MediaKey {
    PlayPause,
    Next,
    Previous,
    Stop,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub enum Playback {
    #[default]
    Unknown,
    Playing,
    Paused,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Bmp,
    Gif,
    Webp,
}

impl ImageKind {
    pub fn mime(self) -> &'static str {
        match self {
            ImageKind::Png => "image/png",
            ImageKind::Jpeg => "image/jpeg",
            ImageKind::Bmp => "image/bmp",
            ImageKind::Gif => "image/gif",
            ImageKind::Webp => "image/webp",
        }
    }

}

/// An encoded image: an app icon or album art. Serializes as a `data:` URL.
#[derive(Clone, PartialEq)]
pub struct AppIcon {
    pub data: Arc<[u8]>,
    pub kind: ImageKind,
}

impl std::fmt::Debug for AppIcon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AppIcon({:?}, {} bytes)", self.kind, self.data.len())
    }
}

#[cfg(feature = "serde")]
impl Serialize for AppIcon {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("data:{};base64,{}", self.kind.mime(), icon::base64(&self.data)))
    }
}

/// One app that can receive media keys.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct Target {
    /// `smtc:<AppUserModelID>` or `exe:<file name>`.
    pub id: String,
    pub name: String,
    pub playback: Playback,
    /// "Title — Artist" of the current track, when the app reports one.
    pub track: Option<String>,
    /// False for a remembered target whose app is closed.
    pub running: bool,
    /// True when keys reach the app through window messages instead of SMTC.
    pub via_window: bool,
    /// App volume from 0 to 1, when the app has an audio session.
    pub volume: Option<f32>,
    pub muted: bool,
    /// Album art when [`Settings::album_art`] is on and the app has some, else the exe icon.
    pub icon: Option<AppIcon>,
}

/// An app playing audio that isn't a media key target, as in the Volume Mixer.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct MixerApp {
    /// `exe:<file name>`, accepted wherever a target id is.
    pub id: String,
    pub name: String,
    pub volume: f32,
    pub muted: bool,
    pub icon: Option<AppIcon>,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct Device {
    pub id: String,
    pub name: String,
    pub default: bool,
}

/// The default output device's state plus the other devices.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct Audio {
    pub devices: Vec<Device>,
    /// Master volume from 0 to 1.
    pub volume: f32,
    pub muted: bool,
}

impl Audio {
    pub fn device(&self) -> Option<&Device> {
        self.devices.iter().find(|d| d.default)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "camelCase"))]
pub enum TrayStyle {
    /// A ring around a speaker that fills with the volume.
    #[default]
    Ring,
    /// The volume as a number.
    Number,
}

/// How app rows show their volume.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "camelCase"))]
pub enum VolumeBars {
    /// A small bar that fills with the volume. Drag or scroll it; click to mute.
    #[default]
    Compact,
    /// A slider under every app, always visible.
    Always,
    /// A percentage; clicking the row opens its slider.
    OnClick,
}

/// Preferences shared by both front ends, stored in `config.ini`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct Settings {
    pub autostart: bool,
    /// Forced dark (`true`) or light (`false`) theme. `None` follows Windows.
    pub dark: Option<bool>,
    /// Show the current track's album art in place of the app icon.
    pub album_art: bool,
    pub tray_style: TrayStyle,
    pub volume_bars: VolumeBars,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            autostart: false,
            dark: None,
            album_art: false,
            tray_style: TrayStyle::Ring,
            volume_bars: VolumeBars::Compact,
        }
    }
}

/// One settings change.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Deserialize), serde(rename_all = "camelCase"))]
pub enum Setting {
    Autostart(bool),
    Dark(Option<bool>),
    AlbumArt(bool),
    TrayStyle(TrayStyle),
    VolumeBars(VolumeBars),
}

/// Windows pages for arranging tray icons. Windows lets only the user move icons,
/// so the app opens the right page instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Deserialize), serde(rename_all = "camelCase"))]
pub enum IconPage {
    /// Notification Area Icons: set this app to always show on the taskbar.
    ShowOurs,
    /// System Icons: turn off the Windows volume icon.
    HideWindows,
}

/// Opens a Windows page for arranging tray icons.
pub fn open_icon_page(page: IconPage) {
    util::open_icon_page(page);
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct State {
    pub targets: Vec<Target>,
    pub mixer: Vec<MixerApp>,
    pub audio: Audio,
    /// Chosen target id. `None` leaves media keys to Windows.
    pub selected: Option<String>,
    pub settings: Settings,
}

impl State {
    /// The target that currently receives media keys, if routing is active.
    pub fn routed(&self) -> Option<&Target> {
        let id = self.selected.as_deref()?;
        self.targets.iter().find(|t| t.id == id && t.running)
    }
}

/// Where the tray icon sits, for placing a flyout next to it. All values are physical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct Anchor {
    pub icon: Rect,
    /// The work area (screen minus taskbar) of the icon's monitor.
    pub work: Rect,
    /// The whole monitor.
    pub screen: Rect,
    /// DPI scale of that monitor.
    pub scale: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// How long a flyout takes to slide out from behind the taskbar.
pub const SLIDE: std::time::Duration = std::time::Duration::from_millis(150);

/// How far a flyout opened at `since` has slid out, from 0 (hidden behind the
/// taskbar) to 1 (in place). Eases out, and is always 1 when Windows animations are off.
pub fn slide_progress(since: std::time::Instant) -> f32 {
    if !util::animations_enabled() {
        return 1.0;
    }
    let t = (since.elapsed().as_secs_f32() / SLIDE.as_secs_f32()).min(1.0);
    1.0 - (1.0 - t).powi(3)
}

/// How long a closing flyout takes to slide back behind the taskbar.
pub const SLIDE_OUT: std::time::Duration = std::time::Duration::from_millis(90);

/// How far a flyout closing since `since` is still out, from 1 down to 0 (hidden).
/// Eases in, and is always 0 when Windows animations are off.
pub fn slide_out_progress(since: std::time::Instant) -> f32 {
    if !util::animations_enabled() {
        return 0.0;
    }
    let t = (since.elapsed().as_secs_f32() / SLIDE_OUT.as_secs_f32()).min(1.0);
    1.0 - t * t
}

impl Anchor {
    /// Top-left corner for a flyout of the given physical size: flush against the
    /// taskbar like the Windows flyouts, centered on the icon, kept inside the work area.
    pub fn place(&self, width: i32, height: i32) -> (i32, i32) {
        // Margin from the screen edges along the taskbar; none towards the taskbar.
        let gap = (12.0 * self.scale) as i32;
        let (w, i) = (self.work, self.icon);
        let clamp_x = |x: i32| x.clamp(w.left + gap, (w.right - width - gap).max(w.left + gap));
        let clamp_y = |y: i32| y.clamp(w.top + gap, (w.bottom - height - gap).max(w.top + gap));
        let center_x = (i.left + i.right) / 2 - width / 2;
        let center_y = (i.top + i.bottom) / 2 - height / 2;
        if i.top >= w.bottom {
            (clamp_x(center_x), w.bottom - height)
        } else if i.bottom <= w.top {
            (clamp_x(center_x), w.top)
        } else if i.left >= w.right {
            (w.right - width, clamp_y(center_y))
        } else if i.right <= w.left {
            (w.left, clamp_y(center_y))
        } else {
            (w.right - width - gap, w.bottom - height)
        }
    }

    /// Like [`Anchor::place`], but `1 - progress` of the way towards the screen edge
    /// behind the taskbar, where the flyout starts fully off screen.
    pub fn place_sliding(&self, width: i32, height: i32, progress: f32) -> (i32, i32) {
        let (x, y) = self.place(width, height);
        let (w, i, s) = (self.work, self.icon, self.screen);
        let rest = 1.0 - progress.clamp(0.0, 1.0);
        let off = |distance: i32| (distance as f32 * rest).round() as i32;
        if i.bottom <= w.top {
            (x, y - off(y + height - s.top))
        } else if i.top < w.bottom && i.left >= w.right {
            (x + off(s.right - x), y)
        } else if i.top < w.bottom && i.right <= w.left {
            (x - off(x + width - s.left), y)
        } else {
            (x, y + off(s.bottom - y))
        }
    }
}

/// What the flyout shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub enum Page {
    #[default]
    Apps,
    Settings,
}

#[derive(Clone, Debug)]
pub enum Event {
    State(State),
    /// Show the flyout next to `anchor` on `page`. With `toggle` (a tray click), an
    /// open flyout closes instead.
    Flyout { anchor: Anchor, page: Page, toggle: bool },
    /// The user picked Quit in the tray menu. The tray icon is already gone.
    Quit,
}

pub(crate) enum Cmd {
    Refresh { resubscribe: bool },
    /// Volumes or devices changed. Cheaper than a full refresh.
    RefreshAudio { devices: bool },
    Select(Option<String>),
    Press(MediaKey),
    Set(Setting),
}

pub(crate) struct Shared {
    state: Mutex<State>,
    on_event: Box<dyn Fn(Event) + Send + Sync>,
    tx: Sender<Cmd>,
}

impl Shared {
    pub(crate) fn emit(&self, event: Event) {
        (self.on_event)(event)
    }

    pub(crate) fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    pub(crate) fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }
}

/// Handle to the running core. Cheap to clone and usable from any thread.
///
/// Volume calls act immediately on the calling thread, so sliders and wheels feel
/// instant. Everything else goes through the worker.
#[derive(Clone)]
pub struct Core {
    shared: Arc<Shared>,
}

impl Core {
    /// Starts the hooks, tray and worker.
    ///
    /// Returns `None` when another instance already runs. That instance is asked to
    /// show its window, so the caller should simply exit.
    /// `on_event` runs on core threads and must hand work off quickly.
    pub fn start(on_event: impl Fn(Event) + Send + Sync + 'static) -> Option<Core> {
        if !input::claim_single_instance() {
            return None;
        }
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), on_event: Box::new(on_event), tx });
        input::spawn(shared.clone());
        media::spawn(shared.clone(), rx);
        Some(Core { shared })
    }

    pub fn state(&self) -> State {
        self.shared.state()
    }

    /// Chooses the target for media keys. `None` hands them back to Windows.
    pub fn select(&self, id: Option<String>) {
        self.shared.send(Cmd::Select(id));
    }

    /// Sends a key to the routed target, or to Windows' current session in automatic mode.
    pub fn press(&self, key: MediaKey) {
        self.shared.send(Cmd::Press(key));
    }

    pub fn set(&self, setting: Setting) {
        self.shared.send(Cmd::Set(setting));
    }

    /// Changes an app's volume by `delta` (volume runs from 0 to 1).
    pub fn adjust_volume(&self, id: &str, delta: f32) {
        util::com_init();
        volume::adjust(id, |level| level + delta);
        self.shared.send(Cmd::RefreshAudio { devices: false });
    }

    pub fn set_volume(&self, id: &str, level: f32) {
        util::com_init();
        volume::adjust(id, |_| level);
        self.shared.send(Cmd::RefreshAudio { devices: false });
    }

    pub fn toggle_mute(&self, id: &str) {
        util::com_init();
        volume::toggle_mute(id);
        self.shared.send(Cmd::RefreshAudio { devices: false });
    }

    /// Changes the master volume by `delta`. Raising it also unmutes.
    pub fn adjust_master(&self, delta: f32) {
        util::com_init();
        audio::adjust_master(delta);
    }

    pub fn set_master(&self, level: f32) {
        util::log(format!("set master {level:.2}"));
        util::com_init();
        audio::set_master(level);
    }

    pub fn toggle_master_mute(&self) {
        util::com_init();
        audio::toggle_master_mute();
    }

    /// Makes `id` the default output device.
    pub fn set_device(&self, id: &str) {
        util::com_init();
        audio::set_default_device(id);
    }

    /// Where to open the flyout without a tray click: next to the tray icon, or the
    /// bottom-right corner of the primary screen.
    pub fn default_anchor(&self) -> Anchor {
        input::default_anchor()
    }

    /// Removes the tray icon. Call before exiting for reasons other than [`Event::Quit`].
    pub fn shutdown(&self) {
        util::log("shutdown");
        input::remove_tray();
    }
}

/// Renders the tray icon as PNG, for previews and tests.
#[doc(hidden)]
pub fn preview_tray_icon(level: f32, muted: bool, style: TrayStyle, light_taskbar: bool, size: i32) -> Option<Vec<u8>> {
    let look = tray_icon::Look { level, muted, style, light_taskbar };
    let mut pixels = tray_icon::bgra(&look, size)?;
    pixels.chunks_exact_mut(4).for_each(|px| px.swap(0, 2));
    Some(icon::encode_png(size as u32, size as u32, &pixels))
}

/// Moves a window (by its raw handle), keeping its size, to where `anchor` places a
/// flyout `progress` of the way through its slide (see [`slide_progress`]). Call after
/// resizing so the bottom stays against the taskbar.
pub fn place_window(hwnd: isize, anchor: &Anchor, progress: f32) {
    util::place_window(hwnd, anchor, progress);
}

/// Hides (cloaks) or shows a window without changing its visibility state, for
/// moving a just-created window into place unseen.
pub fn set_cloaked(hwnd: isize, cloaked: bool) {
    util::set_cloaked(hwnd, cloaked);
}

/// Like [`place_window`], for a borderless window of the given physical size,
/// which may not have applied yet.
pub fn place_window_sized(hwnd: isize, anchor: &Anchor, width: i32, height: i32, progress: f32) {
    util::place_window_sized(hwnd, anchor, width, height, progress);
}

/// The page to open on launch: `--settings` opens the settings page.
pub fn start_page() -> Page {
    if std::env::args().any(|a| a == "--settings") { Page::Settings } else { Page::Apps }
}

/// True when the app was launched with `--pinned`: the flyout stays open when it
/// loses focus, which helps comparing front ends side by side.
pub fn start_pinned() -> bool {
    std::env::args().any(|a| a == "--pinned")
}

/// True when the app was launched by autostart and should stay in the tray.
pub fn start_hidden() -> bool {
    std::env::args().any(|a| a == "--hidden")
}

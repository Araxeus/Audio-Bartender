//! Media key routing and volume control shared by the GUI front ends.
//!
//! The core owns:
//! - a low-level keyboard hook that swallows media keys while a target is chosen,
//! - a low-level mouse hook that turns the wheel over the tray icon into master volume,
//! - a worker that tracks media sessions, audio sessions and output devices,
//! - the tray icon, drawn as a live volume indicator, and its right-click menu.
//!
//! A front end calls [`Core::start`], renders [`State`] snapshots from [`Event::State`],
//! and reacts to [`Event::Show`], [`Event::Flyout`] and [`Event::Quit`].

mod audio;
mod config;
mod icon;
mod input;
mod media;
mod names;
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

/// Preferences shared by both front ends, stored in `config.ini`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct Settings {
    pub autostart: bool,
    /// Forced dark (`true`) or light (`false`) theme. `None` follows Windows.
    pub dark: Option<bool>,
    /// The window draws its own title bar instead of the native one.
    pub custom_title_bar: bool,
    /// The close button hides the window to the tray instead of quitting.
    pub close_to_tray: bool,
    /// Show the current track's album art in place of the app icon.
    pub album_art: bool,
    pub tray_style: TrayStyle,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            autostart: false,
            dark: None,
            custom_title_bar: true,
            close_to_tray: true,
            album_art: false,
            tray_style: TrayStyle::Ring,
        }
    }
}

/// One settings change.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Deserialize), serde(rename_all = "camelCase"))]
pub enum Setting {
    Autostart(bool),
    Dark(Option<bool>),
    CustomTitleBar(bool),
    CloseToTray(bool),
    AlbumArt(bool),
    TrayStyle(TrayStyle),
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

impl Anchor {
    /// Top-left corner for a flyout of the given physical size: against the taskbar,
    /// centered on the icon and kept inside the work area.
    pub fn place(&self, width: i32, height: i32) -> (i32, i32) {
        let gap = (12.0 * self.scale) as i32;
        let (w, i) = (self.work, self.icon);
        let clamp_x = |x: i32| x.clamp(w.left + gap, (w.right - width - gap).max(w.left + gap));
        let clamp_y = |y: i32| y.clamp(w.top + gap, (w.bottom - height - gap).max(w.top + gap));
        let center_x = (i.left + i.right) / 2 - width / 2;
        let center_y = (i.top + i.bottom) / 2 - height / 2;
        if i.top >= w.bottom {
            (clamp_x(center_x), w.bottom - height - gap)
        } else if i.bottom <= w.top {
            (clamp_x(center_x), w.top + gap)
        } else if i.left >= w.right {
            (w.right - width - gap, clamp_y(center_y))
        } else if i.right <= w.left {
            (w.left + gap, clamp_y(center_y))
        } else {
            (w.right - width - gap, w.bottom - height - gap)
        }
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    State(State),
    /// The user asked to see the main window (a second launch or the tray menu).
    Show,
    /// The tray icon was left-clicked. Show the flyout next to it.
    Flyout(Anchor),
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

    /// Removes the tray icon. Call before exiting for reasons other than [`Event::Quit`].
    pub fn shutdown(&self) {
        input::remove_tray();
    }
}

/// Switches a native title bar between dark and light.
pub fn set_dark_title_bar(hwnd: isize, dark: bool) {
    util::set_dark_title_bar(hwnd, dark);
}

/// Renders the tray icon as PNG, for previews and tests.
#[doc(hidden)]
pub fn preview_tray_icon(level: f32, muted: bool, style: TrayStyle, light_taskbar: bool, size: i32) -> Option<Vec<u8>> {
    let look = tray_icon::Look { level, muted, style, light_taskbar };
    let mut pixels = tray_icon::bgra(&look, size)?;
    pixels.chunks_exact_mut(4).for_each(|px| px.swap(0, 2));
    Some(icon::encode_png(size as u32, size as u32, &pixels))
}

/// True when the app was launched by autostart and should stay in the tray.
pub fn start_hidden() -> bool {
    std::env::args().any(|a| a == "--hidden")
}

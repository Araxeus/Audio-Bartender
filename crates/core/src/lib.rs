//! Media key routing shared by the GUI front ends.
//!
//! The core owns three things:
//! - a low-level keyboard hook that swallows media keys while a target is chosen,
//! - a worker that tracks media sessions (SMTC plus a few window-only players) and
//!   forwards keys to the chosen one,
//! - the tray icon and its menu.
//!
//! A front end calls [`Core::start`], renders [`State`] snapshots from [`Event::State`],
//! and shows or quits on [`Event::Show`] and [`Event::Quit`].

mod config;
mod icon;
mod input;
mod media;
mod names;
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

/// An app icon as PNG bytes. Serializes as a `data:` URL.
#[derive(Clone, PartialEq)]
pub struct AppIcon(pub Arc<[u8]>);

impl AppIcon {
    pub fn png(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for AppIcon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AppIcon({} bytes)", self.0.len())
    }
}

#[cfg(feature = "serde")]
impl Serialize for AppIcon {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("data:image/png;base64,{}", icon::base64(&self.0)))
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
    /// The app's own icon, when its exe has one.
    pub icon: Option<AppIcon>,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct State {
    pub targets: Vec<Target>,
    /// Chosen target id. `None` leaves media keys to Windows.
    pub selected: Option<String>,
    pub autostart: bool,
    /// Forced dark (`true`) or light (`false`) theme. `None` follows Windows.
    pub dark: Option<bool>,
    /// The window draws its own title bar instead of the native one.
    pub custom_title_bar: bool,
}

impl State {
    /// The target that currently receives media keys, if routing is active.
    pub fn routed(&self) -> Option<&Target> {
        let id = self.selected.as_deref()?;
        self.targets.iter().find(|t| t.id == id && t.running)
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    State(State),
    /// The user asked to see the window (tray click or a second launch).
    Show,
    /// The user picked Quit in the tray. The tray icon is already gone.
    Quit,
}

pub(crate) enum Cmd {
    Refresh { resubscribe: bool },
    Select(Option<String>),
    Press(MediaKey),
    SetAutostart(bool),
    Volume { id: String, delta: f32 },
    SetDark(Option<bool>),
    SetCustomTitleBar(bool),
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
#[derive(Clone)]
pub struct Core {
    shared: Arc<Shared>,
}

impl Core {
    /// Starts the hook, tray and media worker.
    ///
    /// Returns `None` when another instance already runs. That instance is asked to
    /// show its window, so the caller should simply exit.
    /// `on_event` runs on core threads and must hand work off quickly.
    pub fn start(on_event: impl Fn(Event) + Send + Sync + 'static) -> Option<Core> {
        if !input::claim_single_instance() {
            return None;
        }
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            on_event: Box::new(on_event),
            tx,
        });
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

    pub fn set_autostart(&self, on: bool) {
        self.shared.send(Cmd::SetAutostart(on));
    }

    /// Changes a target's app volume by `delta` (volume runs from 0 to 1).
    pub fn adjust_volume(&self, id: String, delta: f32) {
        self.shared.send(Cmd::Volume { id, delta });
    }

    /// Forces dark or light, or follows Windows with `None`. Both front ends share it.
    pub fn set_dark(&self, dark: Option<bool>) {
        self.shared.send(Cmd::SetDark(dark));
    }

    /// Switches between the in-app title bar and the native one. Both front ends share it.
    pub fn set_custom_title_bar(&self, on: bool) {
        self.shared.send(Cmd::SetCustomTitleBar(on));
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

/// True when the app was launched by autostart and should stay in the tray.
pub fn start_hidden() -> bool {
    std::env::args().any(|a| a == "--hidden")
}

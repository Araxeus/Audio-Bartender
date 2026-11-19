use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APPCOMMAND};
use windows::core::RuntimeType;

use crate::config::{self, Config};
use crate::names::Names;
use crate::util::{Process, main_window, processes};
use crate::volume;
use crate::{Cmd, Event, MediaKey, Playback, Shared, State, Target, input};

/// Players without SMTC support. They get keys as WM_APPCOMMAND window messages.
const WINDOW_PLAYERS: &[&str] = &[
    "vlc.exe",
    "mpc-hc64.exe",
    "mpc-hc.exe",
    "mpc-be64.exe",
    "mpc-be.exe",
    "aimp.exe",
    "musicbee.exe",
    "winamp.exe",
    "potplayermini64.exe",
    "potplayermini.exe",
    "foobar2000.exe",
];

/// Window-only players appear and vanish without events, so they are polled.
const POLL: Duration = Duration::from_secs(2);

pub fn spawn(shared: Arc<Shared>, rx: Receiver<Cmd>) {
    std::thread::Builder::new()
        .name("mediakeys-media".into())
        .spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            Worker::new(shared).run(rx);
        })
        .expect("spawn media thread");
}

struct Subscription {
    session: Session,
    tokens: [i64; 2],
}

struct Worker {
    shared: Arc<Shared>,
    tx: Sender<Cmd>,
    manager: Option<Manager>,
    subscriptions: Vec<Subscription>,
    config: Config,
    names: Names,
}

impl Worker {
    fn new(shared: Arc<Shared>) -> Worker {
        let tx = shared.tx.clone();
        let manager = Manager::RequestAsync().and_then(|op| op.join()).ok();
        if let Some(manager) = &manager {
            let _ = manager.SessionsChanged(&refresh_on(&tx, true));
            let _ = manager.CurrentSessionChanged(&refresh_on(&tx, false));
        }
        Worker { shared, tx, manager, subscriptions: Vec::new(), config: Config::load(), names: Names::default() }
    }

    fn run(mut self, rx: Receiver<Cmd>) {
        self.refresh(true);
        loop {
            let cmd = match rx.recv_timeout(POLL) {
                Ok(cmd) => cmd,
                Err(RecvTimeoutError::Timeout) => {
                    // Another running front end may have changed the shared config.
                    self.config = Config::load();
                    Cmd::Refresh { resubscribe: false }
                }
                Err(RecvTimeoutError::Disconnected) => return,
            };
            match cmd {
                Cmd::Refresh { mut resubscribe } => {
                    // Session events arrive in bursts, so fold queued refreshes into one.
                    let mut pending = Vec::new();
                    while let Ok(next) = rx.try_recv() {
                        match next {
                            Cmd::Refresh { resubscribe: r } => resubscribe |= r,
                            other => pending.push(other),
                        }
                    }
                    self.refresh(resubscribe);
                    for cmd in pending {
                        self.handle(cmd);
                    }
                }
                other => self.handle(other),
            }
        }
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Refresh { resubscribe } => self.refresh(resubscribe),
            Cmd::Select(id) => {
                let state = self.shared.state();
                self.config.target_name =
                    id.as_ref().and_then(|id| state.targets.iter().find(|t| &t.id == id)).map(|t| t.name.clone());
                self.config.target = id;
                self.config.save();
                self.refresh(false);
            }
            Cmd::Press(key) => self.press(key),
            Cmd::SetAutostart(on) => {
                config::set_autostart(on);
                self.refresh(false);
            }
            Cmd::Volume { id, delta } => {
                let procs = processes();
                volume::adjust(&volume::sessions(&procs), &volume::exes_for(&id), delta);
                // Queue the refresh so a burst of wheel events costs one refresh.
                let _ = self.tx.send(Cmd::Refresh { resubscribe: false });
            }
            Cmd::SetDark(dark) => {
                self.config.dark = dark;
                self.config.save();
                self.refresh(false);
            }
            Cmd::SetCustomTitleBar(on) => {
                self.config.custom_title_bar = on;
                self.config.save();
                self.refresh(false);
            }
        }
    }

    fn sessions(&self) -> Vec<Session> {
        let Some(manager) = &self.manager else { return Vec::new() };
        let Ok(list) = manager.GetSessions() else { return Vec::new() };
        let n = list.Size().unwrap_or(0);
        (0..n).filter_map(|i| list.GetAt(i).ok()).collect()
    }

    /// Re-reads every session and player, then publishes the state if it changed.
    fn refresh(&mut self, resubscribe: bool) {
        let sessions = self.sessions();
        if resubscribe {
            self.resubscribe(&sessions);
        }
        let procs = processes();
        let mut targets: Vec<Target> = Vec::new();

        for session in &sessions {
            let Ok(aumid) = session.SourceAppUserModelId() else { continue };
            let aumid = aumid.to_string();
            let id = format!("smtc:{aumid}");
            let playback = playback(session);
            let track = track(session);

            // Browsers expose one session per tab, so merge them and prefer the playing one.
            if let Some(existing) = targets.iter_mut().find(|t| t.id == id) {
                if existing.playback != Playback::Playing && playback == Playback::Playing {
                    existing.playback = playback;
                    existing.track = track;
                }
                continue;
            }
            let name = self.names.for_aumid(&aumid, &procs);
            targets.push(Target { id, name, playback, track, running: true, via_window: false, volume: None });
        }

        for exe in WINDOW_PLAYERS {
            let has_session = targets.iter().any(|t| t.id.to_lowercase() == format!("smtc:{exe}"));
            let windowed = procs.iter().any(|p| p.exe == *exe && main_window(p.pid).is_some());
            if has_session || !windowed {
                continue;
            }
            let name = self.names.for_exe(exe, &procs);
            targets.push(Target {
                id: format!("exe:{exe}"),
                name,
                playback: Playback::Unknown,
                track: None,
                running: true,
                via_window: true,
                volume: None,
            });
        }

        // Keep a remembered target in the list while its app is closed.
        if let Some(id) = &self.config.target
            && !targets.iter().any(|t| &t.id == id)
        {
            let name = self.config.target_name.clone().unwrap_or_else(|| id.clone());
            targets.push(Target {
                id: id.clone(),
                name,
                playback: Playback::Unknown,
                track: None,
                running: false,
                via_window: id.starts_with("exe:"),
                volume: None,
            });
        }

        let sessions = volume::sessions(&procs);
        for target in targets.iter_mut().filter(|t| t.running) {
            target.volume = volume::level(&sessions, &volume::exes_for(&target.id));
        }

        targets.sort_by_key(|t| (!t.running, t.name.to_lowercase()));
        let state = State {
            targets,
            selected: self.config.target.clone(),
            autostart: config::autostart(),
            dark: self.config.dark,
            custom_title_bar: self.config.custom_title_bar,
        };
        self.publish(state);
    }

    fn publish(&self, state: State) {
        {
            let mut current = self.shared.state.lock().unwrap();
            if *current == state {
                return;
            }
            *current = state.clone();
        }
        input::set_routing(state.routed().map(|t| t.name.clone()));
        self.shared.emit(Event::State(state));
    }

    fn resubscribe(&mut self, sessions: &[Session]) {
        for sub in self.subscriptions.drain(..) {
            let _ = sub.session.RemovePlaybackInfoChanged(sub.tokens[0]);
            let _ = sub.session.RemoveMediaPropertiesChanged(sub.tokens[1]);
        }
        for session in sessions {
            let (Ok(a), Ok(b)) = (
                session.PlaybackInfoChanged(&refresh_on(&self.tx, false)),
                session.MediaPropertiesChanged(&refresh_on(&self.tx, false)),
            )
            else {
                continue;
            };
            self.subscriptions.push(Subscription { session: session.clone(), tokens: [a, b] });
        }
    }

    /// Sends `key` to the routed target, or to Windows' current session in automatic mode.
    fn press(&mut self, key: MediaKey) {
        let state = self.shared.state();
        crate::util::log(format!("press {key:?} -> {:?}", state.routed().map(|t| &t.id)));
        match state.routed() {
            Some(target) if target.via_window => press_window(&target.id["exe:".len()..], key),
            Some(target) => {
                let aumid = &target.id["smtc:".len()..];
                let matching: Vec<Session> = self
                    .sessions()
                    .into_iter()
                    .filter(|s| s.SourceAppUserModelId().is_ok_and(|a| a.to_string() == aumid))
                    .collect();
                let session = matching
                    .iter()
                    .find(|s| playback(s) == Playback::Playing)
                    .or_else(|| matching.first());
                if let Some(session) = session {
                    press_session(session, key);
                }
            }
            None => {
                let current = self.manager.as_ref().and_then(|m| m.GetCurrentSession().ok());
                if let Some(session) = current {
                    press_session(&session, key);
                }
            }
        }
    }
}

/// An event handler that queues a refresh.
fn refresh_on<S: RuntimeType + 'static, A: RuntimeType + 'static>(
    tx: &Sender<Cmd>,
    resubscribe: bool,
) -> TypedEventHandler<S, A> {
    let tx = tx.clone();
    TypedEventHandler::new(move |_, _| {
        let _ = tx.send(Cmd::Refresh { resubscribe });
        Ok(())
    })
}

fn playback(session: &Session) -> Playback {
    let status = session.GetPlaybackInfo().and_then(|info| info.PlaybackStatus());
    match status {
        Ok(Status::Playing) => Playback::Playing,
        Ok(Status::Paused) => Playback::Paused,
        Ok(Status::Stopped) | Ok(Status::Closed) => Playback::Stopped,
        _ => Playback::Unknown,
    }
}

fn track(session: &Session) -> Option<String> {
    let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let title = props.Title().ok()?.to_string();
    let artist = props.Artist().map(|a| a.to_string()).unwrap_or_default();
    match (title.is_empty(), artist.is_empty()) {
        (true, _) => None,
        (false, true) => Some(title),
        (false, false) => Some(format!("{title} — {artist}")),
    }
}

fn press_session(session: &Session, key: MediaKey) {
    let op = match key {
        MediaKey::PlayPause => session.TryTogglePlayPauseAsync(),
        MediaKey::Next => session.TrySkipNextAsync(),
        MediaKey::Previous => session.TrySkipPreviousAsync(),
        MediaKey::Stop => session.TryStopAsync(),
    };
    let result = op.and_then(|op| op.join());
    crate::util::log(format!("  smtc {key:?}: {result:?}"));
}

fn press_window(exe: &str, key: MediaKey) {
    // APPCOMMAND_MEDIA_* values from winuser.h.
    let command: isize = match key {
        MediaKey::Next => 11,
        MediaKey::Previous => 12,
        MediaKey::Stop => 13,
        MediaKey::PlayPause => 14,
    };
    let windows = processes().into_iter().filter(|p: &Process| p.exe == exe).find_map(|p| main_window(p.pid));
    if let Some(hwnd) = windows {
        unsafe {
            let _ = PostMessageW(
                Some(hwnd),
                WM_APPCOMMAND,
                WPARAM(hwnd.0 as usize),
                LPARAM(command << 16),
            );
        }
    }
}

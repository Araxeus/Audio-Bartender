use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as MediaProperties,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APPCOMMAND};
use windows::core::RuntimeType;

use crate::config::Config;
use crate::names::Names;
use crate::util::{Process, main_window, process_path, processes};
use crate::{
    AppIcon, Audio, Cmd, Device, Event, ImageKind, MediaKey, MixerApp, Playback, Setting, Shared, State, Target, audio,
    icon, input, volume,
};

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

/// Window-only players and app volumes change without events, so they are polled.
const POLL: Duration = Duration::from_secs(2);

/// How long after a track starts its art is fetched again on every refresh. Apps
/// report the new title first and the matching thumbnail a moment later, so an
/// early fetch can return nothing or the previous track's art.
const ART_SETTLE: Duration = Duration::from_secs(8);
/// How often a track that settled without art is asked for it again.
const ART_RETRY: Duration = Duration::from_secs(10);

/// Album art above this size is skipped to keep state snapshots small.
const MAX_ART_BYTES: u64 = 512 * 1024;

pub fn spawn(shared: Arc<Shared>, rx: Receiver<Cmd>, config: Config) {
    std::thread::Builder::new()
        .name("bartender-media".into())
        .spawn(move || {
            crate::util::com_init();
            Worker::new(shared, config).run(rx);
        })
        .expect("spawn media thread");
}

struct Subscription {
    session: Session,
    tokens: [i64; 2],
}

/// A media target before volumes and icons are filled in.
struct MediaTarget {
    target: Target,
    /// Album art cache key: the app id plus the track.
    art_key: Option<String>,
    props: Option<MediaProperties>,
}

struct Worker {
    shared: Arc<Shared>,
    tx: Sender<Cmd>,
    manager: Option<Manager>,
    subscriptions: Vec<Subscription>,
    watcher: Option<audio::Watcher>,
    config: Config,
    names: Names,
    media: Vec<MediaTarget>,
    devices: Vec<Device>,
    /// Exe icons by exe name. `None` marks an exe without a usable icon.
    icons: HashMap<String, Option<AppIcon>>,
    /// Album art by target id: the key it was fetched for, the art, when that track
    /// was first seen, and when its art was last fetched.
    art: HashMap<String, (String, Option<AppIcon>, Instant, Instant)>,
    last_refresh: Instant,
    /// The device each process was last sent to, so routes are applied once per
    /// process and again when a process starts or a device returns.
    routed: HashMap<u32, String>,
}

impl Worker {
    fn new(shared: Arc<Shared>, config: Config) -> Worker {
        let tx = shared.tx.clone();
        let manager = Manager::RequestAsync().and_then(|op| op.join()).ok();
        if let Some(manager) = &manager {
            let _ = manager.SessionsChanged(&refresh_on(&tx, true));
            let _ = manager.CurrentSessionChanged(&refresh_on(&tx, false));
        }
        let audio_tx = tx.clone();
        let watcher = audio::Watcher::new(move |devices| {
            let _ = audio_tx.send(Cmd::RefreshAudio { devices });
        });
        Worker {
            shared,
            tx,
            manager,
            subscriptions: Vec::new(),
            watcher,
            config,
            names: Names::default(),
            media: Vec::new(),
            devices: Vec::new(),
            icons: HashMap::new(),
            art: HashMap::new(),
            last_refresh: Instant::now(),
            routed: HashMap::new(),
        }
    }

    fn run(mut self, rx: Receiver<Cmd>) {
        self.devices = self.watcher.as_ref().map(|w| audio::devices(w.enumerator())).unwrap_or_default();
        self.refresh(true);
        loop {
            crate::update::tick(self.config.settings.auto_update, self.config.last_update_check, &self.tx);
            // Busy event streams must not starve the periodic refresh.
            let wait = POLL.saturating_sub(self.last_refresh.elapsed());
            let first = match rx.recv_timeout(wait) {
                Ok(cmd) => cmd,
                Err(RecvTimeoutError::Timeout) => {
                    // Another running front end may have changed the shared config.
                    self.config = Config::load();
                    Cmd::Refresh { resubscribe: false }
                }
                Err(RecvTimeoutError::Disconnected) => return,
            };

            // Events arrive in bursts (wheel ticks, session changes), so fold queued
            // refreshes into one and run the other commands in order.
            let (mut refresh, mut resubscribe, mut audio, mut devices) = (false, false, false, false);
            let mut commands = Vec::new();
            for cmd in std::iter::once(first).chain(std::iter::from_fn(|| rx.try_recv().ok())) {
                match cmd {
                    Cmd::Refresh { resubscribe: r } => {
                        refresh = true;
                        resubscribe |= r;
                    }
                    Cmd::RefreshAudio { devices: d } => {
                        audio = true;
                        devices |= d;
                    }
                    other => commands.push(other),
                }
            }
            if devices {
                self.reload_devices();
            }
            if refresh {
                self.refresh(resubscribe);
            } else if audio {
                self.publish();
            }
            for cmd in commands {
                self.handle(cmd);
            }
        }
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::ToggleOverflowFlyout(anchor) => {
                if let Err(err) = input::close_xaml_overflow() {
                    crate::util::log(format!("could not invoke the tray overflow caret: {err}"));
                }
                self.shared.emit(Event::Flyout { anchor, page: crate::Page::Apps, toggle: true });
            }
            Cmd::Refresh { resubscribe } => self.refresh(resubscribe),
            Cmd::RefreshAudio { devices } => {
                if devices {
                    self.reload_devices();
                }
                self.publish();
            }
            Cmd::Select(id) => {
                self.config.target_name = id
                    .as_ref()
                    .and_then(|id| self.media.iter().find(|m| &m.target.id == id))
                    .map(|m| m.target.name.clone());
                self.config.target = id;
                self.config.save();
                self.refresh(false);
            }
            Cmd::Route { app, device } => self.route(app, device),
            Cmd::CheckUpdate => crate::update::check(self.tx.clone()),
            Cmd::UpdateChecked(at) => {
                self.config.last_update_check = at;
                self.config.save();
            }
            Cmd::Press(key) => {
                let state = self.shared.state();
                self.press(state.routed(), key);
            }
            Cmd::PressFor(id, key) => {
                let state = self.shared.state();
                if let Some(target) = state.targets.iter().find(|t| t.id == id && t.running) {
                    self.press(Some(target), key);
                }
            }
            Cmd::Set(setting) => {
                let preview = matches!(setting, Setting::Tooltip(_) | Setting::MediaButtons(_));
                let before = self.config.settings.clone();
                let s = &mut self.config.settings;
                match setting {
                    Setting::Autostart(on) => crate::autostart::set(on),
                    Setting::AutoUpdate(on) => s.auto_update = on,
                    Setting::Dark(dark) => s.dark = dark,
                    Setting::AlbumArt(on) => s.album_art = on,
                    Setting::SpeakerPosition(on) => s.speaker_position = on,
                    Setting::TrayStyle(style) => s.tray_style = style,
                    Setting::TrayMiddle(middle) => s.tray_middle = middle,
                    Setting::VolumeBars(bars) => s.volume_bars = bars,
                    Setting::Tooltip(style) => s.tooltip = style,
                    Setting::MediaButtons(buttons) => s.media_buttons = buttons,
                    Setting::MediaControls(controls) => s.media_controls = controls,
                    Setting::LevelMeters(meters) => s.level_meters = meters,
                    Setting::DeviceChip(chip) => s.device_chip = chip,
                    Setting::ShowMediaKeys(on) => s.show_media_keys = on,
                    Setting::ShowOtherApps(on) => s.show_other_apps = on,
                    Setting::ShowVolume(on) => s.show_volume = on,
                    Setting::Accent(accent) => s.accent = accent,
                }
                // Keep at least one part of the flyout showing.
                if self.config.settings.shows_nothing() {
                    self.config.settings = before;
                    self.publish();
                    return;
                }
                self.config.save();
                self.refresh(false);
                // After the refresh, so the tray already has the new style.
                if preview {
                    crate::input::preview_tooltip();
                }
            }
        }
    }

    fn reload_devices(&mut self) {
        if let Some(watcher) = &mut self.watcher {
            watcher.follow_default();
            self.devices = audio::devices(watcher.enumerator());
        }
        // A device that comes back gets its apps again.
        self.routed.clear();
    }

    /// Saves where `app` plays and sends its running processes there.
    fn route(&mut self, app: String, device: Option<String>) {
        let exes = volume::exes_for(&app);
        match &device {
            Some(device) => _ = self.config.routes.insert(app, device.clone()),
            None => _ = self.config.routes.remove(&app),
        }
        self.config.save();
        // Windows takes a route only for a process with an audio session.
        let sessions = volume::sessions(&processes());
        for session in sessions.iter().filter(|s| exes.contains(&s.exe)) {
            crate::route::set(session.pid, device.as_deref());
            match &device {
                Some(device) => _ = self.routed.insert(session.pid, device.clone()),
                None => _ = self.routed.remove(&session.pid),
            }
        }
        self.publish();
    }

    /// Sends each audio session of a routed app to its device, once per process, so
    /// apps that start playing later and choices Windows forgot are covered. Windows
    /// takes a route only for a process with an audio session, so this follows sessions.
    fn apply_routes(&mut self, sessions: &[volume::Session]) {
        self.routed.retain(|pid, _| sessions.iter().any(|s| s.pid == *pid));
        for (app, device) in &self.config.routes {
            let exes = volume::exes_for(app);
            for session in sessions.iter().filter(|s| exes.contains(&s.exe)) {
                if self.routed.get(&session.pid) != Some(device) && crate::route::set(session.pid, Some(device)) {
                    self.routed.insert(session.pid, device.clone());
                }
            }
        }
    }

    fn sessions(&self) -> Vec<Session> {
        let Some(manager) = &self.manager else { return Vec::new() };
        let Ok(list) = manager.GetSessions() else { return Vec::new() };
        let n = list.Size().unwrap_or(0);
        (0..n).filter_map(|i| list.GetAt(i).ok()).collect()
    }

    /// Re-reads every media session and player, then publishes.
    fn refresh(&mut self, resubscribe: bool) {
        self.last_refresh = Instant::now();
        let sessions = self.sessions();
        if resubscribe {
            self.resubscribe(&sessions);
        }
        let procs = processes();
        let mut media: Vec<MediaTarget> = Vec::new();

        for session in &sessions {
            let Ok(aumid) = session.SourceAppUserModelId() else { continue };
            let aumid = aumid.to_string();
            let id = format!("smtc:{aumid}");
            let playback = playback(session);
            let props = session.TryGetMediaPropertiesAsync().ok().and_then(|op| op.join().ok());
            let track = props.as_ref().and_then(track);
            let art_key = track.as_ref().map(|t| format!("{id}|{t}"));

            // Browsers expose one session per tab, so merge them and prefer the playing one.
            if let Some(existing) = media.iter_mut().find(|m| m.target.id == id) {
                if existing.target.playback != Playback::Playing && playback == Playback::Playing {
                    existing.target.playback = playback;
                    existing.target.track = track;
                    existing.art_key = art_key;
                    existing.props = props;
                }
                continue;
            }
            let name = self.names.for_aumid(&aumid, &procs);
            media.push(MediaTarget { target: target(id, name, playback, track, true, false), art_key, props });
        }

        for exe in WINDOW_PLAYERS {
            let has_session = media.iter().any(|m| m.target.id.to_lowercase() == format!("smtc:{exe}"));
            let windowed = procs.iter().any(|p| p.exe == *exe && main_window(p.pid).is_some());
            if has_session || !windowed {
                continue;
            }
            let name = self.names.for_exe(exe, &procs);
            let target = target(format!("exe:{exe}"), name, Playback::Unknown, None, true, true);
            media.push(MediaTarget { target, art_key: None, props: None });
        }

        // Keep a remembered target in the list while its app is closed.
        if let Some(id) = &self.config.target
            && !media.iter().any(|m| &m.target.id == id)
        {
            let name = self.config.target_name.clone().unwrap_or_else(|| id.clone());
            let target = target(id.clone(), name, Playback::Unknown, None, false, id.starts_with("exe:"));
            media.push(MediaTarget { target, art_key: None, props: None });
        }

        if self.config.settings.album_art {
            for m in &media {
                let (Some(key), Some(props)) = (&m.art_key, &m.props) else { continue };
                let since = match self.art.get(&m.target.id) {
                    Some((cached, art, since, tried)) if cached == key => {
                        // A track still without art keeps being asked, more slowly, since
                        // some apps attach it long after the track starts.
                        let settled = since.elapsed() > ART_SETTLE;
                        if settled && (art.is_some() || tried.elapsed() < ART_RETRY) {
                            continue;
                        }
                        *since
                    }
                    _ => Instant::now(),
                };
                let fetched = read_art(props);
                // Keep earlier art if a later fetch comes back empty.
                let previous = self
                    .art
                    .get(&m.target.id)
                    .filter(|(cached, ..)| cached == key)
                    .and_then(|(_, art, ..)| art.clone());
                let art = fetched.or(previous);
                self.art.insert(m.target.id.clone(), (key.clone(), art, since, Instant::now()));
            }
        }

        media.sort_by_key(|m| (!m.target.running, m.target.name.to_lowercase()));
        self.media = media;
        self.publish();
    }

    /// Fills in volumes, icons, the mixer and the master volume, then publishes if anything changed.
    fn publish(&mut self) {
        let procs = processes();
        let sessions = volume::sessions(&procs);
        self.apply_routes(&sessions);

        let mut covered: Vec<String> = Vec::new();
        let mut targets = Vec::with_capacity(self.media.len());
        for i in 0..self.media.len() {
            let mut target = self.media[i].target.clone();
            if target.running {
                let exes = volume::exes_for(&target.id);
                if let Some((level, muted)) = volume::level(&sessions, &exes) {
                    target.volume = Some(level);
                    target.muted = muted;
                }
                let art = if self.config.settings.album_art { self.art_for(&self.media[i]) } else { None };
                target.icon = art.or_else(|| self.icon_for(&exes, &procs));
                covered.extend(exes);
            }
            target.route = self.config.routes.get(&target.id).cloned();
            targets.push(target);
        }

        let mut mixer: Vec<MixerApp> = Vec::new();
        for session in &sessions {
            if covered.contains(&session.exe) || mixer.iter().any(|a| a.id[4..] == session.exe) {
                continue;
            }
            let exe = session.exe.clone();
            let id = format!("exe:{exe}");
            mixer.push(MixerApp {
                route: self.config.routes.get(&id).cloned(),
                id,
                name: self.names.for_exe(&exe, &procs),
                volume: (session.level() * 100.0).round() / 100.0,
                muted: session.muted(),
                icon: self.icon_for(std::slice::from_ref(&exe), &procs),
            });
        }
        mixer.sort_by_key(|a| a.name.to_lowercase());

        let (volume, muted) = self.watcher.as_ref().and_then(|w| w.master()).map(audio::read).unwrap_or_default();
        let mut settings = self.config.settings.clone();
        settings.autostart = crate::autostart::enabled();

        // Windows' current session is what media keys reach in Automatic.
        let current = self
            .manager
            .as_ref()
            .and_then(|m| m.GetCurrentSession().ok())
            .and_then(|s| s.SourceAppUserModelId().ok())
            .map(|aumid| format!("smtc:{aumid}"))
            .filter(|id| targets.iter().any(|t| &t.id == id));
        let state = State {
            current,
            targets,
            mixer,
            audio: Audio { devices: self.devices.clone(), volume, muted },
            selected: self.config.target.clone(),
            settings,
            update: crate::update::status(),
            icon_on_taskbar: crate::input::icon_on_taskbar(),
            speaker_position_active: crate::input::speaker_position_active(),
        };
        {
            let mut current = self.shared.state.lock().unwrap();
            if *current == state {
                return;
            }
            *current = state.clone();
        }
        input::update_tray(&state);
        self.shared.emit(Event::State(state));
    }

    fn art_for(&self, m: &MediaTarget) -> Option<AppIcon> {
        let key = m.art_key.as_ref()?;
        let (cached, art, ..) = self.art.get(&m.target.id)?;
        if cached == key { art.clone() } else { None }
    }

    /// The icon of the first running exe in `exes`, extracted once per exe.
    fn icon_for(&mut self, exes: &[String], procs: &[Process]) -> Option<AppIcon> {
        let process = procs.iter().find(|p| exes.contains(&p.exe))?;
        if let Some(icon) = self.icons.get(&process.exe) {
            return icon.clone();
        }
        let icon = process_path(process.pid).and_then(|path| icon::extract(&path));
        self.icons.insert(process.exe.clone(), icon.clone());
        icon
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
            ) else {
                continue;
            };
            self.subscriptions.push(Subscription { session: session.clone(), tokens: [a, b] });
        }
    }

    /// Sends `key` to `target`, or to Windows' current session without one.
    fn press(&mut self, target: Option<&Target>, key: MediaKey) {
        crate::util::log(format!("press {key:?} -> {:?}", target.map(|t| &t.id)));
        match target {
            Some(target) if target.via_window => press_window(&target.id["exe:".len()..], key),
            Some(target) => {
                let aumid = &target.id["smtc:".len()..];
                let matching: Vec<Session> = self
                    .sessions()
                    .into_iter()
                    .filter(|s| s.SourceAppUserModelId().is_ok_and(|a| a == aumid))
                    .collect();
                let session = matching.iter().find(|s| playback(s) == Playback::Playing).or_else(|| matching.first());
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

fn target(
    id: String,
    name: String,
    playback: Playback,
    track: Option<String>,
    running: bool,
    via_window: bool,
) -> Target {
    Target { id, name, playback, track, running, via_window, volume: None, muted: false, icon: None, route: None }
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

fn track(props: &MediaProperties) -> Option<String> {
    let title = props.Title().ok()?.to_string();
    let artist = props.Artist().map(|a| a.to_string()).unwrap_or_default();
    match (title.is_empty(), artist.is_empty()) {
        (true, _) => None,
        (false, true) => Some(title),
        (false, false) => Some(format!("{title} — {artist}")),
    }
}

/// The track's thumbnail as reported by the app, usually a square album cover.
fn read_art(props: &MediaProperties) -> Option<AppIcon> {
    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.join().ok()?;
    let size = stream.Size().ok()?;
    if size == 0 || size > MAX_ART_BYTES {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    reader.LoadAsync(size as u32).ok()?.join().ok()?;
    let mut data = vec![0u8; size as usize];
    reader.ReadBytes(&mut data).ok()?;
    let kind = sniff(&data)?;
    Some(AppIcon { data: Arc::from(data), kind, art: true })
}

/// Apps often leave the content type blank, so read the magic bytes.
fn sniff(data: &[u8]) -> Option<ImageKind> {
    match data {
        [0x89, b'P', b'N', b'G', ..] => Some(ImageKind::Png),
        [0xFF, 0xD8, ..] => Some(ImageKind::Jpeg),
        [b'G', b'I', b'F', ..] => Some(ImageKind::Gif),
        [b'B', b'M', ..] => Some(ImageKind::Bmp),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(ImageKind::Webp),
        _ => None,
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
    let window = processes().into_iter().filter(|p: &Process| p.exe == exe).find_map(|p| main_window(p.pid));
    if let Some(hwnd) = window {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_APPCOMMAND, WPARAM(hwnd.0 as usize), LPARAM(command << 16));
        }
    }
}

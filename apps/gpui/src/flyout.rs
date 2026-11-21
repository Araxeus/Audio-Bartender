//! The whole UI: media key target and per-app volume, settings, output device and
//! master volume, opened next to the tray icon.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use audio_bartender_core::{
    Accent, Anchor, Core, IconPage, LevelMeters, MediaButtons, MediaControls, MediaKey, Page, Playback, Setting,
    Settings, State, Target, TooltipStyle, TrayStyle, VolumeBars,
};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::theme::Theme;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon, Placement, Sizable, StyledExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::theme::{self, DARK_FOOTER, hex};
use crate::ui::{self, Volumes};

pub const WIDTH: f32 = 360.;
/// The settings panel beside the flyout, two columns wide.
pub const PANEL: f32 = 640.;
const MAX_HEIGHT: f32 = 640.;

/// Each part is a rounded card with a shadow, `MARGIN` inside its box so the shadow
/// fits in the window region. Toward the taskbar the margin is [`edge`], so the cards
/// float `FLOAT` clear of it on Windows 11 and sit flush against it on Windows 10.
const MARGIN: f32 = 8.;
const FLOAT: f32 = 12.;
const RADIUS: f32 = 8.;
/// How far the panel's box overlaps the flyout's, leaving 8 px between the cards.
const OVERLAP: f32 = 2. * MARGIN - 8.;
/// The width the panel adds to the window.
const PANEL_PART: f32 = PANEL + 2. * MARGIN - OVERLAP;
/// The window's fixed size. Its region shows only the flyout and the open panel.
pub const FRAME_WIDTH: f32 = WIDTH + 2. * MARGIN + PANEL_PART;
pub const FRAME_HEIGHT: f32 = MAX_HEIGHT + MARGIN + FLOAT;

/// The margin on the taskbar side of each card.
fn edge(anchor: &Anchor) -> f32 {
    if anchor.floats() { FLOAT } else { 0. }
}

/// The window in physical pixels, with the flyout `height` tall.
pub fn frame(anchor: &Anchor, height: f32) -> audio_bartender_core::Frame {
    let physical = |v: f32| (v * anchor.scale).round() as i32;
    audio_bartender_core::Frame {
        width: physical(FRAME_WIDTH),
        height: physical(FRAME_HEIGHT),
        panel: physical(PANEL_PART),
        visible: physical(height + MARGIN + edge(anchor)),
    }
}

/// A part of the flyout as a rounded card with a soft shadow, kept inside its margin.
fn card(element: Div, dark: bool) -> Div {
    element.rounded(px(RADIUS)).shadow(vec![BoxShadow {
        color: black().opacity(if dark { 0.45 } else { 0.18 }),
        offset: point(px(0.), px(2.)),
        blur_radius: px(6.),
        spread_radius: px(-1.),
        inset: false,
    }])
}

/// A card's box: its margin, with [`edge`] toward the taskbar.
fn card_box(card: Div, anchor: &Anchor) -> Div {
    let edge = px(edge(anchor));
    let card_box = div().flex_none().p(px(MARGIN)).child(card);
    if anchor.grows_down() { card_box.pt(edge) } else { card_box.pb(edge) }
}

/// The corner radius for a card's first and last children, inside its border.
fn inner_radius() -> Pixels {
    px(RADIUS - 1.)
}

/// The segmented control's track in light mode, as in the Tauri front end.
const TRACK_LIGHT: u32 = 0xdcdce4;

// Estimated heights, for sizing the flyout before its content is measured.
const CONTROLS: f32 = HEADER;
/// The header's height with its bottom border, as in the Tauri front end.
const HEADER: f32 = 45.;
const VOLUME: f32 = 92.;
const LABEL: f32 = 27.;
const TARGET_ROW: f32 = 55.;
const MIXER_ROW: f32 = 35.;
const SLIDER: f32 = 32.;
const SETTING_ROW: f32 = 40.;
/// How often live levels are read while the flyout is open.
const METER_FRAME: Duration = Duration::from_millis(33);
/// Time constant, in seconds, for shown levels to catch up with readings.
const LEVEL_EASE: f32 = 0.045;
/// Extra height of each media controls layout.
const BANNER: f32 = 66.;
const ROW_CONTROLS: f32 = 36.;
const DOCK: f32 = 56.;

/// Estimated height of the flyout with the given state.
pub fn height(state: &State) -> f32 {
    let middle = {
        let always = state.settings.volume_bars == VolumeBars::Always;
        let sliders = if always { state.targets.iter().filter(|t| t.volume.is_some()).count() } else { 0 };
        let targets = LABEL + TARGET_ROW * (1 + state.targets.len()) as f32 + SLIDER * sliders as f32;
        let mixer = if state.mixer.is_empty() { 0. } else { LABEL + MIXER_ROW * state.mixer.len() as f32 };
        let controls = match state.settings.media_controls {
            MediaControls::Banner => BANNER,
            MediaControls::Row => ROW_CONTROLS,
            MediaControls::None | MediaControls::Dock | MediaControls::Hover => 0.,
        };
        targets + mixer + controls
    };
    let dock = if state.settings.media_controls == MediaControls::Dock { DOCK } else { 0. };
    (CONTROLS + middle + VOLUME + dock + 12.).min(MAX_HEIGHT)
}

/// A row's hover listener.
type HoverHandler = Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>;

/// A setting picked from a few modes, shown as a segmented control.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Theme,
    TrayIcon,
    Tooltip,
    TooltipButtons,
    MediaControls,
    VolumeBars,
    LevelMeters,
}

impl Mode {
    fn title(self) -> &'static str {
        match self {
            Mode::Theme => "Theme",
            Mode::TrayIcon => "Tray icon",
            Mode::Tooltip => "Tooltip",
            Mode::TooltipButtons => "Tooltip buttons",
            Mode::MediaControls => "Media controls",
            Mode::VolumeBars => "Volume bars",
            Mode::LevelMeters => "Level meters",
        }
    }

    /// Each mode: the setting it applies, its name, a short description, and whether it's current.
    fn options(self, s: &Settings) -> Vec<(Setting, &'static str, &'static str, bool)> {
        let buttons = |b: MediaButtons, name, about| (Setting::MediaButtons(b), name, about, s.media_buttons == b);
        let controls = |c: MediaControls, name, about| (Setting::MediaControls(c), name, about, s.media_controls == c);
        let bars = |b: VolumeBars, name, about| (Setting::VolumeBars(b), name, about, s.volume_bars == b);
        match self {
            Mode::Theme => vec![
                (Setting::Dark(None), "System", "Follow Windows", s.dark.is_none()),
                (Setting::Dark(Some(false)), "Light", "Always light", s.dark == Some(false)),
                (Setting::Dark(Some(true)), "Dark", "Always dark", s.dark == Some(true)),
            ],
            Mode::TrayIcon => vec![
                (
                    Setting::TrayStyle(TrayStyle::Ring),
                    "Ring",
                    "A ring that fills with the volume",
                    s.tray_style == TrayStyle::Ring,
                ),
                (
                    Setting::TrayStyle(TrayStyle::Number),
                    "Number",
                    "The volume as a number",
                    s.tray_style == TrayStyle::Number,
                ),
            ],
            Mode::Tooltip => vec![
                (
                    Setting::Tooltip(TooltipStyle::NowPlaying),
                    "Now playing",
                    "Track, album art, device and volume",
                    s.tooltip == TooltipStyle::NowPlaying,
                ),
                (
                    Setting::Tooltip(TooltipStyle::Compact),
                    "Compact",
                    "Device, volume and the media app",
                    s.tooltip == TooltipStyle::Compact,
                ),
                (
                    Setting::Tooltip(TooltipStyle::Text),
                    "Text",
                    "The plain Windows tooltip",
                    s.tooltip == TooltipStyle::Text,
                ),
            ],
            Mode::TooltipButtons => vec![
                buttons(MediaButtons::None, "None", "No buttons on the tooltip"),
                buttons(MediaButtons::Hover, "Hover", "Shown while the pointer is on the tooltip"),
                buttons(MediaButtons::Inline, "Inline", "Small buttons at the end of the status line"),
                buttons(MediaButtons::Capsule, "Capsule", "A capsule on the line above the volume"),
                buttons(MediaButtons::Player, "Player", "A big play button between the skip buttons"),
            ],
            Mode::MediaControls => vec![
                controls(MediaControls::None, "None", "No media buttons in the flyout"),
                controls(MediaControls::Banner, "Banner", "A now playing card at the top"),
                controls(MediaControls::Row, "Row", "Under the selected app"),
                controls(MediaControls::Dock, "Dock", "A now playing line above the device volume"),
                controls(MediaControls::Hover, "Hover", "On each app's row while hovered, for that app"),
            ],
            Mode::VolumeBars => vec![
                bars(VolumeBars::None, "None", "No volume on app rows"),
                bars(VolumeBars::Compact, "Compact", "A small bar to drag or scroll, click to mute"),
                bars(VolumeBars::Always, "Always", "A slider under every app"),
                bars(VolumeBars::OnHover, "On hover", "A percentage that turns into a slider on hover"),
            ],
            Mode::LevelMeters => {
                let meters = |m: LevelMeters, name, about| (Setting::LevelMeters(m), name, about, s.level_meters == m);
                vec![
                    meters(LevelMeters::None, "None", "No live levels"),
                    meters(LevelMeters::Volume, "Volume", "The level moves inside the volume control"),
                    meters(LevelMeters::Ring, "Ring", "A ring around the app icon lights up"),
                    meters(LevelMeters::Bars, "Bars", "Equalizer bars before the app name"),
                    meters(LevelMeters::Line, "Line", "A line along the bottom of the row"),
                    meters(LevelMeters::Strip, "Strip", "A meter at the row's left edge"),
                ]
            }
        }
    }
}

/// The Automatic row's line, naming the app Windows sends media keys to now.
fn windows_decides(state: &State) -> String {
    let current = state.current.as_deref().and_then(|id| state.targets.iter().find(|t| t.id == id));
    match current {
        Some(target) => format!("Windows decides · {}", target.name),
        None => "Windows decides".into(),
    }
}

/// What the media keys control right now, for the banner and the dock.
struct NowPlaying {
    title: String,
    subtitle: String,
    playing: bool,
    target: Option<Target>,
}

impl NowPlaying {
    fn new(state: &State) -> NowPlaying {
        let Some(target) = state.now_playing() else {
            return NowPlaying {
                title: "Automatic".into(),
                subtitle: "Media keys go where Windows decides".into(),
                playing: false,
                target: None,
            };
        };
        let (title, subtitle) = match (&target.track, target.running) {
            (_, false) => (target.name.clone(), "Not running".into()),
            (Some(track), true) => match track.split_once(" — ") {
                Some((title, artist)) => (title.to_owned(), format!("{artist} · {}", target.name)),
                None => (track.clone(), target.name.clone()),
            },
            (None, true) => (target.name.clone(), "Nothing playing".into()),
        };
        let playing = target.running && target.playback == Playback::Playing;
        NowPlaying { title, subtitle, playing, target: Some(target.clone()) }
    }
}

/// The laid-out heights of the flyout's three parts and the settings panel,
/// zero until first measured.
#[derive(Default)]
struct Measured {
    controls: Rc<Cell<f32>>,
    middle: Rc<Cell<f32>>,
    volume: Rc<Cell<f32>>,
    panel: Rc<Cell<f32>>,
}

impl Measured {
    // Each part's border adds a pixel above and below.

    fn flyout(&self) -> Option<f32> {
        let parts = [self.controls.get(), self.middle.get(), self.volume.get()];
        parts.iter().all(|h| *h > 0.).then(|| parts.iter().sum::<f32>() + 2.)
    }

    fn panel(&self) -> Option<f32> {
        let panel = self.panel.get();
        (panel > 0.).then_some(panel + 2.)
    }
}

fn hwnd(window: &Window) -> Option<isize> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else { return None };
    Some(win32.hwnd.get())
}

/// Fills its parent and records the parent's height, redrawing `view` when it changes.
fn measure(cell: &Rc<Cell<f32>>, view: EntityId) -> impl IntoElement + use<> {
    let cell = cell.clone();
    canvas(
        move |bounds, window, _| {
            let height = f32::from(bounds.size.height);
            if (cell.get() - height).abs() > 0.5 {
                cell.set(height);
                window.on_next_frame(move |_, cx| cx.notify(view));
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub struct Flyout {
    core: Core,
    state: Entity<State>,
    anchor: Anchor,
    volumes: Volumes,
    page: Page,
    devices_open: bool,
    /// Stays open when it loses focus.
    pinned: bool,
    /// The app row under the pointer, for the Hover media controls.
    hovered: Option<String>,
    /// Latest live level per app row id, read while meters show.
    levels: HashMap<String, f32>,
    /// The levels on screen, easing towards `levels` every frame.
    shown_levels: HashMap<String, f32>,
    last_frame: Instant,
    /// The volume bar style of the frame being drawn.
    state_bars: VolumeBars,
    dark: Option<bool>,
    accent: Accent,
    /// The flyout's height.
    height: f32,
    /// The flyout and panel heights last sent to the window region.
    region: Option<(f32, Option<f32>)>,
    /// The panel height the window region shows, set once the region applies.
    shown_panel: Rc<Cell<Option<f32>>>,
    measured: Measured,
    /// When the window opened, for sliding it out from behind the taskbar.
    opened: Instant,
    /// The slide has finished and the window sits in its final place.
    settled: bool,
    /// When the flyout started closing, sliding back behind the taskbar.
    closing: Option<Instant>,
    _subscriptions: Vec<Subscription>,
}

impl Flyout {
    pub fn new(
        core: Core,
        state: Entity<State>,
        anchor: Anchor,
        page: Page,
        pinned: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (dark, accent) = (state.read(cx).settings.dark, state.read(cx).settings.accent);
        theme::apply(window, cx, dark, accent);
        // GPUI opens windows only where their center is on screen, so the window starts
        // hidden and appears once the first frame moves it behind the taskbar.
        if let Some(hwnd) = hwnd(window) {
            audio_bartender_core::set_cloaked(hwnd, true);
        }
        // Zero makes the first render size and place the window like every later one.
        let height = 0.;
        let _subscriptions = vec![
            cx.observe_in(&state, window, |this: &mut Self, state, window, cx| {
                let (dark, accent) = (state.read(cx).settings.dark, state.read(cx).settings.accent);
                if (dark, accent) != (this.dark, this.accent) {
                    (this.dark, this.accent) = (dark, accent);
                    theme::apply(window, cx, dark, accent);
                }
                cx.notify();
            }),
            cx.observe_window_appearance(window, |this, window, cx| theme::apply(window, cx, this.dark, this.accent)),
            // Clicking anywhere else closes the flyout, like the Windows one.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() && !this.pinned {
                    this.close(cx);
                }
            }),
        ];
        let volumes = Volumes::new(core.clone(), cx);
        // Ends when the flyout closes and its entity goes away.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(METER_FRAME).await;
                if this.update(cx, |this, cx| this.poll_levels(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Flyout {
            core,
            state,
            anchor,
            volumes,
            page,
            devices_open: false,
            pinned,
            hovered: None,
            levels: HashMap::new(),
            shown_levels: HashMap::new(),
            last_frame: Instant::now(),
            state_bars: VolumeBars::default(),
            dark,
            accent,
            height,
            region: None,
            shown_panel: Rc::default(),
            measured: Measured::default(),
            opened: Instant::now(),
            settled: false,
            closing: None,
            _subscriptions,
        }
    }

    pub fn open_page(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        self.devices_open = false;
        cx.notify();
    }

    /// Reads live levels for every app row, redrawing when one moved.
    fn poll_levels(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let showing = state.settings.level_meters != LevelMeters::None && !self.devices_open;
        if !showing {
            if !self.levels.is_empty() {
                self.levels.clear();
                cx.notify();
            }
            return;
        }
        let ids = state.targets.iter().map(|t| &t.id).chain(state.mixer.iter().map(|m| &m.id));
        let levels: HashMap<String, f32> = ids.map(|id| (id.clone(), self.core.level(id))).collect();
        let moved = levels.iter().any(|(id, level)| (self.levels.get(id).copied().unwrap_or(0.) - level).abs() > 0.004);
        if moved || levels.len() != self.levels.len() {
            self.levels = levels;
            cx.notify();
        }
    }

    fn level(&self, id: &str) -> f32 {
        self.shown_levels.get(id).copied().unwrap_or(0.)
    }

    /// Moves the shown levels towards the latest readings, and keeps frames coming
    /// while any meter moves, so they glide instead of stepping at the poll rate.
    fn ease_levels(&mut self, window: &mut Window) {
        let dt = self.last_frame.elapsed().as_secs_f32().min(0.1);
        self.last_frame = Instant::now();
        let k = 1. - (-dt / LEVEL_EASE).exp();
        for (id, target) in &self.levels {
            let shown = self.shown_levels.entry(id.clone()).or_default();
            *shown += (target - *shown) * k;
        }
        self.shown_levels.retain(|id, shown| {
            if !self.levels.contains_key(id) {
                *shown *= 1. - k;
            }
            *shown > 0.005
        });
        if !self.shown_levels.is_empty() {
            window.request_animation_frame();
        }
    }

    /// Tracks which row the pointer is on, for Hover media controls and On hover volume.
    fn track_hover(id: &str, cx: &mut Context<Self>) -> HoverHandler {
        let id = id.to_owned();
        Box::new(cx.listener(move |this, hovered: &bool, _, cx| {
            if *hovered {
                this.hovered = Some(id.clone());
            } else if this.hovered.as_ref() == Some(&id) {
                this.hovered = None;
            }
            cx.notify();
        }))
    }

    /// The row's volume control beside its name: a bar, or a percentage that turns
    /// into a slider while the row is hovered or its volume is changing.
    fn volume_control(
        &self,
        id: &str,
        level: f32,
        muted: bool,
        meter: Option<f32>,
        theme: &Theme,
    ) -> Option<AnyElement> {
        match self.state_bars {
            VolumeBars::Compact => Some(self.volumes.bar(id, level, muted, meter, theme).into_any_element()),
            VolumeBars::OnHover if self.hovered.as_deref() == Some(id) || self.volumes.changing(id) => {
                Some(self.volumes.hover_slider(id, level, meter, theme).into_any_element())
            }
            VolumeBars::OnHover => Some(self.volumes.pill(id, level, muted, meter, theme).into_any_element()),
            VolumeBars::None | VolumeBars::Always => None,
        }
    }

    /// Bars before the name that move with the app's level.
    fn live_bars(level: f32, theme: &Theme) -> Option<Div> {
        if level < 0.02 {
            return None;
        }
        // A slight wobble per bar, so they move like an equalizer rather than in lockstep.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0., |d| (d.as_millis() % 100_000) as f32 / 1000.);
        // Music rarely drops below about a quarter, so spread the useful range over the bars' height.
        let shape = ((level - 0.25) / 0.7).clamp(0.12, 1.);
        Some(h_flex().flex_none().items_end().gap(px(2.)).h(px(11.)).children((0..3).map(|i| {
            let i = i as f32;
            let wobble = 0.55 + 0.45 * (0.5 + 0.5 * (t * (8. + i * 3.1) + i * 2.1).sin());
            div().w(px(2.5)).rounded(px(1.)).bg(theme.success).h(px(2. + 9. * shape * wobble))
        })))
    }

    /// A line along the bottom of the row, or a strip at its left edge.
    fn row_meter(style: LevelMeters, level: f32, theme: &Theme) -> Option<Div> {
        let level = level.clamp(0., 1.);
        match style {
            LevelMeters::Line => Some(div().absolute().left(px(10.)).right(px(10.)).bottom(px(1.)).h(px(2.)).child(
                div().h_full().w(relative(level)).rounded_full().bg(linear_gradient(
                    90.,
                    linear_color_stop(theme.primary.opacity(0.4), 0.),
                    linear_color_stop(theme.primary, 1.),
                )),
            )),
            LevelMeters::Strip => Some(
                div()
                    .absolute()
                    .left(px(2.))
                    .top(px(8.))
                    .bottom(px(8.))
                    .w(px(3.))
                    .flex()
                    .flex_col()
                    .justify_end()
                    .child(div().w_full().h(relative(level)).rounded_full().bg(linear_gradient(
                        0.,
                        linear_color_stop(theme.success, 0.),
                        linear_color_stop(rgb(0xfacc15), 1.),
                    ))),
            ),
            _ => None,
        }
    }

    /// The avatar, with a ring that lights up with the level when meters are rings.
    fn ringed(avatar: AnyElement, ring: Option<f32>, size: Pixels, theme: &Theme) -> Div {
        div().relative().flex_none().size(size).child(avatar).when_some(ring, |d, level| {
            d.child(
                div()
                    .absolute()
                    .top(px(-3.))
                    .left(px(-3.))
                    .right(px(-3.))
                    .bottom(px(-3.))
                    .rounded(theme.radius + px(3.))
                    .border_2()
                    .border_color(theme.primary.opacity((level * 1.4).min(1.))),
            )
        })
    }

    /// Slides the flyout back behind the taskbar, then closes its window.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.closing.is_none() {
            self.closing = Some(Instant::now());
            crate::flyout_closed(cx);
            cx.notify();
        }
    }

    /// Fits the window region to the flyout and the open panel, and slides the
    /// window in or out. The window itself keeps one size, so nothing jumps.
    fn fit(&mut self, state: &State, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hwnd) = hwnd(window) else { return };
        let (anchor, frame) = (self.anchor, self.frame());
        // GPUI ignores bounds whose center is off screen, which the slide's start is.
        let size = size(px(FRAME_WIDTH), px(FRAME_HEIGHT));
        if window.viewport_size() != size {
            window.resize(size);
        }
        if let Some(closing) = self.closing {
            if audio_bartender_core::slide_out_progress(closing) <= 0.0 {
                window.remove_window();
                return;
            }
            window.request_animation_frame();
            window.on_next_frame(move |_, _| {
                let progress = audio_bartender_core::slide_out_progress(closing);
                audio_bartender_core::place_window(hwnd, &anchor, frame, progress);
            });
            return;
        }
        self.height = self.measured.flyout().map_or_else(|| height(state), |h| h.ceil().min(MAX_HEIGHT));
        // The panel joins the region once it's measured, so it never shows half laid out.
        let panel = self
            .measured
            .panel()
            .filter(|_| self.page == Page::Settings)
            .map(|h| h.ceil().min(MAX_HEIGHT) + MARGIN + edge(&self.anchor));
        let region = (self.height, panel);
        if Some(region) == self.region && self.settled {
            return;
        }
        self.region = Some(region);
        let progress = audio_bartender_core::slide_progress(self.opened);
        if progress < 1.0 {
            window.request_animation_frame();
        } else {
            self.settled = true;
        }
        let frame = self.frame();
        let scale = window.scale_factor();
        let (opened, shown, view) = (self.opened, self.shown_panel.clone(), cx.entity_id());
        window.on_next_frame(move |_, cx| {
            let progress = audio_bartender_core::slide_progress(opened);
            audio_bartender_core::place_window(hwnd, &anchor, frame, progress);
            let overlap = (OVERLAP * scale).round() as i32;
            let parts = frame.parts(&anchor, panel.map(|h| (h * scale).round() as i32), overlap);
            audio_bartender_core::set_window_region(hwnd, &parts);
            audio_bartender_core::set_cloaked(hwnd, false);
            // A closed panel stays drawn until the region no longer shows it.
            if shown.replace(panel) != panel {
                cx.notify(view);
            }
        });
    }

    /// The window in physical pixels, with the flyout at its current height.
    fn frame(&self) -> audio_bartender_core::Frame {
        frame(&self.anchor, self.height)
    }

    fn label(text: &'static str, theme: &Theme) -> Div {
        div()
            .px_4()
            .pt(px(10.))
            .pb(px(2.))
            .text_size(px(11.))
            .line_height(px(15.))
            .font_semibold()
            .text_color(theme.muted_foreground)
            .child(text)
    }

    fn controls(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let dark = theme.is_dark();
        let settings_open = self.page == Page::Settings;
        let icon_color = theme.foreground.opacity(0.8);
        let (core_theme, core_quit) = (self.core.clone(), self.core.clone());
        h_flex()
            .justify_between()
            .h(px(HEADER))
            .px_3()
            .rounded_t(inner_radius())
            .border_b_1()
            .border_color(theme.border)
            .when(dark, |d| d.bg(hex(DARK_FOOTER)))
            .child(
                h_flex()
                    .gap_2()
                    .pl_1()
                    .child(
                        ui::logo(px(18.), self.accent.colors(dark).gradient)
                            .child(svg().path("brand/glyph.svg").size(px(14.)).text_color(white())),
                    )
                    .child(div().text_size(px(13.)).font_semibold().child("Audio Bartender")),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("theme")
                            .small()
                            .ghost()
                            .icon(Icon::new(if dark { IconName::Sun } else { IconName::Moon }).text_color(icon_color))
                            .tooltip("Toggle dark mode")
                            .tooltip_placement(Placement::Bottom)
                            .on_click(move |_, _, _| core_theme.set(Setting::Dark(Some(!dark)))),
                    )
                    .child(
                        Button::new("settings")
                            .small()
                            .ghost()
                            .icon(Icon::new(IconName::Settings).text_color(if settings_open {
                                theme.primary
                            } else {
                                icon_color
                            }))
                            .tooltip("Settings")
                            .tooltip_placement(Placement::Bottom)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let page = if this.page == Page::Settings { Page::Apps } else { Page::Settings };
                                this.open_page(page, cx);
                            })),
                    )
                    .child(
                        Button::new("pin")
                            .small()
                            .ghost()
                            .icon(
                                Icon::new(if self.pinned { IconName::Pin } else { IconName::PinOff })
                                    .text_color(if self.pinned { theme.primary } else { icon_color }),
                            )
                            .tooltip(if self.pinned {
                                "Unpin: close when clicking elsewhere"
                            } else {
                                "Pin: stay open"
                            })
                            .tooltip_placement(Placement::Bottom)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pinned = !this.pinned;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("quit")
                            .small()
                            .ghost()
                            .icon(Icon::new(IconName::Power).text_color(icon_color))
                            .tooltip("Quit")
                            .tooltip_placement(Placement::Bottom)
                            .on_click(move |_, _, cx| {
                                core_quit.shutdown();
                                cx.quit();
                            }),
                    ),
            )
    }

    /// Previous, play/pause and next. With `target` they control that app; without,
    /// wherever media keys go.
    /// Drawn here rather than with `Button`, which keeps the arrow cursor on top of a
    /// clickable row.
    fn transport(&self, id: &str, target: Option<String>, playing: bool, small: bool, theme: &Theme) -> Div {
        let (size, icon_size) = if small { (px(22.), px(14.)) } else { (px(28.), px(16.)) };
        let button = |name: &str, key: MediaKey, icon: IconName, primary: bool| {
            let (core, target) = (self.core.clone(), target.clone());
            let (bg, hover) = if primary {
                (theme.primary, theme.primary_hover)
            } else {
                (transparent_black(), theme.secondary_hover)
            };
            let color = if primary { white() } else { theme.foreground.opacity(0.8) };
            div()
                .id(SharedString::from(format!("{id}-{name}")))
                .flex_none()
                .size(size)
                .flex()
                .items_center()
                .justify_center()
                .rounded(theme.radius)
                .bg(bg)
                .cursor_pointer()
                .hover(move |st| st.bg(hover))
                .child(Icon::new(icon).size(icon_size).text_color(color))
                .on_click(move |_, _, cx| {
                    match &target {
                        Some(target) => core.press_for(target, key),
                        None => core.press(key),
                    }
                    cx.stop_propagation();
                })
        };
        let play = if playing { IconName::Pause } else { IconName::Play };
        h_flex()
            .flex_none()
            .gap_1()
            .child(button("prev", MediaKey::Previous, IconName::SkipBack, false))
            .child(button("play", MediaKey::PlayPause, play, true))
            .child(button("next", MediaKey::Next, IconName::SkipForward, false))
    }

    /// Art, track and buttons for whatever the media keys control.
    fn now_playing(&self, state: &State, theme: &Theme, art: Pixels) -> Div {
        let now = NowPlaying::new(state);
        let avatar = match &now.target {
            Some(t) => ui::avatar(t.icon.as_ref(), &t.name, t.running, art, theme),
            None => ui::logo(art, self.accent.colors(theme.is_dark()).gradient)
                .child(Icon::new(IconName::Sparkles).small().text_color(white()))
                .into_any_element(),
        };
        let subtitle = h_flex()
            .gap_1()
            .text_xs()
            .text_color(theme.muted_foreground)
            .when(now.playing, |d| d.child(Self::playing_bars("now-playing", theme.success)))
            .child(div().truncate().child(now.subtitle));
        h_flex()
            .gap_3()
            .child(avatar)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().font_semibold().truncate().child(now.title))
                    .child(subtitle),
            )
            .child(self.transport("now-playing", None, now.playing, false, theme))
    }

    fn device_header(&self, state: &State, theme: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let name = state.audio.device().map(|d| d.name.clone()).unwrap_or_else(|| "No output device".into());
        let open = self.devices_open;
        h_flex()
            .id("device")
            .gap_2()
            .px_2()
            .py(px(7.))
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(|s| s.bg(theme.secondary_hover))
            .child(Icon::new(IconName::Speaker).small())
            .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).child(name))
            .child(Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronUp }).small())
            .on_click(cx.listener(|this, _, _, cx| {
                this.devices_open = !this.devices_open;
                cx.notify();
            }))
    }

    fn devices(&self, state: &State, theme: &Theme, cx: &mut Context<Self>) -> Div {
        v_flex().px_2().py_1().children(state.audio.devices.iter().map(|device| {
            let (core, id) = (self.core.clone(), device.id.clone());
            h_flex()
                .id(SharedString::from(device.id.clone()))
                .gap_2()
                .px_2()
                .py_2()
                .rounded(theme.radius)
                .cursor_pointer()
                .hover(|s| s.bg(theme.secondary_hover))
                .when(device.default, |d| d.text_color(theme.primary))
                .child(div().w(px(16.)).when(device.default, |d| d.child(Icon::new(IconName::Check).small())))
                .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).child(device.name.clone()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    core.set_device(&id);
                    this.devices_open = false;
                    cx.notify();
                }))
        }))
    }

    /// Three bars bouncing out of phase, like an equalizer.
    fn playing_bars(id: &str, color: Hsla) -> impl IntoElement {
        h_flex().items_end().gap(px(2.)).h(px(10.)).children((0..3).map(|i| {
            let offset = [0.0, 0.6, 0.3][i];
            div().w(px(2.)).h(px(3.)).bg(color).with_animation(
                SharedString::from(format!("{id}-bar-{i}")),
                Animation::new(Duration::from_millis(1000)).repeat(),
                move |bar, t| {
                    let phase = ((t + offset) % 1.0) * std::f32::consts::TAU;
                    bar.h(px(3. + 7. * (0.5 - 0.5 * phase.cos())))
                },
            )
        }))
    }

    /// "▮▮▮ Playing · Title — Artist": status in green while playing, the track muted.
    fn detail(target: &Target, theme: &Theme, fake_bars: bool) -> Div {
        let line = h_flex().gap_1().text_xs().text_color(theme.muted_foreground).overflow_hidden();
        if !target.running {
            return line.child("Not running");
        }
        let status = match target.playback {
            Playback::Playing => "Playing",
            Playback::Paused => "Paused",
            Playback::Stopped => "Stopped",
            Playback::Unknown if target.via_window => "Window messages",
            Playback::Unknown => "Idle",
        };
        let playing = target.playback == Playback::Playing;
        line.when(playing && fake_bars, |d| d.child(Self::playing_bars(&target.id, theme.success)))
            .child(div().flex_none().when(playing, |d| d.text_color(theme.success)).child(status))
            .when_some(target.track.clone(), |d, track| d.child(div().truncate().child(format!("· {track}"))))
    }

    /// The only control that picks where media keys go.
    fn radio(id: &str, theme: &Theme, checked: bool, core: &Core, target: Option<String>) -> Stateful<Div> {
        let core = core.clone();
        let hover = theme.secondary_hover;
        div()
            .id(SharedString::from(format!("radio-{id}")))
            .flex_none()
            .p(px(6.))
            .m(px(-6.))
            .rounded_full()
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .child(
                div()
                    .size(px(16.))
                    .rounded_full()
                    .border_color(if checked { theme.primary } else { theme.muted_foreground })
                    .map(|d| if checked { d.border(px(4.5)) } else { d.border(px(1.5)) }),
            )
            .on_click(move |_, _, cx| {
                core.select(target.clone());
                cx.stop_propagation();
            })
    }

    fn row(theme: &Theme, id: SharedString, checked: bool, clickable: bool) -> Stateful<Div> {
        let hover = theme.secondary_hover;
        v_flex()
            .id(id)
            .mx_2()
            .rounded(theme.radius)
            .when(checked, |d| d.bg(theme.primary.opacity(0.13)))
            .when(clickable, |d| d.cursor_pointer().when(!checked, |d| d.hover(move |s| s.bg(hover))))
    }

    /// Click handler for a card that sends media keys to `target`, unless the
    /// press started on one of its volume controls.
    fn select_on_click(&self, target: Option<String>) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static + use<> {
        let core = self.core.clone();
        let controls = self.volumes.hit_test();
        move |event, _, _| {
            if let ClickEvent::Mouse(click) = event
                && controls(click.down.position)
            {
                return;
            }
            core.select(target.clone());
        }
    }

    fn targets(&mut self, state: &State, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme().clone();
        let bars = state.settings.volume_bars;
        let controls = state.settings.media_controls;
        let meters = state.settings.level_meters;
        self.state_bars = bars;
        let mut rows = Vec::new();

        let automatic = state.selected.is_none();
        // In Automatic the buttons reach Windows' current session, so show its state.
        let automatic_playing = state.now_playing().is_some_and(|t| t.running && t.playback == Playback::Playing);
        rows.push(
            Self::row(&theme, "automatic".into(), automatic, true)
                .on_click(self.select_on_click(None))
                .child(
                    h_flex()
                        .gap_3()
                        .px_2()
                        .py(px(10.))
                        .child(
                            ui::logo(px(28.), self.accent.colors(theme.is_dark()).gradient)
                                .child(Icon::new(IconName::Sparkles).small().text_color(white())),
                        )
                        .child(
                            v_flex().flex_1().child(div().text_size(px(13.)).font_semibold().child("Automatic")).child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .truncate()
                                    .child(windows_decides(state)),
                            ),
                        )
                        .child(Self::radio("automatic", &theme, automatic, &self.core, None)),
                )
                .when(automatic && controls == MediaControls::Row, |d| {
                    d.child(Self::row_controls(self.transport("automatic", None, automatic_playing, false, &theme)))
                })
                .into_any_element(),
        );

        for target in &state.targets {
            let checked = state.selected.as_deref() == Some(target.id.as_str());
            let level = self.level(&target.id);
            let in_volume = (meters == LevelMeters::Volume).then_some(level);
            let slider = target
                .volume
                .filter(|_| bars == VolumeBars::Always)
                .map(|volume| self.volumes.slider_row(&target.id, volume, target.muted, in_volume, &theme));
            let volume = target
                .volume
                .and_then(|volume| self.volume_control(&target.id, volume, target.muted, in_volume, &theme));
            let avatar = ui::avatar(target.icon.as_ref(), &target.name, target.running, px(28.), &theme);
            let name = h_flex()
                .gap_1p5()
                .min_w_0()
                .children((meters == LevelMeters::Bars).then(|| Self::live_bars(level, &theme)).flatten())
                .child(div().text_size(px(13.)).font_semibold().truncate().child(target.name.clone()));
            let detail = || Self::detail(target, &theme, meters != LevelMeters::Bars);
            rows.push(
                Self::row(&theme, target.id.clone().into(), checked, true)
                    .relative()
                    .on_hover(Self::track_hover(&target.id, cx))
                    .on_click(self.select_on_click(Some(target.id.clone())))
                    .children(Self::row_meter(meters, level, &theme))
                    .child(
                        h_flex()
                            .gap_3()
                            .px_2()
                            .py(px(10.))
                            .child(Self::ringed(
                                avatar,
                                (meters == LevelMeters::Ring).then_some(level),
                                px(28.),
                                &theme,
                            ))
                            .child(
                                v_flex().flex_1().min_w_0().when(!target.running, |d| d.opacity(0.6)).child(name).map(
                                    |d| {
                                        if controls == MediaControls::Hover && target.running {
                                            // The track line swaps to buttons for this app while hovered.
                                            // Chosen at render time, because a hover style that hides
                                            // elements can change between prepaint and paint.
                                            let hovered = self.hovered.as_ref() == Some(&target.id);
                                            let line = if hovered {
                                                let playing = target.playback == Playback::Playing;
                                                self.transport(
                                                    &target.id,
                                                    Some(target.id.clone()),
                                                    playing,
                                                    true,
                                                    &theme,
                                                )
                                            } else {
                                                detail()
                                            };
                                            // The buttons sit a little below the name.
                                            d.child(
                                                div()
                                                    .h(px(26.))
                                                    .flex()
                                                    .map(|d| if hovered { d.items_end() } else { d.items_center() })
                                                    .child(line),
                                            )
                                        } else {
                                            d.child(detail())
                                        }
                                    },
                                ),
                            )
                            .children(volume)
                            .child(Self::radio(&target.id, &theme, checked, &self.core, Some(target.id.clone()))),
                    )
                    .when(checked && controls == MediaControls::Row, |d| {
                        let playing = target.running && target.playback == Playback::Playing;
                        d.child(Self::row_controls(self.transport(&target.id, None, playing, false, &theme)))
                    })
                    .when_some(slider, |d, slider| {
                        // Under the name, clear of the avatar.
                        d.child(div().pl(px(38.)).pr_2().pb_1p5().mt(px(-4.)).child(slider))
                    })
                    .into_any_element(),
            );
        }
        rows
    }

    /// Buttons under the selected row, lined up with its name.
    fn row_controls(buttons: Div) -> Div {
        h_flex().pl(px(48.)).pr_2().pb_2().mt(px(-2.)).child(buttons)
    }

    fn mixer(&mut self, state: &State, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme().clone();
        let bars = state.settings.volume_bars;
        let meters = state.settings.level_meters;
        self.state_bars = bars;
        let mut rows = Vec::new();
        for app in &state.mixer {
            let level = self.level(&app.id);
            let in_volume = (meters == LevelMeters::Volume).then_some(level);
            let name = h_flex()
                .gap_1p5()
                .min_w_0()
                .children((meters == LevelMeters::Bars).then(|| Self::live_bars(level, &theme)).flatten())
                .child(div().text_size(px(13.)).font_semibold().truncate().child(app.name.clone()));
            let avatar = ui::avatar(app.icon.as_ref(), &app.name, true, px(22.), &theme);
            let main = h_flex().gap_3().px_2().child(Self::ringed(
                avatar,
                (meters == LevelMeters::Ring).then_some(level),
                px(22.),
                &theme,
            ));
            let row = Self::row(&theme, app.id.clone().into(), false, false)
                .relative()
                .on_hover(Self::track_hover(&app.id, cx))
                .children(Self::row_meter(meters, level, &theme));
            let row = match bars {
                VolumeBars::Always => {
                    let slider = self.volumes.slider_row(&app.id, app.volume, app.muted, in_volume, &theme);
                    row.child(main.py_0p5().child(name.w(px(96.)).flex_none()).child(div().flex_1().child(slider)))
                }
                VolumeBars::None => row,
                _ => row.child(
                    main.py(px(5.))
                        .child(name.flex_1())
                        .children(self.volume_control(&app.id, app.volume, app.muted, in_volume, &theme)),
                ),
            };
            rows.push(row.into_any_element());
        }
        rows
    }

    /// One color dot per accent; the chosen one gets a ring.
    fn swatches(&self, current: Accent, theme: &Theme) -> AnyElement {
        let dark = theme.is_dark();
        let swatch = |accent: Accent| {
            let core = self.core.clone();
            let selected = accent == current;
            let fill: Hsla = rgb(accent.colors(dark).accent).into();
            div()
                .id(SharedString::from(format!("accent-{}", accent.name())))
                .size(px(22.))
                .p(px(2.))
                .rounded_full()
                .cursor_pointer()
                .border_2()
                .border_color(if selected { fill } else { transparent_black() })
                .child(div().size_full().rounded_full().bg(fill))
                .tooltip(move |window, cx| Tooltip::new(accent.label()).build(window, cx))
                .on_click(move |_, _, _| core.set(Setting::Accent(accent)))
        };
        v_flex()
            .mx_2()
            .px_2()
            .pt(px(6.))
            .pb_2()
            .gap(px(6.))
            .child(div().text_size(px(13.)).line_height(px(18.)).child("Accent color"))
            .children(
                Accent::ALL.chunks(7).map(|row| h_flex().justify_between().children(row.iter().map(|a| swatch(*a)))),
            )
            .into_any_element()
    }

    /// A switch drawn like the Tauri one: an outlined track when off, filled with the
    /// accent when on.
    fn switch(id: &'static str, on: bool, theme: &Theme, toggle: impl Fn(bool) + 'static) -> AnyElement {
        let (accent, muted) = (theme.primary, theme.muted_foreground);
        div()
            .id(id)
            .flex_none()
            .relative()
            .w(px(40.))
            .h(px(20.))
            .rounded(px(10.))
            .border(px(1.5))
            .border_color(if on { accent } else { muted })
            .when(on, |d| d.bg(accent))
            .cursor_pointer()
            .child(
                div()
                    .absolute()
                    .top(px(3.))
                    .left(px(if on { 23. } else { 3. }))
                    .size(px(11.))
                    .rounded_full()
                    .bg(if on { white() } else { muted }),
            )
            .on_click(move |_, _, _| toggle(!on))
            .into_any_element()
    }

    /// A switch for a setting.
    fn setting_switch(&self, id: &'static str, on: bool, make: fn(bool) -> Setting, theme: &Theme) -> AnyElement {
        let core = self.core.clone();
        Self::switch(id, on, theme, move |on| core.set(make(on)))
    }

    /// A small bordered button, for actions inside a settings row.
    fn small_button(id: &'static str, label: &'static str, theme: &Theme) -> Stateful<Div> {
        let hover = theme.secondary_hover;
        div()
            .id(id)
            .flex_none()
            .px(px(9.))
            .py(px(3.))
            .rounded(px(6.))
            .border_1()
            .border_color(theme.border)
            .bg(theme::card(theme.is_dark()))
            .text_size(px(12.))
            .line_height(px(17.))
            .cursor_pointer()
            .hover(move |st| st.bg(hover))
            .child(label)
    }

    /// A group heading in the settings panel.
    fn panel_label(text: &'static str, theme: &Theme) -> Div {
        div()
            .px_4()
            .pt(px(10.))
            .pb(px(2.))
            .text_size(px(11.))
            .line_height(px(15.))
            .font_semibold()
            .text_color(theme.muted_foreground)
            .child(text)
    }

    /// A setting's name, then a segmented control of its modes, then a line about
    /// the current one.
    fn segmented(&self, mode: Mode, s: &Settings, theme: &Theme) -> AnyElement {
        let options = mode.options(s);
        let about = options.iter().find(|o| o.3).map_or("", |o| o.2);
        let (fg, muted) = (theme.foreground, theme.muted_foreground);
        let dark = theme.is_dark();
        let (track, chip) = if dark { (theme.muted, theme.secondary) } else { (hex(TRACK_LIGHT), theme::card(false)) };
        let shadow = vec![BoxShadow {
            color: black().opacity(if dark { 0.25 } else { 0.12 }),
            offset: point(px(0.), px(1.)),
            blur_radius: px(2.),
            spread_radius: px(0.),
            inset: false,
        }];
        let buttons = options.into_iter().enumerate().map(|(i, (setting, name, _, on))| {
            let core = self.core.clone();
            div()
                .id(SharedString::from(format!("mode-{}-{i}", mode.title())))
                .flex_1()
                .min_w_0()
                .h(px(24.))
                .px_1()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .text_size(px(12.))
                .cursor_pointer()
                .map(|d| {
                    if on {
                        d.bg(chip).text_color(fg).font_semibold().shadow(shadow.clone())
                    } else {
                        d.text_color(muted).hover(move |st| st.text_color(fg))
                    }
                })
                .child(name)
                .on_click(move |_, _, _| core.set(setting.clone()))
        });
        v_flex()
            .mx_2()
            .px_2()
            .pt(px(6.))
            .pb_2()
            .gap(px(6.))
            .child(div().text_size(px(13.)).line_height(px(18.)).child(mode.title()))
            .child(h_flex().p(px(2.)).gap(px(2.)).rounded(px(7.)).bg(track).children(buttons))
            .child(div().h(px(16.)).text_size(px(12.)).line_height(px(16.)).text_color(muted).truncate().child(about))
            .into_any_element()
    }

    /// Every setting at once, in two columns beside the flyout, so changes show in
    /// the app list as they're made.
    fn settings_panel(&self, state: &State, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let s = &state.settings;
        let row = |title: &'static str, control: AnyElement| {
            h_flex()
                .mx_2()
                .px_2()
                .h(px(SETTING_ROW))
                .gap_3()
                .text_size(px(13.))
                .line_height(px(18.))
                .child(div().flex_1().min_w_0().child(title))
                .child(control)
                .into_any_element()
        };
        // Windows 11 moves our icon at once. Windows 10 shows where it is and opens the
        // page to change it, and the page that hides the Windows volume icon.
        let icon_row = if audio_bartender_core::windows_11() {
            let core = self.core.clone();
            let on = state.icon_on_taskbar.unwrap_or(false);
            row(
                "Show icon on the taskbar",
                Self::switch("icon-on-taskbar", on, theme, move |on| core.set_icon_on_taskbar(on)),
            )
        } else {
            let core = self.core.clone();
            let place = match state.icon_on_taskbar {
                Some(true) => "On the taskbar",
                Some(false) => "In the overflow",
                None => "",
            };
            row(
                "Our tray icon",
                h_flex()
                    .gap_2()
                    .child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(place))
                    .child(
                        Self::small_button("icon-place", "Change", theme)
                            .on_click(move |_, _, _| core.set_icon_on_taskbar(true)),
                    )
                    .into_any_element(),
            )
        };
        let volume_row = (!audio_bartender_core::windows_11()).then(|| {
            row(
                "Windows volume icon",
                Self::small_button("hide-windows", "Hide", theme)
                    .on_click(|_, _, _| audio_bartender_core::open_icon_page(IconPage::HideWindows))
                    .into_any_element(),
            )
        });
        let update = {
            let core = self.core.clone();
            let busy = state.update.busy();
            h_flex()
                .mx_2()
                .px_2()
                .h(px(SETTING_ROW))
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(theme.muted_foreground)
                        .child(state.update.summary.clone()),
                )
                .child(
                    Self::small_button("check-update", "Check now", theme)
                        .when(busy, |d| d.opacity(0.5))
                        .when(!busy, |d| d.on_click(move |_, _, _| core.check_update())),
                )
                .into_any_element()
        };
        let left = v_flex()
            .flex_1()
            .min_w_0()
            .child(Self::panel_label("GENERAL", theme))
            .child(row("Start with Windows", self.setting_switch("autostart", s.autostart, Setting::Autostart, theme)))
            .child(row(
                "Album art as app icon",
                self.setting_switch("album-art", s.album_art, Setting::AlbumArt, theme),
            ))
            .child(row(
                "Automatic updates",
                self.setting_switch("auto-update", s.auto_update, Setting::AutoUpdate, theme),
            ))
            .child(update)
            .child(icon_row)
            .children(volume_row)
            .child(Self::panel_label("LOOK", theme))
            .child(self.swatches(s.accent, theme))
            .child(self.segmented(Mode::Theme, s, theme))
            .child(self.segmented(Mode::TrayIcon, s, theme));
        let right = v_flex()
            .flex_1()
            .min_w_0()
            .child(Self::panel_label("TRAY TOOLTIP", theme))
            .child(self.segmented(Mode::Tooltip, s, theme))
            .child(self.segmented(Mode::TooltipButtons, s, theme))
            .child(Self::panel_label("FLYOUT", theme))
            .child(self.segmented(Mode::MediaControls, s, theme))
            .child(self.segmented(Mode::VolumeBars, s, theme))
            .child(self.segmented(Mode::LevelMeters, s, theme));
        let header = h_flex()
            .justify_between()
            .h(px(HEADER))
            .px_3()
            .border_b_1()
            .border_color(theme.border)
            .when(theme.is_dark(), |d| d.bg(hex(DARK_FOOTER)))
            .child(div().pl_1().text_size(px(13.)).font_semibold().child("Settings"))
            .child(
                Button::new("close-settings")
                    .small()
                    .ghost()
                    .icon(Icon::new(IconName::Close).text_color(theme.foreground.opacity(0.8)))
                    .tooltip("Close settings")
                    .tooltip_placement(Placement::Bottom)
                    .on_click(cx.listener(|this, _, _, cx| this.open_page(Page::Apps, cx))),
            );
        let view = cx.entity_id();
        v_flex()
            .flex_none()
            .relative()
            .bg(theme::card(theme.is_dark()))
            .border_1()
            .border_color(theme.border)
            .child(header.rounded_t(inner_radius()))
            .child(
                h_flex()
                    .items_start()
                    .pb_2()
                    .child(left)
                    .child(div().w(px(1.)).self_stretch().my_3().bg(theme.border))
                    .child(right),
            )
            .child(measure(&self.measured.panel, view))
    }
}

impl Render for Flyout {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ease_levels(window);
        let state = self.state.read(cx).clone();
        let theme = cx.theme().clone();
        let view = cx.entity_id();
        self.volumes.begin_frame();
        self.fit(&state, window, cx);

        let controls = self.controls(&theme, cx);
        let master = self.volumes.slider_row(ui::MASTER, state.audio.volume, state.audio.muted, None, &theme);
        let device_header = self.device_header(&state, &theme, cx);
        let dock = (state.settings.media_controls == MediaControls::Dock).then(|| {
            self.now_playing(&state, &theme, px(36.)).px_2().pb_2().mb_1().border_b_1().border_color(theme.border)
        });
        let middle = if self.devices_open {
            vec![self.devices(&state, &theme, cx).into_any_element()]
        } else {
            let mut items = Vec::new();
            if state.settings.media_controls == MediaControls::Banner {
                let banner = self.now_playing(&state, &theme, px(44.));
                items.push(
                    div()
                        .mx_2()
                        .mt_2()
                        .p_2p5()
                        .rounded(theme.radius * 1.5)
                        .bg(theme.secondary)
                        .child(banner)
                        .into_any_element(),
                );
            }
            items.push(Self::label("MEDIA KEYS GO TO", &theme).into_any_element());
            items.extend(self.targets(&state, cx));
            // Other apps are listed only for their volume.
            if !state.mixer.is_empty() && state.settings.volume_bars != VolumeBars::None {
                items.push(Self::label("OTHER APPS", &theme).into_any_element());
                items.extend(self.mixer(&state, cx));
            }
            items
        };

        let dark = theme.is_dark();
        let flyout = v_flex()
            .flex_none()
            .w(px(WIDTH))
            .h(px(self.height))
            .bg(theme.muted)
            .border_1()
            .border_color(theme.border)
            .child(controls.relative().child(measure(&self.measured.controls, view)))
            .child({
                // Only a capped window scrolls, with a scrollbar so it's clear there's more;
                // otherwise rounding could leave a pixel to scroll.
                let content =
                    v_flex().relative().pb(px(14.)).children(middle).child(measure(&self.measured.middle, view));
                let area = div().flex_1().min_h_0().child(content);
                if self.height >= MAX_HEIGHT {
                    // Room on the right, so the overlay bar doesn't sit against the rows.
                    area.pr(px(10.)).overflow_y_scrollbar().into_any_element()
                } else {
                    area.overflow_hidden().into_any_element()
                }
            })
            // Volume sits at the bottom, next to the tray icon the user just clicked.
            .child(
                v_flex()
                    .px_2()
                    .pt_2()
                    .pb_3()
                    .rounded_b(inner_radius())
                    .bg(theme::card(theme.is_dark()))
                    .border_t_1()
                    .border_color(theme.border)
                    .relative()
                    .children(dock)
                    .child(device_header)
                    .child(div().px_2().pt(px(2.)).child(master))
                    .child(measure(&self.measured.volume, view)),
            );
        let flyout = card_box(card(flyout, dark), &self.anchor);
        // Drawn while open, so it gets measured, and while the region still shows it.
        let left = self.frame().panel_left(&self.anchor);
        let panel = (self.page == Page::Settings || self.shown_panel.get().is_some()).then(|| {
            let panel = card(self.settings_panel(&state, &theme, cx).w(px(PANEL)), dark);
            let panel = card_box(panel, &self.anchor);
            if left { panel.mr(px(-OVERLAP)) } else { panel.ml(px(-OVERLAP)) }
        });
        let (before, after) = if left { (panel, None) } else { (None, panel) };
        // The flyout and panel sit against the taskbar edge. The rest is outside the region.
        div()
            .flex()
            .size_full()
            .map(|d| if self.anchor.grows_down() { d.items_start() } else { d.items_end() })
            .map(|d| if left { d.justify_end() } else { d.justify_start() })
            .text_color(theme.foreground)
            // Tauri's line height. GPUI's default leaves taller rows.
            .line_height(relative(1.4))
            .children(before)
            .child(flyout)
            .children(after)
    }
}

//! The whole UI: media key target and per-app volume, settings, output device and
//! master volume, opened next to the tray icon.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::theme::Theme;
use gpui_kit::component::{ActiveTheme, Icon, Sizable, StyledExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use mediakeys_core::{
    Accent, Anchor, Core, IconPage, MediaButtons, MediaControls, MediaKey, Page, Playback, Setting, State, Target, TooltipStyle, TrayStyle,
    VolumeBars,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::theme::{self, DARK_FOOTER, hex};
use crate::ui::{self, Volumes};

pub const WIDTH: f32 = 360.;
const MAX_HEIGHT: f32 = 640.;

// Estimated heights, for sizing the window before its content is measured.
const CONTROLS: f32 = 45.;
const VOLUME: f32 = 92.;
const LABEL: f32 = 34.;
const TARGET_ROW: f32 = 58.;
const MIXER_ROW: f32 = 35.;
const SLIDER: f32 = 32.;
const SETTING_ROW: f32 = 50.;
const SETTINGS: usize = 9;
/// Extra height of each media controls layout.
const BANNER: f32 = 66.;
const ROW_CONTROLS: f32 = 36.;
const DOCK: f32 = 56.;
/// A setting with its choices under its title.
const STACKED_ROW: f32 = 72.;

/// Estimated height that fits `page` with the given state.
pub fn height(state: &State, page: Page) -> f32 {
    let middle = if page == Page::Settings {
        LABEL + SETTING_ROW * SETTINGS as f32 + STACKED_ROW
    } else {
        let always = state.settings.volume_bars == VolumeBars::Always;
        let sliders = if always { state.targets.iter().filter(|t| t.volume.is_some()).count() } else { 0 };
        let targets = LABEL + TARGET_ROW * (1 + state.targets.len()) as f32 + SLIDER * sliders as f32;
        let mixer = if state.mixer.is_empty() { 0. } else { LABEL + MIXER_ROW * state.mixer.len() as f32 };
        let controls = match state.settings.media_controls {
            MediaControls::Banner => BANNER,
            MediaControls::Row => ROW_CONTROLS,
            MediaControls::Dock | MediaControls::Hover => 0.,
        };
        targets + mixer + controls
    };
    let dock = if state.settings.media_controls == MediaControls::Dock { DOCK } else { 0. };
    (CONTROLS + middle + VOLUME + dock + 12.).min(MAX_HEIGHT)
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
        let selected = state.selected.as_deref().and_then(|id| state.targets.iter().find(|t| t.id == id));
        let Some(target) = selected else {
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

/// The laid-out heights of the flyout's three parts, zero until first measured.
#[derive(Default)]
struct Measured {
    controls: Rc<Cell<f32>>,
    middle: Rc<Cell<f32>>,
    volume: Rc<Cell<f32>>,
}

impl Measured {
    fn total(&self) -> Option<f32> {
        let parts = [self.controls.get(), self.middle.get(), self.volume.get()];
        // The window border adds a pixel above and below.
        parts.iter().all(|h| *h > 0.).then(|| parts.iter().sum::<f32>() + 2.)
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
    dark: Option<bool>,
    accent: Accent,
    height: f32,
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
            mediakeys_core::set_cloaked(hwnd, true);
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
        Flyout {
            core,
            state,
            anchor,
            volumes,
            page,
            devices_open: false,
            pinned,
            hovered: None,
            dark,
            accent,
            height,
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

    /// Slides the flyout back behind the taskbar, then closes its window.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.closing.is_none() {
            self.closing = Some(Instant::now());
            crate::flyout_closed(cx);
            cx.notify();
        }
    }

    /// Resizes the window to its content and slides it in or out.
    /// GPUI resizes downward from the top, so the window moves once the new size applies.
    fn fit(&mut self, state: &State, window: &mut Window) {
        if let Some(closing) = self.closing {
            if mediakeys_core::slide_out_progress(closing) <= 0.0 {
                window.remove_window();
                return;
            }
            window.request_animation_frame();
            let Some(hwnd) = hwnd(window) else { return };
            let anchor = self.anchor;
            window.on_next_frame(move |_, _| {
                mediakeys_core::place_window(hwnd, &anchor, mediakeys_core::slide_out_progress(closing));
            });
            return;
        }
        let height = match self.measured.total() {
            Some(total) => total.ceil().min(MAX_HEIGHT),
            None => height(state, self.page),
        };
        let resized = height != self.height;
        if !resized && self.settled {
            return;
        }
        if resized {
            self.height = height;
            window.resize(size(px(WIDTH), px(height)));
        }
        let progress = mediakeys_core::slide_progress(self.opened);
        if progress < 1.0 {
            window.request_animation_frame();
        } else {
            self.settled = true;
        }
        let Some(hwnd) = hwnd(window) else { return };
        let (anchor, opened) = (self.anchor, self.opened);
        window.on_next_frame(move |_, _| {
            mediakeys_core::place_window(hwnd, &anchor, mediakeys_core::slide_progress(opened));
            mediakeys_core::set_cloaked(hwnd, false);
        });
    }

    fn label(text: &'static str, theme: &Theme) -> Div {
        div().px_4().pt_3().pb_1().text_xs().font_semibold().text_color(theme.muted_foreground).child(text)
    }

    fn controls(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let dark = theme.is_dark();
        let settings_open = self.page == Page::Settings;
        let (core_theme, core_quit) = (self.core.clone(), self.core.clone());
        h_flex()
            .justify_between()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .when(dark, |d| d.bg(hex(DARK_FOOTER)))
            .child(
                h_flex()
                    .gap_2()
                    .pl_1()
                    .child(ui::logo(px(18.), self.accent.colors(dark).gradient))
                    .child(div().text_sm().font_semibold().child("Media Keys Selector")),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("theme")
                            .small()
                            .ghost()
                            .icon(if dark { IconName::Sun } else { IconName::Moon })
                            .tooltip("Toggle dark mode")
                            .on_click(move |_, _, _| core_theme.set(Setting::Dark(Some(!dark)))),
                    )
                    .child(
                        Button::new("settings")
                            .small()
                            .ghost()
                            .icon(Icon::new(IconName::Settings).when(settings_open, |i| i.text_color(theme.primary)))
                            .tooltip("Settings")
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
                                    .when(self.pinned, |i| i.text_color(theme.primary)),
                            )
                            .tooltip(if self.pinned { "Unpin: close when clicking elsewhere" } else { "Pin: stay open" })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pinned = !this.pinned;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("quit").small().ghost().icon(IconName::Power).tooltip("Quit").on_click(
                            move |_, _, cx| {
                                core_quit.shutdown();
                                cx.quit();
                            },
                        ),
                    ),
            )
    }

    /// Previous, play/pause and next. With `target` they control that app; without,
    /// wherever media keys go.
    fn transport(&self, id: &str, target: Option<String>, playing: bool, small: bool) -> Div {
        let button = |name: &str, key: MediaKey, icon: IconName, primary: bool| {
            let (core, target) = (self.core.clone(), target.clone());
            Button::new(SharedString::from(format!("{id}-{name}")))
                .map(|b| if small { b.xsmall() } else { b.small() })
                .map(|b| if primary { b.primary() } else { b.ghost() })
                .icon(icon)
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
            .child(self.transport("now-playing", None, now.playing, false))
    }

    fn device_header(&self, state: &State, theme: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let name = state.audio.device().map(|d| d.name.clone()).unwrap_or_else(|| "No output device".into());
        let open = self.devices_open;
        h_flex()
            .id("device")
            .gap_2()
            .px_2()
            .py_1p5()
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(|s| s.bg(theme.secondary_hover))
            .child(Icon::new(IconName::Speaker).small())
            .child(div().flex_1().min_w_0().truncate().text_sm().child(name))
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
                .child(div().flex_1().min_w_0().truncate().text_sm().child(device.name.clone()))
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
    fn detail(target: &Target, theme: &Theme) -> Div {
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
        line.when(playing, |d| d.child(Self::playing_bars(&target.id, theme.success)))
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
        let mut rows = Vec::new();

        let automatic = state.selected.is_none();
        rows.push(
            Self::row(&theme, "automatic".into(), automatic, true)
                .on_click(self.select_on_click(None))
                .child(
                    h_flex()
                        .gap_3()
                        .px_2()
                        .py_2()
                        .child(
                            ui::logo(px(28.), self.accent.colors(theme.is_dark()).gradient)
                                .child(Icon::new(IconName::Sparkles).small().text_color(white())),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .child(div().text_sm().font_semibold().child("Automatic"))
                                .child(div().text_xs().text_color(theme.muted_foreground).child("Windows decides")),
                        )
                        .child(Self::radio("automatic", &theme, automatic, &self.core, None)),
                )
                .when(automatic && controls == MediaControls::Row, |d| {
                    d.child(Self::row_controls(self.transport("automatic", None, false, false)))
                })
                .into_any_element(),
        );

        for target in &state.targets {
            let checked = state.selected.as_deref() == Some(target.id.as_str());
            let expanded = self.volumes.expanded().as_deref() == Some(target.id.as_str());
            let slider = target
                .volume
                .filter(|_| bars == VolumeBars::Always || (bars == VolumeBars::OnClick && expanded))
                .map(|level| self.volumes.slider_row(&target.id, level, target.muted, &theme));
            // With On click, a click on the card opens its slider; otherwise it picks the card.
            let toggles = bars == VolumeBars::OnClick && target.volume.is_some();
            let volume = target.volume.and_then(|level| match bars {
                VolumeBars::Always => None,
                VolumeBars::OnClick => Some(self.volumes.pill(&target.id, level, target.muted, &theme).into_any_element()),
                VolumeBars::Compact => Some(self.volumes.bar(&target.id, level, target.muted, &theme).into_any_element()),
            });
            rows.push(
                Self::row(&theme, target.id.clone().into(), checked, true)
                    .on_hover({
                        let id = target.id.clone();
                        cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered {
                                this.hovered = Some(id.clone());
                            } else if this.hovered.as_ref() == Some(&id) {
                                this.hovered = None;
                            }
                            cx.notify();
                        })
                    })
                    .map(|d| {
                        if toggles {
                            d.on_click(self.volumes.on_toggle(&target.id))
                        } else {
                            d.on_click(self.select_on_click(Some(target.id.clone())))
                        }
                    })
                    .child(
                        h_flex()
                            .gap_3()
                            .px_2()
                            .py_2()
                            .child(ui::avatar(target.icon.as_ref(), &target.name, target.running, px(28.), &theme))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .when(!target.running, |d| d.opacity(0.6))
                                    .child(div().text_sm().font_semibold().truncate().child(target.name.clone()))
                                    .map(|d| {
                                        if controls == MediaControls::Hover && target.running {
                                            // The track line swaps to buttons for this app while hovered.
                                            // Chosen at render time, because a hover style that hides
                                            // elements can change between prepaint and paint.
                                            let line = if self.hovered.as_ref() == Some(&target.id) {
                                                let playing = target.playback == Playback::Playing;
                                                self.transport(&target.id, Some(target.id.clone()), playing, true)
                                            } else {
                                                Self::detail(target, &theme)
                                            };
                                            d.child(div().h(px(22.)).flex().items_center().child(line))
                                        } else {
                                            d.child(Self::detail(target, &theme))
                                        }
                                    }),
                            )
                            .children(volume)
                            .child(Self::radio(&target.id, &theme, checked, &self.core, Some(target.id.clone()))),
                    )
                    .when(checked && controls == MediaControls::Row, |d| {
                        let playing = target.running && target.playback == Playback::Playing;
                        d.child(Self::row_controls(self.transport(&target.id, None, playing, false)))
                    })
                    .when_some(slider, |d, slider| {
                        // Under the name, clear of the avatar.
                        let indent = if bars == VolumeBars::Always { px(38.) } else { px(8.) };
                        d.child(div().pl(indent).pr_2().pb_1p5().mt(px(-4.)).child(slider))
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
        let mut rows = Vec::new();
        for app in &state.mixer {
            let expanded = self.volumes.expanded().as_deref() == Some(app.id.as_str());
            let name = div().text_sm().truncate().child(app.name.clone());
            let main = h_flex()
                .gap_3()
                .px_2()
                .child(ui::avatar(app.icon.as_ref(), &app.name, true, px(22.), &theme));
            let row = Self::row(&theme, app.id.clone().into(), false, bars == VolumeBars::OnClick);
            let row = match bars {
                VolumeBars::Always => {
                    let slider = self.volumes.slider_row(&app.id, app.volume, app.muted, &theme);
                    row.child(main.py_0p5().child(name.w(px(96.)).flex_none()).child(div().flex_1().child(slider)))
                }
                VolumeBars::Compact => row.child(
                    main.py_1()
                        .child(name.flex_1().min_w_0())
                        .child(self.volumes.bar(&app.id, app.volume, app.muted, &theme)),
                ),
                VolumeBars::OnClick => {
                    let slider = expanded.then(|| self.volumes.slider_row(&app.id, app.volume, app.muted, &theme));
                    row.on_click(self.volumes.on_toggle(&app.id))
                        .child(
                            main.py_1p5()
                                .child(name.flex_1().min_w_0())
                                .child(self.volumes.pill(&app.id, app.volume, app.muted, &theme)),
                        )
                        .when_some(slider, |d, slider| d.child(div().px_2().pb_2().child(slider)))
                }
            };
            rows.push(row.into_any_element());
        }
        rows
    }

    /// One color dot per accent; the chosen one gets a ring.
    fn swatches(&self, current: Accent, theme: &Theme) -> AnyElement {
        let dark = theme.is_dark();
        h_flex()
            .gap(px(6.))
            .children(Accent::ALL.into_iter().map(|accent| {
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
            }))
            .into_any_element()
    }

    fn settings(&self, state: &State, theme: &Theme) -> Vec<AnyElement> {
        let s = state.settings.clone();
        let row = |title: &'static str, control: AnyElement| {
            h_flex()
                .mx_2()
                .px_2()
                .h(px(SETTING_ROW))
                .gap_3()
                .border_b_1()
                .border_color(theme.border)
                .child(div().flex_1().min_w_0().text_sm().child(title))
                .child(control)
                .into_any_element()
        };
        let stacked = |title: &'static str, control: AnyElement| {
            v_flex()
                .mx_2()
                .px_2()
                .py_2()
                .gap_1p5()
                .h(px(STACKED_ROW))
                .justify_center()
                .border_b_1()
                .border_color(theme.border)
                .child(div().text_sm().child(title))
                .child(control)
                .into_any_element()
        };
        let switch = |id: &'static str, on: bool, make: fn(bool) -> Setting| {
            let core = self.core.clone();
            Switch::new(id).checked(on).on_click(move |on, _, _| core.set(make(*on))).into_any_element()
        };
        let choice = |id: &'static str, label: &'static str, selected: bool, setting: Setting| {
            let core = self.core.clone();
            Button::new(id)
                .small()
                .label(label)
                .map(|b| if selected { b.primary() } else { b.ghost() })
                .on_click(move |_, _, _| core.set(setting.clone()))
        };
        let page_button = |id: &'static str, label: &'static str, page: IconPage| {
            Button::new(id).small().ghost().label(label).on_click(move |_, _, _| mediakeys_core::open_icon_page(page))
        };
        vec![
            Self::label("SETTINGS", theme).into_any_element(),
            row("Start with Windows", switch("autostart", s.autostart, Setting::Autostart)),
            row("Album art as app icon", switch("album-art", s.album_art, Setting::AlbumArt)),
            row(
                "Theme",
                h_flex()
                    .gap_1()
                    .child(choice("theme-system", "System", s.dark.is_none(), Setting::Dark(None)))
                    .child(choice("theme-light", "Light", s.dark == Some(false), Setting::Dark(Some(false))))
                    .child(choice("theme-dark", "Dark", s.dark == Some(true), Setting::Dark(Some(true))))
                    .into_any_element(),
            ),
            row(
                "Tray icon",
                h_flex()
                    .gap_1()
                    .child(choice("tray-ring", "Ring", s.tray_style == TrayStyle::Ring, Setting::TrayStyle(TrayStyle::Ring)))
                    .child(choice(
                        "tray-number",
                        "Number",
                        s.tray_style == TrayStyle::Number,
                        Setting::TrayStyle(TrayStyle::Number),
                    ))
                    .into_any_element(),
            ),
            row("Accent color", self.swatches(s.accent, theme)),
            row(
                "Tooltip",
                h_flex()
                    .gap_1()
                    .child(choice(
                        "tip-now-playing",
                        "Now playing",
                        s.tooltip == TooltipStyle::NowPlaying,
                        Setting::Tooltip(TooltipStyle::NowPlaying),
                    ))
                    .child(choice("tip-compact", "Compact", s.tooltip == TooltipStyle::Compact, Setting::Tooltip(TooltipStyle::Compact)))
                    .child(choice("tip-text", "Text", s.tooltip == TooltipStyle::Text, Setting::Tooltip(TooltipStyle::Text)))
                    .into_any_element(),
            ),
            row(
                "Media controls",
                h_flex()
                    .gap_1()
                    .children(MediaControls::ALL.map(|c| {
                        let id = SharedString::from(format!("controls-{}", c.name()));
                        let core = self.core.clone();
                        Button::new(id)
                            .small()
                            .label(c.label())
                            .map(|b| if s.media_controls == c { b.primary() } else { b.ghost() })
                            .on_click(move |_, _, _| core.set(Setting::MediaControls(c)))
                    }))
                    .into_any_element(),
            ),
            stacked(
                "Media buttons on the tooltip",
                h_flex()
                    .gap_1()
                    .children(
                        [
                            ("buttons-none", "None", MediaButtons::None),
                            ("buttons-hover", "Hover", MediaButtons::Hover),
                            ("buttons-inline", "Inline", MediaButtons::Inline),
                            ("buttons-capsule", "Capsule", MediaButtons::Capsule),
                            ("buttons-player", "Player", MediaButtons::Player),
                        ]
                        .map(|(id, label, b)| choice(id, label, s.media_buttons == b, Setting::MediaButtons(b))),
                    )
                    .into_any_element(),
            ),
            row(
                "Volume bars",
                h_flex()
                    .gap_1()
                    .child(choice(
                        "bars-compact",
                        "Compact",
                        s.volume_bars == VolumeBars::Compact,
                        Setting::VolumeBars(VolumeBars::Compact),
                    ))
                    .child(choice("bars-always", "Always", s.volume_bars == VolumeBars::Always, Setting::VolumeBars(VolumeBars::Always)))
                    .child(choice(
                        "bars-click",
                        "On click",
                        s.volume_bars == VolumeBars::OnClick,
                        Setting::VolumeBars(VolumeBars::OnClick),
                    ))
                    .into_any_element(),
            ),
            row(
                "Tray icons",
                h_flex()
                    .gap_1()
                    .child(page_button("show-ours", "Show ours", IconPage::ShowOurs))
                    .child(page_button("hide-windows", "Hide Windows'", IconPage::HideWindows))
                    .into_any_element(),
            ),
        ]
    }
}

impl Render for Flyout {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx).clone();
        let theme = cx.theme().clone();
        let view = cx.entity_id();
        self.volumes.begin_frame();
        self.fit(&state, window);

        let controls = self.controls(&theme, cx);
        let master = self.volumes.slider_row(ui::MASTER, state.audio.volume, state.audio.muted, &theme);
        let device_header = self.device_header(&state, &theme, cx);
        let dock = (state.settings.media_controls == MediaControls::Dock).then(|| {
            self.now_playing(&state, &theme, px(36.)).px_2().pb_2().mb_1().border_b_1().border_color(theme.border)
        });
        let middle = if self.devices_open {
            vec![self.devices(&state, &theme, cx).into_any_element()]
        } else if self.page == Page::Settings {
            self.settings(&state, &theme)
        } else {
            let mut items = Vec::new();
            if state.settings.media_controls == MediaControls::Banner {
                let banner = self.now_playing(&state, &theme, px(44.));
                items.push(div().mx_2().mt_2().p_2p5().rounded(theme.radius * 1.5).bg(theme.secondary).child(banner).into_any_element());
            }
            items.push(Self::label("MEDIA KEYS GO TO", &theme).into_any_element());
            items.extend(self.targets(&state, cx));
            if !state.mixer.is_empty() {
                items.push(Self::label("OTHER APPS", &theme).into_any_element());
                items.extend(self.mixer(&state, cx));
            }
            items
        };

        v_flex()
            .size_full()
            .bg(theme.muted)
            .text_color(theme.foreground)
            .border_1()
            .border_color(theme.border)
            .child(controls.relative().child(measure(&self.measured.controls, view)))
            .child(
                // Only a capped window scrolls; otherwise rounding could leave a pixel to scroll.
                div()
                    .id("middle")
                    .flex_1()
                    .min_h_0()
                    .map(|d| if self.height >= MAX_HEIGHT { d.overflow_y_scroll() } else { d.overflow_hidden() })
                    .child(
                    v_flex().relative().pb_2().children(middle).child(measure(&self.measured.middle, view)),
                ),
            )
            // Volume sits at the bottom, next to the tray icon the user just clicked.
            .child(
                v_flex()
                    .gap_1()
                    .px_2()
                    .pt_2()
                    .pb_3()
                    .bg(theme.background)
                    .border_t_1()
                    .border_color(theme.border)
                    .relative()
                    .children(dock)
                    .child(device_header)
                    .child(div().px_2().child(master))
                    .child(measure(&self.measured.volume, view)),
            )
    }
}

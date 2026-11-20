//! The whole UI: media key target and per-app volume, settings, output device and
//! master volume, opened next to the tray icon.

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::theme::Theme;
use gpui_kit::component::{ActiveTheme, Icon, Sizable, StyledExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use mediakeys_core::{Anchor, Core, IconPage, MediaKey, Page, Playback, Setting, State, Target, TrayStyle};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::theme::{self, DARK_FOOTER, hex};
use crate::ui::{self, Volumes};

pub const WIDTH: f32 = 360.;
const MAX_HEIGHT: f32 = 640.;

// Row heights for sizing the window to its content.
const CONTROLS: f32 = 45.;
const VOLUME: f32 = 92.;
const LABEL: f32 = 34.;
const TARGET_ROW: f32 = 58.;
const MIXER_ROW: f32 = 35.;
const SLIDER: f32 = 40.;
const SETTING_ROW: f32 = 50.;
const DEVICE_ROW: f32 = 36.;
const SETTINGS: usize = 5;

/// Height that fits `page` with the given state, before any slider is revealed.
pub fn height(state: &State, page: Page) -> f32 {
    content_height(state, page, false, false)
}

fn content_height(state: &State, page: Page, devices_open: bool, slider: bool) -> f32 {
    let middle = if devices_open {
        8. + DEVICE_ROW * state.audio.devices.len() as f32
    } else if page == Page::Settings {
        LABEL + SETTING_ROW * SETTINGS as f32
    } else {
        let targets = LABEL + TARGET_ROW * (1 + state.targets.len()) as f32;
        let mixer = if state.mixer.is_empty() { 0. } else { LABEL + MIXER_ROW * state.mixer.len() as f32 };
        targets + mixer + if slider { SLIDER } else { 0. }
    };
    (CONTROLS + middle + VOLUME + 12.).min(MAX_HEIGHT)
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
    dark: Option<bool>,
    height: f32,
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
        let dark = state.read(cx).settings.dark;
        theme::apply(window, cx, dark);
        let height = height(state.read(cx), page);
        let _subscriptions = vec![
            cx.observe_in(&state, window, |this: &mut Self, state, window, cx| {
                let dark = state.read(cx).settings.dark;
                if dark != this.dark {
                    this.dark = dark;
                    theme::apply(window, cx, dark);
                }
                cx.notify();
            }),
            cx.observe_window_appearance(window, |this, window, cx| theme::apply(window, cx, this.dark)),
            // Clicking anywhere else closes the flyout, like the Windows one.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() && !this.pinned {
                    window.remove_window();
                    crate::flyout_closed(cx);
                }
            }),
        ];
        let volumes = Volumes::new(core.clone(), cx);
        Flyout { core, state, anchor, volumes, page, devices_open: false, pinned, dark, height, _subscriptions }
    }

    pub fn open_page(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        self.devices_open = false;
        cx.notify();
    }

    /// Resizes the window to its content, keeping the bottom against the taskbar.
    fn fit(&mut self, state: &State, window: &mut Window) {
        let slider = self.volumes.expanded().is_some() && self.page == Page::Apps;
        let height = content_height(state, self.page, self.devices_open, slider);
        if height == self.height {
            return;
        }
        self.height = height;
        if let Ok(handle) = HasWindowHandle::window_handle(window)
            && let RawWindowHandle::Win32(win32) = handle.as_raw()
        {
            mediakeys_core::place_window(win32.hwnd.get(), &self.anchor, WIDTH, height);
        }
    }

    fn label(text: &'static str, theme: &Theme) -> Div {
        div().px_4().pt_3().pb_1().text_xs().font_semibold().text_color(theme.muted_foreground).child(text)
    }

    fn controls(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let press = |key: MediaKey, core: &Core| {
            let core = core.clone();
            move |_: &ClickEvent, _: &mut Window, _: &mut App| core.press(key)
        };
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
                    .gap_1()
                    .child(Button::new("prev").small().ghost().icon(IconName::SkipBack).on_click(press(MediaKey::Previous, &self.core)))
                    .child(Button::new("play").small().primary().icon(IconName::Play).on_click(press(MediaKey::PlayPause, &self.core)))
                    .child(Button::new("next").small().ghost().icon(IconName::SkipForward).on_click(press(MediaKey::Next, &self.core))),
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

    fn radio(theme: &Theme, checked: bool) -> Div {
        div()
            .flex_none()
            .size(px(16.))
            .rounded_full()
            .border_color(if checked { theme.primary } else { theme.muted_foreground })
            .map(|d| if checked { d.border(px(4.5)) } else { d.border(px(1.5)) })
    }

    fn row(theme: &Theme, id: SharedString, checked: bool) -> Stateful<Div> {
        let hover = theme.secondary_hover;
        v_flex()
            .id(id)
            .mx_2()
            .rounded(theme.radius)
            .cursor_pointer()
            .when(checked, |d| d.bg(theme.primary.opacity(0.13)))
            .when(!checked, |d| d.hover(move |s| s.bg(hover)))
    }

    fn targets(&mut self, state: &State, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme().clone();
        let mut rows = Vec::new();

        let core = self.core.clone();
        let automatic = state.selected.is_none();
        rows.push(
            Self::row(&theme, "automatic".into(), automatic)
                .on_click(move |_, _, _| core.select(None))
                .child(
                    h_flex()
                        .gap_3()
                        .px_2()
                        .py_2()
                        .child(ui::logo(px(28.)).child(Icon::new(IconName::Sparkles).small().text_color(white())))
                        .child(
                            v_flex()
                                .flex_1()
                                .child(div().text_sm().font_semibold().child("Automatic"))
                                .child(div().text_xs().text_color(theme.muted_foreground).child("Windows decides")),
                        )
                        .child(Self::radio(&theme, automatic)),
                )
                .into_any_element(),
        );

        for target in &state.targets {
            let checked = state.selected.as_deref() == Some(target.id.as_str());
            let expanded = self.volumes.expanded().as_deref() == Some(target.id.as_str());
            let slider = target
                .volume
                .filter(|_| expanded)
                .map(|level| self.volumes.slider_row(&target.id, level, target.muted, window, cx));
            let (core, id) = (self.core.clone(), target.id.clone());
            rows.push(
                Self::row(&theme, target.id.clone().into(), checked)
                    .on_click(move |_, _, _| core.select(Some(id.clone())))
                    .when(target.volume.is_some(), |d| d.on_scroll(self.volumes.on_wheel(&target.id)))
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
                                    .child(Self::detail(target, &theme)),
                            )
                            .when_some(target.volume, |d, level| {
                                d.child(self.volumes.pill(&target.id, level, target.muted, &theme))
                            })
                            .child(Self::radio(&theme, checked)),
                    )
                    .when_some(slider, |d, slider| d.child(div().px_2().pb_2().child(slider)))
                    .into_any_element(),
            );
        }
        rows
    }

    fn mixer(&mut self, state: &State, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme().clone();
        let mut rows = Vec::new();
        for app in &state.mixer {
            let expanded = self.volumes.expanded().as_deref() == Some(app.id.as_str());
            let slider = expanded.then(|| self.volumes.slider_row(&app.id, app.volume, app.muted, window, cx));
            rows.push(
                Self::row(&theme, app.id.clone().into(), false)
                    .cursor_default()
                    .on_scroll(self.volumes.on_wheel(&app.id))
                    .child(
                        h_flex()
                            .gap_3()
                            .px_2()
                            .py_1p5()
                            .child(ui::avatar(app.icon.as_ref(), &app.name, true, px(22.), &theme))
                            .child(div().flex_1().min_w_0().truncate().text_sm().child(app.name.clone()))
                            .child(self.volumes.pill(&app.id, app.volume, app.muted, &theme)),
                    )
                    .when_some(slider, |d, slider| d.child(div().px_2().pb_2().child(slider)))
                    .into_any_element(),
            );
        }
        rows
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
        self.fit(&state, window);

        let controls = self.controls(&theme, cx);
        let master = self.volumes.slider_row(ui::MASTER, state.audio.volume, state.audio.muted, window, cx);
        let device_header = self.device_header(&state, &theme, cx);
        let middle = if self.devices_open {
            vec![self.devices(&state, &theme, cx).into_any_element()]
        } else if self.page == Page::Settings {
            self.settings(&state, &theme)
        } else {
            let mut items = vec![Self::label("MEDIA KEYS GO TO", &theme).into_any_element()];
            items.extend(self.targets(&state, window, cx));
            if !state.mixer.is_empty() {
                items.push(Self::label("OTHER APPS", &theme).into_any_element());
                items.extend(self.mixer(&state, window, cx));
            }
            items
        };

        v_flex()
            .size_full()
            .bg(theme.muted)
            .text_color(theme.foreground)
            .border_1()
            .border_color(theme.border)
            .child(controls)
            .child(div().id("middle").flex_1().overflow_y_scroll().pb_2().children(middle))
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
                    .child(device_header)
                    .child(div().px_2().child(master)),
            )
    }
}

//! The tray flyout: output device, master volume, media key target and per-app volume.

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::theme::Theme;
use gpui_kit::component::{ActiveTheme, Icon, Sizable, StyledExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use mediakeys_core::{Core, MediaKey, Playback, State, Target};

use crate::theme::{self, DARK_FOOTER, hex};
use crate::ui::{self, Volumes};

pub const WIDTH: f32 = 360.;

const MAX_HEIGHT: f32 = 640.;

/// Height that fits the current content. A revealed slider scrolls the app list,
/// so the volume section stays against the taskbar.
pub fn height(state: &State) -> f32 {
    let controls = 45.;
    let targets = 34. + 58. * (1 + state.targets.len()) as f32;
    let mixer = if state.mixer.is_empty() { 0. } else { 34. + 35. * state.mixer.len() as f32 };
    let volume = 92.;
    (controls + targets + mixer + volume + 12.).min(MAX_HEIGHT)
}

pub struct Flyout {
    core: Core,
    state: Entity<State>,
    volumes: Volumes,
    devices_open: bool,
    _subscriptions: Vec<Subscription>,
}

impl Flyout {
    pub fn new(core: Core, state: Entity<State>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dark = state.read(cx).settings.dark;
        theme::apply(window, cx, dark);
        let _subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            // Clicking anywhere else closes the flyout, like the Windows one.
            cx.observe_window_activation(window, |_, window, cx| {
                if !window.is_window_active() {
                    window.remove_window();
                    crate::flyout_closed(cx);
                }
            }),
        ];
        let volumes = Volumes::new(core.clone(), cx);
        Flyout { core, state, volumes, devices_open: false, _subscriptions }
    }

    fn label(text: &'static str, theme: &Theme) -> Div {
        div().px_4().pt_3().pb_1().text_xs().font_semibold().text_color(theme.muted_foreground).child(text)
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

    fn row_detail(target: &Target, theme: &Theme) -> Option<Div> {
        let status = match target.playback {
            _ if !target.running => "Not running",
            Playback::Playing => "Playing",
            Playback::Paused => "Paused",
            Playback::Stopped => "Stopped",
            Playback::Unknown => return target.track.clone().map(|t| div().child(t)),
        };
        let playing = target.playback == Playback::Playing && target.running;
        let text = match &target.track {
            Some(track) if target.running => format!("{status} · {track}"),
            _ => status.to_owned(),
        };
        Some(div().when(playing, |d| d.text_color(theme.success)).child(text))
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
            let slider =
                target.volume.filter(|_| expanded).map(|level| self.volumes.slider_row(&target.id, level, target.muted, window, cx));
            let (core, id) = (self.core.clone(), target.id.clone());
            let detail = Self::row_detail(target, &theme);
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
                                    .when_some(detail, |d, detail| {
                                        d.child(detail.text_xs().text_color(theme.muted_foreground).truncate())
                                    }),
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
}

impl Render for Flyout {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx).clone();
        let theme = cx.theme().clone();
        let dark = theme.is_dark();

        let master = self.volumes.slider_row(ui::MASTER, state.audio.volume, state.audio.muted, window, cx);
        let device_header = self.device_header(&state, &theme, cx);
        let middle = if self.devices_open {
            vec![self.devices(&state, &theme, cx).into_any_element()]
        } else {
            let mut items = vec![Self::label("MEDIA KEYS GO TO", &theme).into_any_element()];
            items.extend(self.targets(&state, window, cx));
            if !state.mixer.is_empty() {
                items.push(Self::label("OTHER APPS", &theme).into_any_element());
                items.extend(self.mixer(&state, window, cx));
            }
            items
        };

        let press = |key: MediaKey, core: &Core| {
            let core = core.clone();
            move |_: &ClickEvent, _: &mut Window, _: &mut App| core.press(key)
        };
        let core = self.core.clone();

        v_flex()
            .size_full()
            .bg(theme.muted)
            .text_color(theme.foreground)
            .border_1()
            .border_color(theme.border)
            .child(
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
                                Button::new("open")
                                    .small()
                                    .ghost()
                                    .icon(IconName::AppWindow)
                                    .tooltip("Open Media Keys Selector")
                                    .on_click(|_, window, cx| {
                                        window.remove_window();
                                        crate::show_main(cx);
                                    }),
                            )
                            .child(
                                Button::new("quit")
                                    .small()
                                    .ghost()
                                    .icon(IconName::Power)
                                    .tooltip("Quit")
                                    .on_click(move |_, _, cx| {
                                        core.shutdown();
                                        cx.quit();
                                    }),
                            ),
                    ),
            )
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

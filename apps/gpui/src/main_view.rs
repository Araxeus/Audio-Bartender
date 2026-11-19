use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::theme::Theme;
use gpui_kit::component::{ActiveTheme, Icon, Sizable, StyledExt, TitleBar};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use mediakeys_core::{Core, IconPage, MediaKey, MixerApp, Playback, Setting, State, Target, TrayStyle};

use crate::theme::{self, DARK_FOOTER, hex};
use crate::ui::{self, Volumes};

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Targets,
    Settings,
}

pub struct MainView {
    core: Core,
    state: Entity<State>,
    dark: Option<bool>,
    custom_title_bar: bool,
    page: Page,
    volumes: Volumes,
    _subscriptions: Vec<Subscription>,
}

impl MainView {
    pub fn new(core: Core, state: Entity<State>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = state.read(cx).settings.clone();
        theme::apply(window, cx, settings.dark);
        let _subscriptions = vec![
            cx.observe_in(&state, window, |this, state, window, cx| {
                let settings = state.read(cx).settings.clone();
                if settings.dark != this.dark {
                    this.dark = settings.dark;
                    theme::apply(window, cx, settings.dark);
                }
                if settings.custom_title_bar != this.custom_title_bar {
                    // GPUI fixes the title bar mode at creation, so reopen in place.
                    this.custom_title_bar = settings.custom_title_bar;
                    let bounds = window.bounds();
                    window.remove_window();
                    cx.defer(move |cx| crate::open_main(cx, Some(bounds)));
                }
                cx.notify();
            }),
            cx.observe_window_appearance(window, |this, window, cx| theme::apply(window, cx, this.dark)),
        ];
        let volumes = Volumes::new(core.clone(), cx);
        MainView {
            core,
            state,
            dark: settings.dark,
            custom_title_bar: settings.custom_title_bar,
            page: Page::Targets,
            volumes,
            _subscriptions,
        }
    }

    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.page = Page::Settings;
        cx.notify();
    }

    fn header(&self, state: &State, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let routed = state.routed().map(|t| t.name.clone());
        let status = match (&routed, &state.selected) {
            (Some(name), _) => format!("Captured · sending to {name}"),
            (None, Some(_)) => "Waiting for the chosen app · Windows decides meanwhile".to_owned(),
            (None, None) => "Windows decides".to_owned(),
        };
        let dot = if routed.is_some() { theme.primary } else { theme.muted_foreground };
        let dark = theme.is_dark();
        let settings_open = self.page == Page::Settings;

        let title = if settings_open {
            h_flex()
                .gap_2()
                .child(
                    Button::new("back")
                        .ghost()
                        .icon(IconName::ArrowLeft)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.page = Page::Targets;
                            cx.notify();
                        })),
                )
                .child(div().text_xl().font_semibold().child("Settings"))
        } else {
            h_flex().child(div().text_xl().font_semibold().child("Media keys go to"))
        };

        let core = self.core.clone();
        h_flex()
            .items_start()
            .justify_between()
            .px_5()
            .pt(if state.settings.custom_title_bar { px(4.) } else { px(16.) })
            .pb_2()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(div().size(px(8.)).rounded_full().bg(dot))
                            .child(status),
                    )
                    .child(title),
            )
            .child(
                h_flex()
                    .gap_0p5()
                    .child(
                        Button::new("theme")
                            .ghost()
                            .icon(if dark { IconName::Sun } else { IconName::Moon })
                            .tooltip("Toggle dark mode")
                            .on_click(move |_, _, _| core.set(Setting::Dark(Some(!dark)))),
                    )
                    .child(
                        Button::new("settings")
                            .ghost()
                            .icon(Icon::new(IconName::Settings).when(settings_open, |i| i.text_color(theme.primary)))
                            .tooltip("Settings")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.page = if this.page == Page::Settings { Page::Targets } else { Page::Settings };
                                cx.notify();
                            })),
                    ),
            )
    }

    fn card(theme: &Theme, checked: bool) -> Stateful<Div> {
        let hover = theme.secondary_hover;
        v_flex()
            .id("card")
            .relative()
            .overflow_hidden()
            .w_full()
            .rounded(theme.radius_lg)
            .border_1()
            .when(checked, |d| d.border_color(theme.primary).bg(theme.primary.opacity(0.13)))
            .when(!checked, |d| d.border_color(theme.border).bg(theme.background).hover(move |s| s.bg(hover)))
    }

    fn radio(theme: &Theme, checked: bool) -> Div {
        div()
            .flex_none()
            .size(px(18.))
            .rounded_full()
            .border_color(if checked { theme.primary } else { theme.muted_foreground })
            .map(|d| if checked { d.border(px(5.)) } else { d.border(px(1.5)) })
    }

    fn detail(target: &Target, theme: &Theme) -> AnyElement {
        let line = h_flex().gap_1().text_xs().text_color(theme.muted_foreground).overflow_hidden();
        if !target.running {
            return line.child("Not running · keys go to Windows until it starts").into_any_element();
        }
        let playing = target.playback == Playback::Playing;
        let status = match target.playback {
            Playback::Playing => "Playing",
            Playback::Paused => "Paused",
            Playback::Stopped => "Stopped",
            Playback::Unknown if target.via_window => "Window messages",
            Playback::Unknown => "Idle",
        };
        line.when(playing, |d| d.child(div().flex_none().size(px(6.)).rounded_full().bg(theme.success)))
            .child(div().flex_none().when(playing, |d| d.text_color(theme.success)).child(status))
            .when_some(target.track.clone(), |d, track| d.child(div().truncate().child(format!("· {track}"))))
            .into_any_element()
    }

    fn automatic(&self, state: &State, theme: &Theme) -> AnyElement {
        let core = self.core.clone();
        let checked = state.selected.is_none();
        Self::card(theme, checked)
            .id("automatic")
            .cursor_pointer()
            .on_click(move |_, _, _| core.select(None))
            .child(
                h_flex()
                    .gap_3()
                    .px_3()
                    .py_2p5()
                    .child(ui::logo(px(32.)).child(Icon::new(IconName::Sparkles).small().text_color(white())))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().font_semibold().child("Automatic"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("Windows picks the most recent player"),
                            ),
                    )
                    .child(Self::radio(theme, checked)),
            )
            .into_any_element()
    }

    fn target(&mut self, state: &State, target: &Target, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let checked = state.selected.as_deref() == Some(target.id.as_str());
        let expanded = self.volumes.expanded().as_deref() == Some(target.id.as_str());
        let core = self.core.clone();
        let id = target.id.clone();
        let slider =
            target.volume.filter(|_| expanded).map(|level| self.volumes.slider_row(&target.id, level, target.muted, window, cx));

        Self::card(&theme, checked)
            .id(SharedString::from(target.id.clone()))
            .cursor_pointer()
            .on_click(move |_, _, _| core.select(Some(id.clone())))
            .when(target.volume.is_some(), |d| d.on_scroll(self.volumes.on_wheel(&target.id)))
            .child(
                h_flex()
                    .gap_3()
                    .px_3()
                    .py_2p5()
                    .child(ui::avatar(target.icon.as_ref(), &target.name, target.running, px(32.), &theme))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .when(!target.running, |d| d.opacity(0.6))
                            .child(div().font_semibold().truncate().child(target.name.clone()))
                            .child(Self::detail(target, &theme)),
                    )
                    .when_some(target.volume, |d, level| d.child(self.volumes.pill(&target.id, level, target.muted, &theme)))
                    .child(Self::radio(&theme, checked)),
            )
            .when_some(slider, |d, slider| d.child(div().px_3().pb_2().child(slider)))
            .into_any_element()
    }

    fn mixer_app(&mut self, app: &MixerApp, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let expanded = self.volumes.expanded().as_deref() == Some(app.id.as_str());
        let slider = expanded.then(|| self.volumes.slider_row(&app.id, app.volume, app.muted, window, cx));
        Self::card(&theme, false)
            .id(SharedString::from(app.id.clone()))
            .on_scroll(self.volumes.on_wheel(&app.id))
            .child(
                h_flex()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .child(ui::avatar(app.icon.as_ref(), &app.name, true, px(24.), &theme))
                    .child(div().flex_1().min_w_0().truncate().text_sm().child(app.name.clone()))
                    .child(self.volumes.pill(&app.id, app.volume, app.muted, &theme)),
            )
            .when_some(slider, |d, slider| d.child(div().px_3().pb_2().child(slider)))
            .into_any_element()
    }

    fn targets_page(&mut self, state: &State, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let mut items = vec![self.automatic(state, &theme)];
        for target in &state.targets {
            items.push(self.target(state, target, window, cx));
        }
        let hint = |text: &'static str| div().py_1().text_center().text_xs().text_color(theme.muted_foreground).child(text);
        if state.targets.is_empty() {
            items.push(hint("No media apps found. Start playing something and it shows up here.").into_any_element());
        }
        if !state.mixer.is_empty() {
            items.push(
                div()
                    .pt_3()
                    .text_xs()
                    .font_semibold()
                    .text_color(theme.muted_foreground)
                    .child("OTHER APPS")
                    .into_any_element(),
            );
            for app in &state.mixer {
                items.push(self.mixer_app(app, window, cx));
            }
        }
        items.push(hint("Scroll over an app to change its volume").into_any_element());
        v_flex().gap_1p5().children(items)
    }

    fn settings_page(&self, state: &State, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let s = state.settings.clone();
        let row = |title: &'static str, detail: &'static str, control: AnyElement| {
            h_flex()
                .gap_3()
                .px_3()
                .py_2p5()
                .border_b_1()
                .border_color(theme.border)
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_sm().font_semibold().child(title))
                        .child(div().text_xs().text_color(theme.muted_foreground).child(detail)),
                )
                .child(control)
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

        v_flex()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .overflow_hidden()
            .child(row("Start with Windows", "Launch in the tray when you sign in", switch("autostart", s.autostart, Setting::Autostart)))
            .child(row(
                "Close button hides to tray",
                "Off: closing the window quits the app",
                switch("close-to-tray", s.close_to_tray, Setting::CloseToTray),
            ))
            .child(row(
                "In-app title bar",
                "Draw the title bar in the app's theme",
                switch("titlebar", s.custom_title_bar, Setting::CustomTitleBar),
            ))
            .child(row(
                "Album art as app icon",
                "Show the playing track's cover instead",
                switch("album-art", s.album_art, Setting::AlbumArt),
            ))
            .child(row(
                "Replace the Windows volume icon",
                "Show this icon on the taskbar, then turn Volume off",
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("show-ours")
                            .small()
                            .label("Show this")
                            .on_click(|_, _, _| mediakeys_core::open_icon_page(IconPage::ShowOurs)),
                    )
                    .child(
                        Button::new("hide-windows")
                            .small()
                            .label("Hide Windows'")
                            .on_click(|_, _, _| mediakeys_core::open_icon_page(IconPage::HideWindows)),
                    )
                    .into_any_element(),
            ))
            .child(row(
                "Theme",
                "Follow Windows or pick one",
                h_flex()
                    .gap_1()
                    .child(choice("theme-system", "System", s.dark.is_none(), Setting::Dark(None)))
                    .child(choice("theme-light", "Light", s.dark == Some(false), Setting::Dark(Some(false))))
                    .child(choice("theme-dark", "Dark", s.dark == Some(true), Setting::Dark(Some(true))))
                    .into_any_element(),
            ))
            .child(row(
                "Tray icon",
                "How the tray shows the volume",
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
            ))
    }

    fn master(&mut self, state: &State, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let device = state.audio.device().map(|d| d.name.clone()).unwrap_or_else(|| "No output device".into());
        let row = self.volumes.slider_row(ui::MASTER, state.audio.volume, state.audio.muted, window, cx);
        v_flex()
            .px_5()
            .pt_2()
            .gap_1()
            .child(
                h_flex()
                    .gap_1p5()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(IconName::Speaker).xsmall())
                    .child(div().truncate().child(device)),
            )
            .child(row)
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx).clone();
        let theme = cx.theme().clone();
        let dark = theme.is_dark();

        let press = |key: MediaKey, core: &Core| {
            let core = core.clone();
            move |_: &ClickEvent, _: &mut Window, _: &mut App| core.press(key)
        };
        let quit = {
            let core = self.core.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                core.shutdown();
                cx.quit();
            }
        };

        let body = match self.page {
            Page::Targets => self.targets_page(&state, window, cx).into_any_element(),
            Page::Settings => self.settings_page(&state, cx).into_any_element(),
        };
        let header = self.header(&state, &theme, cx);
        let master = self.master(&state, window, cx);

        v_flex()
            .size_full()
            .bg(theme.muted)
            .text_color(theme.foreground)
            .when(state.settings.custom_title_bar, |d| {
                d.child(
                    TitleBar::new().child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(ui::logo(px(16.)).child(Icon::new(IconName::Play).xsmall().text_color(white())))
                            .child("Media Keys Selector"),
                    ),
                )
            })
            .child(header)
            .child(div().id("body").flex_1().overflow_y_scroll().px_5().py_1p5().child(body))
            .child(
                v_flex()
                    .border_t_1()
                    .border_color(theme.border)
                    .when(dark, |d| d.bg(hex(DARK_FOOTER)))
                    .child(master)
                    .child(
                        h_flex()
                            .justify_between()
                            .px_5()
                            .py_3()
                            .child(
                                h_flex()
                                    .gap_1p5()
                                    .child(
                                        Button::new("prev")
                                            .icon(IconName::SkipBack)
                                            .tooltip("Previous track")
                                            .on_click(press(MediaKey::Previous, &self.core)),
                                    )
                                    .child(
                                        Button::new("play")
                                            .primary()
                                            .icon(IconName::Play)
                                            .tooltip("Play / pause")
                                            .on_click(press(MediaKey::PlayPause, &self.core)),
                                    )
                                    .child(
                                        Button::new("next")
                                            .icon(IconName::SkipForward)
                                            .tooltip("Next track")
                                            .on_click(press(MediaKey::Next, &self.core)),
                                    ),
                            )
                            .child(Button::new("quit").ghost().icon(IconName::Power).label("Quit").on_click(quit)),
                    ),
            )
    }
}

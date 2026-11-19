#![windows_subsystem = "windows"]

use std::borrow::Cow;

use futures::StreamExt;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::component::{ActiveTheme, Icon, Sizable, StyledExt, TitleBar};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use mediakeys_core::{Core, Event, MediaKey, Playback, State, Target};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

gpui_kit::assets::icon_assets!(
    MediaIcons,
    [Play, SkipBack, SkipForward, Sparkles, Moon, Sun, AppWindow, Volume2, VolumeX]
);

/// The component defaults plus the app's own icons.
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = MediaIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(MediaIcons.list(path)?);
        Ok(paths)
    }
}

struct Globals {
    core: Core,
    state: Entity<State>,
    window: Option<AnyWindowHandle>,
}

impl Global for Globals {}

const AVATAR_COLORS: [u32; 8] = [0x6d5ef5, 0x0ea5e9, 0xf97316, 0x10b981, 0xec4899, 0xeab308, 0xef4444, 0x14b8a6];

/// Volume change per wheel notch.
const VOLUME_STEP: f32 = 0.05;

/// Same hash as the Tauri front end, so both pick the same color per app.
fn avatar_color(name: &str) -> Hsla {
    let hash = name.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32));
    rgb(AVATAR_COLORS[hash.unsigned_abs() as usize % AVATAR_COLORS.len()]).into()
}

fn hex(color: u32) -> Hsla {
    rgb(color).into()
}

/// Applies light or dark mode with the app palette, which matches the Tauri front end.
fn apply_theme(window: &mut Window, cx: &mut App, dark: Option<bool>) {
    match dark {
        Some(dark) => Theme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, Some(window), cx),
        None => Theme::sync_system_appearance(Some(window), cx),
    }
    let dark = cx.theme().is_dark();
    Theme::update(cx, |t| {
        let (accent, hover) = if dark { (0x9d8cff, 0xb0a3ff) } else { (0x6d5ef5, 0x7f72f7) };
        t.primary = hex(accent);
        t.primary_hover = hex(hover);
        t.primary_active = hex(accent);
        t.primary_foreground = white();
        t.button_primary = hex(accent);
        t.button_primary_hover = hex(hover);
        t.button_primary_active = hex(accent);
        t.button_primary_foreground = white();
        t.ring = hex(accent);

        // `muted` is the window, `background` the cards.
        let (window_bg, card, hover, border, text, muted) = if dark {
            (0x121218, 0x1c1c25, 0x23232e, 0x2a2a37, 0xececf3, 0x8e8ea3)
        } else {
            (0xf3f3f6, 0xffffff, 0xf7f7fa, 0xe4e4ea, 0x1b1b22, 0x6b6b78)
        };
        t.muted = hex(window_bg);
        t.title_bar = hex(window_bg);
        t.title_bar_border = hex(window_bg);
        t.background = hex(card);
        t.secondary_hover = hex(hover);
        t.border = hex(border);
        t.foreground = hex(text);
        t.muted_foreground = hex(muted);
        t.button = hex(card);
        t.button_hover = hex(hover);
        t.button_active = hex(border);
        t.button_foreground = hex(text);
        t.popover = hex(card);
        t.popover_foreground = hex(text);
        if dark {
            t.success = hex(0x4ade80);
            t.switch = hex(border);
        }
    });

    // The native caption ignores the app theme unless told.
    if let Ok(handle) = HasWindowHandle::window_handle(window)
        && let RawWindowHandle::Win32(win32) = handle.as_raw()
    {
        mediakeys_core::set_dark_title_bar(win32.hwnd.get(), dark);
    }
}

struct MainView {
    core: Core,
    state: Entity<State>,
    dark: Option<bool>,
    custom_title_bar: bool,
    _subscriptions: Vec<Subscription>,
}

impl MainView {
    fn new(core: Core, state: Entity<State>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let current = state.read(cx).clone();
        apply_theme(window, cx, current.dark);
        let _subscriptions = vec![
            cx.observe_in(&state, window, |this, state, window, cx| {
                let next = state.read(cx).clone();
                if next.dark != this.dark {
                    this.dark = next.dark;
                    apply_theme(window, cx, next.dark);
                }
                if next.custom_title_bar != this.custom_title_bar {
                    // GPUI fixes the title bar mode at creation, so reopen in place.
                    this.custom_title_bar = next.custom_title_bar;
                    let bounds = window.bounds();
                    window.remove_window();
                    cx.defer(move |cx| open(cx, Some(bounds)));
                }
                cx.notify();
            }),
            cx.observe_window_appearance(window, |this, window, cx| apply_theme(window, cx, this.dark)),
        ];
        MainView { core, state, dark: current.dark, custom_title_bar: current.custom_title_bar, _subscriptions }
    }

    #[allow(clippy::too_many_arguments)]
    fn option(
        &self,
        id: Option<String>,
        name: String,
        avatar: Div,
        detail: AnyElement,
        checked: bool,
        running: bool,
        volume: Option<f32>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let element_id = SharedString::from(id.clone().unwrap_or_else(|| "automatic".into()));

        let radio = div()
            .flex_none()
            .size(px(18.))
            .rounded_full()
            .border_color(if checked { theme.primary } else { theme.muted_foreground })
            .map(|d| if checked { d.border(px(5.)) } else { d.border(px(1.5)) });

        let select = {
            let (core, id) = (self.core.clone(), id.clone());
            move |_: &ClickEvent, _: &mut Window, _: &mut App| core.select(id.clone())
        };

        let hover = theme.secondary_hover;
        h_flex()
            .id(element_id)
            .relative()
            .overflow_hidden()
            .w_full()
            .gap_3()
            .px_3()
            .py_2p5()
            .rounded(theme.radius_lg)
            .border_1()
            .cursor_pointer()
            .when(checked, |d| d.border_color(theme.primary).bg(theme.primary.opacity(0.13)))
            .when(!checked, |d| d.border_color(theme.border).bg(theme.background).hover(move |s| s.bg(hover)))
            .on_click(select)
            .child(avatar)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .when(!running, |d| d.opacity(0.6))
                    .child(div().font_semibold().child(name))
                    .child(detail),
            )
            .when_some(volume.zip(id), |d, (level, id)| {
                let core = self.core.clone();
                let icon = if level == 0.0 { IconName::VolumeX } else { IconName::Volume2 };
                d.on_scroll(move |event, _, cx| {
                    // Windows reports three lines per wheel notch by default.
                    let notches = match event.delta {
                        ScrollDelta::Lines(lines) => lines.y / 3.0,
                        ScrollDelta::Pixels(pixels) => f32::from(pixels.y) / 100.0,
                    };
                    core.adjust_volume(id.clone(), notches * VOLUME_STEP);
                    cx.stop_propagation();
                })
                .child(
                    h_flex()
                        .flex_none()
                        .min_w(px(46.))
                        .justify_end()
                        .gap_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(Icon::new(icon).xsmall())
                        .child(format!("{}%", (level * 100.0).round())),
                )
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .h(px(2.))
                        .w(relative(level))
                        .bg(theme.primary)
                        .opacity(0.55),
                )
            })
            .child(radio)
    }

    fn detail(target: &Target, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
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
}

fn avatar(theme: &Theme) -> Div {
    div().flex_none().size_8().rounded(theme.radius).flex().items_center().justify_center()
}

fn logo(size: Pixels) -> Div {
    div()
        .flex_none()
        .size(size)
        .rounded(size * 0.22)
        .flex()
        .items_center()
        .justify_center()
        .bg(linear_gradient(135., linear_color_stop(rgb(0x6366f1), 0.), linear_color_stop(rgb(0xa855f7), 1.)))
}

impl Render for MainView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx).clone();
        let theme = cx.theme().clone();
        let dark = theme.is_dark();
        let routed = state.routed().map(|t| t.name.clone());

        let status = match (&routed, &state.selected) {
            (Some(name), _) => format!("Captured · sending to {name}"),
            (None, Some(_)) => "Waiting for the chosen app · Windows decides meanwhile".to_owned(),
            (None, None) => "Windows decides".to_owned(),
        };

        let auto_avatar = logo(px(32.)).child(Icon::new(IconName::Sparkles).small().text_color(white()));
        let auto_detail = div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child("Windows picks the most recent player")
            .into_any_element();

        let mut options = vec![
            self.option(None, "Automatic".into(), auto_avatar, auto_detail, state.selected.is_none(), true, None, cx)
                .into_any_element(),
        ];
        for target in &state.targets {
            let initial = target.name.chars().next().unwrap_or('?').to_uppercase().to_string();
            let avatar =
                avatar(&theme).bg(avatar_color(&target.name)).text_color(white()).font_semibold().child(initial);
            let checked = state.selected.as_deref() == Some(target.id.as_str());
            let detail = Self::detail(target, cx);
            options.push(
                self.option(
                    Some(target.id.clone()),
                    target.name.clone(),
                    avatar,
                    detail,
                    checked,
                    target.running,
                    target.volume,
                    cx,
                )
                .into_any_element(),
            );
        }
        let hint = |text: &'static str| div().py_1().text_center().text_color(theme.muted_foreground).child(text);
        if state.targets.is_empty() {
            options.push(
                hint("No media apps found. Start playing something and it shows up here.")
                    .py_4()
                    .text_sm()
                    .into_any_element(),
            );
        } else if state.targets.iter().any(|t| t.volume.is_some()) {
            options.push(hint("Scroll over an app to change its volume").text_xs().into_any_element());
        }

        let press = |key: MediaKey, core: &Core| {
            let core = core.clone();
            move |_: &ClickEvent, _: &mut Window, _: &mut App| core.press(key)
        };
        let core = self.core.clone();
        let dot = if routed.is_some() { theme.primary } else { theme.muted_foreground };

        let tools = h_flex()
            .gap_0p5()
            .child({
                let core = self.core.clone();
                let custom = state.custom_title_bar;
                Button::new("titlebar")
                    .ghost()
                    .icon(Icon::new(IconName::AppWindow).when(custom, |i| i.text_color(theme.primary)))
                    .tooltip("Toggle in-app title bar")
                    .on_click(move |_, _, _| core.set_custom_title_bar(!custom))
            })
            .child({
                let core = self.core.clone();
                Button::new("theme")
                    .ghost()
                    .icon(if dark { IconName::Sun } else { IconName::Moon })
                    .tooltip("Toggle dark mode")
                    .on_click(move |_, _, _| core.set_dark(Some(!dark)))
            });

        v_flex()
            .size_full()
            .bg(theme.muted)
            .text_color(theme.foreground)
            .when(state.custom_title_bar, |d| {
                d.child(
                    TitleBar::new().child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(logo(px(16.)).child(Icon::new(IconName::Play).xsmall().text_color(white())))
                            .child("Media Keys Selector"),
                    ),
                )
            })
            .child(
                h_flex()
                    .items_start()
                    .justify_between()
                    .px_5()
                    .pt(if state.custom_title_bar { px(4.) } else { px(16.) })
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
                            .child(div().text_xl().font_semibold().child("Media keys go to")),
                    )
                    .child(tools),
            )
            .child(
                div()
                    .id("targets")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_5()
                    .py_1p5()
                    .child(v_flex().gap_1p5().children(options)),
            )
            .child(
                h_flex()
                    .justify_between()
                    .px_5()
                    .py_3()
                    .border_t_1()
                    .border_color(theme.border)
                    .when(dark, |d| d.bg(hex(0x0f0f14)))
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
                    .child(
                        Switch::new("autostart")
                            .checked(state.autostart)
                            .label("Start with Windows")
                            .on_click(move |on, _, _| core.set_autostart(*on)),
                    ),
            )
    }
}

/// Focuses the window, or opens it. Closing the window frees its GPU resources,
/// and the app stays alive in the tray.
fn show(cx: &mut App) {
    if let Some(handle) = cx.global::<Globals>().window
        && handle.update(cx, |_, window, _| window.activate_window()).is_ok()
    {
        return;
    }
    open(cx, None);
}

fn open(cx: &mut App, bounds: Option<Bounds<Pixels>>) {
    let globals = cx.global::<Globals>();
    let (core, state) = (globals.core.clone(), globals.state.clone());
    let custom = state.read(cx).custom_title_bar;

    // The title also names the taskbar and Alt+Tab entries when the caption is hidden.
    let title = Some("Media Keys Selector".into());
    let base = if custom {
        let options = TitleBar::window_options();
        WindowOptions { titlebar: Some(TitlebarOptions { title, ..TitleBar::title_bar_options() }), ..options }
    } else {
        WindowOptions { titlebar: Some(TitlebarOptions { title, ..Default::default() }), ..Default::default() }
    };
    let bounds = bounds.unwrap_or_else(|| Bounds::centered(None, size(px(400.), px(560.)), cx));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(340.), px(420.))),
        app_id: Some("MediaKeysSelector".into()),
        ..base
    };
    if let Ok((handle, _)) =
        gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| MainView::new(core, state, window, cx)))
    {
        cx.global_mut::<Globals>().window = Some(handle);
    }
}

fn main() {
    let (tx, mut rx) = futures::channel::mpsc::unbounded();
    let Some(core) = Core::start(move |event| {
        let _ = tx.unbounded_send(event);
    }) else {
        return;
    };
    let hidden = mediakeys_core::start_hidden();

    gpui_kit::application().with_assets(AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        cx.set_quit_mode(QuitMode::Explicit);
        let state = cx.new(|_| core.state());
        cx.set_global(Globals { core, state, window: None });
        if !hidden {
            show(cx);
        }

        cx.spawn(async move |cx| {
            while let Some(event) = rx.next().await {
                cx.update(|cx| match event {
                    Event::State(next) => {
                        let state = cx.global::<Globals>().state.clone();
                        state.update(cx, |state, cx| {
                            *state = next;
                            cx.notify();
                        });
                    }
                    Event::Show => show(cx),
                    Event::Quit => cx.quit(),
                });
            }
        })
        .detach();
    });
}

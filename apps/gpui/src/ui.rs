//! Pieces shared by the main window and the tray flyout.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::theme::Theme;
use gpui_kit::component::{ActiveTheme, Icon, Sizable, StyledExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use mediakeys_core::{AppIcon, Core, ImageKind};

/// Volume change per wheel notch.
pub const WHEEL_STEP: f32 = 0.02;

/// Slider id of the master volume.
pub const MASTER: &str = "master";

/// While the user drags a slider, incoming state must not move it back.
const DRAG_GRACE: Duration = Duration::from_millis(600);

const AVATAR_COLORS: [u32; 8] = [0x6d5ef5, 0x0ea5e9, 0xf97316, 0x10b981, 0xec4899, 0xeab308, 0xef4444, 0x14b8a6];

/// Same hash as the Tauri front end, so both pick the same color per app.
fn avatar_color(name: &str) -> Hsla {
    let hash = name.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32));
    rgb(AVATAR_COLORS[hash.unsigned_abs() as usize % AVATAR_COLORS.len()]).into()
}

pub fn image(icon: &AppIcon) -> Arc<Image> {
    let format = match icon.kind {
        ImageKind::Png => ImageFormat::Png,
        ImageKind::Jpeg => ImageFormat::Jpeg,
        ImageKind::Bmp => ImageFormat::Bmp,
        ImageKind::Gif => ImageFormat::Gif,
        ImageKind::Webp => ImageFormat::Webp,
    };
    // GPUI decodes each distinct image once and caches it by content hash.
    Arc::new(Image::from_bytes(format, icon.data.to_vec()))
}

pub fn logo(size: Pixels) -> Div {
    div()
        .flex_none()
        .size(size)
        .rounded(size * 0.22)
        .flex()
        .items_center()
        .justify_center()
        .bg(linear_gradient(135., linear_color_stop(rgb(0x6366f1), 0.), linear_color_stop(rgb(0xa855f7), 1.)))
}

/// The app's icon (or album art), else its first letter on a colored square.
pub fn avatar(icon: Option<&AppIcon>, name: &str, running: bool, size: Pixels, theme: &Theme) -> AnyElement {
    let frame = div().flex_none().size(size).rounded(theme.radius).overflow_hidden().flex().items_center().justify_center();
    match icon {
        Some(icon) => {
            // Album art fills the square, cropped; exe icons keep a little margin.
            let art = icon.kind != ImageKind::Png;
            let inner = if art { size } else { size * 0.9 };
            frame
                .child(
                    img(image(icon))
                        .size(inner)
                        .when(art, |i| i.object_fit(ObjectFit::Cover))
                        .when(!running, |i| i.grayscale(true).opacity(0.6)),
                )
                .into_any_element()
        }
        None => {
            let initial = name.chars().next().unwrap_or('?').to_uppercase().to_string();
            frame.bg(avatar_color(name)).text_color(white()).font_semibold().child(initial).into_any_element()
        }
    }
}

pub fn volume_icon(level: f32, muted: bool) -> IconName {
    if muted || level == 0.0 {
        IconName::VolumeX
    } else if level < 0.34 {
        IconName::Volume
    } else if level < 0.67 {
        IconName::Volume1
    } else {
        IconName::Volume2
    }
}

/// Volume sliders for one view, plus which app row has its slider revealed.
pub struct Volumes {
    core: Core,
    sliders: HashMap<String, Entity<SliderState>>,
    touched: Rc<RefCell<HashMap<String, Instant>>>,
    expanded: Rc<RefCell<Option<String>>>,
    view: AnyWeakEntity,
    subscriptions: Vec<Subscription>,
}

impl Volumes {
    pub fn new<V: 'static>(core: Core, cx: &mut Context<V>) -> Volumes {
        Volumes {
            core,
            sliders: HashMap::new(),
            touched: Rc::default(),
            expanded: Rc::default(),
            view: cx.entity().downgrade().into(),
            subscriptions: Vec::new(),
        }
    }

    pub fn expanded(&self) -> Option<String> {
        self.expanded.borrow().clone()
    }

    fn set_volume(core: &Core, id: &str, level: f32) {
        if id == MASTER { core.set_master(level) } else { core.set_volume(id, level) }
    }

    /// The slider for `id`, created on first use and kept in step with `level`
    /// unless the user moved it a moment ago.
    pub fn slider<V: 'static>(&mut self, id: &str, level: f32, window: &mut Window, cx: &mut Context<V>) -> Entity<SliderState> {
        let value = (level * 100.0).round();
        if let Some(slider) = self.sliders.get(id) {
            let touched = self.touched.borrow().get(id).is_some_and(|t| t.elapsed() < DRAG_GRACE);
            if !touched && slider.read(cx).value().end() != value {
                slider.update(cx, |s, cx| s.set_value(value, window, cx));
            }
            return slider.clone();
        }
        let slider = cx.new(|_| SliderState::new().min(0.).max(100.).step(1.).default_value(value));
        let (core, touched, key) = (self.core.clone(), self.touched.clone(), id.to_owned());
        self.subscriptions.push(cx.subscribe(&slider, move |_, _, event: &SliderEvent, _| {
            if let SliderEvent::Change(value) = event {
                touched.borrow_mut().insert(key.clone(), Instant::now());
                Self::set_volume(&core, &key, value.end() / 100.0);
            }
        }));
        self.sliders.insert(id.to_owned(), slider.clone());
        slider
    }

    /// Wheel handler that changes `id`'s volume and reveals its slider.
    pub fn on_wheel(&self, id: &str) -> impl Fn(&ScrollEvent, &mut Window, &mut App) + 'static + use<> {
        let (core, expanded, view, id) = (self.core.clone(), self.expanded.clone(), self.view.clone(), id.to_owned());
        move |event, _, cx| {
            // Windows reports three lines per wheel notch by default.
            let notches = match event.delta {
                ScrollDelta::Lines(lines) => lines.y / 3.0,
                ScrollDelta::Pixels(pixels) => f32::from(pixels.y) / 100.0,
            };
            if id == MASTER {
                core.adjust_master(notches * WHEEL_STEP);
            } else {
                core.adjust_volume(&id, notches * WHEEL_STEP);
                if expanded.borrow().as_deref() != Some(id.as_str()) {
                    *expanded.borrow_mut() = Some(id.clone());
                    notify(&view, cx);
                }
            }
            cx.stop_propagation();
        }
    }

    /// The "80%" pill on an app row. Clicking it reveals or hides the slider.
    pub fn pill(&self, id: &str, level: f32, muted: bool, theme: &Theme) -> Stateful<Div> {
        let (expanded, view, key) = (self.expanded.clone(), self.view.clone(), id.to_owned());
        let open = self.expanded.borrow().as_deref() == Some(id);
        h_flex()
            .id(SharedString::from(format!("pill-{id}")))
            .flex_none()
            .min_w(px(52.))
            .justify_end()
            .gap_1()
            .px_1p5()
            .py_0p5()
            .rounded(theme.radius)
            .text_xs()
            .text_color(if open { theme.primary } else { theme.muted_foreground })
            .hover(|s| s.bg(theme.secondary_hover))
            .child(Icon::new(volume_icon(level, muted)).xsmall())
            .child(if muted { "Muted".to_owned() } else { format!("{}%", (level * 100.0).round()) })
            .on_click(move |_, _, cx| {
                let mut current = expanded.borrow_mut();
                *current = if current.as_deref() == Some(key.as_str()) { None } else { Some(key.clone()) };
                drop(current);
                notify(&view, cx);
                cx.stop_propagation();
            })
    }

    /// Mute button, slider and value, for the master volume or a revealed app row.
    pub fn slider_row<V: 'static>(
        &mut self,
        id: &str,
        level: f32,
        muted: bool,
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> Div {
        let slider = self.slider(id, level, window, cx);
        let theme = cx.theme().clone();
        let (core, key) = (self.core.clone(), id.to_owned());
        h_flex()
            .w_full()
            .gap_2()
            .on_scroll(self.on_wheel(id))
            .child(
                div()
                    .id(SharedString::from(format!("mute-{id}")))
                    .flex_none()
                    .p_1()
                    .rounded(theme.radius)
                    .cursor_pointer()
                    .text_color(if muted { theme.danger } else { theme.foreground })
                    .hover(|s| s.bg(theme.secondary_hover))
                    .child(Icon::new(volume_icon(level, muted)).small())
                    .on_click(move |_, _, cx| {
                        if key == MASTER { core.toggle_master_mute() } else { core.toggle_mute(&key) }
                        cx.stop_propagation();
                    }),
            )
            .child(div().flex_1().child(Slider::new(&slider)))
            .child(
                div()
                    .flex_none()
                    .w(px(30.))
                    .text_right()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(format!("{}", (level * 100.0).round())),
            )
    }
}

fn notify(view: &AnyWeakEntity, cx: &mut App) {
    if let Some(view) = view.upgrade() {
        cx.notify(view.entity_id());
    }
}

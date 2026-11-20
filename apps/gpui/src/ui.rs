//! Pieces shared by the main window and the tray flyout.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use audio_bartender_core::{AppIcon, Core, ImageKind};
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::theme::Theme;
use gpui_kit::component::{Icon, Sizable, StyledExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

/// Volume change per wheel notch.
pub const WHEEL_STEP: f32 = 0.02;

/// Slider id of the master volume.
pub const MASTER: &str = "master";

/// Width of the compact volume bar.
pub const BAR_WIDTH: f32 = 84.;

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

pub fn logo(size: Pixels, [from, to]: [u32; 2]) -> Div {
    div().flex_none().size(size).rounded(size * 0.22).flex().items_center().justify_center().bg(linear_gradient(
        135.,
        linear_color_stop(rgb(from), 0.),
        linear_color_stop(rgb(to), 1.),
    ))
}

/// The app's icon (or album art), else its first letter on a colored square.
pub fn avatar(icon: Option<&AppIcon>, name: &str, running: bool, size: Pixels, theme: &Theme) -> AnyElement {
    let frame =
        div().flex_none().size(size).rounded(theme.radius).overflow_hidden().flex().items_center().justify_center();
    match icon {
        Some(icon) => {
            // Album art fills the square, cropped; exe icons keep a little margin.
            let art = icon.art;
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

/// Volume controls for one view: sliders, compact bars and percentages.
pub struct Volumes {
    core: Core,
    /// Levels the user set a moment ago, shown until the audio state catches up.
    local: Rc<RefCell<HashMap<String, (f32, Instant)>>>,
    /// Where each volume control was drawn, so clicks on them don't reach the row.
    controls: Rc<RefCell<HashMap<String, Bounds<Pixels>>>>,
    view: AnyWeakEntity,
}

impl Volumes {
    pub fn new<V: 'static>(core: Core, cx: &mut Context<V>) -> Volumes {
        Volumes { core, local: Rc::default(), controls: Rc::default(), view: cx.entity().downgrade().into() }
    }

    /// True while the user is changing `id`'s volume, so its slider stays put.
    pub fn changing(&self, id: &str) -> bool {
        self.local.borrow().get(id).is_some_and(|(_, at)| at.elapsed() < DRAG_GRACE)
    }

    /// Forgets where controls were drawn. Call at the start of each render.
    pub fn begin_frame(&self) {
        self.controls.borrow_mut().clear();
    }

    /// Tells whether a window position is on a volume control.
    pub fn hit_test(&self) -> impl Fn(Point<Pixels>) -> bool + 'static + use<> {
        let controls = self.controls.clone();
        move |position| controls.borrow().values().any(|bounds| bounds.contains(&position))
    }

    /// Sets `id`'s volume and shows the new level right away.
    fn set_level(core: &Core, local: &RefCell<HashMap<String, (f32, Instant)>>, id: &str, level: f32) {
        local.borrow_mut().insert(id.to_owned(), (level, Instant::now()));
        if id == MASTER { core.set_master(level) } else { core.set_volume(id, level) }
    }

    /// `level`, or the user's own value while the audio state catches up with it.
    fn shown(&self, id: &str, level: f32) -> f32 {
        match self.local.borrow().get(id) {
            Some((local, at)) if at.elapsed() < DRAG_GRACE => *local,
            _ => level,
        }
    }

    /// Fills its parent and records the parent's bounds as a volume control.
    fn track(&self, key: String) -> impl IntoElement + use<> {
        let controls = self.controls.clone();
        canvas(move |bounds, _, _| _ = controls.borrow_mut().insert(key, bounds), |_, _, _, _| {})
            .absolute()
            .top_0()
            .left_0()
            .size_full()
    }

    /// Wheel handler that changes `id`'s volume. The wheel scrolls the list
    /// everywhere else, so only volume controls get this.
    pub fn on_wheel(&self, id: &str) -> impl Fn(&ScrollEvent, &mut Window, &mut App) + 'static + use<> {
        let (core, id) = (self.core.clone(), id.to_owned());
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
            }
            cx.stop_propagation();
        }
    }

    /// The "80%" on an app row, which turns into a slider while the row is hovered.
    /// `meter` washes it with the app's live level.
    pub fn pill(&self, id: &str, level: f32, muted: bool, meter: Option<f32>, theme: &Theme) -> Div {
        h_flex()
            .relative()
            .flex_none()
            .w(px(BAR_WIDTH))
            .h(px(22.))
            .overflow_hidden()
            .justify_center()
            .gap_1()
            .rounded(theme.radius)
            .text_xs()
            .text_color(theme.muted_foreground)
            .when_some(meter, |d, meter| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(relative(meter.clamp(0.0, 1.0)))
                        .bg(theme.primary.opacity(0.18)),
                )
            })
            .child(Icon::new(volume_icon(level, muted)).xsmall())
            .child(if muted { "Muted".to_owned() } else { format!("{}%", (level * 100.0).round()) })
            .on_scroll(self.on_wheel(id))
    }

    /// A slider the size of the pill, for a hovered row.
    pub fn hover_slider(&self, id: &str, level: f32, meter: Option<f32>, theme: &Theme) -> Div {
        h_flex().flex_none().w(px(BAR_WIDTH)).h(px(22.)).child(self.slider(id, level, meter, theme))
    }

    /// A small bar that fills with the volume. Dragging sets it, the wheel nudges
    /// it, and a click without dragging toggles mute.
    /// `meter` draws the app's live level over the volume fill.
    pub fn bar(&self, id: &str, level: f32, muted: bool, meter: Option<f32>, theme: &Theme) -> Stateful<Div> {
        let level = self.shown(id, level);
        let fill = if muted { theme.muted_foreground.opacity(0.25) } else { theme.primary.opacity(0.3) };
        let key = format!("bar-{id}");
        let (core, local, drag_key, target) = (self.core.clone(), self.local.clone(), key.clone(), id.to_owned());
        let (mute_core, mute_id) = (self.core.clone(), id.to_owned());
        let bar_view = self.view.clone();
        h_flex()
            .id(SharedString::from(key.clone()))
            .relative()
            .flex_none()
            .w(px(BAR_WIDTH))
            .h(px(22.))
            .overflow_hidden()
            .rounded(theme.radius)
            .bg(theme.secondary)
            .border_1()
            .border_color(theme.border)
            .cursor_ew_resize()
            .hover(|s| s.border_color(theme.primary.opacity(0.6)))
            .child(div().absolute().top_0().bottom_0().left_0().w(relative(level.clamp(0.0, 1.0))).bg(fill))
            .when_some(meter, |d, meter| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(relative(meter.clamp(0.0, 1.0)))
                        .bg(theme.primary.opacity(0.55)),
                )
            })
            .child(
                h_flex()
                    .relative()
                    .size_full()
                    .justify_center()
                    .gap_1()
                    .text_xs()
                    .text_color(if muted { theme.danger } else { theme.foreground })
                    .child(Icon::new(volume_icon(level, muted)).xsmall())
                    .child(if muted { "Muted".to_owned() } else { format!("{}%", (level * 100.0).round()) }),
            )
            .child(self.track(key.clone()))
            .on_scroll(self.on_wheel(id))
            .on_click(move |_, _, cx| {
                mute_core.toggle_mute(&mute_id);
                cx.stop_propagation();
            })
            .on_drag(DragVolume(key), |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            })
            .on_drag_move(move |event: &DragMoveEvent<DragVolume>, _, cx| {
                if event.drag(cx).0 != drag_key {
                    return;
                }
                let bounds = event.bounds;
                let x = f32::from(event.event.position.x - bounds.left()) / f32::from(bounds.size.width);
                Self::set_level(&core, &local, &target, (x.clamp(0.0, 1.0) * 50.0).round() / 50.0);
                notify(&bar_view, cx);
            })
    }

    /// A thin track with a round thumb, styled like the Tauri range input. Click
    /// or drag to set the volume, or use the wheel.
    /// With `meter`, the volume fill dims and the live level fills it in full color.
    pub fn slider(&self, id: &str, level: f32, meter: Option<f32>, theme: &Theme) -> Stateful<Div> {
        let level = self.shown(id, level).clamp(0.0, 1.0);
        let track = hex_color(if theme.is_dark() { 0x33333f } else { 0xdcdce4 });
        let key = format!("slider-{id}");
        let at = |position: Point<Pixels>, bounds: Bounds<Pixels>| {
            let inner = f32::from(bounds.size.width) - 2.0 * THUMB_INSET;
            let x = f32::from(position.x - bounds.left()) - THUMB_INSET;
            (x / inner).clamp(0.0, 1.0)
        };
        let (down_core, down_local, controls, down_key, down_id) =
            (self.core.clone(), self.local.clone(), self.controls.clone(), key.clone(), id.to_owned());
        let (core, local, drag_key, target) = (self.core.clone(), self.local.clone(), key.clone(), id.to_owned());
        let view = self.view.clone();
        let down_view = self.view.clone();
        div()
            .id(SharedString::from(key.clone()))
            .relative()
            .flex_1()
            .h(px(20.))
            .cursor_pointer()
            .child(self.track(key.clone()))
            .child(
                div()
                    .absolute()
                    .left(px(THUMB_INSET))
                    .right(px(THUMB_INSET))
                    .top(px(8.))
                    .h(px(4.))
                    .rounded_full()
                    .bg(track)
                    .child(div().h_full().w(relative(level)).rounded_full().bg(if meter.is_some() {
                        theme.primary.opacity(0.35)
                    } else {
                        theme.primary
                    }))
                    .when_some(meter, |d, meter| {
                        d.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .h_full()
                                .w(relative(meter.clamp(0.0, level)))
                                .rounded_full()
                                .bg(theme.primary),
                        )
                    })
                    .child(
                        // An accent ring, a gap in the card color, then the accent dot.
                        div()
                            .absolute()
                            .top(px(-7.))
                            .left(relative(level))
                            .ml(px(-9.))
                            .size(px(18.))
                            .p(px(1.))
                            .rounded_full()
                            .bg(theme.primary)
                            .child(
                                div()
                                    .size_full()
                                    .rounded_full()
                                    .border(px(3.))
                                    .border_color(theme.background)
                                    .bg(theme.primary),
                            ),
                    ),
            )
            .on_scroll(self.on_wheel(id))
            .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                let Some(bounds) = controls.borrow().get(&down_key).copied() else { return };
                Self::set_level(
                    &down_core,
                    &down_local,
                    &down_id,
                    (at(event.position, bounds) * 100.0).round() / 100.0,
                );
                notify(&down_view, cx);
            })
            .on_click(|_, _, cx| cx.stop_propagation())
            .on_drag(DragVolume(key), |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            })
            .on_drag_move(move |event: &DragMoveEvent<DragVolume>, _, cx| {
                if event.drag(cx).0 != drag_key {
                    return;
                }
                Self::set_level(
                    &core,
                    &local,
                    &target,
                    (at(event.event.position, event.bounds) * 100.0).round() / 100.0,
                );
                notify(&view, cx);
            })
    }

    /// Mute button, slider and value, for the master volume or an app.
    pub fn slider_row(&self, id: &str, level: f32, muted: bool, meter: Option<f32>, theme: &Theme) -> Div {
        let (core, key) = (self.core.clone(), id.to_owned());
        let shown = self.shown(id, level);
        h_flex()
            .w_full()
            .gap_2()
            .child(
                div()
                    .id(SharedString::from(format!("mute-{id}")))
                    .flex_none()
                    .p_1()
                    .rounded(theme.radius)
                    .cursor_pointer()
                    .text_color(if muted { theme.danger } else { theme.foreground })
                    .hover(|s| s.bg(theme.secondary_hover))
                    .child(Icon::new(volume_icon(shown, muted)).small())
                    .on_scroll(self.on_wheel(id))
                    .on_click(move |_, _, cx| {
                        if key == MASTER {
                            core.toggle_master_mute()
                        } else {
                            core.toggle_mute(&key)
                        }
                        cx.stop_propagation();
                    }),
            )
            .child(self.slider(id, level, meter, theme))
            .child(
                div()
                    .flex_none()
                    .w(px(30.))
                    .text_right()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .on_scroll(self.on_wheel(id))
                    .child(format!("{}", (shown * 100.0).round())),
            )
    }
}

/// Space at each end of a slider track, so the thumb stays inside the control.
const THUMB_INSET: f32 = 9.;

fn hex_color(color: u32) -> Hsla {
    rgb(color).into()
}

/// A volume control being dragged, by its element key.
#[derive(Clone)]
struct DragVolume(String);

impl Render for DragVolume {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

fn notify(view: &AnyWeakEntity, cx: &mut App) {
    if let Some(view) = view.upgrade() {
        cx.notify(view.entity_id());
    }
}

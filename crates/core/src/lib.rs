//! Media key routing and volume control shared by the GUI front ends.
//!
//! The core owns:
//! - a low-level keyboard hook that swallows media keys while a target is chosen,
//! - a low-level mouse hook that turns the wheel over the tray icon into master volume,
//! - a worker that tracks media sessions, audio sessions and output devices,
//! - the tray icon, drawn as a live volume indicator, and its right-click menu.
//!
//! A front end calls [`Core::start`], renders [`State`] snapshots from [`Event::State`],
//! shows its flyout on [`Event::Flyout`] and exits on [`Event::Quit`]. The flyout is
//! the whole UI.

mod audio;
mod autostart;
mod card;
mod config;
mod icon;
mod input;
mod media;
mod meters;
mod names;
mod route;
mod spectrum;
mod tooltip;
mod tray_icon;
mod update;
mod util;
mod volume;

/// Appends a line to `%TEMP%\audio_bartender.log` when `AUDIO_BARTENDER_LOG` is set.
pub use update::{Flavor, UpdateState, UpdateStatus, set_flavor};
pub use util::log;

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "camelCase"))]
pub enum MediaKey {
    PlayPause,
    Next,
    Previous,
    Stop,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub enum Playback {
    #[default]
    Unknown,
    Playing,
    Paused,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Bmp,
    Gif,
    Webp,
}

impl ImageKind {
    pub fn mime(self) -> &'static str {
        match self {
            ImageKind::Png => "image/png",
            ImageKind::Jpeg => "image/jpeg",
            ImageKind::Bmp => "image/bmp",
            ImageKind::Gif => "image/gif",
            ImageKind::Webp => "image/webp",
        }
    }
}

/// An encoded image: an app icon or album art. Serializes as `{ url, art }` with a
/// `data:` URL.
#[derive(Clone, PartialEq)]
pub struct AppIcon {
    pub data: Arc<[u8]>,
    pub kind: ImageKind,
    /// Album art, which fills a square cropped, unlike an app icon.
    pub art: bool,
}

impl std::fmt::Debug for AppIcon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AppIcon({:?}, {} bytes)", self.kind, self.data.len())
    }
}

#[cfg(feature = "serde")]
impl Serialize for AppIcon {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("AppIcon", 2)?;
        s.serialize_field("url", &format!("data:{};base64,{}", self.kind.mime(), icon::base64(&self.data)))?;
        s.serialize_field("art", &self.art)?;
        s.end()
    }
}

/// One app that can receive media keys.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct Target {
    /// `smtc:<AppUserModelID>` or `exe:<file name>`.
    pub id: String,
    pub name: String,
    pub playback: Playback,
    /// "Title — Artist" of the current track, when the app reports one.
    pub track: Option<String>,
    /// False for a remembered target whose app is closed.
    pub running: bool,
    /// True when keys reach the app through window messages instead of SMTC.
    pub via_window: bool,
    /// App volume from 0 to 1, when the app has an audio session.
    pub volume: Option<f32>,
    pub muted: bool,
    /// Album art when [`Settings::album_art`] is on and the app has some, else the exe icon.
    pub icon: Option<AppIcon>,
    /// The output device this app is sent to, when not the default.
    pub route: Option<String>,
}

/// An app playing audio that isn't a media key target, as in the Volume Mixer.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct MixerApp {
    /// `exe:<file name>`, accepted wherever a target id is.
    pub id: String,
    pub name: String,
    pub volume: f32,
    pub muted: bool,
    pub icon: Option<AppIcon>,
    /// The output device this app is sent to, when not the default.
    pub route: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct Device {
    pub id: String,
    pub name: String,
    /// The name without its adapter, such as "Headphones" for "Headphones (WH-1000XM5)".
    pub short: String,
    pub kind: DeviceKind,
    pub default: bool,
}

/// What kind of output a device is, for its icon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub enum DeviceKind {
    #[default]
    Speakers,
    /// Headphones and headsets.
    Headphones,
    /// A TV or monitor over HDMI or DisplayPort.
    Display,
}

/// How an app row shows that the app plays on another device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "camelCase"))]
pub enum DeviceChip {
    /// Nothing on the row; the device list still shows every route.
    None,
    /// A chip with the device's name beside the app name.
    #[default]
    Beside,
    /// The device's icon in place of the speaker in the row's volume control.
    Volume,
    /// The device's icon after the app name.
    Icon,
}

impl DeviceChip {
    pub const ALL: [DeviceChip; 4] = [DeviceChip::None, DeviceChip::Beside, DeviceChip::Volume, DeviceChip::Icon];

    /// The config file name.
    pub fn name(self) -> &'static str {
        match self {
            DeviceChip::None => "none",
            DeviceChip::Beside => "beside",
            DeviceChip::Volume => "volume",
            DeviceChip::Icon => "icon",
        }
    }

    pub fn from_name(name: &str) -> Option<DeviceChip> {
        DeviceChip::ALL.into_iter().find(|c| c.name() == name)
    }
}

/// The default output device's state plus the other devices.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct Audio {
    pub devices: Vec<Device>,
    /// Master volume from 0 to 1.
    pub volume: f32,
    pub muted: bool,
}

impl Audio {
    pub fn device(&self) -> Option<&Device> {
        self.devices.iter().find(|d| d.default)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "camelCase"))]
pub enum TrayStyle {
    /// A 270° dial that fills with the volume, around the middle.
    #[default]
    Dial,
    /// A ring that fills with the volume, around the middle.
    Ring,
    /// Only the middle.
    Plain,
    /// Windows' own volume icon: no waves at 0, then one to three.
    Native,
    /// Waves set by the volume, rippling outward with the sound playing.
    LiveWaves,
    /// Four rising bars, filled in turn.
    SignalSteps,
    /// Bars bouncing with the sound, scaled by the volume.
    Equalizer,
}

impl TrayStyle {
    /// In the order the settings show them: the styles with a middle, then the rest.
    pub const ALL: [TrayStyle; 7] = [
        TrayStyle::Dial,
        TrayStyle::Ring,
        TrayStyle::Plain,
        TrayStyle::Native,
        TrayStyle::LiveWaves,
        TrayStyle::SignalSteps,
        TrayStyle::Equalizer,
    ];

    /// The config file name.
    pub fn name(self) -> &'static str {
        match self {
            TrayStyle::Ring => "ring",
            TrayStyle::Dial => "dial",
            TrayStyle::Plain => "plain",
            TrayStyle::Native => "native",
            TrayStyle::LiveWaves => "live_waves",
            TrayStyle::SignalSteps => "signal_steps",
            TrayStyle::Equalizer => "equalizer",
        }
    }

    pub fn from_name(name: &str) -> Option<TrayStyle> {
        TrayStyle::ALL.into_iter().find(|s| s.name() == name)
    }

    /// The name the settings show.
    pub fn label(self) -> &'static str {
        match self {
            TrayStyle::Ring => "Ring",
            TrayStyle::Dial => "Dial",
            TrayStyle::Plain => "Plain",
            TrayStyle::Native => "Native",
            TrayStyle::LiveWaves => "Live waves",
            TrayStyle::SignalSteps => "Steps",
            TrayStyle::Equalizer => "Equalizer",
        }
    }

    /// What the settings say about it.
    pub fn about(self) -> &'static str {
        match self {
            TrayStyle::Ring => "A ring that fills with the volume",
            TrayStyle::Dial => "A dial that fills with the volume",
            TrayStyle::Plain => "Only the middle, without the volume",
            TrayStyle::Native => "Windows' own volume icon",
            TrayStyle::LiveWaves => "Waves that ripple with the sound playing",
            TrayStyle::SignalSteps => "Four bars that fill as the volume climbs",
            TrayStyle::Equalizer => "Bars that bounce with the sound",
        }
    }

    /// Whether it draws a middle, which [`TrayMiddle`] picks.
    pub fn has_middle(self) -> bool {
        matches!(self, TrayStyle::Ring | TrayStyle::Dial | TrayStyle::Plain)
    }
}

/// What a ring, a dial or a plain tray icon shows in its middle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "camelCase"))]
pub enum TrayMiddle {
    #[default]
    Speaker,
    /// Play or pause, for what the media keys control.
    PlayState,
    /// The cover of what's playing, in color.
    AlbumArt,
    /// The volume as a number.
    Number,
    /// A dot that swells with the sound playing.
    Pulse,
}

impl TrayMiddle {
    pub const ALL: [TrayMiddle; 5] =
        [TrayMiddle::Speaker, TrayMiddle::PlayState, TrayMiddle::AlbumArt, TrayMiddle::Number, TrayMiddle::Pulse];

    /// The config file name.
    pub fn name(self) -> &'static str {
        match self {
            TrayMiddle::Speaker => "speaker",
            TrayMiddle::PlayState => "play_state",
            TrayMiddle::AlbumArt => "album_art",
            TrayMiddle::Number => "number",
            TrayMiddle::Pulse => "pulse",
        }
    }

    pub fn from_name(name: &str) -> Option<TrayMiddle> {
        TrayMiddle::ALL.into_iter().find(|m| m.name() == name)
    }

    /// The name the settings show.
    pub fn label(self) -> &'static str {
        match self {
            TrayMiddle::Speaker => "Speaker",
            TrayMiddle::PlayState => "Play",
            TrayMiddle::AlbumArt => "Art",
            TrayMiddle::Number => "Number",
            TrayMiddle::Pulse => "Pulse",
        }
    }
}

/// How the tray icon's pulse middle moves with the sound.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "camelCase"))]
pub enum PulseStyle {
    /// A dot that swells with the bass, its edge bulging with the spectrum.
    #[default]
    Dot,
    /// Spokes that stretch, each with its own band.
    Rays,
    /// A circle of dots, each swelling with its own band.
    Orbit,
    /// Bars all around a circle, like a music video's visualizer.
    Spectrum,
}

impl PulseStyle {
    pub const ALL: [PulseStyle; 4] = [PulseStyle::Dot, PulseStyle::Rays, PulseStyle::Orbit, PulseStyle::Spectrum];

    /// The config file name.
    pub fn name(self) -> &'static str {
        match self {
            PulseStyle::Dot => "dot",
            PulseStyle::Rays => "rays",
            PulseStyle::Orbit => "orbit",
            PulseStyle::Spectrum => "spectrum",
        }
    }

    pub fn from_name(name: &str) -> Option<PulseStyle> {
        PulseStyle::ALL.into_iter().find(|p| p.name() == name)
    }

    /// The name the settings show.
    pub fn label(self) -> &'static str {
        match self {
            PulseStyle::Dot => "Dot",
            PulseStyle::Rays => "Rays",
            PulseStyle::Orbit => "Orbit",
            PulseStyle::Spectrum => "Spectrum",
        }
    }
}

/// Whether the tray icon moves with the sound playing.
pub fn tray_icon_live(style: TrayStyle, middle: TrayMiddle) -> bool {
    matches!(style, TrayStyle::LiveWaves | TrayStyle::Equalizer) || (style.has_middle() && middle == TrayMiddle::Pulse)
}

/// How app rows show their volume.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "camelCase"))]
pub enum VolumeBars {
    /// No volume on app rows.
    None,
    /// A small bar that fills with the volume. Drag or scroll it; click to mute.
    #[default]
    Compact,
    /// A slider under every app, always visible.
    Always,
    /// A percentage that turns into a slider while the pointer is on the row.
    OnHover,
}

/// The app's accent color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Deserialize), serde(rename_all = "camelCase"))]
pub enum Accent {
    Red,
    Orange,
    Amber,
    Lime,
    Green,
    Teal,
    #[default]
    Cyan,
    Blue,
    Indigo,
    Purple,
    Fuchsia,
    Pink,
    /// Muted: a blue grey.
    Slate,
    /// Muted: a neutral grey, near black in light themes.
    Graphite,
}

/// One accent's colors for a light or dark theme, as `0xRRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct AccentColors {
    pub accent: u32,
    /// A lighter shade for hovered buttons.
    pub hover: u32,
    /// The logo's gradient, from top left to bottom right.
    pub gradient: [u32; 2],
}

impl Accent {
    /// In swatch order: the hues around the wheel, then the muted ones.
    pub const ALL: [Accent; 14] = [
        Accent::Red,
        Accent::Orange,
        Accent::Amber,
        Accent::Lime,
        Accent::Green,
        Accent::Teal,
        Accent::Cyan,
        Accent::Blue,
        Accent::Indigo,
        Accent::Purple,
        Accent::Fuchsia,
        Accent::Pink,
        Accent::Slate,
        Accent::Graphite,
    ];

    /// The config file and front end name.
    pub fn name(self) -> &'static str {
        match self {
            Accent::Red => "red",
            Accent::Orange => "orange",
            Accent::Amber => "amber",
            Accent::Lime => "lime",
            Accent::Green => "green",
            Accent::Teal => "teal",
            Accent::Cyan => "cyan",
            Accent::Blue => "blue",
            Accent::Indigo => "indigo",
            Accent::Purple => "purple",
            Accent::Fuchsia => "fuchsia",
            Accent::Pink => "pink",
            Accent::Slate => "slate",
            Accent::Graphite => "graphite",
        }
    }

    pub fn from_name(name: &str) -> Option<Accent> {
        Accent::ALL.into_iter().find(|a| a.name() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            Accent::Red => "Red",
            Accent::Orange => "Orange",
            Accent::Amber => "Amber",
            Accent::Lime => "Lime",
            Accent::Green => "Green",
            Accent::Teal => "Teal",
            Accent::Cyan => "Cyan",
            Accent::Blue => "Blue",
            Accent::Indigo => "Indigo",
            Accent::Purple => "Purple",
            Accent::Fuchsia => "Fuchsia",
            Accent::Pink => "Hot pink",
            Accent::Slate => "Slate",
            Accent::Graphite => "Graphite",
        }
    }

    /// Light themes use a deeper shade, so white text on it stays readable. Dark
    /// themes use one bright enough to stand out, still dark enough for white text.
    pub fn colors(self, dark: bool) -> AccentColors {
        let (light, dark_accent, gradient) = match self {
            Accent::Red => (0xdc2626, 0xef4444, [0xf87171, 0xf97316]),
            Accent::Orange => (0xea580c, 0xf97316, [0xfb923c, 0xec4899]),
            Accent::Amber => (0xb45309, 0xd97706, [0xfbbf24, 0xf97316]),
            Accent::Lime => (0x4d7c0f, 0x65a30d, [0xa3e635, 0x22c55e]),
            Accent::Green => (0x16a34a, 0x22c55e, [0x4ade80, 0x14b8a6]),
            Accent::Teal => (0x0d9488, 0x14b8a6, [0x2dd4bf, 0x0ea5e9]),
            Accent::Cyan => (0x0891b2, 0x06b6d4, [0x22d3ee, 0x3b82f6]),
            Accent::Blue => (0x2563eb, 0x3b82f6, [0x3b82f6, 0x06b6d4]),
            Accent::Indigo => (0x4f46e5, 0x6366f1, [0x818cf8, 0x3b82f6]),
            Accent::Purple => (0x6d5ef5, 0x9d8cff, [0x6366f1, 0xa855f7]),
            Accent::Fuchsia => (0xc026d3, 0xd946ef, [0xe879f9, 0x8b5cf6]),
            Accent::Pink => (0xdb2777, 0xf0469c, [0xf472b6, 0xa855f7]),
            Accent::Slate => (0x475569, 0x64748b, [0x64748b, 0x334155]),
            Accent::Graphite => (0x27272a, 0x71717a, [0x52525b, 0x18181b]),
        };
        let accent = if dark { dark_accent } else { light };
        AccentColors { accent, hover: mix(accent, 0xffffff, 0.15), gradient }
    }
}

/// `a` moved `t` of the way towards `b`, per channel.
pub(crate) fn mix(a: u32, b: u32, t: f32) -> u32 {
    let channel = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xff) as f32, ((b >> shift) & 0xff) as f32);
        ((x + (y - x) * t).round() as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

/// Front ends get the name and both themes' colors, so the palette lives in one place.
#[cfg(feature = "serde")]
impl Serialize for Accent {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("Accent", 4)?;
        s.serialize_field("name", self.name())?;
        s.serialize_field("label", self.label())?;
        s.serialize_field("light", &self.colors(false))?;
        s.serialize_field("dark", &self.colors(true))?;
        s.end()
    }
}

/// What the tray icon's tooltip looks like.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "camelCase"))]
pub enum TooltipStyle {
    /// A card with the media key app's track and art, the device and the volume.
    #[default]
    NowPlaying,
    /// A small card with the device, the volume and the media key app.
    Compact,
    /// The plain Windows tooltip.
    Text,
}

/// How app rows show which apps are making sound.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "camelCase"))]
pub enum LevelMeters {
    None,
    /// Inside the row's volume control.
    #[default]
    Volume,
    /// A ring around the app icon.
    Ring,
    /// Equalizer bars before the app name.
    Bars,
    /// A line along the bottom of the row.
    Line,
    /// A strip at the row's left edge.
    Strip,
}

impl LevelMeters {
    pub const ALL: [LevelMeters; 6] = [
        LevelMeters::None,
        LevelMeters::Volume,
        LevelMeters::Ring,
        LevelMeters::Bars,
        LevelMeters::Line,
        LevelMeters::Strip,
    ];

    /// The config file name.
    pub fn name(self) -> &'static str {
        match self {
            LevelMeters::None => "none",
            LevelMeters::Volume => "volume",
            LevelMeters::Ring => "ring",
            LevelMeters::Bars => "bars",
            LevelMeters::Line => "line",
            LevelMeters::Strip => "strip",
        }
    }

    pub fn from_name(name: &str) -> Option<LevelMeters> {
        LevelMeters::ALL.into_iter().find(|m| m.name() == name)
    }
}

/// Where the flyout puts previous, play/pause and next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "camelCase"))]
pub enum MediaControls {
    None,
    /// A now playing card at the top.
    Banner,
    /// Under the selected app's row.
    Row,
    /// A now playing line above the device volume.
    Dock,
    /// On each app's row while hovered, controlling that app.
    #[default]
    Hover,
}

impl MediaControls {
    pub const ALL: [MediaControls; 5] =
        [MediaControls::None, MediaControls::Banner, MediaControls::Row, MediaControls::Dock, MediaControls::Hover];

    /// The config file and front end name.
    pub fn name(self) -> &'static str {
        match self {
            MediaControls::None => "none",
            MediaControls::Banner => "banner",
            MediaControls::Row => "row",
            MediaControls::Dock => "dock",
            MediaControls::Hover => "hover",
        }
    }

    pub fn from_name(name: &str) -> Option<MediaControls> {
        MediaControls::ALL.into_iter().find(|c| c.name() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            MediaControls::None => "None",
            MediaControls::Banner => "Banner",
            MediaControls::Row => "Row",
            MediaControls::Dock => "Dock",
            MediaControls::Hover => "Hover",
        }
    }
}

/// Media buttons on the tooltip cards. Each choice has a look on both cards.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "camelCase"))]
pub enum MediaButtons {
    None,
    /// Shown on the art or the app line while the pointer is on the card.
    #[default]
    Hover,
    /// Small buttons at the end of the status line.
    Inline,
    /// A capsule on the line between the track and the volume.
    Capsule,
    /// A row under the track, or a dock beside the compact card.
    Player,
}

impl MediaButtons {
    /// The config file name.
    pub fn name(self) -> &'static str {
        match self {
            MediaButtons::None => "none",
            MediaButtons::Hover => "hover",
            MediaButtons::Inline => "inline",
            MediaButtons::Capsule => "capsule",
            MediaButtons::Player => "player",
        }
    }

    pub fn from_name(name: &str) -> Option<MediaButtons> {
        [MediaButtons::None, MediaButtons::Hover, MediaButtons::Inline, MediaButtons::Capsule, MediaButtons::Player]
            .into_iter()
            .find(|b| b.name() == name)
    }
}

/// Preferences shared by both front ends, stored in `config.ini`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct Settings {
    pub autostart: bool,
    /// Install new releases from GitHub, checked once a day. On by default for installed
    /// copies and off for portable ones.
    pub auto_update: bool,
    /// Forced dark (`true`) or light (`false`) theme. `None` follows Windows.
    pub dark: Option<bool>,
    /// Show the current track's album art in place of the app icon.
    pub album_art: bool,
    /// Keep the chosen app in the list while it's closed. Off, closing it goes back to Automatic.
    pub keep_closed_target: bool,
    pub tray_style: TrayStyle,
    pub tray_middle: TrayMiddle,
    pub pulse_style: PulseStyle,
    /// Draw our icon over the Windows 11 Quick Settings speaker.
    pub speaker_position: bool,
    pub volume_bars: VolumeBars,
    pub tooltip: TooltipStyle,
    pub media_buttons: MediaButtons,
    pub media_controls: MediaControls,
    pub level_meters: LevelMeters,
    pub device_chip: DeviceChip,
    /// Parts of the flyout to show. The now playing card follows `media_controls`.
    pub show_media_keys: bool,
    pub show_other_apps: bool,
    /// The output device and the master volume.
    pub show_volume: bool,
    pub accent: Accent,
}

impl Settings {
    /// True when the flyout would show nothing but its header.
    pub fn shows_nothing(&self) -> bool {
        let card = matches!(self.media_controls, MediaControls::Banner | MediaControls::Dock);
        !(self.show_media_keys || self.show_other_apps || self.show_volume || card)
    }
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            // Read from the registry, where the installer's startup task sets it.
            autostart: false,

            // Config::load turns it on for installed copies.
            auto_update: false,
            dark: None,
            album_art: true,
            keep_closed_target: true,
            tray_style: TrayStyle::Dial,
            tray_middle: TrayMiddle::Speaker,
            pulse_style: PulseStyle::Dot,
            speaker_position: util::windows_11(),
            volume_bars: VolumeBars::Compact,
            tooltip: TooltipStyle::NowPlaying,
            media_buttons: MediaButtons::Hover,
            media_controls: MediaControls::Hover,
            level_meters: LevelMeters::Volume,
            device_chip: DeviceChip::Beside,
            show_media_keys: true,
            show_other_apps: true,
            show_volume: true,
            accent: Accent::Cyan,
        }
    }
}

/// One settings change.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Deserialize), serde(rename_all = "camelCase"))]
pub enum Setting {
    Autostart(bool),
    AutoUpdate(bool),
    Dark(Option<bool>),
    AlbumArt(bool),
    KeepClosedTarget(bool),
    TrayStyle(TrayStyle),
    TrayMiddle(TrayMiddle),
    PulseStyle(PulseStyle),
    SpeakerPosition(bool),
    VolumeBars(VolumeBars),
    Tooltip(TooltipStyle),
    MediaButtons(MediaButtons),
    MediaControls(MediaControls),
    LevelMeters(LevelMeters),
    DeviceChip(DeviceChip),
    ShowMediaKeys(bool),
    ShowOtherApps(bool),
    ShowVolume(bool),
    Accent(Accent),
}

/// Arranging tray icons. Windows 10 lets only the user move icons, so the app opens
/// the right page there. Windows 11 keeps each icon's place in the registry, so the
/// app moves its own icon out of the overflow directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Deserialize), serde(rename_all = "camelCase"))]
pub enum IconPage {
    /// Notification Area Icons, where this app's icon can be set to always show.
    ShowOurs,
    /// System Icons: turn off the Windows volume icon.
    HideWindows,
}

/// Opens Windows' page of per-app volumes and devices.
pub fn open_app_volume_settings() {
    util::open_app_volume_settings();
}

/// Opens a Windows 10 page for arranging tray icons.
pub fn open_icon_page(page: IconPage) {
    util::open_icon_page(page);
}

/// True on Windows 11, where flyouts float above the taskbar.
pub fn windows_11() -> bool {
    util::windows_11()
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct State {
    pub targets: Vec<Target>,
    pub mixer: Vec<MixerApp>,
    pub audio: Audio,
    /// Chosen target id. `None` leaves media keys to Windows.
    pub selected: Option<String>,
    /// The target id of Windows' current media session, which media keys reach when
    /// none is chosen.
    pub current: Option<String>,
    pub settings: Settings,
    pub update: UpdateStatus,
    /// This app's tray icon shows on the taskbar rather than in the overflow, when
    /// Windows says.
    pub icon_on_taskbar: Option<bool>,
    /// The primary taskbar speaker is currently covered by our interactive icon.
    pub speaker_position_active: bool,
}

impl State {
    /// The target that media keys reach: the chosen one while its app runs, or else
    /// Windows' current one.
    pub fn now_playing(&self) -> Option<&Target> {
        self.routed().or_else(|| {
            let id = self.current.as_deref()?;
            self.targets.iter().find(|t| t.id == id)
        })
    }

    /// The target that currently receives media keys, if routing is active.
    pub fn routed(&self) -> Option<&Target> {
        let id = self.selected.as_deref()?;
        self.targets.iter().find(|t| t.id == id && t.running)
    }
}

/// Where the tray icon sits, for placing a flyout next to it. All values are physical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct Anchor {
    pub icon: Rect,
    /// The work area (screen minus taskbar) of the icon's monitor.
    pub work: Rect,
    /// The whole monitor.
    pub screen: Rect,
    /// DPI scale of that monitor.
    pub scale: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// How long a flyout takes to slide out from behind the taskbar.
pub const SLIDE: std::time::Duration = std::time::Duration::from_millis(150);

/// How far a flyout opened at `since` has slid out, from 0 (hidden behind the
/// taskbar) to 1 (in place). Eases out, and is always 1 when Windows animations are off.
pub fn slide_progress(since: std::time::Instant) -> f32 {
    if !util::animations_enabled() {
        return 1.0;
    }
    let t = (since.elapsed().as_secs_f32() / SLIDE.as_secs_f32()).min(1.0);
    1.0 - (1.0 - t).powi(3)
}

/// How long a closing flyout takes to slide back behind the taskbar.
pub const SLIDE_OUT: std::time::Duration = std::time::Duration::from_millis(90);

/// How far a flyout closing since `since` is still out, from 1 down to 0 (hidden).
/// Eases in, and is always 0 when Windows animations are off.
pub fn slide_out_progress(since: std::time::Instant) -> f32 {
    if !util::animations_enabled() {
        return 0.0;
    }
    let t = (since.elapsed().as_secs_f32() / SLIDE_OUT.as_secs_f32()).min(1.0);
    1.0 - t * t
}

impl Anchor {
    /// Top-left corner for a flyout of the given physical size: flush against the
    /// taskbar like the Windows flyouts, centered on the icon, kept inside the work area.
    pub fn place(&self, width: i32, height: i32) -> (i32, i32) {
        // Margin from the screen edges along the taskbar; none towards the taskbar.
        let gap = (12.0 * self.scale) as i32;
        let (w, i) = (self.work, self.icon);
        let clamp_x = |x: i32| x.clamp(w.left + gap, (w.right - width - gap).max(w.left + gap));
        let clamp_y = |y: i32| y.clamp(w.top + gap, (w.bottom - height - gap).max(w.top + gap));
        let center_x = (i.left + i.right) / 2 - width / 2;
        let center_y = (i.top + i.bottom) / 2 - height / 2;
        if i.top >= w.bottom {
            (clamp_x(center_x), w.bottom - height)
        } else if i.bottom <= w.top {
            (clamp_x(center_x), w.top)
        } else if i.left >= w.right {
            (w.right - width, clamp_y(center_y))
        } else if i.right <= w.left {
            (w.left, clamp_y(center_y))
        } else {
            (w.right - width - gap, w.bottom - height)
        }
    }

    /// True when a side panel `panel` wide fits left of a flyout `width` wide, so the
    /// window grows leftwards. Otherwise it grows rightwards.
    pub fn panel_left(&self, width: i32, panel: i32, height: i32) -> bool {
        let (x, _) = self.place(width, height);
        x - panel >= self.work.left
    }

    /// Like [`Anchor::place`] for a window `width` wide that includes a side panel
    /// `panel` wide. The flyout part stays where it would be alone.
    pub fn place_with_panel(&self, width: i32, panel: i32, height: i32) -> (i32, i32) {
        let flyout = width - panel;
        let (x, y) = self.place(flyout, height);
        let x = if self.panel_left(flyout, panel, height) { x - panel } else { x };
        (x.clamp(self.work.left, (self.work.right - width).max(self.work.left)), y)
    }

    /// True when the taskbar is at the top, so the flyout hangs down from it.
    pub fn grows_down(&self) -> bool {
        self.icon.bottom <= self.work.top
    }

    /// True when the flyout floats clear of the taskbar, as on Windows 11. On Windows
    /// 10 it sits flush against the taskbar, unless that is at the side.
    pub fn floats(&self) -> bool {
        let side = self.icon.top < self.work.bottom
            && !self.grows_down()
            && (self.icon.left >= self.work.right || self.icon.right <= self.work.left);
        util::windows_11() || side
    }

    /// Top-left corner for `frame`, with its showing parts against the taskbar.
    pub fn place_frame(&self, frame: Frame) -> (i32, i32) {
        let (x, y) = self.place_with_panel(frame.width, frame.panel, frame.visible);
        if self.grows_down() { (x, y) } else { (x, y + frame.visible - frame.height) }
    }

    /// Like [`Anchor::place_frame`], but `1 - progress` of the way towards the screen
    /// edge behind the taskbar, where the flyout starts fully off screen.
    pub fn place_sliding(&self, frame: Frame, progress: f32) -> (i32, i32) {
        let (x, y) = self.place_frame(frame);
        let (w, i, s) = (self.work, self.icon, self.screen);
        let rest = 1.0 - progress.clamp(0.0, 1.0);
        let off = |distance: i32| (distance as f32 * rest).round() as i32;
        if self.grows_down() {
            (x, y - off(y + frame.visible - s.top))
        } else if i.top < w.bottom && i.left >= w.right {
            (x + off(s.right - x), y)
        } else if i.top < w.bottom && i.right <= w.left {
            (x - off(x + frame.width - s.left), y)
        } else {
            let top = y + frame.height - frame.visible;
            (x, y + off(s.bottom - top))
        }
    }
}

/// A flyout window that never changes size, so opening the settings panel or
/// listing another app never makes it jump. Its window region shows only the
/// flyout, `visible` tall, and the settings panel beside it while that is open.
/// Sizes are in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: i32,
    pub height: i32,
    /// The settings panel's width, part of `width` whether or not it shows.
    pub panel: i32,
    /// The flyout's height.
    pub visible: i32,
}

impl Frame {
    /// True when the panel sits left of the flyout.
    pub fn panel_left(&self, anchor: &Anchor) -> bool {
        anchor.panel_left(self.width - self.panel, self.panel, self.visible)
    }

    /// The rectangles that show, as (left, top, right, bottom) inside the frame: the
    /// flyout, and the panel `panel_height` tall when it's open. Both sit against the
    /// taskbar. The panel overlaps the flyout by `border`, so their borders meet as one line.
    pub fn parts(&self, anchor: &Anchor, panel_height: Option<i32>, border: i32) -> Vec<(i32, i32, i32, i32)> {
        let left = self.panel_left(anchor);
        let span = |h: i32| if anchor.grows_down() { (0, h) } else { (self.height - h, self.height) };
        let flyout = self.width - self.panel;
        let x = if left { self.panel } else { 0 };
        let (top, bottom) = span(self.visible);
        let mut parts = vec![(x, top, x + flyout, bottom)];
        if let Some(height) = panel_height {
            let (top, bottom) = span(height);
            let (l, r) = if left { (0, self.panel + border) } else { (flyout - border, self.width) };
            parts.push((l, top, r, bottom));
        }
        parts
    }
}

/// What the flyout shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub enum Page {
    #[default]
    Apps,
    Settings,
}

#[derive(Clone, Debug)]
pub enum Event {
    DismissFlyout,
    State(State),
    /// Show the flyout next to `anchor` on `page`. With `toggle` (a tray click), an
    /// open flyout closes instead.
    Flyout {
        anchor: Anchor,
        page: Page,
        toggle: bool,
    },
    /// The user picked Quit in the tray menu. The tray icon is already gone.
    Quit,
}

pub(crate) enum Cmd {
    ToggleOverflowFlyout(Anchor),
    Refresh {
        resubscribe: bool,
    },
    /// Volumes or devices changed. Cheaper than a full refresh.
    RefreshAudio {
        devices: bool,
    },
    Select(Option<String>),
    Press(MediaKey),
    /// A key for one target, whether or not media keys go to it.
    PressFor(String, MediaKey),
    /// Check for a newer release now, whatever the automatic setting says.
    CheckUpdate,
    /// A check finished at this time (ms since the epoch).
    UpdateChecked(u64),
    Set(Setting),
    /// Send an app to an output device, or back to the default with `None`.
    Route {
        app: String,
        device: Option<String>,
    },
}

pub(crate) struct Shared {
    state: Mutex<State>,
    on_event: Box<dyn Fn(Event) + Send + Sync>,
    tx: Sender<Cmd>,
}

impl Shared {
    pub(crate) fn emit(&self, event: Event) {
        (self.on_event)(event)
    }

    pub(crate) fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    pub(crate) fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }
}

/// Handle to the running core. Cheap to clone and usable from any thread.
///
/// Volume calls act immediately on the calling thread, so sliders and wheels feel
/// instant. Everything else goes through the worker.
#[derive(Clone)]
pub struct Core {
    shared: Arc<Shared>,
}

impl Core {
    /// Starts the hooks, tray and worker.
    ///
    /// Returns `None` when another instance already runs. That instance is asked to
    /// show its window, so the caller should simply exit.
    /// `on_event` runs on core threads and must hand work off quickly.
    pub fn start(on_event: impl Fn(Event) + Send + Sync + 'static) -> Option<Core> {
        autostart::handle_helper_args();
        update::clean_up();
        // A running copy shows its flyout instead, so check that before handing off.
        if !input::claim_single_instance() || autostart::hand_off() {
            return None;
        }
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), on_event: Box::new(on_event), tx });
        let config = config::Config::load();
        input::spawn(shared.clone(), config.settings.speaker_position);
        media::spawn(shared.clone(), rx, config);
        Some(Core { shared })
    }

    pub fn state(&self) -> State {
        self.shared.state()
    }

    /// Chooses the target for media keys. `None` hands them back to Windows.
    pub fn select(&self, id: Option<String>) {
        self.shared.send(Cmd::Select(id));
    }

    /// Sends a key to the routed target, or to Windows' current session in automatic mode.
    pub fn press(&self, key: MediaKey) {
        self.shared.send(Cmd::Press(key));
    }

    /// Shows or hides this app's icon on the taskbar. Windows 11 does it at once.
    /// Windows 10 lets only the user, so there it opens the page for it.
    pub fn set_icon_on_taskbar(&self, show: bool) {
        if !(util::windows_11() && input::set_icon_on_taskbar(show)) {
            util::open_icon_page(IconPage::ShowOurs);
        }
        self.shared.send(Cmd::Refresh { resubscribe: false });
    }

    /// Sends the app with id `app` to the output device `device`, or back to the
    /// default device with `None`. The choice is saved and applied again as the app
    /// starts, since Windows doesn't always keep it.
    pub fn set_route(&self, app: &str, device: Option<&str>) {
        self.shared.send(Cmd::Route { app: app.to_owned(), device: device.map(str::to_owned) });
    }

    /// Checks GitHub for a newer release and installs it if there is one.
    pub fn check_update(&self) {
        self.shared.send(Cmd::CheckUpdate);
    }

    /// Sends a key to the target with `id`, leaving where media keys go unchanged.
    pub fn press_for(&self, id: &str, key: MediaKey) {
        self.shared.send(Cmd::PressFor(id.to_owned(), key));
    }

    pub fn set(&self, setting: Setting) {
        self.shared.send(Cmd::Set(setting));
    }

    /// Changes an app's volume by `delta` (volume runs from 0 to 1).
    pub fn adjust_volume(&self, id: &str, delta: f32) {
        util::com_init();
        volume::adjust(id, |level| level + delta);
        self.shared.send(Cmd::RefreshAudio { devices: false });
    }

    /// The live level of the app with target or mixer `id`, 0 to 1, already scaled
    /// by its volume. Meters run only while a front end keeps asking.
    pub fn level(&self, id: &str) -> f32 {
        meters::level(id)
    }

    pub fn set_volume(&self, id: &str, level: f32) {
        util::com_init();
        volume::adjust(id, |_| level);
        self.shared.send(Cmd::RefreshAudio { devices: false });
    }

    pub fn toggle_mute(&self, id: &str) {
        util::com_init();
        volume::toggle_mute(id);
        self.shared.send(Cmd::RefreshAudio { devices: false });
    }

    /// Changes the master volume by `delta`. Raising it also unmutes.
    pub fn adjust_master(&self, delta: f32) {
        util::com_init();
        audio::adjust_master(delta);
    }

    pub fn set_master(&self, level: f32) {
        util::log(format!("set master {level:.2}"));
        util::com_init();
        audio::set_master(level);
    }

    pub fn toggle_master_mute(&self) {
        util::com_init();
        audio::toggle_master_mute();
    }

    /// Makes `id` the default output device.
    pub fn set_device(&self, id: &str) {
        util::com_init();
        audio::set_default_device(id);
    }

    /// Where to open the flyout without a tray click: next to the tray icon, or the
    /// bottom-right corner of the primary screen.
    pub fn default_anchor(&self) -> Anchor {
        input::default_anchor()
    }

    /// Removes the tray icon. Call before exiting for reasons other than [`Event::Quit`].
    pub fn shutdown(&self) {
        util::log("shutdown");
        input::remove_tray();
    }
}

/// Renders the tray icon as PNG, for previews and tests.
#[doc(hidden)]
pub fn preview_tray_icon(
    level: f32,
    muted: bool,
    style: TrayStyle,
    middle: TrayMiddle,
    pulse: PulseStyle,
    light_taskbar: bool,
    size: i32,
) -> Option<Vec<u8>> {
    let look = tray_icon::Look { level, muted, style, middle, pulse, light_taskbar, ..sample_look() };
    tray_png(&look, size)
}

/// A playing look with a sample spectrum, so live styles show their shape.
fn sample_look() -> tray_icon::Look {
    tray_icon::Look {
        level: 0.6,
        spectrum: spectrum::Spectrum {
            bands: std::array::from_fn(|b| (1.0 - b as f32 / 14.0) * (0.75 + 0.25 * (b as f32 * 1.9).sin())),
            bass: 0.8,
            level: 0.6,
        },
        playing: Some(true),
        ..Default::default()
    }
}

fn tray_png(look: &tray_icon::Look, size: i32) -> Option<Vec<u8>> {
    let mut pixels = tray_icon::bgra(look, size)?;
    pixels.as_chunks_mut::<4>().0.iter_mut().for_each(|px| px.swap(0, 2));
    Some(icon::encode_png(size as u32, size as u32, &pixels))
}

/// What a tray swatch shows, for its cache.
type SwatchKey = (TrayStyle, TrayMiddle, PulseStyle, bool, usize);

static SWATCHES: std::sync::LazyLock<Mutex<std::collections::HashMap<SwatchKey, Arc<[u8]>>>> =
    std::sync::LazyLock::new(Mutex::default);

/// A tray icon look as a PNG for the settings to pick from, 40 px square, in light
/// marks for a dark panel or dark marks for a light one. `art` is the current
/// cover, which the album art middle shows. Drawn once per look and cover.
pub fn tray_swatch(
    style: TrayStyle,
    middle: TrayMiddle,
    pulse: PulseStyle,
    dark: bool,
    art: Option<&AppIcon>,
) -> Option<Arc<[u8]>> {
    let key = (style, middle, pulse, dark, art.map_or(0, |a| a.data.as_ptr() as usize));
    let mut swatches = SWATCHES.lock().unwrap();
    if let Some(png) = swatches.get(&key) {
        return Some(png.clone());
    }
    // The cover decodes through COM, which this thread may not have started.
    util::com_init();
    let look = tray_icon::Look { style, middle, pulse, light_taskbar: !dark, art: art.cloned(), ..sample_look() };
    let png: Arc<[u8]> = tray_png(&look, 40)?.into();
    if swatches.len() > 128 {
        swatches.clear();
    }
    swatches.insert(key, png.clone());
    Some(png)
}

/// [`tray_swatch`] as a `data:` URL.
pub fn tray_swatch_url(
    style: TrayStyle,
    middle: TrayMiddle,
    pulse: PulseStyle,
    dark: bool,
    art: Option<&AppIcon>,
) -> Option<String> {
    tray_swatch(style, middle, pulse, dark, art).map(|png| format!("data:image/png;base64,{}", icon::base64(&png)))
}

/// A window's top-left corner on screen, for noticing that the user moved it.
pub fn window_origin(hwnd: isize) -> Option<(i32, i32)> {
    util::window_origin(hwnd)
}

/// Moves a flyout window (by its raw handle) to where `anchor` places `frame`,
/// `progress` of the way through its slide (see [`slide_progress`]).
pub fn place_window(hwnd: isize, anchor: &Anchor, frame: Frame, progress: f32) {
    input::FLYOUT_WINDOW.store(hwnd, std::sync::atomic::Ordering::Relaxed);
    util::place_window(hwnd, anchor, frame, progress);
}

/// Hides (cloaks) or shows a window without changing its visibility state, for
/// moving a just-created window into place unseen.
pub fn set_cloaked(hwnd: isize, cloaked: bool) {
    util::set_cloaked(hwnd, cloaked);
}

/// Limits a window to the union of `parts`, as (left, top, right, bottom) in physical
/// pixels from its client area's top-left corner. Clicks outside go to the windows behind.
pub fn set_window_region(hwnd: isize, parts: &[(i32, i32, i32, i32)]) {
    util::set_window_region(hwnd, parts);
}

/// The page to open on launch: `--settings` opens the settings page.
/// Tells the tray which page the flyout shows, or `None` once it closes. The tray
/// tooltip stays away while the flyout shows its apps.
pub fn set_flyout_page(page: Option<Page>) {
    input::set_flyout_page(page);
}

pub fn start_page() -> Page {
    if std::env::args().any(|a| a == "--settings") { Page::Settings } else { Page::Apps }
}

/// True when the app was launched with `--pinned`: the flyout stays open when it
/// loses focus, which helps comparing front ends side by side.
pub fn start_pinned() -> bool {
    std::env::args().any(|a| a == "--pinned")
}

/// True when the app should start in the tray without its flyout: launched by
/// autostart, or by the elevated task for a launch that didn't ask for the flyout.
pub fn start_hidden() -> bool {
    static HIDDEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HIDDEN.get_or_init(|| {
        if autostart::launched_by_task() {
            !autostart::take_open_request()
        } else {
            std::env::args().any(|a| a == "--hidden")
        }
    })
}

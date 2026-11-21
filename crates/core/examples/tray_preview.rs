//! Writes tray icon previews to the given directory, on dark and light backgrounds:
//! every style, and every middle inside a ring, a dial and on its own.

use audio_bartender_core::{PulseStyle, TrayMiddle, TrayStyle, preview_tray_icon};

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("output directory"));
    let mut looks = Vec::new();
    for style in TrayStyle::ALL {
        if style.has_middle() {
            looks.extend(TrayMiddle::ALL.map(|middle| (style, middle, PulseStyle::Dot)));
            looks.extend(PulseStyle::ALL.map(|pulse| (style, TrayMiddle::Pulse, pulse)));
        } else {
            looks.push((style, TrayMiddle::Speaker, PulseStyle::Dot));
        }
    }
    for size in [16, 24, 32] {
        for (style, middle, pulse) in &looks {
            for (level, muted) in [(0.0, false), (0.2, false), (0.45, false), (0.7, false), (1.0, false), (0.5, true)] {
                for light in [false, true] {
                    let png = preview_tray_icon(level, muted, *style, *middle, *pulse, light, size).expect("render");
                    let name = format!(
                        "{size}-{}-{}{}-{}-{}-{}.png",
                        style.name(),
                        middle.name(),
                        if *middle == TrayMiddle::Pulse { format!("_{}", pulse.name()) } else { String::new() },
                        (level * 100.0) as u32,
                        muted,
                        if light { "light" } else { "dark" }
                    );
                    std::fs::write(dir.join(name), png).unwrap();
                }
            }
        }
    }
}

//! Writes tray icon previews to the given directory, on dark and light backgrounds.

use audio_bartender_core::{TrayStyle, preview_tray_icon};

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("output directory"));
    for size in [16, 24, 32] {
        for style in TrayStyle::ALL {
            for (level, muted) in [(0.0, false), (0.2, false), (0.45, false), (0.7, false), (1.0, false), (0.5, true)] {
                for light in [false, true] {
                    let png = preview_tray_icon(level, muted, style, light, size).expect("render");
                    let name = format!(
                        "{size}-{}-{}-{}-{}.png",
                        style.name(),
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

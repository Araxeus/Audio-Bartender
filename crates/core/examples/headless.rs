//! Runs the core without a GUI for a few seconds, printing events and saving
//! each target's icon to the temp directory.

use mediakeys_core::{Core, Event};

fn main() {
    let _core = Core::start(|event| {
        println!("{event:?}");
        if let Event::State(state) = event {
            for target in state.targets {
                if let Some(icon) = target.icon {
                    let path = std::env::temp_dir().join(format!("mediakeys-icon-{}.png", target.name));
                    let _ = std::fs::write(path, icon.png());
                }
            }
        }
    })
    .expect("another instance runs");
    std::thread::sleep(std::time::Duration::from_secs(4));
}

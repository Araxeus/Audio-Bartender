//! Switches the default output device to another one and back, printing each step.

use std::time::Duration;

use mediakeys_core::Core;

fn wait_default(core: &Core, id: &str) -> bool {
    for _ in 0..30 {
        if core.state().audio.device().is_some_and(|d| d.id == id) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn main() {
    let core = Core::start(|_| {}).expect("another instance runs");
    std::thread::sleep(Duration::from_secs(2));
    let audio = core.state().audio;
    let original = audio.device().expect("a default device").clone();
    let other = audio.devices.iter().find(|d| !d.default).expect("a second device").clone();
    println!("default: {}", original.name);

    core.set_device(&other.id);
    println!("switched to {}: {}", other.name, wait_default(&core, &other.id));
    core.set_device(&original.id);
    println!("switched back to {}: {}", original.name, wait_default(&core, &original.id));
    core.shutdown();
}

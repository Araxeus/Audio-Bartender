//! Runs the core without a GUI for a few seconds and prints a summary of each state.

use mediakeys_core::{Core, Event};

fn main() {
    let _core = Core::start(|event| match event {
        Event::State(state) => {
            let audio = &state.audio;
            println!("master {:.2} muted {} device {:?}", audio.volume, audio.muted, audio.device().map(|d| &d.name));
            for d in &audio.devices {
                println!("  device {} default={}", d.name, d.default);
            }
            for t in &state.targets {
                println!("  target {} vol {:?} muted {} icon {:?}", t.name, t.volume, t.muted, t.icon);
            }
            for a in &state.mixer {
                println!("  mixer {} vol {:.2} icon {:?}", a.name, a.volume, a.icon);
            }
        }
        other => println!("{other:?}"),
    })
    .expect("another instance runs");
    std::thread::sleep(std::time::Duration::from_secs(4));
}

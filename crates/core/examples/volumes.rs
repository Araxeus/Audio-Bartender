//! Prints the master volume and every app volume once.

fn main() {
    let core = mediakeys_core::Core::start(|_| {}).expect("another instance runs");
    std::thread::sleep(std::time::Duration::from_millis(2500));
    let state = core.state();
    print!("master={:.2}", state.audio.volume);
    for t in &state.targets {
        if let Some(v) = t.volume {
            print!(" {}={v:.2}", t.name);
        }
    }
    for a in &state.mixer {
        print!(" {}={:.2}", a.name, a.volume);
    }
    println!();
    core.shutdown();
}

//! Prints the master volume, or sets it: `master [level 0..1]`.

fn main() {
    let core = audio_bartender_core::Core::start(|_| {}).expect("another instance runs");
    if let Some(level) = std::env::args().nth(1).and_then(|a| a.parse::<f32>().ok()) {
        core.set_master(level);
    }
    std::thread::sleep(std::time::Duration::from_millis(1500));
    println!("{:.2}", core.state().audio.volume);
    core.shutdown();
}

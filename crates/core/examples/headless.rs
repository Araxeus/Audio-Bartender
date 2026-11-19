//! Runs the core without a GUI for 10 seconds, printing events.

fn main() {
    let _core = mediakeys_core::Core::start(|event| println!("{event:?}")).expect("another instance runs");
    std::thread::sleep(std::time::Duration::from_secs(10));
}

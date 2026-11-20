//! Live levels of each app's audio. A thread polls the session peak meters while a
//! front end keeps asking for levels, and sleeps once it stops.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::thread::Thread;
use std::time::{Duration, Instant};

use crate::util::{now_ms, processes};
use crate::volume;

/// Smoothed level per exe, 0 to 1.
static LEVELS: LazyLock<Mutex<HashMap<String, f32>>> = LazyLock::new(Mutex::default);
/// When a front end last asked for levels.
static WANTED_MS: AtomicU64 = AtomicU64::new(0);
static THREAD: OnceLock<Thread> = OnceLock::new();

const FRAME: Duration = Duration::from_millis(33);
/// How often to look for new audio sessions.
const RESCAN: Duration = Duration::from_secs(1);
/// Polling stops this long after the last request.
const IDLE_MS: u64 = 1000;

/// The live level of the app with target or mixer `id`, 0 to 1. It already
/// includes the app's volume, so it never passes the volume bar's fill.
pub fn level(id: &str) -> f32 {
    want();
    let exes = volume::exes_for(id);
    let levels = LEVELS.lock().unwrap();
    exes.iter().filter_map(|exe| levels.get(exe)).fold(0., |a, b| a.max(*b))
}

fn want() {
    WANTED_MS.store(now_ms(), Relaxed);
    let thread = THREAD.get_or_init(|| {
        std::thread::Builder::new().name("bartender-meters".into()).spawn(run).expect("spawn meter thread").thread().clone()
    });
    thread.unpark();
}

fn run() {
    crate::util::com_init();
    let mut sessions = Vec::new();
    let mut scanned: Option<Instant> = None;
    let mut smooth: HashMap<String, f32> = HashMap::new();
    loop {
        if now_ms().saturating_sub(WANTED_MS.load(Relaxed)) > IDLE_MS {
            LEVELS.lock().unwrap().clear();
            (sessions, scanned) = (Vec::new(), None);
            smooth.clear();
            std::thread::park_timeout(Duration::from_secs(5));
            continue;
        }
        if scanned.is_none_or(|at| at.elapsed() > RESCAN) {
            sessions = volume::sessions(&processes());
            scanned = Some(Instant::now());
        }
        let mut targets: HashMap<&str, f32> = HashMap::new();
        for session in &sessions {
            let level = if session.muted() { 0. } else { session.peak().clamp(0., 1.) * session.level() };
            let entry = targets.entry(session.exe.as_str()).or_default();
            *entry = entry.max(level);
        }
        // Rise fast and fall slowly, so meters read as motion rather than flicker.
        for (exe, level) in smooth.iter_mut() {
            if !targets.contains_key(exe.as_str()) {
                *level *= 0.78;
            }
        }
        for (exe, target) in targets {
            let level = smooth.entry(exe.to_owned()).or_default();
            *level = if target > *level { *level + (target - *level) * 0.6 } else { *level * 0.78 + target * 0.22 };
        }
        smooth.retain(|_, level| *level > 0.002);
        *LEVELS.lock().unwrap() = smooth.clone();
        std::thread::sleep(FRAME);
    }
}

#[cfg(test)]
mod tests {
    /// Prints live levels for two seconds:
    /// `cargo test -p audio_bartender_core meters_probe -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn meters_probe() {
        crate::util::com_init();
        let exes: Vec<String> = crate::volume::sessions(&crate::util::processes()).into_iter().map(|s| s.exe).collect();
        let every = std::env::var("PROBE_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(250u64);
        for _ in 0..(2000 / every) {
            std::thread::sleep(std::time::Duration::from_millis(every));
            let line: Vec<String> = exes.iter().map(|e| format!("{e} {:.2}", super::level(&format!("exe:{e}")))).collect();
            println!("{}", line.join("  "));
        }
    }
}

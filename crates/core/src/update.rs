//! Updates from GitHub Releases.
//!
//! Checks at most once a day while automatic updates are on, or when the user asks.
//! A newer release is downloaded, checked against the SHA-256 digest GitHub publishes
//! for it, and installed: an installed copy runs the new setup silently, which stops
//! the app, keeps its choices and starts it again; a portable copy swaps its own exe
//! and restarts.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};

use windows::Data::Json::JsonObject;
use windows::Foundation::Uri;
use windows::Storage::Streams::DataReader;
use windows::Web::Http::HttpClient;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Security::Cryptography::{BCRYPT_SHA256_ALG_HANDLE, BCryptHash};
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{HSTRING, PCWSTR, w};

#[cfg(feature = "serde")]
use serde::Serialize;

use crate::Cmd;
use crate::util::{now_ms, wide};

/// Where releases come from.
const REPO: &str = "Araxeus/Audio-Bartender";
const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// Waits this long after launch before a scheduled check, so logon stays quick.
const STARTUP_DELAY_MS: u64 = 60 * 1000;
/// The release asset an installed copy updates from.
const SETUP_ASSET: &str = "AudioBartender-setup.exe";
/// Passed to the new portable copy, which waits for this one to exit.
pub(crate) const UPDATED_ARG: &str = "--updated";

/// Which front end this is, for picking the matching portable download.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    Native,
    Web,
}

static FLAVOR: OnceLock<Flavor> = OnceLock::new();

/// Tells the updater which front end is running. Call before `Core::start`.
pub fn set_flavor(flavor: Flavor) {
    let _ = FLAVOR.set(flavor);
}

/// The running front end's name, for telling the two apart in bug reports.
pub fn front_end() -> &'static str {
    match FLAVOR.get() {
        Some(Flavor::Web) => "Tauri",
        _ => "GPUI",
    }
}

/// "Audio Bartender 0.1.0 · GPUI": the app, its version and its front end.
pub fn app_label() -> String {
    format!("Audio Bartender {} · {}", env!("CARGO_PKG_VERSION"), front_end())
}

fn portable_asset() -> &'static str {
    match FLAVOR.get() {
        Some(Flavor::Web) => "AudioBartender-web-portable.exe",
        _ => "AudioBartender-portable.exe",
    }
}

/// What the updater is doing, for the settings page.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase", tag = "state", content = "detail"))]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    /// Downloading and installing this version.
    Installing(String),
    Failed(String),
}

/// The updater's state, and when it last finished a check (ms since the epoch, 0 for never).
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "camelCase"))]
pub struct UpdateStatus {
    pub state: UpdateState,
    pub checked_at: u64,
    /// One line for the settings page, such as "Version 0.1.0 · up to date, checked today".
    pub summary: String,
}

static STATUS: Mutex<UpdateStatus> =
    Mutex::new(UpdateStatus { state: UpdateState::Idle, checked_at: 0, summary: String::new() });
static RUNNING: AtomicBool = AtomicBool::new(false);
static STARTED_MS: OnceLock<u64> = OnceLock::new();

pub fn status() -> UpdateStatus {
    let mut status = STATUS.lock().unwrap().clone();
    status.summary = summary(&status);
    status
}

impl UpdateStatus {
    /// True while a check or an install runs.
    pub fn busy(&self) -> bool {
        matches!(self.state, UpdateState::Checking | UpdateState::Installing(_))
    }
}

fn summary(status: &UpdateStatus) -> String {
    let version = format!("Version {} · {}", env!("CARGO_PKG_VERSION"), front_end());
    let checked = match status.checked_at {
        0 => "not checked yet".to_owned(),
        at => match now_ms().saturating_sub(at) / DAY_MS {
            0 => "checked today".to_owned(),
            1 => "checked yesterday".to_owned(),
            days => format!("checked {days} days ago"),
        },
    };
    match &status.state {
        UpdateState::Checking => format!("{version} · checking for updates…"),
        UpdateState::Installing(next) => format!("{version} · installing {next}…"),
        UpdateState::UpToDate => format!("{version} · up to date, {checked}"),
        UpdateState::Failed(message) => format!("{version} · {message}"),
        UpdateState::Idle => format!("{version} · {checked}"),
    }
}

fn set_state(state: UpdateState, tx: &Sender<Cmd>) {
    STATUS.lock().unwrap().state = state;
    let _ = tx.send(Cmd::RefreshAudio { devices: false });
}

/// Starts a scheduled check when automatic updates are on and the last one is a day old.
pub fn tick(enabled: bool, last_check: u64, tx: &Sender<Cmd>) {
    let now = now_ms();
    let started = *STARTED_MS.get_or_init(|| now);
    STATUS.lock().unwrap().checked_at = last_check;
    if !enabled || crate::autostart::dev() || now.saturating_sub(started) < STARTUP_DELAY_MS {
        return;
    }
    if now.saturating_sub(last_check) >= DAY_MS {
        check(tx.clone());
    }
}

/// Checks for a newer release on a thread of its own, and installs it if there is one.
pub fn check(tx: Sender<Cmd>) {
    if RUNNING.swap(true, Relaxed) {
        return;
    }
    std::thread::Builder::new()
        .name("bartender-update".into())
        .spawn(move || {
            crate::util::com_init();
            set_state(UpdateState::Checking, &tx);
            let state = match run(&tx) {
                Ok(state) => state,
                Err(message) => {
                    crate::util::log(format!("update failed: {message}"));
                    UpdateState::Failed(message)
                }
            };
            let at = now_ms();
            STATUS.lock().unwrap().checked_at = at;
            let _ = tx.send(Cmd::UpdateChecked(at));
            set_state(state, &tx);
            RUNNING.store(false, Relaxed);
        })
        .expect("spawn update thread");
}

/// A published release: its tag, and each asset's name, download URL and digest.
struct Release {
    tag: String,
    assets: Vec<(String, String, String)>,
}

fn client() -> Result<HttpClient, String> {
    let client = HttpClient::new().map_err(err)?;
    let headers = client.DefaultRequestHeaders().map_err(err)?;
    let _ = headers.UserAgent().and_then(|ua| ua.TryParseAdd(&HSTRING::from("AudioBartender-updater")));
    let _ = headers.Accept().and_then(|a| a.TryParseAdd(&HSTRING::from("application/vnd.github+json")));
    Ok(client)
}

/// The latest release of `repo`, or `None` when it has none.
fn latest_release(client: &HttpClient, repo: &str) -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let response = client.GetAsync(&uri(&url)?).map_err(err)?.join().map_err(err)?;
    let code = response.StatusCode().map_err(err)?.0;
    if code == 404 {
        return Ok(None);
    }
    if !(200..300).contains(&code) {
        return Err(format!("GitHub answered {code}"));
    }
    let body = response.Content().map_err(err)?.ReadAsStringAsync().map_err(err)?.join().map_err(err)?;
    let release = JsonObject::Parse(&body).map_err(err)?;
    let text = |o: &JsonObject, key: &str| {
        o.GetNamedStringOrDefault(&HSTRING::from(key), &HSTRING::new()).map(|s| s.to_string()).unwrap_or_default()
    };
    let list = release.GetNamedArray(&HSTRING::from("assets")).map_err(err)?;
    let assets = (0..list.Size().map_err(err)?)
        .filter_map(|i| list.GetObjectAt(i).ok())
        .map(|a| (text(&a, "name"), text(&a, "browser_download_url"), text(&a, "digest")))
        .collect();
    Ok(Some(Release { tag: text(&release, "tag_name"), assets }))
}

fn download(client: &HttpClient, url: &str) -> Result<Vec<u8>, String> {
    let response = client.GetAsync(&uri(url)?).map_err(err)?.join().map_err(err)?;
    if !response.IsSuccessStatusCode().map_err(err)? {
        return Err("The download failed".into());
    }
    let buffer = response.Content().map_err(err)?.ReadAsBufferAsync().map_err(err)?.join().map_err(err)?;
    let mut bytes = vec![0u8; buffer.Length().map_err(err)? as usize];
    DataReader::FromBuffer(&buffer).and_then(|r| r.ReadBytes(&mut bytes)).map_err(err)?;
    Ok(bytes)
}

fn run(tx: &Sender<Cmd>) -> Result<UpdateState, String> {
    let client = client()?;
    let release = latest_release(&client, REPO)?.ok_or("No release has been published yet")?;
    let latest = release.tag.trim_start_matches('v').to_owned();
    if !newer(&latest, env!("CARGO_PKG_VERSION")) {
        return Ok(UpdateState::UpToDate);
    }

    let installed = crate::autostart::installed();
    let wanted = if installed { SETUP_ASSET } else { portable_asset() };
    let tag = &release.tag;
    let (_, url, digest) = release
        .assets
        .iter()
        .find(|(name, _, _)| name == wanted)
        .ok_or_else(|| format!("Release {tag} has no {wanted}"))?;
    let expected = digest
        .strip_prefix("sha256:")
        .ok_or_else(|| format!("Release {tag} doesn't publish a checksum for {wanted}"))?
        .to_lowercase();

    set_state(UpdateState::Installing(latest.clone()), tx);
    let bytes = download(&client, url)?;
    if sha256_hex(&bytes)? != expected {
        return Err(format!("{wanted} didn't match its published checksum"));
    }

    crate::util::log(format!("updating to {latest} from {wanted}"));
    if installed { install_setup(&bytes) } else { swap_portable(&bytes) }?;
    Ok(UpdateState::Installing(latest))
}

fn err(e: windows::core::Error) -> String {
    e.message().to_string()
}

fn uri(url: &str) -> Result<Uri, String> {
    Uri::CreateUri(&HSTRING::from(url)).map_err(err)
}

/// True when version `a` is newer than `b`. Both look like 1.2.3.
fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| v.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parse(a) > parse(b)
}

fn sha256_hex(data: &[u8]) -> Result<String, String> {
    let mut out = [0u8; 32];
    let status = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut out) };
    if status.is_err() {
        return Err("Couldn't compute the download's checksum".into());
    }
    Ok(out.iter().map(|b| format!("{b:02x}")).collect())
}

/// Runs the new setup silently for the same kind of install. Setup stops this app
/// and starts the new one. An all-users install needs admin, so it asks for UAC
/// unless this copy already runs elevated.
fn install_setup(bytes: &[u8]) -> Result<(), String> {
    let dir = std::env::temp_dir().join("AudioBartender-update");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let setup = dir.join(SETUP_ASSET);
    std::fs::write(&setup, bytes).map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
    let all_users = program_files.is_some_and(|p| exe.starts_with(p));
    let mode = if all_users { "/ALLUSERS" } else { "/CURRENTUSER" };
    let args = format!("/VERYSILENT /SUPPRESSMSGBOXES /NORESTART {mode}");
    if all_users && !crate::autostart::is_elevated() {
        run_elevated(&setup, &args)
    } else {
        std::process::Command::new(&setup)
            .raw_arg(&args)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Couldn't start setup: {e}"))
    }
}

fn run_elevated(file: &Path, args: &str) -> Result<(), String> {
    let (file, params) = (wide(&file.to_string_lossy()), wide(args));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut info)
            .map_err(|_| "The update needs admin rights, and the prompt was declined".to_owned())?;
        if !info.hProcess.is_invalid() {
            let _ = CloseHandle(info.hProcess);
        }
    }
    Ok(())
}

/// Puts the new exe in place of this one and starts it. A running exe can't be
/// overwritten, but it can be renamed, so this one moves aside to `.old`, which the
/// new copy deletes when it starts.
fn swap_portable(bytes: &[u8]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let new = exe.with_extension("new");
    let old = exe.with_extension("old");
    std::fs::write(&new, bytes).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(e.to_string());
    }
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    std::process::Command::new(&exe)
        .args(["--hidden", UPDATED_ARG])
        .creation_flags(DETACHED_PROCESS)
        .spawn()
        .map_err(|e| e.to_string())?;
    crate::input::remove_tray();
    std::process::exit(0);
}

/// Deletes the exe a portable update moved aside.
pub fn clean_up() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old"));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn compares_versions() {
        assert!(super::newer("0.2.0", "0.1.9"));
        assert!(super::newer("1.0.0", "0.10.0"));
        assert!(super::newer("0.10.0", "0.9.0"));
        assert!(!super::newer("0.1.0", "0.1.0"));
        assert!(!super::newer("0.1.0", "0.2.0"));
    }

    /// Reads a real release and checks one asset against its digest:
    /// `cargo test -p audio_bartender_core live_release -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_release() {
        crate::util::com_init();
        let client = super::client().unwrap();
        let release = super::latest_release(&client, super::REPO).unwrap().unwrap();
        println!("tag {}", release.tag);
        let (name, url, digest) = release.assets.iter().find(|a| a.0.ends_with(".exe")).unwrap();
        let bytes = super::download(&client, url).unwrap();
        let hash = super::sha256_hex(&bytes).unwrap();
        println!("{name}: {} bytes, digest {digest}, computed sha256:{hash}", bytes.len());
        assert_eq!(digest, &format!("sha256:{hash}"));
        assert!(super::latest_release(&client, "Araxeus/this-repo-does-not-exist-ab").unwrap().is_none());
    }

    #[test]
    fn hashes_sha256() {
        assert_eq!(
            super::sha256_hex(b"abc").unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}

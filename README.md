# Media Keys Selector

Picks which app receives the keyboard media keys (play/pause, next, previous, stop) on Windows 10/11.

## Layout

| Path | What |
|---|---|
| `crates/core` | Keyboard hook, SMTC session tracking, key forwarding, tray icon, config, autostart |
| `apps/gpui` | GPUI Kit front end → `mediakeys-gpui.exe` |
| `apps/tauri` | Tauri 2 front end (vanilla HTML/CSS/JS in `ui/`) → `mediakeys-tauri.exe` |
| `assets/icon.ico` | Shared icon, drawn by `scripts/make-icon.ps1` |

## How it works

- Automatic mode installs nothing in the key path. Windows routes media keys as usual.
- Choosing an app turns on a low-level keyboard hook. The hook swallows media keys before other apps' hotkeys see them.
- The worker forwards each key to the chosen app through SMTC (`TryTogglePlayPauseAsync` and friends).
- Players without SMTC (VLC 3, MPC-HC, AIMP, MusicBee, Winamp, PotPlayer) get `WM_APPCOMMAND` window messages instead.
- When the chosen app is closed, keys go back to Windows until it starts again.
- The choice is stored in `%APPDATA%\MediaKeysSelector\config.ini`.
- "Start with Windows" writes `HKCU\...\Run\MediaKeysSelector` with `--hidden`, so the app starts in the tray.
- Closing the window keeps the app running in the tray. Quit lives in the tray menu.
- Scrolling over an app's card changes that app's volume (5% per wheel notch), like the Volume Mixer.
- The header has two toggles, one for the in-app title bar and one for dark mode. Both are stored in `config.ini`, so the two front ends share them.
- The single-instance lock is off for now (`SINGLE_INSTANCE` in `crates/core/src/input.rs`), so both front ends can run side by side. With it on, a second launch focuses the running instance.

## Build and compare

```powershell
pwsh scripts/measure.ps1          # release build, copy to dist/, measure + screenshot both
pwsh scripts/measure.ps1 -NoBuild
```

`dist/` gets both exes, `gpui.png`, `tauri.png` and `results.csv`.

```powershell
pwsh scripts/side-by-side.ps1     # open both next to each other, Enter closes them
```

Set `MEDIAKEYS_LOG=1` to log routing and forwarded keys to `%TEMP%\mediakeys.log`.

## Gotchas

- Windows never calls a low-level hook in a process that registered raw keyboard input. tao does that by default, so the Tauri app sets `DeviceEventFilter::Always`. Any new front end must avoid raw keyboard input too.
- The hook runs on its own thread. Windows silently drops a hook whose thread answers slowly, so nothing else may run there.
- GPUI hardcodes Windows SDK 10.0.26100 for `fxc.exe`. `.cargo/config.toml` points `GPUI_FXC_PATH` at 10.0.22621.
- Hardware that sends media keys as raw HID reports straight to an app bypasses the hook.

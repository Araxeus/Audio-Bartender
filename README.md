# Media Keys Selector

Picks which app receives the keyboard media keys (play/pause, next, previous, stop) on Windows 10/11, and replaces the Windows volume tray icon with one that also controls per-app volume and output devices.

## Layout

| Path | What |
|---|---|
| `crates/core` | Hooks, SMTC tracking, key forwarding, audio sessions, devices, tray icon, config |
| `apps/gpui` | GPUI Kit front end → `mediakeys-gpui.exe` |
| `apps/tauri` | Tauri 2 front end (vanilla HTML/CSS/JS in `ui/`) → `mediakeys-tauri.exe` |
| `assets/icon.ico` | App icon, drawn by `scripts/make-icon.ps1` |

## Media keys

- Automatic mode installs nothing in the key path. Windows routes media keys as usual.
- Choosing an app turns on key capture. A low-level keyboard hook swallows media keys before other apps' hotkeys see them.
- The worker forwards each key to the chosen app through SMTC (`TryTogglePlayPauseAsync` and friends).
- Players without SMTC (VLC 3, MPC-HC, AIMP, MusicBee, Winamp, PotPlayer) get `WM_APPCOMMAND` window messages instead.
- When the chosen app is closed, keys go back to Windows until it starts again.

## Tray

- The tray icon shows the master volume: a ring that fills around a speaker, or the number. It follows the taskbar's light or dark theme.
- Wheel over the icon: master volume, 2% per notch. Middle click: mute.
- Left click opens the flyout:
  - output device chooser
  - master volume slider with mute
  - media key target list with each app's volume
  - other apps playing audio, as in the Volume Mixer
  - media controls, open, quit
- In the flyout and the main window, scrolling over an app changes its volume (5% per notch) and reveals its slider. Clicking the percentage also reveals it.
- Right click opens a quick menu: master volume, targets and other apps with their volumes, settings, autostart and quit. Scrolling over a volume entry changes it while the menu stays open; clicking an app in Other apps mutes it.
- Settings → "Replace the Windows volume icon" shows this icon on the taskbar and hides Windows' volume icon, or brings it back. It edits Explorer's `IconStreams` store, so it restarts Explorer (open File Explorer windows close).

## Settings

Stored in `%APPDATA%\MediaKeysSelector\config.ini` and shared by both front ends:

- Start with Windows (`HKCU\...\Run\MediaKeysSelector` with `--hidden`)
- Close button hides to tray, or quits
- In-app title bar
- Album art as app icon (the track thumbnail the app reports to SMTC)
- Replace the Windows volume icon (restarts Explorer)
- Theme: System, Light, Dark
- Tray icon: Ring or Number

`--replace-volume-icon=on` (or `off`) flips that switch from the command line.

`--settings` opens the main window on the settings page, as does Settings… in the tray menu.

The single-instance lock is off for now (`SINGLE_INSTANCE` in `crates/core/src/input.rs`), so both front ends can run side by side. With it on, a second launch focuses the running instance.

## Build and compare

```powershell
pwsh scripts/measure.ps1          # release build, copy to dist/, measure + screenshot both
pwsh scripts/measure.ps1 -NoBuild
pwsh scripts/side-by-side.ps1     # open both next to each other, Enter closes them
```

Set `MEDIAKEYS_LOG=1` to log routing, forwarded keys and tray wheel steps to `%TEMP%\mediakeys.log`.

## Gotchas

- Windows never calls a low-level hook in a process that registered raw input. tao does that by default, so the Tauri app sets `DeviceEventFilter::Always`. Any new front end must avoid raw input too.
- The hooks run on their own thread. Windows silently drops a hook whose thread answers slowly, so nothing else may run there.
- The shell never sends wheel events to tray icons. The mouse hook checks the pointer against the icon's last known rectangle instead.
- For an icon in the overflow area the shell reports the overflow button's rectangle. The core ignores rectangles that don't contain the pointer.
- Apps can't move tray icons on Windows 10 through any API. Explorer's `ITrayNotify` lists items but `SetPreference` returns `ERROR_NOT_FOUND` for every item on 19045. The replace setting therefore edits `IconStreams` (20-byte header, 1640-byte records: ROT13 exe path at 0, preference at 528, icon GUID at 1068; the volume icon is GUID `7820AE73-23E3-4229-82C1-E41CB67D5B9C`). Explorer only saves this value on a clean exit, so the setting asks Explorer to exit, edits, and starts it again.
- Changing the default output device uses `IPolicyConfig`, which is undocumented but stable since Windows 7.
- The Tauri flyout keeps its webview for 60 s after closing, so reopening is instant, then frees it.
- GPUI hardcodes Windows SDK 10.0.26100 for `fxc.exe`. `.cargo/config.toml` points `GPUI_FXC_PATH` at 10.0.22621.
- Hardware that sends media keys as raw HID reports straight to an app bypasses the hook.

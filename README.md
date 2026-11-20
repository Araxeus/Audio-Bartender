# Audio Bartender

A tray audio mixer for Windows 10/11. It replaces the Windows volume icon with a flyout for per-app volume, live level meters, output devices and media controls, and it picks which app receives the keyboard media keys (play/pause, next, previous, stop).

## Layout

| Path | What |
|---|---|
| `crates/core` | Hooks, SMTC tracking, key forwarding, audio sessions, devices, tray icon, config |
| `apps/gpui` | GPUI Kit front end → `audio_bartender_gpui.exe` |
| `apps/tauri` | Tauri 2 front end (vanilla HTML/CSS/JS in `ui/`) → `audio_bartender_tauri.exe` |
| `assets/icon.ico` | App icon, drawn by `scripts/make-icon.ps1` |

## Media keys

- Automatic mode installs nothing in the key path. Windows routes media keys as usual.
- Choosing an app turns on key capture. A low-level keyboard hook swallows media keys before other apps' hotkeys see them.
- The worker forwards each key to the chosen app through SMTC (`TryTogglePlayPauseAsync` and friends).
- Players without SMTC (VLC 3, MPC-HC, AIMP, MusicBee, Winamp, PotPlayer) get `WM_APPCOMMAND` window messages instead.
- When the chosen app is closed, keys go back to Windows until it starts again.

## The flyout

The flyout is the whole UI. Left-click the tray icon to open or close it; it also opens on launch, from the tray menu, and from a second launch. It closes when you click elsewhere unless it's pinned.

- Top row: media controls, then theme, settings, pin (stay open) and quit.
- Middle: the media key target list with each app's volume, then other apps playing audio, as in the Volume Mixer. Clicking a card (or its radio button) picks where media keys go; with On click volume bars, only the radio button does. The wheel changes volume (2% per notch) only over a volume control and scrolls the list everywhere else. The settings page replaces this list.
- Bottom, next to the tray: the output device (click to choose another) and the master volume.

## Tray icon

- Shows the master volume: a ring that fills around a speaker, or the number. It follows the taskbar's light or dark theme.
- Wheel over the icon: master volume, 2% per notch, with a live tooltip. Middle click: mute.
- Right click opens a quick menu in the app's theme: master volume, targets and other apps with their volumes, the Windows volume icon's links (Sound settings, Volume mixer, Sounds, troubleshooter), settings and quit. Scrolling over a volume entry changes it while the menu stays open; clicking an app in Other apps mutes it.
- New tray icons start in the overflow area, and Windows lets only the user move them. Drag the icon onto the taskbar, and the Windows volume icon into the overflow. Settings → "Tray icons" opens the Windows pages for this.

## Settings

Stored in `%APPDATA%\AudioBartender\config.ini` and shared by both front ends:

- Start with Windows (`HKCU\...\Run\AudioBartender` with `--hidden`)
- Album art as app icon (the track thumbnail the app reports to SMTC)
- Theme: System, Light, Dark
- Accent color: Blue (default), Cyan, Teal, Purple, Hot pink or Orange. The core owns the palette, so both front ends and the tooltip cards match
- Tray icon: Ring or Number
- Tooltip: Now playing (a card with the media key app's track and art, the device and the volume), Compact (device, volume and media key app) or Text (the plain Windows tooltip)
- Volume bars: Compact (a small bar to drag or scroll; click to mute), Always (a slider under every app) or On click (a percentage; click the card for its slider)

Launch flags: `--hidden` starts in the tray only, `--settings` opens on the settings page, `--pinned` starts pinned (handy for comparing both front ends side by side).

The single-instance lock is off for now (`SINGLE_INSTANCE` in `crates/core/src/input.rs`), so both front ends can run side by side. With it on, a second launch opens the running instance's flyout.

## Build and compare

```powershell
pwsh scripts/measure.ps1          # release build, copy to dist/, measure + screenshot both
pwsh scripts/measure.ps1 -NoBuild
pwsh scripts/side-by-side.ps1     # open both flyouts pinned, next to each other; Enter closes them
```

Set `AUDIO_BARTENDER_LOG=1` to log routing, forwarded keys and tray wheel steps to `%TEMP%\audio_bartender.log`.

## Gotchas

- Windows never calls a low-level hook in a process that registered raw input. tao does that by default, so the Tauri app sets `DeviceEventFilter::Always`. Any new front end must avoid raw input too.
- The hooks run on their own thread. Windows silently drops a hook whose thread answers slowly, so nothing else may run there.
- The shell never sends wheel events to tray icons. The mouse hook checks the pointer against the icon's last known rectangle instead.
- For an icon in the overflow area the shell reports the overflow button's rectangle. The core ignores rectangles that don't contain the pointer.
- Apps can't move tray icons on Windows 10. Explorer's `ITrayNotify` (the interface behind the Notification Area Icons page) lists every item, including the volume icon by GUID `7820AE73-23E3-4229-82C1-E41CB67D5B9C`, but `SetPreference` returns `ERROR_NOT_FOUND` for every item on 19045, from any thread or apartment. The other route edits Explorer's binary `IconStreams` value and needs an Explorer restart. So the settings open the two Windows pages instead.
- Changing the default output device uses `IPolicyConfig`, which is undocumented but stable since Windows 7.
- The Tauri flyout keeps its webview for 60 s after closing, so reopening is instant, then frees it.
- GPUI can only grow a window downward, so the GPUI flyout resizes itself through Win32 (`place_window` in core) to keep its bottom against the taskbar.
- GPUI hardcodes Windows SDK 10.0.26100 for `fxc.exe`. `.cargo/config.toml` points `GPUI_FXC_PATH` at 10.0.22621.
- Hardware that sends media keys as raw HID reports straight to an app bypasses the hook.

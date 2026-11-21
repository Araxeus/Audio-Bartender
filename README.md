# Audio Bartender

A tray audio mixer for Windows 10/11 with per-app volume, live level meters, output devices and media controls. You can choose which app receives the keyboard media keys (play/pause, next, previous, stop) and place the mixer over the Windows 11 taskbar speaker.

## Install

Download from the releases page:

| File | What |
|---|---|
| `AudioBartender-setup.exe` | Installer. Asks whether to install for all users or just you, and which interface to use: Native (GPUI, recommended) or Web (Tauri, needs the WebView2 Runtime). Either one installs as `AudioBartender.exe` |
| `AudioBartender-portable.exe` | The Native interface, nothing to install |
| `AudioBartender-web-portable.exe` | The Web interface, nothing to install |

Silent installs take `/VERYSILENT`, `/ALLUSERS` or `/CURRENTUSER`, `/INTERFACE=native|web` and `/TASKS="startup"`.

## Start with Windows

The switch in Settings adds or removes `HKCU\...\Run\AudioBartender` with `--hidden`. A copy disabled in Task Manager's Startup tab shows as off, and turning it on clears that mark.

- **Portable:** that's all. The app never asks for admin rights.
- **Installed for all users:** setup also registers an `AudioBartender` scheduled task that runs the app elevated, so the keyboard hook also sees keys typed while an admin window has focus. An unelevated launch hands off to the task, and a launch that should show the flyout still shows it. The app spots an installed copy by Inno's `unins000.exe` beside it.
- **Installed just for you:** no admin and no task, so it behaves like the portable copy.

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

- Top row: the app name, then theme, settings, pin (stay open) and quit.
- Middle: the media key target list with each app's volume, then other apps playing audio, as in the Volume Mixer. Clicking a card (or its radio button) picks where media keys go. The wheel changes volume (2% per notch) only over a volume control and scrolls the list everywhere else. The settings page replaces this list.
- Bottom, next to the tray: the output device (click to choose another) and the master volume.

## Tray icon

- Shows the master volume: a ring that fills around a speaker, or the number. It follows the taskbar's light or dark theme.
- Wheel over the icon: master volume, 2% per notch, with a live tooltip. Middle click: mute.
- Right click opens a quick menu in the app's theme: master volume, targets and other apps with their volumes, the Windows volume icon's links (Sound settings, Volume mixer, Sounds, troubleshooter), settings and quit. Scrolling over a volume entry changes it while the menu stays open; clicking an app in Other apps mutes it.
- New tray icons start in the overflow area. On Windows 11, Settings → "Show icon on the taskbar" moves ours at once. On Windows 10, Windows lets only the user move icons, so Settings shows where ours is and opens the Windows pages to move it and hide the Windows volume icon.

### Hiding the Windows volume icon on Windows 11

Windows 11 has no setting to hide its volume icon. The [Windhawk](https://windhawk.net/) mod [Taskbar tray system icon tweaks](https://windhawk.net/mods/taskbar-tray-system-icon-tweaks) can, with its "Hide volume icon" option, so Audio Bartender's icon takes its place.

- On Windows 11, turn on **Settings → Use Windows speaker position** to put Audio Bartender over the speaker beside Wi-Fi and battery. Left-click opens the mixer, right-click opens its menu, middle-click mutes, and the wheel adjusts volume. Turning the switch off removes the overlay. Quitting reveals the Windows speaker.
- Audio Bartender locates the speaker on the primary taskbar using its localized UI Automation name. It draws the icon in a layered child window of the taskbar, so the artwork and hover target follow Quick Settings when it opens and closes. A separate backup icon starts in the hidden-icons menu. Turning speaker position off restores the ordinary icon with its previous pin preference.
- If Explorer temporarily refuses the child window, Audio Bartender retries while the backup stays available. An auto-hiding taskbar or an unrecognized layout also uses the backup. Launching Audio Bartender again opens the running mixer. Speaker position is off by default and requires no additional installation.

## Settings

Stored in `%APPDATA%\AudioBartender\config.ini` and shared by both front ends. The settings button opens a panel beside the flyout with every setting visible at once, so each change shows in the app list as it's made. Each mode setting is a segmented control with a line about the current choice.

- Start with Windows (see above)
- Album art as app icon (the track thumbnail the app reports to SMTC)
- Automatic updates: on for installed copies, off for portable ones. Once a day, the app asks GitHub for the latest release, checks the download against its published SHA-256 digest, and installs it. An installed copy runs the new setup silently, which asks for UAC on an all-users install unless the app already runs elevated. A portable copy swaps in the new exe and restarts. "Check now" works with the setting off.
- Tray icon placement on Windows 11, or links to the notification area and system icon settings on Windows 10
- Theme: System, Light, Dark
- Accent color: Blue, Cyan (default), Teal, Purple, Hot pink or Orange. The core owns the palette, so both front ends and the tooltip cards match
- Tray icon: Ring or Number
- Tooltip: Now playing, Compact or Text
- Tooltip buttons: None, Hover (default), Inline, Capsule or Player. Cards with buttons stay open while hovered and change the volume on scroll
- Media controls: None, Banner, Row, Dock or Hover (default, on each app's row and for that app)
- Volume bars: None, Compact (default), Always or On hover (a percentage that turns into a slider while the row is hovered)
- Level meters: None, In volume (default), Icon ring, Live bars, Underline or Edge strip

Launch flags: `--hidden` starts in the tray only, `--settings` opens with the settings panel, `--pinned` starts pinned, and `--dev` turns off the single-instance lock and the hand-off to the elevated task.

Both front ends share one single-instance lock, so a second launch of either opens the running copy's flyout.

## Build and compare

```powershell
pwsh build.ps1                    # release build of both, then the installer, all in dist/
pwsh dist/dev.ps1 --pinned        # run both front ends from dist/ side by side
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
- On Windows 11 build 26300.9974, the legacy volume GUID returned no rectangle. The Quick Settings speaker exposed its physical bounds through UI Automation and had no window handle of its own. Windhawk’s mod collapses the speaker inside Explorer’s XAML layout. Audio Bartender uses the reported bounds for its own window, so a change to Explorer’s layout can fall back to the regular tray icon. [Windhawk mod source](https://github.com/ramensoftware/windhawk-mods/blob/main/mods/taskbar-tray-system-icon-tweaks.wh.cpp)
- Changing the default output device uses `IPolicyConfig`, which is undocumented but stable since Windows 7.
- The Tauri flyout keeps its webview for 60 s after closing, so reopening is instant, then frees it.
- GPUI can only grow a window downward, so the GPUI flyout resizes itself through Win32 (`place_window` in core) to keep its bottom against the taskbar.
- GPUI hardcodes Windows SDK 10.0.26100 for `fxc.exe`. `.cargo/config.toml` points `GPUI_FXC_PATH` at 10.0.22621, and CI overrides it with the runner's newest SDK.
- Hardware that sends media keys as raw HID reports straight to an app bypasses the hook.

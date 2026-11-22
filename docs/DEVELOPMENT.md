# Development

## Layout

| Path | What |
|---|---|
| `crates/core` | Hooks, SMTC tracking, key forwarding, audio sessions, devices, tray icon, config |
| `apps/gpui` | GPUI Kit front end → `audio_bartender_gpui.exe` |
| `apps/tauri` | Tauri 2 front end (vanilla HTML/CSS/JS in `ui/`) → `audio_bartender_tauri.exe` |
| `assets/icon.ico` | App icon, drawn by `scripts/make-icon.ps1` |
| `assets/logo` | Logo files and brand notes |
| `site` | The website, served by GitHub Pages |
| `xtask` | `cargo xtask release` |
| `site/demo` | The live demo: `mock.js` stands in for the core so the real Tauri UI runs in a browser |

## Build and measure

```powershell
pwsh build.ps1                    # release build of both, then the installer, all in dist/
pwsh dist/dev.ps1 --pinned        # run both front ends from dist/ side by side
pwsh scripts/measure.ps1          # release build, copy to dist/, measure + screenshot both
pwsh scripts/measure.ps1 -NoBuild
pwsh scripts/measure.ps1 -NoBuild -Cold   # right after a reboot: cold start only
pwsh scripts/measure.ps1 -NoBuild -NoReopen   # skip the reopen timings and their 65 s idle wait
pwsh scripts/side-by-side.ps1     # open both flyouts pinned, next to each other; Enter closes them
```

## Website demo

The site runs the real flyout from `apps/tauri/ui` in an iframe over a simulated taskbar.

- `site/demo/mock.js` fakes `window.__TAURI__`: every `invoke` command and the `state` and `page` events, with simulated apps, devices and spectrum.
- `site/demo/trayicon.js` ports `crates/core/src/tray_icon.rs` to canvas.
- `site/demo/host.js` draws the desktop and taskbar, and places the flyout like `Anchor::place`.
- `scripts/build-demo.ps1` copies the UI into `site/demo/ui` (gitignored). The Pages workflow runs it on every deploy.

A new `invoke` command in the Tauri app needs a matching entry in `mock.js`, or the demo rejects it.

Preview the site before pushing:

```powershell
bun scripts/dev.js   # prints the link to open
```

The script builds the demo, serves `site/`, and reloads the browser on every save. Edits to the Tauri UI rebuild the demo first. It needs a local server, because opening `index.html` straight from disk blocks the demo's scripts.

Set `AUDIO_BARTENDER_LOG=1` to log routing, forwarded keys and tray wheel steps to `%TEMP%\audio_bartender.log`.

Launch flags:

- `--hidden` starts in the tray only
- `--settings` opens with the settings panel
- `--pinned` starts pinned
- `--dev` turns off the single-instance lock and the hand-off to the elevated task

Both front ends share one single-instance lock, so a second launch of either opens the running copy's flyout. Settings live in `%APPDATA%\AudioBartender\config.ini`, shared by both.

## Releasing

```powershell
cargo xtask release            # asks patch, minor or major, then whether to push
cargo xtask release minor --push
```

It bumps the version in `Cargo.toml` and `apps/tauri/tauri.conf.json`, commits `chore(release): vX.Y.Z`, tags it, and pushes `main` and the tag together. The tag starts the Release workflow, which builds the installer and both portable exes, attests them and publishes the release. It refuses a dirty tree or a branch other than `main`.

## Media keys

- Automatic mode installs nothing in the key path. Windows routes media keys as usual.
- Choosing an app turns on key capture. A low-level keyboard hook swallows media keys before other apps' hotkeys see them.
- The worker forwards each key to the chosen app through SMTC (`TryTogglePlayPauseAsync` and friends).
- Players without SMTC (VLC 3, MPC-HC, AIMP, MusicBee, Winamp, PotPlayer) get `WM_APPCOMMAND` window messages instead.
- When the chosen app is closed, keys go back to Windows until it starts again.

## Start with Windows

The switch adds or removes `HKCU\...\Run\AudioBartender` with `--hidden`. A copy disabled in Task Manager's Startup tab shows as off, and turning it on clears that mark.

- **Portable:** that's all. The app never asks for admin rights.
- **Installed for all users:** setup also registers an `AudioBartender` scheduled task that runs the app elevated, so the keyboard hook sees keys typed while an admin window has focus. An unelevated launch hands off to the task. The app spots an installed copy by Inno's `unins000.exe` beside it.
- **Installed just for you:** no admin and no task, so it behaves like the portable copy.

## Per-app output devices

This uses the same private Windows interface as EarTrumpet and Windows' own "App volume and device preferences" page. Windows can forget the choice after a restart or a driver update, so the app saves it and applies it again whenever the app plays.

## Live tray icons

Live waves, Equalizer and the pulse styles read a loopback capture of the default output. An FFT splits it into 12 bands, and the icon redraws about 15 times a second while sound plays. Nothing leaves the app.

## Windows volume icon replacement

- Audio Bartender finds the Quick Settings speaker on the primary taskbar by its localized UI Automation name.
- It draws its icon in a layered child window of the taskbar, so the icon and hover target follow Quick Settings as it opens and closes.
- The hover tile uses the fill and border colors from the Windows 11 Quick Settings button template, scaled to the taskbar DPI.
- Once the tile paints and receives hits at the speaker position, the ordinary tray icon hides from the taskbar and the overflow.
- A covered speaker (a language picker, another window) keeps its place while the tile still receives hits. A lost tile brings back the ordinary icon at its saved pin position.
- Replacement defaults on for Windows 11. The first time Explorer registers the ordinary icon, Audio Bartender requests a taskbar pin. Later launches use the pin state Explorer saved.
- It checks again after Explorer restarts, the display layout changes, or Windows resumes. An auto-hiding taskbar or an unknown layout falls back to the ordinary icon.

## Gotchas

- Windows never calls a low-level hook in a process that registered raw input. tao does that by default, so the Tauri app sets `DeviceEventFilter::Always`. Any new front end must avoid raw input too.
- The hooks run on their own thread. Windows silently drops a hook whose thread answers slowly, so nothing else may run there.
- The shell never sends wheel events to tray icons. The mouse hook checks the pointer against the icon's last known rectangle instead.
- For an icon in the overflow area the shell reports the overflow button's rectangle. The core ignores rectangles that don't contain the pointer.
- Apps can't move tray icons on Windows 10. Explorer's `ITrayNotify` lists every item, including the volume icon by GUID `7820AE73-23E3-4229-82C1-E41CB67D5B9C`, but `SetPreference` returns `ERROR_NOT_FOUND` for every item on 19045. The other route edits Explorer's binary `IconStreams` value and needs an Explorer restart. So the settings open the two Windows pages instead.
- On Windows 11 build 26300.9974, the legacy volume GUID returned no rectangle. The Quick Settings speaker exposes its bounds through UI Automation and has no window handle. Windhawk's mod collapses the speaker inside Explorer's XAML layout. A change to Explorer's layout can push Audio Bartender back to the regular tray icon. [Windhawk mod source](https://github.com/ramensoftware/windhawk-mods/blob/main/mods/taskbar-tray-system-icon-tweaks.wh.cpp)
- Changing the default output device uses `IPolicyConfig`, which is undocumented but stable since Windows 7.
- The Tauri flyout keeps its webview for 60 s after closing, so reopening is instant, then frees it.
- GPUI can only grow a window downward, so the GPUI flyout resizes itself through Win32 (`place_window` in core) to keep its bottom against the taskbar.
- GPUI hardcodes Windows SDK 10.0.26100 for `fxc.exe`. `.cargo/config.toml` points `GPUI_FXC_PATH` at 10.0.22621, and CI overrides it with the runner's newest SDK.
- Hardware that sends media keys as raw HID reports straight to an app bypasses the hook.

## Credits

The website demo's Chrome tracks use YouTube thumbnails, in `site/demo/covers`. Five come from [Blender Studio](https://studio.blender.org/films/) open movies, shared under the Creative Commons Attribution (CC BY) license:

- Big Buck Bunny
- Sintel
- Tears of Steel
- Spring
- Agent 327: Operation Barbershop

import { MASTER, Volumes, avatar, button, detailText, el, follow, icon, invoke, isDark, logo, setting } from "./common.js";

const $ = (id) => document.getElementById(id);
const params = new URLSearchParams(location.search);

let page = location.hash === "#settings" ? "settings" : "apps";
let pinned = params.get("pinned") === "true";
const windows11 = params.get("win11") === "true";
document.body.classList.toggle("floats", params.get("floats") !== "false");
document.body.dataset.down = params.get("down") === "true";
let devicesOpen = false;
let first = true;
let lastSize = "";
/** The window region shows the settings panel. A closed panel stays drawn until it doesn't. */
let shownPanel = false;
let latest = null;
/** Every accent with its colors, from the core. */
let accents = [];
invoke("accents").then((list) => {
  accents = list;
  rerender();
});
const volumes = new Volumes(() => {});

function rerender() {
  if (latest) render(latest);
}

/** The only control that picks where media keys go. */
function radio(checked, id) {
  return button("radio-hit", checked ? null : "Send media keys here", el("span", "radio small" + (checked ? " checked" : "")), () =>
    invoke("select", { id }),
  );
}

function row(checked, clickable, ...children) {
  const node = el("div", "fly-row" + (checked ? " checked" : "") + (clickable ? " clickable" : ""));
  node.append(el("div", "fly-row-main", ...children));
  return node;
}

/** Makes a card send media keys to `id` when clicked, except on its volume controls. */
function selectable(node, id) {
  node.addEventListener("click", () => {
    if (!volumes.grabbed) invoke("select", { id });
  });
}

/** The row's volume control for the chosen style, beside the name. */
function volumeControl(bars, id, level, muted) {
  if (bars === "onHover") return volumes.hoverControl(id, level, muted);
  if (bars === "compact") return volumes.bar(id, level, muted);
  // "none" and "always" put nothing beside the name.
  return null;
}

/** Latest live level per app row id, and the level on screen easing towards it. */
const levels = new Map();
const shownLevels = new Map();
/** Time constant, in ms, for shown levels to catch up with readings. */
const LEVEL_EASE = 45;

/** Equalizer bars before an app's name, driven by its live level. */
function liveBars() {
  return el("span", "live-bars", el("i"), el("i"), el("i"));
}

/** Gives a row its meters. CSS shows the chosen style from the row's `--lv`. */
function metered(node, id) {
  node.dataset.levelId = id;
  node.append(el("span", "meter-line", el("i")), el("span", "meter-strip", el("i")));
  showLevel(node, shownLevels.get(id) ?? 0);
  return node;
}

function showLevel(node, level) {
  node.style.setProperty("--lv", level.toFixed(3));
  // Music rarely drops below about a quarter, so spread the useful range over the
  // bars' height, and wobble each bar at its own speed so they move like an equalizer.
  const t = performance.now() / 1000;
  const shape = Math.min(1, Math.max(0.12, (level - 0.25) / 0.7));
  [0, 1, 2].forEach((i) => {
    const wobble = 0.55 + 0.45 * (0.5 + 0.5 * Math.sin(t * (8 + i * 3.1) + i * 2.1));
    node.style.setProperty(`--b${i}`, (shape * wobble).toFixed(3));
  });
  node.classList.toggle("quiet", level < 0.02);
}

/** Reads live levels while the app list shows meters. */
async function pollLevels() {
  const style = latest?.settings.levelMeters;
  if (!style || style === "none" || devicesOpen || document.hidden) return;
  const rows = [...document.querySelectorAll("[data-level-id]")];
  if (!rows.length) return;
  const values = await invoke("levels", { ids: rows.map((r) => r.dataset.levelId) });
  rows.forEach((row, i) => levels.set(row.dataset.levelId, values[i]));
}
setInterval(pollLevels, 33);

/** Eases shown levels towards the readings every display frame, so meters glide. */
let lastFrame = performance.now();
function easeLevels(now) {
  const k = 1 - Math.exp(-Math.min(100, now - lastFrame) / LEVEL_EASE);
  lastFrame = now;
  if (latest?.settings.levelMeters && latest.settings.levelMeters !== "none") {
    for (const row of document.querySelectorAll("[data-level-id]")) {
      const id = row.dataset.levelId;
      const shown = (shownLevels.get(id) ?? 0) + ((levels.get(id) ?? 0) - (shownLevels.get(id) ?? 0)) * k;
      shownLevels.set(id, shown);
      showLevel(row, shown);
    }
  }
  requestAnimationFrame(easeLevels);
}
requestAnimationFrame(easeLevels);

/** Previous, play/pause and next. With `id` they control that app; without, wherever media keys go. */
function transport(playing, id = null, size = "small") {
  const press = (key) => (id ? invoke("press_for", { id, key }) : invoke("press", { key }));
  return el(
    "div",
    "transport",
    button(`icon ghost ${size}`, "Previous track", icon("prev"), () => press("previous")),
    button(`icon primary ${size}`, playing ? "Pause" : "Play", icon(playing ? "pause" : "play"), () => press("playPause")),
    button(`icon ghost ${size}`, "Next track", icon("next"), () => press("next")),
  );
}

/** The Automatic row's line, naming the app Windows sends media keys to now. */
function windowsDecides(state) {
  const current = state.targets.find((t) => t.id === state.current);
  return current ? `Windows decides · ${current.name}` : "Windows decides";
}

/** What the media keys control right now, for the banner and the dock. */
function nowPlaying(state) {
  // Without a chosen target, Windows' current session is what the keys reach.
  const t = state.targets.find((t) => t.id === (state.selected ?? state.current));
  if (!t) return { title: "Automatic", subtitle: "Media keys go where Windows decides", playing: false, target: null };
  if (!t.running) return { title: t.name, subtitle: "Not running", playing: false, target: t };
  const playing = t.playback === "playing";
  if (!t.track) return { title: t.name, subtitle: "Nothing playing", playing, target: t };
  const [title, artist] = t.track.split(" \u2014 ");
  return { title, subtitle: artist ? `${artist} \u00b7 ${t.name}` : t.name, playing, target: t };
}

/** Art, track and buttons for whatever the media keys control. */
function nowPlayingView(state, className) {
  const now = nowPlaying(state);
  const art = now.target ? avatar(now.target.icon, now.target.name, now.target.running) : logo();
  const bars = now.playing ? el("span", "bars", el("i"), el("i"), el("i")) : null;
  const text = el("div", "text", el("div", "name", now.title), el("div", "detail", bars, now.subtitle));
  return el("div", "now-playing " + className, art, text, transport(now.playing));
}

/** The Back Bar mark on an accent tile, in its small-size cut. */
function brandMark() {
  const tile = el("span", "avatar brand-logo logo");
  tile.innerHTML = '<svg viewBox="0 0 256 256"><path fill="currentColor" d="M58 106A6 6 0 0 1 64 100H72A6 6 0 0 1 78 106V128L92 142V196A12 12 0 0 1 80 208H56A12 12 0 0 1 44 196V142L58 128Z M118 54A6 6 0 0 1 124 48H132A6 6 0 0 1 138 54V80L152 94V196A12 12 0 0 1 140 208H116A12 12 0 0 1 104 196V94L118 80Z M178 134A6 6 0 0 1 184 128H192A6 6 0 0 1 198 134V148L212 162V196A12 12 0 0 1 200 208H176A12 12 0 0 1 164 196V162L178 148Z"/></svg>';
  return tile;
}

function controls() {
  return [
    el("div", "brand", brandMark(), el("span", "", "Audio Bartender")),
    el(
      "div",
      "transport",
      button("icon ghost small" + (page === "settings" ? " active" : ""), "Settings", icon("settings"), () => {
        page = page === "settings" ? "apps" : "settings";
        devicesOpen = false;
        rerender();
      }),
      button(
        "icon ghost small" + (pinned ? " active" : ""),
        pinned ? "Unpin: close when clicking elsewhere" : "Pin: stay open",
        icon(pinned ? "pin" : "pinOff"),
        () => {
          pinned = !pinned;
          invoke("set_pinned", { pinned });
          rerender();
        },
      ),
      button("icon ghost small", "Quit", icon("power"), () => invoke("quit")),
    ),
  ];
}

function deviceHeader(state) {
  const device = state.audio.devices.find((d) => d.default);
  return button(
    "device-header",
    "Choose output device",
    [icon("speaker"), el("span", "device-name", device?.name ?? "No output device"), icon(devicesOpen ? "chevronDown" : "chevronUp")],
    () => {
      devicesOpen = !devicesOpen;
      rerender();
    },
  );
}

function devices(state) {
  return state.audio.devices.map((device) =>
    button("device" + (device.default ? " current" : ""), null, [el("span", "check", device.default ? icon("check") : null), device.name], () => {
      invoke("set_device", { id: device.id });
      devicesOpen = false;
      rerender();
    }),
  );
}

/** "▮▮▮ Playing · Title — Artist": status in green while playing, the track muted. */
function detail(t) {
  const { text, playing } = detailText(t);
  const line = el("div", "detail");
  if (playing) {
    const bars = el("span", "bars", el("i"), el("i"), el("i"));
    line.append(el("span", "playing", bars, "Playing"), text.slice("Playing".length));
  } else {
    line.textContent = t.running ? text : "Not running";
  }
  return line;
}

/** The track line, which swaps to buttons for this app while hovered in Hover mode. */
function trackLine(t, mode) {
  if (mode !== "hover" || !t.running) return detail(t);
  return el("div", "hover-swap", detail(t), transport(t.playback === "playing", t.id, "tiny"));
}

function targets(state) {
  const bars = state.settings.volumeBars;
  const mode = state.settings.mediaControls;
  const automatic = state.selected === null;
  // The buttons reach Windows' current session, so show its state.
  const current = state.targets.find((t) => t.id === state.current);
  const currentPlaying = Boolean(current?.running && current.playback === "playing");
  const decides = el("div", "detail", windowsDecides(state));
  // With Hover controls the line swaps to buttons while the row is hovered.
  const line = mode === "hover" ? el("div", "hover-swap", decides, transport(currentPlaying, null, "tiny")) : decides;
  const auto = row(automatic, true, logo("avatar small"), el("div", "text", el("div", "name small", "Automatic"), line), radio(automatic, null));
  if (automatic && mode === "row") auto.append(el("div", "row-controls", transport(currentPlaying)));
  selectable(auto, null);
  const rows = [auto];
  for (const t of state.targets) {
    const checked = state.selected === t.id;
    const hasVolume = t.volume != null;
    const node = row(
      checked,
      true,
      avatar(t.icon, t.name, t.running),
      el("div", "text" + (t.running ? "" : " offline"), el("div", "name small", liveBars(), t.name), trackLine(t, mode)),
      hasVolume ? volumeControl(bars, t.id, t.volume, t.muted) : null,
      radio(checked, t.id),
    );
    node.querySelector(".avatar").classList.add("small");
    if (checked && mode === "row") node.append(el("div", "row-controls", transport(t.running && t.playback === "playing")));
    if (hasVolume && bars === "always") {
      // Under the name, clear of the avatar.
      const slider = volumes.sliderRow(t.id, t.volume, t.muted);
      slider.classList.add("inline");
      node.append(slider);
    }
    selectable(node, t.id);
    rows.push(metered(node, t.id));
  }
  return rows;
}

function mixer(state) {
  const bars = state.settings.volumeBars;
  return state.mixer.map((app) => {
    const icon = avatar(app.icon, app.name);
    icon.classList.add("tiny");
    let node;
    if (bars === "always") {
      node = row(false, false, icon, el("div", "mixer-name", liveBars(), app.name), volumes.sliderRow(app.id, app.volume, app.muted));
      node.classList.add("always");
    } else {
      node = row(false, false, icon, el("div", "text", el("div", "name small", liveBars(), app.name)), volumeControl(bars, app.id, app.volume, app.muted));
    }
    node.classList.add("mixer");
    return metered(node, app.id);
  });
}

function toggle(on, onChange) {
  const label = el("label", "switch");
  const input = el("input");
  input.type = "checkbox";
  input.checked = on;
  input.addEventListener("change", () => onChange(input.checked));
  label.append(input, el("span", "track", el("span", "thumb")));
  return label;
}

/** One color dot per accent; the chosen one gets a ring. */
function swatches(current) {
  const dark = isDark();
  return el(
    "div",
    "swatches",
    ...accents.map((accent) => {
      const dot = button("swatch" + (current.name === accent.name ? " selected" : ""), accent.label, el("span"), () =>
        setting("accent", accent.name),
      );
      dot.style.setProperty("--swatch", "#" + accent[dark ? "dark" : "light"].accent.toString(16).padStart(6, "0"));
      return dot;
    }),
  );
}

/** Settings picked from a few modes: the setting's name, then each value, name and description. */
const MODES = {
  dark: {
    title: "Theme",
    options: [
      [null, "System", "Follow Windows"],
      [false, "Light", "Always light"],
      [true, "Dark", "Always dark"],
    ],
  },
  trayStyle: {
    title: "Tray icon",
    options: [
      ["ring", "Ring", "A ring that fills with the volume"],
      ["number", "Number", "The volume as a number"],
    ],
  },
  tooltip: {
    title: "Tooltip",
    options: [
      ["nowPlaying", "Now playing", "Track, album art, device and volume"],
      ["compact", "Compact", "Device, volume and the media app"],
      ["text", "Text", "The plain Windows tooltip"],
    ],
  },
  mediaButtons: {
    title: "Tooltip buttons",
    options: [
      ["none", "None", "No buttons on the tooltip"],
      ["hover", "Hover", "Shown while the pointer is on the tooltip"],
      ["inline", "Inline", "Small buttons at the end of the status line"],
      ["capsule", "Capsule", "A capsule on the line above the volume"],
      ["player", "Player", "A big play button between the skip buttons"],
    ],
  },
  mediaControls: {
    title: "Media controls",
    options: [
      ["none", "None", "No media buttons in the flyout"],
      ["banner", "Banner", "A now playing card at the top"],
      ["row", "Row", "Under the selected app"],
      ["dock", "Dock", "A now playing line above the device volume"],
      ["hover", "Hover", "On each app's row while hovered, for that app"],
    ],
  },
  volumeBars: {
    title: "Volume bars",
    options: [
      ["none", "None", "No volume on app rows"],
      ["compact", "Compact", "A small bar to drag or scroll, click to mute"],
      ["always", "Always", "A slider under every app"],
      ["onHover", "On hover", "A percentage that turns into a slider on hover"],
    ],
  },
  levelMeters: {
    title: "Level meters",
    options: [
      ["none", "None", "No live levels"],
      ["volume", "Volume", "The level moves inside the volume control"],
      ["ring", "Ring", "A ring around the app icon lights up"],
      ["bars", "Bars", "Equalizer bars before the app name"],
      ["line", "Line", "A line along the bottom of the row"],
      ["strip", "Strip", "A meter at the row's left edge"],
    ],
  },
};

/** A setting's name, a segmented control of its modes, and a line about the current one. */
function segmented(s, key) {
  const { title, options } = MODES[key];
  const current = options.find(([value]) => value === s[key]);
  return el(
    "div",
    "mode",
    el("span", "mode-title", title),
    el("div", "segmented", ...options.map(([value, name]) => button(s[key] === value ? "on" : "", null, name, () => setting(key, value)))),
    el("span", "mode-about", current?.[2] ?? ""),
  );
}

/** The update status line with a button to check now. */
function updateRow(update) {
  const busy = ["checking", "installing"].includes(update.state.state);
  const check = button("small-button", null, "Check now", () => invoke("check_update"));
  check.disabled = busy;
  return el("div", "fly-setting update", el("span", "summary", update.summary), check);
}

/**
 * Windows 11 moves our icon at once. Windows 10 shows where it is and opens the page
 * to change it, and the page that hides the Windows volume icon.
 */
function iconRows({ iconOnTaskbar: onTaskbar, settings: { speakerPosition }, speakerPositionActive }) {
  const row = (title, control) => el("div", "fly-setting", el("span", "", title), control);
  if (windows11) {
    const speaker = row(speakerPosition && !speakerPositionActive ? "Speaker position (using fallback)" : "Use Windows speaker position", toggle(speakerPosition, (on) => setting("speakerPosition", on)));
    if (speakerPosition) return [speaker];
    return [row("Show icon on the taskbar", toggle(onTaskbar === true, (show) => invoke("set_icon_on_taskbar", { show }))), speaker];
  }
  const place = onTaskbar == null ? "" : onTaskbar ? "On the taskbar" : "In the overflow";
  return [
    row(
      "Our tray icon",
      el("div", "buttons", el("span", "place", place), button("small-button", null, "Change", () => invoke("set_icon_on_taskbar", { show: true }))),
    ),
    row("Windows volume icon", button("small-button", null, "Hide", () => invoke("open_icon_page", { page: "hideWindows" }))),
  ];
}

/** Every setting at once, in two columns beside the flyout. */
function settingsPanel(state) {
  const s = state.settings;
  const row = (title, control) => el("div", "fly-setting", el("span", "", title), control);
  const close = button("icon ghost small", "Close settings", icon("close"), () => {
    page = "apps";
    rerender();
  });
  const left = el(
    "div",
    "panel-column",
    el("div", "section-label", "General"),
    row("Start with Windows", toggle(s.autostart, (on) => setting("autostart", on))),
    row("Album art as app icon", toggle(s.albumArt, (on) => setting("albumArt", on))),
    row("Automatic updates", toggle(s.autoUpdate, (on) => setting("autoUpdate", on))),
    updateRow(state.update),
    ...iconRows(state),
    el("div", "section-label", "Look"),
    el("div", "mode", el("span", "mode-title", "Accent color"), swatches(s.accent)),
    segmented(s, "dark"),
    segmented(s, "trayStyle"),
  );
  const right = el(
    "div",
    "panel-column",
    el("div", "section-label", "Tray tooltip"),
    segmented(s, "tooltip"),
    segmented(s, "mediaButtons"),
    el("div", "section-label", "Flyout"),
    segmented(s, "mediaControls"),
    segmented(s, "volumeBars"),
    segmented(s, "levelMeters"),
  );
  return [
    el("header", "flyout-controls", el("span", "panel-title", "Settings"), close),
    el("div", "panel-body", left, el("div", "panel-divider"), right),
  ];
}

function render(state) {
  latest = state;
  document.body.dataset.meters = state.settings.levelMeters;
  $("controls").replaceChildren(...controls());
  $("dock").replaceChildren(...(state.settings.mediaControls === "dock" ? [nowPlayingView(state, "dock")] : []));
  $("device").replaceChildren(deviceHeader(state));
  $("master").replaceChildren(volumes.sliderRow(MASTER, state.audio.volume, state.audio.muted));

  const panel = page === "settings" || shownPanel;
  $("panel").hidden = !panel;
  $("panel").replaceChildren(...(panel ? settingsPanel(state) : []));

  let middle;
  if (devicesOpen) {
    middle = devices(state);
  } else {
    middle = [
      ...(state.settings.mediaControls === "banner" ? [nowPlayingView(state, "banner")] : []),
      el("div", "section-label", "Media keys go to"),
      ...targets(state),
      // Other apps are listed only for their volume.
      ...(state.mixer.length && state.settings.volumeBars !== "none" ? [el("div", "section-label", "Other apps"), ...mixer(state)] : []),
    ];
  }
  $("middle").replaceChildren(...middle);

  // A newly opened panel joins the window region once it's painted.
  const opening = page === "settings" && !shownPanel;
  reportSize(!opening);
  if (opening) requestAnimationFrame(() => requestAnimationFrame(() => reportSize(true)));
  if (first) {
    first = false;
    invoke("ready");
  }
}

/** Reports the flyout and open panel heights, so the window region shows exactly them. */
function reportSize(withPanel) {
  const flyout = Math.ceil($("column").getBoundingClientRect().height);
  const panel = withPanel && page === "settings" ? Math.ceil($("panel").getBoundingClientRect().height) : null;
  const size = `${flyout}/${panel}`;
  if (size === lastSize) return;
  lastSize = size;
  invoke("flyout_size", { flyout, panel }).then(([left, down, floats]) => {
    document.body.dataset.panel = left ? "left" : "right";
    document.body.dataset.down = down;
    document.body.classList.toggle("floats", floats);
    const hidden = shownPanel && panel === null;
    shownPanel = panel !== null;
    if (hidden) rerender();
  });
}

follow(volumes, render);
window.__TAURI__.event.listen("page", (event) => {
  page = event.payload;
  devicesOpen = false;
  rerender();
});

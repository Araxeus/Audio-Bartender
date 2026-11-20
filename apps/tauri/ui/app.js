import { MASTER, Volumes, avatar, button, detailText, el, follow, icon, invoke, isDark, logo, setting } from "./common.js";

const $ = (id) => document.getElementById(id);
const params = new URLSearchParams(location.search);

let page = location.hash === "#settings" ? "settings" : "apps";
let pinned = params.get("pinned") === "true";
let devicesOpen = false;
let first = true;
let lastHeight = 0;
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
  if (!style || style === "none" || page !== "apps" || devicesOpen || document.hidden) return;
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

/** What the media keys control right now, for the banner and the dock. */
function nowPlaying(state) {
  const t = state.targets.find((t) => t.id === state.selected);
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
  const dark = isDark();
  return [
    el("div", "brand", brandMark(), el("span", "", "Audio Bartender")),
    el(
      "div",
      "transport",
      button("icon ghost small", "Toggle dark mode", icon(dark ? "sun" : "moon"), () => setting("dark", !dark)),
      button("icon ghost small" + (page === "settings" ? " active" : ""), "Settings", icon("settings"), () => {
        page = page === "settings" ? "apps" : "settings";
        devicesOpen = false;
        choice = null;
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
  const auto = row(
    automatic,
    true,
    logo("avatar small"),
    el("div", "text", el("div", "name small", "Automatic"), el("div", "detail", "Windows decides")),
    radio(automatic, null),
  );
  if (automatic && mode === "row") auto.append(el("div", "row-controls", transport(false)));
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

/** Settings with a page of choices: the setting's name, then each value, name and description. */
const CHOICES = {
  dark: {
    title: "Theme",
    options: [
      [null, "System", "Follow Windows"],
      [false, "Light", ""],
      [true, "Dark", ""],
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
      ["volume", "In volume", "The level moves inside the volume control"],
      ["ring", "Icon ring", "A ring around the app icon lights up"],
      ["bars", "Live bars", "Equalizer bars before the app name"],
      ["line", "Underline", "A line along the bottom of the row"],
      ["strip", "Edge strip", "A meter at the row's left edge"],
    ],
  },
};

/** The setting whose choices the settings page shows. */
let choice = null;

function openChoice(key) {
  choice = key;
  rerender();
}

/** A row showing a setting's current choice, which opens its page. */
function nav(s, key) {
  const current = CHOICES[key].options.find(([value]) => value === s[key]);
  return button("fly-setting nav", null, [el("span", "", CHOICES[key].title), el("span", "value", current?.[1] ?? "", icon("chevronRight"))], () =>
    openChoice(key),
  );
}

/** One setting's choices, each with a line about what it does. */
function choicePage(s) {
  const { title, options } = CHOICES[choice];
  const back = button("choice-back", "Back to settings", [icon("chevronLeft"), title], () => openChoice(null));
  return [
    back,
    ...options.map(([value, name, about]) => {
      const on = s[choice] === value;
      return button("choice" + (on ? " on" : ""), null, [el("span", "radio small" + (on ? " checked" : "")), el("span", "choice-text", el("span", "name", name), about ? el("span", "about", about) : null)], () =>
        setting(choice, value),
      );
    }),
  ];
}

function settings(state) {
  const s = state.settings;
  if (choice) return choicePage(s);
  const row = (title, control) => el("div", "fly-setting", el("span", "", title), control);
  return [
    el("div", "section-label", "General"),
    row("Start with Windows", toggle(s.autostart, (on) => setting("autostart", on))),
    row("Album art as app icon", toggle(s.albumArt, (on) => setting("albumArt", on))),
    row(
      "Tray icons",
      el(
        "div",
        "buttons",
        button("small-button", null, "Show ours", () => invoke("open_icon_page", { page: "showOurs" })),
        button("small-button", null, "Hide Windows'", () => invoke("open_icon_page", { page: "hideWindows" })),
      ),
    ),
    el("div", "section-label", "Look"),
    nav(s, "dark"),
    row("Accent color", swatches(s.accent)),
    nav(s, "trayStyle"),
    el("div", "section-label", "Tooltip and flyout"),
    nav(s, "tooltip"),
    nav(s, "mediaButtons"),
    nav(s, "mediaControls"),
    nav(s, "volumeBars"),
    nav(s, "levelMeters"),
  ];
}

function render(state) {
  latest = state;
  document.body.dataset.meters = state.settings.levelMeters;
  $("controls").replaceChildren(...controls());
  $("dock").replaceChildren(...(state.settings.mediaControls === "dock" ? [nowPlayingView(state, "dock")] : []));
  $("device").replaceChildren(deviceHeader(state));
  $("master").replaceChildren(volumes.sliderRow(MASTER, state.audio.volume, state.audio.muted));

  let middle;
  if (devicesOpen) {
    middle = devices(state);
  } else if (page === "settings") {
    middle = settings(state);
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

  // The window fits the content, so report height changes to the host.
  const height = Math.ceil(document.body.getBoundingClientRect().height);
  if (height !== lastHeight) {
    lastHeight = height;
    invoke("flyout_size", { height });
  }
  if (first) {
    first = false;
    invoke("ready");
  }
}

follow(volumes, render);
window.__TAURI__.event.listen("page", (event) => {
  page = event.payload;
  devicesOpen = false;
  choice = null;
  rerender();
});

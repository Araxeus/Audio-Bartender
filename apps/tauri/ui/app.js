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
  if (bars === "onClick") return volumes.pill(id, level, muted);
  if (bars === "compact") return volumes.bar(id, level, muted);
  return null;
}

/** Makes `node` open its slider on click, and appends the slider while open. */
function expandable(node, id, level, muted) {
  node.addEventListener("click", () => {
    if (!volumes.grabbed) volumes.toggle(id);
  });
  if (volumes.expanded === id) node.append(volumes.sliderRow(id, level, muted));
}

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

function controls() {
  const dark = isDark();
  return [
    el("div", "brand", logo("avatar brand-logo"), el("span", "", "Media Keys Selector")),
    el(
      "div",
      "transport",
      button("icon ghost small", "Toggle dark mode", icon(dark ? "sun" : "moon"), () => setting("dark", !dark)),
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
    // With On click, a click on the card opens its slider; otherwise it picks the card.
    const toggles = hasVolume && bars === "onClick";
    const node = row(
      checked,
      true,
      avatar(t.icon, t.name, t.running),
      el("div", "text" + (t.running ? "" : " offline"), el("div", "name small", t.name), trackLine(t, mode)),
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
    if (toggles) expandable(node, t.id, t.volume, t.muted);
    else selectable(node, t.id);
    rows.push(node);
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
      node = row(false, false, icon, el("div", "mixer-name", app.name), volumes.sliderRow(app.id, app.volume, app.muted));
      node.classList.add("always");
    } else {
      node = row(false, bars === "onClick", icon, el("div", "text", el("div", "name small", app.name)), volumeControl(bars, app.id, app.volume, app.muted));
      if (bars === "onClick") expandable(node, app.id, app.volume, app.muted);
    }
    node.classList.add("mixer");
    return node;
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

function choices(options) {
  return el("div", "segmented", ...options.map(([label, selected, action]) => button(selected ? "selected" : "", null, label, action)));
}

function settings(state) {
  const s = state.settings;
  const row = (title, control) => el("div", "fly-setting", el("span", "", title), control);
  return [
    el("div", "section-label", "Settings"),
    row("Start with Windows", toggle(s.autostart, (on) => setting("autostart", on))),
    row("Album art as app icon", toggle(s.albumArt, (on) => setting("albumArt", on))),
    row(
      "Theme",
      choices([
        ["System", s.dark === null, () => setting("dark", null)],
        ["Light", s.dark === false, () => setting("dark", false)],
        ["Dark", s.dark === true, () => setting("dark", true)],
      ]),
    ),
    row(
      "Tray icon",
      choices([
        ["Ring", s.trayStyle === "ring", () => setting("trayStyle", "ring")],
        ["Number", s.trayStyle === "number", () => setting("trayStyle", "number")],
      ]),
    ),
    row("Accent color", swatches(s.accent)),
    row(
      "Tooltip",
      choices([
        ["Now playing", s.tooltip === "nowPlaying", () => setting("tooltip", "nowPlaying")],
        ["Compact", s.tooltip === "compact", () => setting("tooltip", "compact")],
        ["Text", s.tooltip === "text", () => setting("tooltip", "text")],
      ]),
    ),
    row(
      "Media controls",
      choices(
        [
          ["Banner", "banner"],
          ["Row", "row"],
          ["Dock", "dock"],
          ["Hover", "hover"],
        ].map(([label, value]) => [label, s.mediaControls === value, () => setting("mediaControls", value)]),
      ),
    ),
    el(
      "div",
      "fly-setting stacked",
      el("span", "", "Media buttons on the tooltip"),
      choices(
        [
          ["None", "none"],
          ["Hover", "hover"],
          ["Inline", "inline"],
          ["Capsule", "capsule"],
          ["Player", "player"],
        ].map(([label, value]) => [label, s.mediaButtons === value, () => setting("mediaButtons", value)]),
      ),
    ),
    row(
      "Volume bars",
      choices([
        ["Compact", s.volumeBars === "compact", () => setting("volumeBars", "compact")],
        ["Always", s.volumeBars === "always", () => setting("volumeBars", "always")],
        ["On click", s.volumeBars === "onClick", () => setting("volumeBars", "onClick")],
      ]),
    ),
    row(
      "Tray icons",
      el(
        "div",
        "buttons",
        button("small-button", null, "Show ours", () => invoke("open_icon_page", { page: "showOurs" })),
        button("small-button", null, "Hide Windows'", () => invoke("open_icon_page", { page: "hideWindows" })),
      ),
    ),
  ];
}

function render(state) {
  latest = state;
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
      ...(state.mixer.length ? [el("div", "section-label", "Other apps"), ...mixer(state)] : []),
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
  rerender();
});

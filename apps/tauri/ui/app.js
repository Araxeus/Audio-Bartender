import { MASTER, Volumes, avatar, button, detailText, el, follow, icon, invoke, isDark, logo, setting } from "./common.js";

const $ = (id) => document.getElementById(id);
const params = new URLSearchParams(location.search);

let page = location.hash === "#settings" ? "settings" : "apps";
let pinned = params.get("pinned") === "true";
let devicesOpen = false;
let first = true;
let lastHeight = 0;
let latest = null;
const volumes = new Volumes(() => {});

function rerender() {
  if (latest) render(latest);
}

function radio(checked) {
  return el("span", "radio small" + (checked ? " checked" : ""));
}

function row(id, checked, ...children) {
  const node = el("div", "fly-row" + (checked ? " checked" : ""));
  if (id !== undefined) node.addEventListener("click", () => invoke("select", { id }));
  node.append(el("div", "fly-row-main", ...children));
  return node;
}

function controls() {
  const dark = isDark();
  return [
    el(
      "div",
      "transport",
      button("icon ghost small", "Previous track", icon("prev"), () => invoke("press", { key: "previous" })),
      button("icon primary small", "Play / pause", icon("play"), () => invoke("press", { key: "playPause" })),
      button("icon ghost small", "Next track", icon("next"), () => invoke("press", { key: "next" })),
    ),
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

function targets(state) {
  const rows = [
    row(
      null,
      state.selected === null,
      logo("avatar small"),
      el("div", "text", el("div", "name small", "Automatic"), el("div", "detail", "Windows decides")),
      radio(state.selected === null),
    ),
  ];
  for (const t of state.targets) {
    const checked = state.selected === t.id;
    const node = row(
      t.id,
      checked,
      avatar(t.icon, t.name, t.running),
      el("div", "text" + (t.running ? "" : " offline"), el("div", "name small", t.name), detail(t)),
      t.volume != null ? volumes.pill(t.id, t.volume, t.muted) : null,
      radio(checked),
    );
    node.querySelector(".avatar").classList.add("small");
    if (t.volume != null) {
      volumes.wheel(node, t.id);
      if (volumes.expanded === t.id) node.append(volumes.sliderRow(t.id, t.volume, t.muted));
    }
    rows.push(node);
  }
  return rows;
}

function mixer(state) {
  return state.mixer.map((app) => {
    const node = row(undefined, false, avatar(app.icon, app.name), el("div", "text", el("div", "name small", app.name)), volumes.pill(app.id, app.volume, app.muted));
    node.classList.add("mixer");
    node.querySelector(".avatar").classList.add("tiny");
    volumes.wheel(node, app.id);
    if (volumes.expanded === app.id) node.append(volumes.sliderRow(app.id, app.volume, app.muted));
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
  $("device").replaceChildren(deviceHeader(state));
  $("master").replaceChildren(volumes.sliderRow(MASTER, state.audio.volume, state.audio.muted));

  let middle;
  if (devicesOpen) {
    middle = devices(state);
  } else if (page === "settings") {
    middle = settings(state);
  } else {
    middle = [
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

import {
  MASTER,
  Volumes,
  avatar,
  button,
  detailText,
  el,
  follow,
  icon,
  invoke,
  isDark,
  logo,
  setting,
  status,
} from "./common.js";

const appWindow = window.__TAURI__.window.getCurrentWindow();
const $ = (id) => document.getElementById(id);

let page = location.hash === "#settings" ? "settings" : "targets";
let latest = null;
let first = true;
const volumes = new Volumes(() => {});

function radio(checked) {
  return el("span", "radio" + (checked ? " checked" : ""));
}

function card(id, checked, offline) {
  const node = el("div", "option" + (offline ? " offline" : ""));
  node.setAttribute("role", "radio");
  node.setAttribute("aria-checked", String(checked));
  node.tabIndex = 0;
  if (id !== undefined) node.addEventListener("click", () => invoke("select", { id }));
  return node;
}

function automatic(state) {
  const checked = state.selected === null;
  const node = card(null, checked);
  node.append(
    el(
      "div",
      "option-row",
      logo(),
      el("div", "text", el("div", "name", "Automatic"), el("div", "detail", "Windows picks the most recent player")),
      radio(checked),
    ),
  );
  return node;
}

function target(state, t) {
  const checked = state.selected === t.id;
  const node = card(t.id, checked, !t.running);
  const { text, playing } = detailText(t);
  const detail = el("div", "detail");
  if (playing) {
    const bars = el("span", "bars", el("i"), el("i"), el("i"));
    detail.append(el("span", "playing", bars, "Playing"), text.slice("Playing".length));
  } else {
    detail.textContent = text;
  }
  node.append(
    el(
      "div",
      "option-row",
      avatar(t.icon, t.name, t.running),
      el("div", "text", el("div", "name", t.name), detail),
      t.volume != null ? volumes.pill(t.id, t.volume, t.muted) : null,
      radio(checked),
    ),
  );
  if (t.volume != null) {
    volumes.wheel(node, t.id);
    if (volumes.expanded === t.id) node.append(volumes.sliderRow(t.id, t.volume, t.muted));
  }
  return node;
}

function mixerApp(app) {
  const node = el("div", "option mixer");
  node.append(
    el(
      "div",
      "option-row",
      avatar(app.icon, app.name),
      el("div", "text", el("div", "name small", app.name)),
      volumes.pill(app.id, app.volume, app.muted),
    ),
  );
  volumes.wheel(node, app.id);
  if (volumes.expanded === app.id) node.append(volumes.sliderRow(app.id, app.volume, app.muted));
  return node;
}

function targetsPage(state) {
  const items = [automatic(state), ...state.targets.map((t) => target(state, t))];
  if (state.targets.length === 0) {
    items.push(el("div", "hint", "No media apps found. Start playing something and it shows up here."));
  }
  if (state.mixer.length > 0) {
    items.push(el("div", "section-label", "Other apps"), ...state.mixer.map(mixerApp));
  }
  items.push(el("div", "hint", "Scroll over an app to change its volume"));
  return items;
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
  return el(
    "div",
    "segmented",
    ...options.map(([label, selected, action]) => button(selected ? "selected" : "", null, label, action)),
  );
}

function settingsPage(state) {
  const s = state.settings;
  const row = (title, detail, control) =>
    el("div", "setting", el("div", "text", el("div", "name small", title), el("div", "detail", detail)), control);
  return [
    el(
      "div",
      "settings",
      row("Start with Windows", "Launch in the tray when you sign in", toggle(s.autostart, (on) => setting("autostart", on))),
      row(
        "Close button hides to tray",
        "Off: closing the window quits the app",
        toggle(s.closeToTray, (on) => setting("closeToTray", on)),
      ),
      row(
        "In-app title bar",
        "Draw the title bar in the app's theme",
        toggle(s.customTitleBar, (on) => setting("customTitleBar", on)),
      ),
      row(
        "Album art as app icon",
        "Show the playing track's cover instead",
        toggle(s.albumArt, (on) => setting("albumArt", on)),
      ),
      row(
        "Replace the Windows volume icon",
        "Show this icon on the taskbar, then turn Volume off",
        el(
          "div",
          "buttons",
          button("small-button", null, "Show this", () => invoke("open_icon_page", { page: "showOurs" })),
          button("small-button", null, "Hide Windows'", () => invoke("open_icon_page", { page: "hideWindows" })),
        ),
      ),
      row(
        "Theme",
        "Follow Windows or pick one",
        choices([
          ["System", s.dark === null, () => setting("dark", null)],
          ["Light", s.dark === false, () => setting("dark", false)],
          ["Dark", s.dark === true, () => setting("dark", true)],
        ]),
      ),
      row(
        "Tray icon",
        "How the tray shows the volume",
        choices([
          ["Ring", s.trayStyle === "ring", () => setting("trayStyle", "ring")],
          ["Number", s.trayStyle === "number", () => setting("trayStyle", "number")],
        ]),
      ),
    ),
  ];
}

function render(state) {
  latest = state;
  document.documentElement.dataset.titlebar = state.settings.customTitleBar ? "custom" : "native";

  const { routed, text } = status(state);
  $("status").classList.toggle("routed", routed);
  $("status-text").textContent = text;

  const setPage = (next) => {
    page = next;
    render(state);
  };
  $("heading").replaceChildren(
    page === "settings"
      ? el(
          "div",
          "heading-row",
          button("icon ghost", "Back", icon("back"), () => setPage("targets")),
          el("h1", "", "Settings"),
        )
      : el("h1", "", "Media keys go to"),
  );
  const dark = isDark();
  const gear = button("icon ghost" + (page === "settings" ? " active" : ""), "Settings", icon("settings"), () =>
    setPage(page === "settings" ? "targets" : "settings"),
  );
  $("tools").replaceChildren(
    button("icon ghost", "Toggle dark mode", icon(dark ? "sun" : "moon"), () => setting("dark", !dark)),
    gear,
  );

  $("body").replaceChildren(...(page === "settings" ? settingsPage(state) : targetsPage(state)));

  const device = state.audio.devices.find((d) => d.default);
  $("master").replaceChildren(
    el("div", "device-label", icon("speaker"), el("span", "", device?.name ?? "No output device")),
    volumes.sliderRow(MASTER, state.audio.volume, state.audio.muted),
  );
  $("footer-row").replaceChildren(
    el(
      "div",
      "transport",
      button("icon", "Previous track", icon("prev"), () => invoke("press", { key: "previous" })),
      button("icon primary", "Play / pause", icon("play"), () => invoke("press", { key: "playPause" })),
      button("icon", "Next track", icon("next"), () => invoke("press", { key: "next" })),
    ),
    button("ghost text-button", "Quit", [icon("power"), "Quit"], () => invoke("quit")),
  );

  if (first) {
    first = false;
    invoke("ready");
  }
}

$("minimize").addEventListener("click", () => appWindow.minimize());
$("close").addEventListener("click", () => appWindow.close());
follow(volumes, render);
window.__TAURI__.event.listen("page", (event) => {
  page = event.payload;
  if (latest) render(latest);
});

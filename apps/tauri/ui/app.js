const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const list = document.getElementById("list");
const status = document.getElementById("status");
const statusText = document.getElementById("status-text");
const autostart = document.getElementById("autostart");
const themeButton = document.getElementById("theme");
const titleBarButton = document.getElementById("titlebar-toggle");
const appWindow = window.__TAURI__.window.getCurrentWindow();
const systemDark = window.matchMedia("(prefers-color-scheme: dark)");

const AUTO_ICON = '<svg viewBox="0 0 24 24"><path d="M12 2l2.4 6.6L21 11l-6.6 2.4L12 20l-2.4-6.6L3 11l6.6-2.4z"/></svg>';
const SPEAKER = '<svg viewBox="0 0 24 24"><path d="M4 9h4l5-4v14l-5-4H4z"/><path d="M16 8.5a5 5 0 0 1 0 7" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>';
const MUTED = '<svg viewBox="0 0 24 24"><path d="M4 9h4l5-4v14l-5-4H4z"/><path d="M16 9l5 6M21 9l-5 6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>';
const PALETTE = ["#6d5ef5", "#0ea5e9", "#f97316", "#10b981", "#ec4899", "#eab308", "#ef4444", "#14b8a6"];

/** Volume change per wheel notch. */
const STEP = 0.05;

let current = null;

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function colorFor(name) {
  let hash = 0;
  for (const c of name) hash = (hash * 31 + c.charCodeAt(0)) | 0;
  return PALETTE[Math.abs(hash) % PALETTE.length];
}

function isDark() {
  return current?.dark ?? systemDark.matches;
}

function applyTheme() {
  document.documentElement.dataset.theme = isDark() ? "dark" : "light";
}

function detail(target) {
  const line = el("div", "detail");
  if (!target.running) {
    line.textContent = "Not running · keys go to Windows until it starts";
    return line;
  }
  if (target.playback === "playing") {
    const playing = el("span", "playing");
    const bars = el("span", "bars");
    bars.append(el("i"), el("i"), el("i"));
    playing.append(bars, "Playing");
    line.append(playing);
  } else if (target.playback === "paused") {
    line.append("Paused");
  } else if (target.playback === "stopped") {
    line.append("Stopped");
  } else {
    line.append(target.viaWindow ? "Window messages" : "Idle");
  }
  if (target.track) line.append(" · " + target.track);
  return line;
}

function volumeNodes(button, target) {
  const pill = el("span", "volume");
  pill.innerHTML = target.volume === 0 ? MUTED : SPEAKER;
  pill.append(Math.round(target.volume * 100) + "%");

  const bar = el("span", "volume-bar");
  bar.style.width = target.volume * 100 + "%";

  button.title = "Scroll to change volume";
  button.addEventListener(
    "wheel",
    (event) => {
      event.preventDefault();
      // One mouse notch is about 100 px; touchpads send smaller steps.
      invoke("adjust_volume", { id: target.id, delta: (-event.deltaY / 100) * STEP });
    },
    { passive: false },
  );
  return [pill, bar];
}

function option({ id, name, avatar, detailNode, checked, offline, target }) {
  const button = el("button", "option" + (offline ? " offline" : ""));
  button.setAttribute("role", "radio");
  button.setAttribute("aria-checked", String(checked));
  button.addEventListener("click", () => invoke("select", { id }));

  const text = el("div", "text");
  text.append(el("div", "name", name), detailNode);
  button.append(avatar, text);
  if (target?.volume != null) {
    const [pill, bar] = volumeNodes(button, target);
    button.append(pill, el("span", "radio"), bar);
  } else {
    button.append(el("span", "radio"));
  }
  return button;
}

function render(state) {
  current = state;
  applyTheme();
  document.documentElement.dataset.titlebar = state.customTitleBar ? "custom" : "native";
  titleBarButton.setAttribute("aria-pressed", String(state.customTitleBar));

  const routed = state.targets.find((t) => t.id === state.selected && t.running);
  status.classList.toggle("routed", Boolean(routed));
  statusText.textContent = routed
    ? `Captured · sending to ${routed.name}`
    : state.selected
      ? "Waiting for the chosen app · Windows decides meanwhile"
      : "Windows decides";
  autostart.checked = state.autostart;

  const autoAvatar = el("span", "avatar");
  autoAvatar.style.background = "linear-gradient(135deg, #6366f1, #a855f7)";
  autoAvatar.innerHTML = AUTO_ICON;

  const options = [
    option({
      id: null,
      name: "Automatic",
      avatar: autoAvatar,
      detailNode: el("div", "detail", "Windows picks the most recent player"),
      checked: state.selected === null,
    }),
  ];

  for (const target of state.targets) {
    const avatar = el("span", "avatar", target.name.charAt(0).toUpperCase());
    avatar.style.background = colorFor(target.name);
    options.push(
      option({
        id: target.id,
        name: target.name,
        avatar,
        detailNode: detail(target),
        checked: state.selected === target.id,
        offline: !target.running,
        target,
      }),
    );
  }

  if (state.targets.length === 0) {
    options.push(el("div", "empty", "No media apps found. Start playing something and it shows up here."));
  } else if (state.targets.some((t) => t.volume != null)) {
    options.push(el("div", "hint", "Scroll over an app to change its volume"));
  }
  list.replaceChildren(...options);
}

for (const button of document.querySelectorAll("[data-key]")) {
  button.addEventListener("click", () => invoke("press", { key: button.dataset.key }));
}
autostart.addEventListener("change", () => invoke("set_autostart", { on: autostart.checked }));
themeButton.addEventListener("click", () => invoke("set_dark", { dark: !isDark() }));
systemDark.addEventListener("change", applyTheme);
titleBarButton.addEventListener("click", () => invoke("set_custom_title_bar", { on: !current.customTitleBar }));
document.getElementById("minimize").addEventListener("click", () => appWindow.minimize());
document.getElementById("close").addEventListener("click", () => appWindow.close());

listen("state", (event) => render(event.payload));
invoke("state").then((state) => {
  render(state);
  invoke("ready");
});

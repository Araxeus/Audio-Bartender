import { MASTER, Volumes, avatar, button, detailText, el, follow, icon, invoke, logo } from "./common.js";

const $ = (id) => document.getElementById(id);

let devicesOpen = false;
let first = true;
let lastHeight = 0;
const volumes = new Volumes(() => {});

function radio(checked) {
  return el("span", "radio small" + (checked ? " checked" : ""));
}

function row(id, checked, ...children) {
  const node = el("div", "fly-row" + (checked ? " checked" : ""));
  if (id !== undefined) node.addEventListener("click", () => invoke("select", { id }));
  node.append(el("div", "fly-row-main", ...children));
  return node;
}

function deviceHeader(state) {
  const device = state.audio.devices.find((d) => d.default);
  return button(
    "device-header",
    "Choose output device",
    [icon("speaker"), el("span", "device-name", device?.name ?? "No output device"), icon(devicesOpen ? "chevronDown" : "chevronUp")],
    () => {
      devicesOpen = !devicesOpen;
      volumes.rerender();
    },
  );
}

function devices(state) {
  return state.audio.devices.map((device) =>
    button("device" + (device.default ? " current" : ""), null, [el("span", "check", device.default ? icon("check") : null), device.name], () => {
      invoke("set_device", { id: device.id });
      devicesOpen = false;
      volumes.rerender();
    }),
  );
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
    const { text, playing } = detailText(t);
    const node = row(
      t.id,
      checked,
      avatar(t.icon, t.name, t.running),
      el("div", "text" + (t.running ? "" : " offline"), el("div", "name small", t.name), el("div", "detail" + (playing ? " playing" : ""), text)),
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

function render(state) {
  $("device").replaceChildren(deviceHeader(state));
  $("master").replaceChildren(volumes.sliderRow(MASTER, state.audio.volume, state.audio.muted));

  const middle = devicesOpen
    ? devices(state)
    : [
        el("div", "section-label", "Media keys go to"),
        ...targets(state),
        ...(state.mixer.length ? [el("div", "section-label", "Other apps"), ...mixer(state)] : []),
      ];
  $("middle").replaceChildren(...middle);

  $("controls").replaceChildren(
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
      button("icon ghost small", "Open Media Keys Selector", icon("window"), () => invoke("open_main")),
      button("icon ghost small", "Quit", icon("power"), () => invoke("quit")),
    ),
  );

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

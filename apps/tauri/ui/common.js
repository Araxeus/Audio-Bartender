// Shared by the main window and the tray flyout.

export const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

/** Volume change per wheel notch. */
const WHEEL_STEP = 0.02;

/** Slider id of the master volume. */
export const MASTER = "master";

const PALETTE = ["#6d5ef5", "#0ea5e9", "#f97316", "#10b981", "#ec4899", "#eab308", "#ef4444", "#14b8a6"];

// Lucide-style stroke icons, drawn on a 24-unit grid.
const SPEAKER_BODY =
  '<path d="M11 4.7a.7.7 0 0 0-1.2-.5L6.4 7.6A1.4 1.4 0 0 1 5.4 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.4a1.4 1.4 0 0 1 1 .4l3.4 3.4a.7.7 0 0 0 1.2-.5z"/>';
const PIN =
  '<path d="M12 17v5"/><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>';
const STROKES = {
  volume: SPEAKER_BODY,
  volume1: SPEAKER_BODY + '<path d="M16 9a5 5 0 0 1 0 6"/>',
  volume2: SPEAKER_BODY + '<path d="M16 9a5 5 0 0 1 0 6"/><path d="M19.4 5.6a9 9 0 0 1 0 12.8"/>',
  mute: SPEAKER_BODY + '<path d="M22 9l-6 6M16 9l6 6"/>',
  check: '<path d="M20 6 9 17l-5-5"/>',
  chevronDown: '<path d="m6 9 6 6 6-6"/>',
  chevronUp: '<path d="m18 15-6-6-6 6"/>',
  chevronLeft: '<path d="m15 18-6-6 6-6"/>',
  chevronRight: '<path d="m9 18 6-6-6-6"/>',
  close: '<path d="M18 6 6 18M6 6l12 12"/>',
  headphones: '<path d="M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 18 0v7a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3"/>',
  tv: '<rect width="20" height="15" x="2" y="7" rx="2"/><path d="m17 2-5 5-5-5"/>',
  plus: '<path d="M5 12h14M12 5v14"/>',
  settings: '<path d="M20 7h-9M14 17H5"/><circle cx="17" cy="17" r="3"/><circle cx="7" cy="7" r="3"/>',
  power: '<path d="M12 2v10"/><path d="M18.4 6.6a9 9 0 1 1-12.77.04"/>',
  moon: '<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>',
  sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M6.3 17.7l-1.4 1.4M19.1 4.9l-1.4 1.4"/>',
  speaker: '<rect width="16" height="20" x="4" y="2" rx="2"/><circle cx="12" cy="14" r="4"/><path d="M12 6h.01"/>',
  prev: '<path d="M19 20 9 12l10-8zM5 19V5"/>',
  next: '<path d="m5 4 10 8-10 8zM19 5v14"/>',
  // Shifted so its left and right edges sit the same distance from the button's.
  play: '<path d="M5.25 3l14 9-14 9z"/>',
  pause: '<rect x="6" y="4" width="4" height="16" rx="1"/><rect x="14" y="4" width="4" height="16" rx="1"/>',
  pin: PIN,
  pinOff: PIN + '<path d="M2 2l20 20"/>',
};
const SPARKLES = '<path fill="currentColor" d="M12 2l2.4 6.6L21 11l-6.6 2.4L12 20l-2.4-6.6L3 11l6.6-2.4z"/>';

export function icon(name) {
  const span = document.createElement("span");
  span.className = "icon-glyph";
  span.innerHTML =
    name === "sparkles"
      ? `<svg viewBox="0 0 24 24">${SPARKLES}</svg>`
      : `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${STROKES[name]}</svg>`;
  return span;
}

export function el(tag, className, ...children) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  node.append(...children.filter((c) => c != null));
  return node;
}

/** A button that runs `action` without also triggering the row it sits in. */
export function button(className, title, content, action) {
  const node = el("button", className, ...[content].flat());
  if (title) {
    node.dataset.tip = title;
    node.setAttribute("aria-label", title);
  }
  node.addEventListener("click", (event) => {
    event.stopPropagation();
    action(event);
  });
  return node;
}

function colorFor(name) {
  let hash = 0;
  for (const c of name) hash = (hash * 31 + c.charCodeAt(0)) | 0;
  return PALETTE[Math.abs(hash) % PALETTE.length];
}

/** The app's icon (or album art), else its first letter on a colored square. */
export function avatar(image, name, running = true) {
  const box = el("span", "avatar" + (running ? "" : " offline"));
  if (image) {
    const img = el("img", image.art ? "art" : "app-icon");
    img.src = image.url;
    img.alt = "";
    box.append(img);
  } else {
    box.textContent = name.charAt(0).toUpperCase();
    box.style.background = colorFor(name);
  }
  return box;
}

export function logo(className = "avatar") {
  const box = el("span", className + " logo", icon("sparkles"));
  return box;
}

/** The speaker of a volume control, or the icon of the device the app plays on while it isn't muted. */
function levelIcon(level, muted, device) {
  if (device && !muted) {
    const node = icon(device);
    node.classList.add("routed");
    return node;
  }
  return icon(volumeIcon(level, muted));
}

function volumeIcon(level, muted) {
  if (muted || level === 0) return "mute";
  if (level < 0.34) return "volume";
  if (level < 0.67) return "volume1";
  return "volume2";
}

let current = null;
const systemDark = window.matchMedia("(prefers-color-scheme: dark)");

export function isDark() {
  return current?.settings.dark ?? systemDark.matches;
}

const hex = (rgb) => "#" + rgb.toString(16).padStart(6, "0");
const rgba = (rgb, alpha) => `rgba(${rgb >> 16}, ${(rgb >> 8) & 255}, ${rgb & 255}, ${alpha})`;

function applyTheme() {
  const dark = isDark();
  const root = document.documentElement;
  root.dataset.theme = dark ? "dark" : "light";
  // The core owns the accent palette; the page only maps it onto CSS variables.
  const colors = current?.settings.accent[dark ? "dark" : "light"];
  if (!colors) return;
  root.style.setProperty("--accent", hex(colors.accent));
  root.style.setProperty("--accent-soft", rgba(colors.accent, dark ? 0.13 : 0.1));
  root.style.setProperty("--glow", dark ? rgba(colors.accent, 0.1) : "transparent");
  root.style.setProperty("--logo-from", hex(colors.gradient[0]));
  root.style.setProperty("--logo-to", hex(colors.gradient[1]));
}

/** Volume pills, sliders and wheel handling for one page. `rerender` redraws the page. */
export class Volumes {
  constructor(rerender) {
    this.rerender = rerender;
    this.dragging = false;
    // Whether the current press started on a volume control, so the card under it
    // ignores the click. Every press resets it before reaching a control.
    this.grabbed = false;
    document.addEventListener("pointerdown", () => (this.grabbed = false), true);
  }

  /** Marks presses on `node` as volume control presses. */
  grab(node) {
    node.addEventListener("pointerdown", () => (this.grabbed = true));
  }

  setVolume(id, level) {
    if (id === MASTER) invoke("set_master", { level });
    else invoke("set_volume", { id, level });
  }

  toggleMute(id) {
    if (id === MASTER) invoke("toggle_master_mute");
    else invoke("toggle_mute", { id });
  }

  /**
   * Scrolling over `node` changes `id`'s volume. The wheel scrolls the list
   * everywhere else, so only volume controls get this.
   */
  wheel(node, id) {
    node.addEventListener(
      "wheel",
      (event) => {
        event.preventDefault();
        event.stopPropagation();
        // One mouse notch is about 100 px; touchpads send smaller steps.
        const delta = (-event.deltaY / 100) * WHEEL_STEP;
        if (id === MASTER) invoke("adjust_master", { delta });
        else invoke("adjust_volume", { id, delta });
      },
      { passive: false },
    );
  }

  /** The "80%" on an app row, which turns into a slider while the row is hovered. */
  hoverControl(id, level, muted, device = null) {
    const pill = el("span", "pill", levelIcon(level, muted, device), muted ? "Muted" : `${Math.round(level * 100)}%`);
    const node = el("span", "hover-volume", pill, this.range(id, level));
    this.wheel(node, id);
    this.grab(node);
    return node;
  }

  /**
   * A small bar that fills with the volume. Dragging sets it, the wheel nudges
   * it, and a click without dragging toggles mute.
   */
  bar(id, level, muted, device = null) {
    const node = el("span", "bar" + (muted ? " muted" : ""), levelIcon(level, muted, device), muted ? "Muted" : `${Math.round(level * 100)}%`);
    node.style.setProperty("--fill", `${Math.round(level * 100)}%`);
    node.title = "Drag or scroll to change, click to mute";
    this.wheel(node, id);
    this.grab(node);

    let start = null;
    const levelAt = (event) => {
      const rect = node.getBoundingClientRect();
      const x = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
      return Math.round(x * 50) / 50;
    };
    node.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      start = { x: event.clientX, dragging: false };
      node.setPointerCapture(event.pointerId);
      event.stopPropagation();
    });
    node.addEventListener("pointermove", (event) => {
      if (!start) return;
      if (!start.dragging && Math.abs(event.clientX - start.x) <= 2) return;
      if (!start.dragging) {
        start.dragging = true;
        this.dragging = true;
      }
      const value = levelAt(event);
      node.style.setProperty("--fill", `${value * 100}%`);
      node.lastChild.textContent = `${Math.round(value * 100)}%`;
      invoke("set_volume", { id, level: value });
    });
    const end = () => {
      if (!start) return;
      const clicked = !start.dragging;
      start = null;
      if (clicked) {
        this.toggleMute(id);
      } else {
        this.dragging = false;
        this.rerender();
      }
    };
    node.addEventListener("pointerup", end);
    node.addEventListener("pointercancel", end);
    node.addEventListener("click", (event) => event.stopPropagation());
    return node;
  }

  /** Mute button, slider and value, for the master volume or an app row. */
  sliderRow(id, level, muted) {
    const row = el("div", "slider-row");
    const mute = button("mute" + (muted ? " muted" : ""), muted ? "Unmute" : "Mute", icon(volumeIcon(level, muted)), () =>
      this.toggleMute(id),
    );
    const range = this.range(id, level);
    const value = el("span", "value", range.value);
    range.addEventListener("input", () => (value.textContent = range.value));
    row.append(mute, range, value);
    this.wheel(row, id);
    this.grab(row);
    return row;
  }

  /** A volume slider. Renders wait until it's released. */
  range(id, level) {
    const range = el("input", "range");
    range.type = "range";
    range.min = "0";
    range.max = "100";
    range.value = String(Math.round(level * 100));
    range.style.setProperty("--fill", `${range.value}%`);
    range.addEventListener("click", (event) => event.stopPropagation());
    range.addEventListener("pointerdown", () => (this.dragging = true));
    range.addEventListener("input", () => {
      range.style.setProperty("--fill", `${range.value}%`);
      this.setVolume(id, Number(range.value) / 100);
    });
    const release = () => {
      if (!this.dragging) return;
      this.dragging = false;
      this.rerender();
    };
    range.addEventListener("pointerup", release);
    range.addEventListener("change", release);
    return range;
  }
}

/** Calls `render(state)` now and on every change, holding renders while a slider drags. */
export function follow(volumes, render) {
  const draw = () => {
    applyTheme();
    if (!volumes.dragging) render(current);
  };
  volumes.rerender = draw;
  systemDark.addEventListener("change", draw);
  listen("state", (event) => {
    current = event.payload;
    draw();
  });
  return invoke("state").then((state) => {
    current = state;
    draw();
  });
}

export function setting(name, value) {
  invoke("set_setting", { setting: { [name]: value } });
}


/** "Playing · Title — Artist" and friends. */
export function detailText(target) {
  if (!target.running) return { text: "Not running · keys go to Windows until it starts", playing: false };
  const playing = target.playback === "playing";
  const label =
    { playing: "Playing", paused: "Paused", stopped: "Stopped" }[target.playback] ??
    (target.viaWindow ? "Window messages" : "Idle");
  return { text: target.track ? `${label} · ${target.track}` : label, playing };
}

/* Tooltips like GPUI's: they show after half a second, below the control and inside
   its card, because the window region clips anything outside the cards. Native title
   tooltips wait longer. */

const TIP_DELAY = 500;
const tip = el("div", "tip");
tip.hidden = true;
document.body.append(tip);
let tipTimer = 0;
let pointer = { x: 0, y: 0 };

/** The control with a tooltip under the pointer, after any re-render. */
function tipTarget() {
  return document.elementFromPoint(pointer.x, pointer.y)?.closest("[data-tip]") ?? null;
}

function hideTip() {
  clearTimeout(tipTimer);
  tip.hidden = true;
}

function showTip(node) {
  tip.textContent = node.dataset.tip;
  tip.hidden = false;
  const at = node.getBoundingClientRect();
  const card = (node.closest(".flyout-column, .settings-panel") ?? document.body).getBoundingClientRect();
  const { width, height } = tip.getBoundingClientRect();
  const left = Math.min(Math.max(at.left + at.width / 2 - width / 2, card.left + 4), card.right - width - 4);
  const below = at.bottom + 6;
  const top = below + height <= card.bottom - 4 ? below : at.top - height - 6;
  tip.style.left = `${left}px`;
  tip.style.top = `${top}px`;
}

document.addEventListener("pointermove", (event) => {
  pointer = { x: event.clientX, y: event.clientY };
  const node = tipTarget();
  if (!node) return hideTip();
  if (!tip.hidden && tip.textContent === node.dataset.tip) return;
  hideTip();
  tipTimer = setTimeout(() => {
    const still = tipTarget();
    if (still) showTip(still);
  }, TIP_DELAY);
});
document.addEventListener("pointerdown", hideTip);
document.addEventListener("pointerleave", hideTip);
// A re-render can swap the control or change its label, so keep the tip in step.
setInterval(() => {
  if (tip.hidden) return;
  const node = tipTarget();
  if (!node) hideTip();
  else if (node.dataset.tip !== tip.textContent) showTip(node);
}, 200);

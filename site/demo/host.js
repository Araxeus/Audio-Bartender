// A simulated Windows 11 desktop: a taskbar with Audio Bartender's tray icon, and the
// real flyout (apps/tauri/ui, driven by mock.js) in an iframe above it.
//
//   <script src="demo/trayicon.js"></script>
//   <script src="demo/host.js"></script>
//   AudioBartenderDemo.mount(element, { theme: "auto" | "dark" | "light" })

(function () {
  const base = new URL(".", document.currentScript.src).href;
  const TASKBAR = 48;
  // Tall enough for the settings panel, so the desktop never changes size.
  const HEIGHT = 800;
  const FLYOUT = 376;
  const PANEL = 648;
  // The narrowest screen that shows the flyout with the settings panel beside it.
  const MIN_WIDE = FLYOUT + PANEL + 16;
  const GAP = 12;
  // The card's gap to the taskbar, how long a changed setting previews it, and how
  // long the pointer may be off both icon and card before it hides, as in the core.
  const CARD_GAP = 4;
  const PREVIEW_MS = 4000;
  const CARD_GRACE_MS = 300;

  const css = `
.abd [hidden] { display: none !important; }
.abd { position: relative; width: 100%; overflow: hidden; border-radius: 14px; user-select: none; -webkit-user-select: none; }
.abd-screen { position: absolute; left: 0; top: 0; transform-origin: 0 0; overflow: hidden; font: 12px "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;
  --tb: rgba(32, 32, 32, .82); --tb-fg: #fff; --tb-hover: rgba(255,255,255,.08); --tb-line: rgba(255,255,255,.08);
  --wall-a: #0b3a4a; --wall-b: #1b1840; --wall-c: #06b6d4; --wall-d: #7c3aed; --card: #2b2b2b; --card-fg: #fff; --card-muted: #b5b5b5; }
.abd-screen.light { --tb: rgba(238, 240, 243, .85); --tb-fg: #1c1c1c; --tb-hover: rgba(0,0,0,.06); --tb-line: rgba(0,0,0,.08);
  --wall-a: #bfe6f0; --wall-b: #dcd6fb; --wall-c: #22d3ee; --wall-d: #a78bfa; --card: #f9f9f9; --card-fg: #1c1c1c; --card-muted: #5f5f5f; }
.abd-wall { position: absolute; inset: 0; background:
  radial-gradient(60% 70% at 78% 85%, color-mix(in srgb, var(--wall-c) 55%, transparent), transparent 70%),
  radial-gradient(50% 60% at 20% 20%, color-mix(in srgb, var(--wall-d) 45%, transparent), transparent 70%),
  linear-gradient(135deg, var(--wall-a), var(--wall-b)); }
.abd-wall::after { content: ""; position: absolute; inset: 0; background:
  radial-gradient(closest-side, rgba(255,255,255,.12), transparent) 70% 70% / 900px 900px no-repeat; }
.abd-frame { position: absolute; bottom: ${TASKBAR}px; overflow: hidden; pointer-events: none; }
.abd-frame iframe { display: block; width: 100%; height: 100%; border: 0; background: transparent; pointer-events: auto;
  transform: translateY(0); transition: transform .15s cubic-bezier(.2,.8,.3,1), visibility 0s; }
/* Hidden once it slides away, so a closed flyout can never leave a backdrop behind. */
.abd-frame.closed iframe { transform: translateY(105%); visibility: hidden; pointer-events: none;
  transition: transform .09s cubic-bezier(.2,.8,.3,1), visibility 0s .09s; }
.abd-frame.free { overflow: visible; }
.abd-bar { position: absolute; left: 0; right: 0; bottom: 0; height: ${TASKBAR}px; display: flex; align-items: center; justify-content: center;
  background: var(--tb); backdrop-filter: blur(30px) saturate(1.4); border-top: 1px solid var(--tb-line); color: var(--tb-fg); z-index: 2; }
.abd-apps { display: flex; gap: 4px; }
.abd-screen.narrow .abd-bar { justify-content: flex-start; gap: 4px; padding-inline: 8px; }
.abd-screen.narrow .abd-apps { overflow: hidden; }
.abd-screen.narrow .abd-app.optional { display: none; }
.abd-app { position: relative; width: 40px; height: 40px; border-radius: 6px; display: grid; place-items: center; }
.abd-app:hover, .abd-tray > *:hover { background: var(--tb-hover); }
.abd-app img { width: 24px; height: 24px; display: block; }
.abd-app svg { width: 22px; height: 22px; display: block; }
/* Windows 11's mark under an app that's open. */
.abd-app.run::after { content: ""; position: absolute; left: 50%; bottom: 2px; width: 6px; height: 3px; margin-left: -3px; border-radius: 2px; background: color-mix(in srgb, var(--tb-fg) 55%, transparent); }
.abd-tray { position: absolute; right: 8px; top: 0; bottom: 0; display: flex; align-items: center; gap: 2px; }
.abd-screen.narrow .abd-tray { position: static; flex: none; margin-left: auto; }
.abd-tray > * { height: 40px; min-width: 32px; border-radius: 6px; display: grid; place-items: center; padding: 0 6px; }
.abd-tray svg { width: 16px; height: 16px; }
.abd-icon { cursor: default; position: relative; }
.abd-icon canvas { width: 20px; height: 20px; display: block; }
/* Until the tray icon is clicked: a soft glow on it and a ring that pulses out. */
.abd-icon.hint { background: color-mix(in srgb, var(--wall-c) 22%, transparent); box-shadow: 0 0 16px color-mix(in srgb, var(--wall-c) 55%, transparent); }
.abd-icon.hint::after { content: ""; position: absolute; inset: 0; border-radius: 6px; box-shadow: 0 0 0 2px var(--wall-c); animation: abd-ping 1.4s ease-out infinite; pointer-events: none; }
@keyframes abd-ping { 0% { opacity: 1; transform: scale(1); } 100% { opacity: 0; transform: scale(1.7); } }
@media (prefers-reduced-motion: reduce) { .abd-icon.hint::after { animation: none; } }
.abd-clock { text-align: right; line-height: 1.3; font-size: 12px; padding: 0 8px; font-variant-numeric: tabular-nums; }
/* The tray tooltip: the core's card (crates/core/src/card.rs), or Windows' plain tooltip. */
.abd-tip { position: absolute; bottom: ${TASKBAR + CARD_GAP}px; z-index: 3; pointer-events: none; }
.abd-tip.card { pointer-events: auto; }
.abd-plain { padding: 5px 9px 6px; border-radius: 7px; white-space: pre; line-height: 16px; background: var(--card); color: var(--card-fg);
  border: 1px solid var(--tb-line); box-shadow: 0 4px 12px rgba(0,0,0,.25); }
.abd-card { position: relative; border-radius: 8px; overflow: hidden; background: var(--c-card); color: var(--c-text);
  font: 12px "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif; box-shadow: inset 0 0 0 1px var(--c-border), 0 2px 9px rgba(0,0,0,var(--c-shadow)); }
.abd-card > *, .abd-card .abd-reveal > * { position: absolute; box-sizing: border-box; }
.abd-card .abd-line { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.abd-card .abd-status { display: flex; align-items: center; white-space: nowrap; overflow: hidden; }
.abd-card .abd-status > :last-child { overflow: hidden; text-overflow: ellipsis; min-width: 0; }
.abd-card .abd-bars { position: relative; flex: none; width: 16px; height: 18px; }
.abd-card .abd-bars i { position: absolute; bottom: 4px; width: 2px; height: 3px; background: var(--c-playing); animation: abd-bar 1s ease-in-out infinite; }
@keyframes abd-bar { 0%, 100% { height: 3px; } 50% { height: 10px; } }
@media (prefers-reduced-motion: reduce) { .abd-card .abd-bars i { animation: none; height: 7px; } }
.abd-card .abd-art { object-fit: cover; }
.abd-card .abd-tile { display: grid; place-items: center; color: #fff; font-weight: 600; }
.abd-reveal { position: absolute; inset: 0; opacity: 0; transition: opacity .12s; pointer-events: none; }
.abd-tip.hot .abd-reveal, .abd-tip.drag .abd-reveal { opacity: 1; }
.abd-tip.hot .abd-reveal > .abd-cb { pointer-events: auto; }
.abd-cb { display: grid; place-items: center; border: 0; padding: 0; margin: 0; background: transparent; color: var(--c-muted); cursor: default; }
.abd-cb svg { display: block; }
.abd-cb.muted:hover { background: var(--c-hot); color: var(--c-text); }
.abd-cb.muted:active { background: var(--c-press); }
.abd-cb.accent { color: var(--c-accent); }
.abd-cb.accent:hover { background: var(--c-hot); }
.abd-cb.filled { background: var(--c-accent); color: #fff; }
.abd-cb.filled:hover { background: color-mix(in srgb, var(--c-accent) 85%, #fff); }
.abd-cb.filled:active { background: color-mix(in srgb, var(--c-accent) 88%, #000); }
.abd-cb.filled.big { box-shadow: 0 3px 8px color-mix(in srgb, var(--c-accent) 35%, transparent); }
.abd-cb.overlay { color: #fff; }
.abd-cb.overlay:hover { background: rgba(255,255,255,.16); }
.abd-vol .abd-track { position: absolute; left: 0; right: 0; top: 6px; height: 4px; border-radius: 2px; background: var(--c-track); overflow: hidden; }
.abd-vol .abd-track i { position: absolute; left: 0; top: 0; bottom: 0; border-radius: inherit; }
.abd-vol:hover .abd-track, .abd-tip.drag .abd-track { top: 5px; height: 6px; border-radius: 3px; }
.abd-vol .abd-thumb { position: absolute; top: 1px; width: 14px; height: 14px; border-radius: 7px; background: var(--c-card); display: none; }
.abd-vol .abd-thumb i { position: absolute; inset: 1.5px; border-radius: 50%; }
.abd-vol:hover .abd-thumb, .abd-tip.drag .abd-thumb { display: block; }
.abd-notice { position: absolute; left: 50%; top: 18px; transform: translate(-50%, -8px); background: var(--card); color: var(--card-fg); padding: 8px 14px;
  border-radius: 8px; box-shadow: 0 8px 24px rgba(0,0,0,.3); opacity: 0; transition: opacity .2s, transform .2s; pointer-events: none; z-index: 4; white-space: nowrap; }
.abd-notice.show { opacity: 1; transform: translate(-50%, 0); }
.abd-shortcut { position: absolute; left: 22px; top: 22px; width: 84px; display: grid; justify-items: center; gap: 6px; padding: 8px 4px; border-radius: 6px;
  color: #fff; text-shadow: 0 1px 3px rgba(0,0,0,.6); text-align: center; cursor: default; border: 0; background: none; font: inherit; }
.abd-shortcut:hover { background: rgba(255,255,255,.15); }
.abd-shortcut img { width: 44px; height: 44px; }
`;

  // `run` marks an open app, and `optional` hides it on the narrow taskbar.
  const APPS = [
    ["explorer", "File Explorer", false],
    ["chrome", "Google Chrome", true],
    ["spotify", "Spotify", true],
    ["discord", "Discord", true],
    ["steam", "Steam", true, true],
    ["powershell", "Windows PowerShell", false, true],
  ];
  // The Start button: four tiles in Windows 11's blue.
  const START = `<svg viewBox="0 0 24 24" aria-hidden="true"><defs><linearGradient id="abd-start" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#1fa7f5"/><stop offset="1" stop-color="#0567d4"/></linearGradient></defs><path fill="url(#abd-start)" d="M2 2h9.6v9.6H2zM12.4 2H22v9.6h-9.6zM2 12.4h9.6V22H2zM12.4 12.4H22V22h-9.6z"/></svg>`;

  const svg = (inner) => `<svg viewBox="0 0 24 24">${inner}</svg>`;

  // The tooltip card, laid out in the same DIPs as crates/core/src/card.rs.
  const PALETTES = {
    dark: { card: "#1c1c25", border: "#2f2f3d", text: "#ececf3", muted: "#8e8ea3", track: "#33333f", playing: "#4ade80", danger: "#f87171" },
    light: { card: "#ffffff", border: "#e4e4ea", text: "#1b1b22", muted: "#6b6b78", track: "#dcdce4", playing: "#16a34a", danger: "#dc2626" },
  };
  const AVATAR_COLORS = [0x6d5ef5, 0x0ea5e9, 0xf97316, 0x10b981, 0xec4899, 0xeab308, 0xef4444, 0x14b8a6];
  const NOW_PLAYING_WIDTH = 300;
  const COMPACT_WIDTH = 260;
  const DOCK = 102;
  const ART = 56;
  const INLINE = 74;

  const hex = (n) => "#" + n.toString(16).padStart(6, "0");
  const esc = (s) => String(s).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
  const at = (l, t, w, h) => `left:${l}px;top:${t}px;width:${w}px;height:${h}px;`;
  const avatarColor = (name) => {
    const hash = [...name].reduce((h, c) => (Math.imul(h, 31) + c.charCodeAt(0)) | 0, 0);
    return hex(AVATAR_COLORS[Math.abs(hash) % AVATAR_COLORS.length]);
  };

  /** A media key's icon in a 16-unit box, drawn at `size`. */
  function keyIcon(key, playing, size) {
    const shapes = {
      playPause: playing
        ? '<rect x="3" y="2" width="3.6" height="12" rx="1.2"/><rect x="9.4" y="2" width="3.6" height="12" rx="1.2"/>'
        : '<path d="M3.15 2.4 12.85 8 3.15 13.6z"/>',
      previous: '<rect x="2" y="2.5" width="2" height="11" rx="1"/><path d="M14 2.8 5 8l9 5.2z"/>',
      next: '<rect x="12" y="2.5" width="2" height="11" rx="1"/><path d="M2 2.8 11 8l-9 5.2z"/>',
    };
    return `<svg viewBox="0 0 16 16" width="${size}" height="${size}" fill="currentColor">${shapes[key]}</svg>`;
  }

  /** The speaker glyph, with waves for the level or a cross when muted. */
  function volumeIcon(level, muted) {
    const waves = muted || level === 0 ? '<path d="M11 6l4 4m0-4-4 4"/>'
      : ['<path d="M10.6 6.2a2.6 2.6 0 0 1 0 3.6"/>', '<path d="M12.4 4.4a5.2 5.2 0 0 1 0 7.2"/>', '<path d="M14.2 2.6a7.8 7.8 0 0 1 0 10.8"/>']
          .slice(0, level < 0.34 ? 1 : level < 0.67 ? 2 : 3).join("");
    return `<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" stroke-linejoin="round"><path d="M1.5 6h2.6L7.5 3v10L4.1 10H1.5z"/>${waves}</svg>`;
  }

  /** Title and subtitle of the Now Playing card. */
  function titles(info) {
    if (!info.app) return ["Automatic", "Media keys go where Windows decides"];
    if (!info.running || !info.track) return [info.app, null];
    const split = info.track.indexOf(" — ");
    return split < 0 ? [info.track, null] : [info.track.slice(0, split), info.track.slice(split + 3)];
  }

  /** On the Now Playing card: the divider's offset from the top, and its gap to the volume line. */
  const divider = (buttons) => (buttons === "capsule" ? [86, 18] : buttons === "player" ? [120, 11] : [80, 11]);

  /** Offset of the status line from the top of the art, with or without a subtitle above it. */
  const statusTop = (subtitle) => (ART - (20 + 18 * (subtitle ? 2 : 1))) / 2 + 20 + (subtitle ? 18 : 0);

  /** Previous, play/pause and next, centered on `cx`, `gap` apart. A spec is [w, h, radius, tone, icon]. */
  function trio(cx, cy, gap, skip, main) {
    const make = (key, x, [w, h, radius, tone, icon]) => ({ key, l: x - w / 2, t: cy - h / 2, w, h, radius, tone, icon });
    const off = main[0] / 2 + gap + skip[0] / 2;
    return [make("previous", cx - off, skip), make("playPause", cx, main), make("next", cx + off, skip)];
  }

  /** The card's markup and size, for what `mock.js` reports as the tooltip. */
  function card(info) {
    const p = PALETTES[info.dark ? "dark" : "light"];
    const accent = hex(info.accent);
    // An app that isn't running has nothing to control.
    const style = !info.app || info.running ? info.buttons : "none";
    const compact = info.style === "compact";
    const [width, height] = compact
      ? style === "capsule" ? [COMPACT_WIDTH, 108] : style === "player" ? [COMPACT_WIDTH + 70, 78] : [COMPACT_WIDTH, 78]
      : [NOW_PLAYING_WIDTH, divider(style)[0] + divider(style)[1] + 43];
    const center = width / 2;
    const playing = info.playback === "playing" && info.running;
    const parts = [];
    const reveal = [];

    const text = (l, t, w, h, value, css = "") =>
      `<div class="abd-line" style="${at(l, t, w, h)}line-height:${h}px;${css}">${esc(value)}</div>`;

    function avatar(x, y, size) {
      const radius = Math.max(size * 0.12, 4);
      const box = `${at(x, y, size, size)}border-radius:${radius}px;`;
      if (!info.app) {
        const s = size > 30 ? 22 : 11;
        return `<div class="abd-tile" style="${box}background:${accent}"><svg viewBox="0 0 16 16" width="${s}" height="${s}" fill="currentColor"><path d="M6 12.4a2.1 2.1 0 1 1-1.2-1.9V3.2L13 1.6v9.2a2.1 2.1 0 1 1-1.2-1.9V4.4L6 5.6z"/></svg></div>`;
      }
      if (info.icon?.art) return `<img class="abd-art" src="${esc(info.icon.url)}" alt="" style="${box}">`;
      if (info.icon) {
        const inset = size * 0.1;
        return `<img src="${esc(info.icon.url)}" alt="" style="${at(x + inset, y + inset, size - 2 * inset, size - 2 * inset)}object-fit:contain">`;
      }
      return `<div class="abd-tile" style="${box}background:${avatarColor(info.app)};font-size:${size > 30 ? 20 : 11}px">${esc(info.app[0].toUpperCase())}</div>`;
    }

    /** "▮▮▮ Playing". `lead` puts the app name first; `trail` adds " · App" after the state. */
    function status(x, y, right, lead, trail) {
      const box = `class="abd-status" style="${at(x, y, right - x, 18)}"`;
      if (!info.app) return `<div ${box}><span style="color:${p.muted}">Media keys: Automatic</span></div>`;
      const out = lead ? [`<b style="font-weight:600;margin-right:6px;flex:0 1 auto;overflow:hidden;text-overflow:ellipsis">${esc(info.app)}</b>`] : [];
      if (!info.running) out.push(`<span style="color:${p.muted}">Not running</span>`);
      else {
        const isPlaying = info.playback === "playing";
        if (isPlaying) out.push(`<span class="abd-bars">${[0, 0.6, 0.3].map((o, i) => `<i style="left:${i * 4}px;animation-delay:-${o}s"></i>`).join("")}</span>`);
        const state = { playing: "Playing", paused: "Paused", stopped: "Stopped" }[info.playback] ?? "Idle";
        out.push(`<span><span style="color:${isPlaying ? p.playing : p.muted}">${state}</span>${trail ? `<span style="color:${p.muted}"> · ${esc(info.app)}</span>` : ""}</span>`);
      }
      return `<div ${box}>${out.join("")}</div>`;
    }

    /** Glyph, device and percentage on one line, then the volume bar. */
    function volume(x, y, right) {
      const level = Math.min(1, Math.max(0, info.level));
      const tint = info.muted ? p.danger : p.text;
      const fill = info.muted ? p.muted : accent;
      const w = right - x;
      const thumb = Math.min(Math.max(level * w, 6), w - 6) - 7;
      parts.push(
        `<div style="${at(x, y, 18, 20)}display:grid;place-items:center;color:${tint}">${volumeIcon(level, info.muted)}</div>`,
        text(x + 24, y, right - 44 - (x + 24), 20, info.device ?? "No output device"),
        text(right - 60, y, 60, 20, info.muted ? "Muted" : `${Math.round(level * 100)}%`, `text-align:right;font-weight:600;color:${tint}`),
        `<div class="abd-vol" style="${at(x, y + 20, w, 16)}"><div class="abd-track">${level > 0 ? `<i style="width:${Math.max(level * w, 4)}px;background:${fill}"></i>` : ""}</div>` +
          `<div class="abd-thumb" style="left:${thumb}px"><i style="background:${fill}"></i></div></div>`,
      );
    }

    /** The pill behind the Capsule buttons. */
    function pill(buttons) {
      const [first, , last] = buttons;
      const top = Math.min(...buttons.map((b) => b.t)) - 4;
      const h = Math.max(...buttons.map((b) => b.t + b.h)) + 4 - top;
      parts.push(`<div style="${at(first.l - 4, top, last.l + last.w + 4 - (first.l - 4), h)}border-radius:${h / 2}px;background:${p.card};border:1px solid ${p.border};box-shadow:0 2px 4px rgba(0,0,0,${info.dark ? 0.2 : 0.07})"></div>`);
    }

    const x = 12;
    const y = 12;
    const inline = (right, cy) => trio(right + 4 - 35, cy, 2, [22, 22, 5, "muted", 10], [22, 22, 11, "filled", 10]);
    const capsule = (cy) => trio(center, cy, 2, [26, 24, 12, "muted", 11], [34, 24, 12, "accent", 12]);
    let buttons = [];
    // Hover buttons and what they cover show only while the pointer is on the card.
    let hover = false;

    if (compact) {
      const right = width - 12 - (style === "player" ? DOCK : 0);
      const line = style === "capsule" ? 78 : 48;
      if (style === "player") {
        parts.push(`<div style="${at(width - DOCK, 1, DOCK - 1, height - 2)}background:color-mix(in srgb, ${p.card} 97%, ${p.text});border-left:1px solid ${p.border}"></div>`);
      }
      volume(x, y, right);
      parts.push(avatar(x, line, 18), status(x + 26, line, style === "inline" ? right + 4 - INLINE : right, true, false));
      if (style === "capsule") {
        parts.push(`<div style="${at(0, 60, width, 1)}background:${p.border}"></div>`);
        buttons = capsule(60.5);
        pill(buttons);
      } else if (style === "hover") {
        hover = true;
        reveal.push(`<div style="${at(x + 22, line - 3, right + 4 - (x + 22), 24)}border-radius:4px;background:${p.card}"></div>`);
        buttons = trio((x + 26 + right + 4) / 2, line + 9, 6, [40, 24, 6, "muted", 11], [48, 24, 12, "filled", 12]);
      } else if (style === "inline") buttons = inline(right, line + 9);
      else if (style === "player") buttons = trio(width - DOCK / 2, height / 2, 2, [22, 30, 6, "muted", 11], [34, 34, 17, "filled", 14]);
    } else {
      const right = width - 12;
      const [title, subtitle] = titles(info);
      const hasStatus = !!info.app;
      const lines = 1 + (subtitle ? 1 : 0) + (hasStatus || style === "inline" ? 1 : 0);
      let line = y + (ART - (20 + 18 * (lines - 1))) / 2;
      const tx = x + ART + 12;
      parts.push(avatar(x, y, ART), text(tx, line, right - tx, 20, title, "font-size:14px;font-weight:600"));
      line += 20;
      if (subtitle) {
        parts.push(text(tx, line, right - tx, 18, subtitle, `color:${p.muted}`));
        line += 18;
      }
      // The app name only adds something when the title is the track.
      if (hasStatus) parts.push(status(tx, line, style === "inline" ? right + 4 - INLINE : right, false, !!info.track && info.running));
      const [offset, gap] = divider(style);
      parts.push(`<div style="${at(0, offset, width, 1)}background:${p.border}"></div>`);
      volume(x, offset + gap, right);
      if (style === "capsule") {
        buttons = capsule(offset + 0.5);
        pill(buttons);
      } else if (style === "hover") {
        hover = true;
        reveal.push(`<div style="${at(x, y, ART, ART)}border-radius:${ART * 0.12}px;background:rgba(10,10,16,.52)"></div>`);
        buttons = trio(x + ART / 2, y + ART / 2, 0, [14, 24, 4, "overlay", 10], [24, 24, 12, "overlay", 14]);
      } else if (style === "inline") buttons = inline(right, y + statusTop(!!subtitle) + 9);
      else if (style === "player") buttons = trio(center, 92, 14, [30, 30, 15, "muted", 14], [36, 36, 18, "filled", 15]);
    }

    const markup = buttons.map((b) => {
      const big = b.tone === "filled" && b.w >= 34 ? " big" : "";
      return `<button class="abd-cb ${b.tone}${big}" data-key="${b.key}" tabindex="-1" style="${at(b.l, b.t, b.w, b.h)}border-radius:${b.radius}px">${keyIcon(b.key, playing, b.icon)}</button>`;
    });
    if (hover) reveal.push(...markup);
    else parts.push(...markup);
    if (reveal.length) parts.push(`<div class="abd-reveal">${reveal.join("")}</div>`);

    const vars = `--c-card:${p.card};--c-border:${p.border};--c-text:${p.text};--c-muted:${p.muted};--c-track:${p.track};--c-playing:${p.playing};--c-accent:${accent};` +
      `--c-hot:${info.dark ? "rgba(255,255,255,.08)" : "rgba(0,0,0,.064)"};--c-press:${info.dark ? "rgba(255,255,255,.14)" : "rgba(0,0,0,.112)"};--c-shadow:${info.dark ? 0.32 : 0.16}`;
    return `<div class="abd-card" style="width:${width}px;height:${height}px;${vars}">${parts.join("")}</div>`;
  }

  /** Windows' plain tooltip, with the text the core gives the shell. */
  function plainTip(info) {
    const volume = info.muted ? "Muted" : `${Math.round(info.level * 100)}%`;
    const lines = [`${info.device ?? "Volume"}: ${volume}`, `Media keys: ${info.app ?? "Automatic"}`];
    if (info.running && info.track) {
      const state = { playing: "Playing", paused: "Paused", stopped: "Stopped" }[info.playback] ?? "Idle";
      lines.push(`${state} · ${info.track}`);
    }
    return `<div class="abd-plain">${esc(lines.join("\n"))}</div>`;
  }

  function mount(root, opts = {}) {
    if (!document.getElementById("abd-style")) {
      const style = document.createElement("style");
      style.id = "abd-style";
      style.textContent = css;
      document.head.append(style);
    }
    const theme = opts.theme ?? "auto";
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const systemDark = () => {
      if (theme !== "auto") return theme === "dark";
      const forced = document.documentElement.dataset.theme;
      return forced ? forced === "dark" : media.matches;
    };

    root.classList.add("abd");
    root.innerHTML = `
      <div class="abd-screen">
        <div class="abd-wall"></div>
        <button class="abd-shortcut" hidden><img src="${base}../assets/icon.svg" alt="">Audio Bartender</button>
        <div class="abd-frame closed"><iframe title="Audio Bartender, running in your browser" src="${base}ui/index.html?win11=true"></iframe></div>
        <div class="abd-tip" hidden></div>
        <div class="abd-notice"></div>
        <div class="abd-bar">
          <div class="abd-apps">
            <span class="abd-app" title="Start">${START}</span>
            ${APPS.map(([file, name, run, optional]) => `<span class="abd-app${run ? " run" : ""}${optional ? " optional" : ""}" title="${name}"><img src="${base}icons/${file}.png" alt=""></span>`).join("")}
          </div>
          <div class="abd-tray">
            <span title="Show hidden icons">${svg('<path d="m6 15 6-6 6 6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>')}</span>
            <span>${svg('<path d="M2 9a15 15 0 0 1 20 0M5 12.5a10 10 0 0 1 14 0M8.5 16a5 5 0 0 1 7 0" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/><circle cx="12" cy="19.5" r="1.3" fill="currentColor"/>')}</span>
            <span class="abd-icon" aria-label="Audio Bartender tray icon"><canvas width="40" height="40"></canvas></span>
            <span class="abd-clock"><div class="t"></div><div class="d"></div></span>
          </div>
        </div>
      </div>`;

    const $ = (s) => root.querySelector(s);
    const screen = $(".abd-screen");
    const frame = $(".abd-frame");
    const iframe = frame.querySelector("iframe");
    const trayIcon = $(".abd-icon");
    const canvas = trayIcon.querySelector("canvas");
    const tip = $(".abd-tip");
    const notice = $(".abd-notice");
    const shortcut = $(".abd-shortcut");

    let open = opts.open !== false;
    let pinned = false;
    let running = true;
    let size = { flyout: 480, panel: null };
    let drag = { x: 0, y: 0 };
    let width = 1100;
    let scale = 1;
    let dirty = true;
    let hinted = false;
    // The flyout shows only once its page has loaded and rendered.
    let loaded = false;
    let retries = 0;
    let readyTimer = 0;

    const demo = () => {
      try {
        return iframe.contentWindow.__abDemo ?? null;
      } catch {
        return null;
      }
    };

    // The page decides the size; the flyout keeps its place against the tray.
    function layout() {
      // Life size whenever the flyout and its settings panel fit, so the demo looks
      // exactly as the app would on this screen. Narrower slots shrink a screen of that
      // width, and phones get a narrow one, widened only while the settings panel shows.
      const avail = root.clientWidth || MIN_WIDE;
      if (avail >= MIN_WIDE) width = avail;
      else width = avail >= 600 || size.panel != null ? MIN_WIDE : 440;
      scale = avail / width;
      const h = Math.max(size.flyout, size.panel ?? 0) + 8 + 12;
      // `fill` takes the height the page gives the demo, like a full-window desktop.
      const base = opts.fill ? Math.max(root.clientHeight / scale, 560) : HEIGHT;
      const height = Math.max(base, h + TASKBAR + 16);
      screen.classList.toggle("narrow", width < 600);
      screen.style.width = `${width}px`;
      screen.style.height = `${height}px`;
      screen.style.transform = `scale(${scale})`;
      if (opts.fill) root.style.minHeight = `${height * scale}px`;
      else root.style.height = `${height * scale}px`;

      const icon = trayIcon.getBoundingClientRect();
      const box = screen.getBoundingClientRect();
      const iconX = (icon.left + icon.width / 2 - box.left) / scale;
      const w = FLYOUT + (size.panel != null ? PANEL : 0);
      let left = Math.min(Math.max(iconX - FLYOUT / 2, GAP), width - FLYOUT - GAP);
      if (size.panel != null) left = Math.max(0, left - PANEL);
      Object.assign(frame.style, { left: `${left + drag.x}px`, width: `${w}px`, height: `${h}px`, transform: `translateY(${drag.y}px)` });
      frame.classList.toggle("free", drag.x !== 0 || drag.y !== 0);
    }

    function setOpen(next) {
      if (!running) return;
      open = next;
      frame.classList.toggle("closed", !(open && loaded));
      hideTip();
      if (open) {
        demo()?.showPage("apps");
        iframe.focus();
      } else {
        drag = { x: 0, y: 0 };
        layout();
      }
    }

    function showNotice(text, ms = 1600) {
      notice.textContent = text;
      notice.classList.add("show");
      clearTimeout(showNotice.timer);
      showNotice.timer = setTimeout(() => notice.classList.remove("show"), ms);
    }

    // A file that fails to load (GitHub Pages answers a 503 now and then) leaves the
    // flyout blank, so a page that loads without reporting ready loads again.
    function reload() {
      loaded = false;
      frame.classList.add("closed");
      const url = new URL(iframe.src);
      url.searchParams.set("try", String(retries));
      iframe.src = url.href;
    }
    iframe.addEventListener("load", () => {
      clearTimeout(readyTimer);
      if (loaded) return;
      readyTimer = setTimeout(() => {
        if (loaded) return;
        if (retries < 3) {
          retries++;
          reload();
        } else showNotice("The demo didn't load. Reload the page to try again.", 6000);
      }, 3000);
    });

    // What the mock calls back into.
    window.__abDemoHost = {
      systemDark,
      tracks: opts.tracks ?? null,
      changed: () => (dirty = tipDirty = true),
      resize: (flyout, panel) => {
        size = { flyout, panel };
        layout();
      },
      ready: () => {
        loaded = true;
        retries = 0;
        clearTimeout(readyTimer);
        layout();
        frame.classList.toggle("closed", !open);
        if ((!open || opts.hint) && !hinted) {
          hinted = true;
          trayIcon.classList.add("hint");
        }
      },
      pinned: (on) => (pinned = on),
      drag: (dx, dy) => {
        drag = { x: drag.x + dx / scale, y: drag.y + dy / scale };
        layout();
      },
      quit: () => {
        running = false;
        open = false;
        frame.classList.add("closed");
        trayIcon.hidden = true;
        shortcut.hidden = false;
        hideTip();
        showNotice("Audio Bartender closed");
      },
      notice: showNotice,
      previewTip,
      pressed: (key, app, playback) => opts.onPress?.(key, app, playback),
    };

    shortcut.addEventListener("click", () => {
      running = true;
      pinned = false;
      open = true;
      drag = { x: 0, y: 0 };
      trayIcon.hidden = false;
      shortcut.hidden = true;
      retries = 0;
      reload();
    });

    // Tray icon: left click toggles, the wheel sets the volume, middle click mutes.
    trayIcon.addEventListener("click", () => {
      trayIcon.classList.remove("hint");
      setOpen(!open);
    });
    trayIcon.addEventListener("mousedown", (e) => {
      if (e.button !== 1) return;
      e.preventDefault();
      demo()?.toggleMasterMute();
    });
    trayIcon.addEventListener(
      "wheel",
      (e) => {
        e.preventDefault();
        demo()?.adjustMaster((-Math.sign(e.deltaY) * Math.max(1, Math.round(Math.abs(e.deltaY) / 100)) * 0.02));
        if (tipAllowed()) showTip();
      },
      { passive: false },
    );

    // The tooltip shows after a moment on the icon, and a card stays while the pointer
    // is on it or was on it or the icon less than a grace period ago.
    let tipTimer = 0;
    let hideTimer = 0;
    let tipShown = false;
    let tipDirty = false;
    let tipHtml = "";
    let previewUntil = 0;
    let onIcon = false;
    let onTip = false;
    let dragging = null;
    let pressing = null;

    /** Like the core, the tooltip stays off a flyout that shows its apps by the icon. */
    const tipAllowed = () => running && !(open && loaded && size.panel == null && drag.x === 0 && drag.y === 0);

    function renderTip() {
      const info = demo()?.tooltip();
      if (!info) return false;
      const plain = info.style === "text";
      const html = plain ? plainTip(info) : card(info);
      if (html !== tipHtml) {
        tip.innerHTML = tipHtml = html;
        tip.classList.toggle("card", !plain);
      }
      tipDirty = false;
      const icon = trayIcon.getBoundingClientRect();
      const box = screen.getBoundingClientRect();
      const x = (icon.left + icon.width / 2 - box.left) / scale;
      const w = tip.offsetWidth;
      tip.style.left = `${Math.min(Math.max(x - w / 2, 8), width - w - 8)}px`;
      return true;
    }
    function showTip() {
      clearTimeout(hideTimer);
      tip.hidden = false;
      tipShown = renderTip();
      tip.hidden = !tipShown;
    }
    function hideTip() {
      clearTimeout(tipTimer);
      clearTimeout(hideTimer);
      tip.hidden = true;
      tipShown = false;
      previewUntil = 0;
      dragging = null;
      tip.classList.remove("hot", "drag");
    }
    function maybeHide() {
      clearTimeout(hideTimer);
      if (onIcon || onTip || dragging) return;
      const left = previewUntil - Date.now();
      if (left > 0) hideTimer = setTimeout(maybeHide, left);
      else hideTip();
    }
    /** A changed tooltip setting shows the tooltip for a few seconds, wherever the pointer is. */
    function previewTip() {
      if (!running) return;
      showTip();
      previewUntil = Date.now() + PREVIEW_MS;
      hideTimer = setTimeout(maybeHide, PREVIEW_MS);
    }

    trayIcon.addEventListener("mouseenter", () => {
      onIcon = true;
      clearTimeout(hideTimer);
      clearTimeout(tipTimer);
      if (!tipShown) tipTimer = setTimeout(() => tipAllowed() && showTip(), 450);
    });
    trayIcon.addEventListener("mouseleave", () => {
      onIcon = false;
      clearTimeout(tipTimer);
      hideTimer = setTimeout(maybeHide, tip.classList.contains("card") ? CARD_GRACE_MS : 0);
    });
    tip.addEventListener("mouseenter", () => {
      onTip = true;
      clearTimeout(hideTimer);
      tip.classList.add("hot");
    });
    tip.addEventListener("mouseleave", () => {
      onTip = false;
      tip.classList.remove("hot");
      hideTimer = setTimeout(maybeHide, CARD_GRACE_MS);
    });

    // The card's media buttons press on release, and its bar sets the volume on click or drag.
    // The tip itself captures the pointer, since a state change redraws the card under it.
    function setLevel(e) {
      const bar = tip.querySelector(".abd-vol")?.getBoundingClientRect();
      if (bar?.width) demo()?.setMaster((e.clientX - bar.left) / bar.width);
    }
    tip.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      const key = e.target.closest("[data-key]")?.dataset.key;
      if (key) pressing = key;
      else if (e.target.closest(".abd-vol")) {
        dragging = e.pointerId;
        tip.setPointerCapture(e.pointerId);
        tip.classList.add("drag");
        setLevel(e);
      }
    });
    tip.addEventListener("pointermove", (e) => {
      if (dragging === e.pointerId) setLevel(e);
    });
    tip.addEventListener("pointerup", (e) => {
      const key = e.target.closest("[data-key]")?.dataset.key;
      if (key && key === pressing) demo()?.press(key);
      pressing = null;
      if (dragging !== e.pointerId) return;
      dragging = null;
      tip.classList.remove("drag");
      onTip = tip.matches(":hover");
      maybeHide();
    });

    // A click anywhere else on the desktop closes an unpinned flyout.
    screen.addEventListener("pointerdown", (e) => {
      if (!open || pinned || trayIcon.contains(e.target) || tip.contains(e.target)) return;
      setOpen(false);
    });

    // Draws the tray icon: live styles about 15 times a second, others on change.
    let lastDraw = 0;
    let onScreen = true;
    new IntersectionObserver(([entry]) => (onScreen = entry.isIntersecting)).observe(root);
    function tick(now) {
      if (!onScreen || document.hidden) return requestAnimationFrame(tick);
      const d = demo();
      // An iframe whose color scheme differs from its element's gets an opaque
      // backdrop, so the element follows the flyout's theme.
      try {
        const scheme = iframe.contentDocument?.documentElement.dataset.theme === "dark" ? "dark" : "light";
        if (iframe.style.colorScheme !== scheme) iframe.style.colorScheme = scheme;
      } catch {}
      if (d && running) {
        const look = d.look(now, !systemDark());
        const live = ["liveWaves", "equalizer"].includes(look.style) || (window.TrayIcon.hasMiddle(look.style) && look.middle === "pulse");
        if (dirty || (live && now - lastDraw > 66)) {
          window.TrayIcon.draw(canvas, look);
          lastDraw = now;
          dirty = false;
        }
        if (tipShown && tipDirty) renderTip();
      }
      requestAnimationFrame(tick);
    }
    requestAnimationFrame(tick);

    function clock() {
      const now = new Date();
      $(".abd-clock .t").textContent = now.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
      $(".abd-clock .d").textContent = now.toLocaleDateString();
    }
    clock();
    setInterval(clock, 15000);

    function applyTheme() {
      screen.classList.toggle("light", !systemDark());
      dirty = tipDirty = true;
      demo()?.themeChanged();
    }
    applyTheme();
    media.addEventListener("change", applyTheme);
    new MutationObserver(applyTheme).observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });

    new ResizeObserver(layout).observe(root);
    layout();

    // Media keys pressed on the page outside the flyout still reach the demo.
    const KEYS = { MediaPlayPause: "playPause", MediaTrackNext: "next", MediaTrackPrevious: "previous", MediaStop: "stop" };
    document.addEventListener("keydown", (e) => {
      if (!KEYS[e.key]) return;
      e.preventDefault();
      demo()?.press(KEYS[e.key]);
    });

    return {
      press: (key) => demo()?.press(key),
      play: (id, index) => demo()?.play(id, index),
      /** What the media keys reach now: app, track, playing, cover URL. */
      nowPlaying: () => demo()?.tooltip() ?? null,
      open: () => setOpen(true),
      close: () => setOpen(false),
    };
  }

  window.AudioBartenderDemo = { mount };
})();

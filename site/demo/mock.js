// A stand-in for the Rust core, so the real Tauri front end (apps/tauri/ui) runs in a
// browser. It answers the same `invoke` commands and emits the same events, with a
// simulated mixer: fake apps, fake devices and a fake spectrum. Load it before app.js.

(function () {
  const host = (() => {
    try {
      return window.parent !== window ? window.parent.__abDemoHost ?? null : null;
    } catch {
      return null;
    }
  })();

  // The host page decides what "Windows' theme" is, so the flyout matches the taskbar.
  const realMatchMedia = window.matchMedia.bind(window);
  const themeListeners = new Set();
  window.matchMedia = (query) => {
    if (!host || !/prefers-color-scheme:\s*dark/.test(query)) return realMatchMedia(query);
    return {
      get matches() {
        return host.systemDark();
      },
      media: query,
      addEventListener: (_, fn) => themeListeners.add(fn),
      removeEventListener: (_, fn) => themeListeners.delete(fn),
      addListener: (fn) => themeListeners.add(fn),
      removeListener: (fn) => themeListeners.delete(fn),
    };
  };

  // Covers: one painter per track, drawn on canvas for the tray icon and as SVG for the UI.
  const COVERS = {
    dusk: ["#f97316", "#7c3aed", "circle"],
    tide: ["#06b6d4", "#1e3a8a", "waves"],
    neon: ["#ec4899", "#0f172a", "grid"],
    moss: ["#84cc16", "#14532d", "circle"],
    ember: ["#ef4444", "#facc15", "waves"],
    video: ["#0f172a", "#334155", "play"],
  };

  function coverSvg([a, b, shape]) {
    const art = {
      circle: `<circle cx="64" cy="58" r="30" fill="#fff" fill-opacity=".85"/><rect y="92" width="128" height="36" fill="#000" fill-opacity=".25"/>`,
      waves: `<path d="M0 80q32-24 64 0t64 0v48H0z" fill="#fff" fill-opacity=".35"/><path d="M0 96q32-24 64 0t64 0v32H0z" fill="#fff" fill-opacity=".55"/>`,
      grid: `<path d="M0 88h128M0 104h128M0 120h128M32 72v56M64 72v56M96 72v56" stroke="#fff" stroke-opacity=".5" stroke-width="2"/><circle cx="64" cy="48" r="22" fill="#fff" fill-opacity=".9"/>`,
      play: `<rect x="8" y="22" width="112" height="84" rx="10" fill="#1e293b"/><path d="M54 46l26 18-26 18z" fill="#fff"/>`,
    }[shape];
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${a}"/><stop offset="1" stop-color="${b}"/></linearGradient></defs><rect width="128" height="128" fill="url(#g)"/>${art}</svg>`;
    return "data:image/svg+xml;base64," + btoa(svg);
  }

  function coverPainter([a, b, shape]) {
    return (g, x, y, s) => {
      const grad = g.createLinearGradient(x, y, x + s, y + s);
      grad.addColorStop(0, a);
      grad.addColorStop(1, b);
      g.fillStyle = grad;
      g.fillRect(x, y, s, s);
      g.fillStyle = "rgba(255,255,255,.8)";
      g.beginPath();
      if (shape === "play") {
        g.moveTo(x + s * 0.4, y + s * 0.3);
        g.lineTo(x + s * 0.7, y + s * 0.5);
        g.lineTo(x + s * 0.4, y + s * 0.7);
      } else g.arc(x + s / 2, y + s * 0.45, s * 0.22, 0, Math.PI * 2);
      g.fill();
    };
  }

  const TRACKS = {
    "smtc:Spotify": [
      ["Glass Harbor — Lumen Bay", "tide"],
      ["Paper Planets — The Overpass", "dusk"],
      ["Night Shift — Velvet Static", "neon"],
      ["Greenhouse — Moss & Mirrors", "moss"],
      ["Slow Burn — Ember Lane", "ember"],
    ],
    "smtc:Chrome": [
      ["How speakers actually work — Sound Lab", "video"],
      ["Mixing for beginners — Studio Notes", "video"],
    ],
  };
  const trackIndex = { "smtc:Spotify": 0, "smtc:Chrome": 0 };

  // Accents, as crates/core/src/lib.rs defines them.
  const ACCENTS = [
    ["red", "Red", 0xdc2626, 0xef4444, [0xf87171, 0xf97316]],
    ["orange", "Orange", 0xea580c, 0xf97316, [0xfb923c, 0xec4899]],
    ["amber", "Amber", 0xb45309, 0xd97706, [0xfbbf24, 0xf97316]],
    ["lime", "Lime", 0x4d7c0f, 0x65a30d, [0xa3e635, 0x22c55e]],
    ["green", "Green", 0x16a34a, 0x22c55e, [0x4ade80, 0x14b8a6]],
    ["teal", "Teal", 0x0d9488, 0x14b8a6, [0x2dd4bf, 0x0ea5e9]],
    ["cyan", "Cyan", 0x0891b2, 0x06b6d4, [0x22d3ee, 0x3b82f6]],
    ["blue", "Blue", 0x2563eb, 0x3b82f6, [0x3b82f6, 0x06b6d4]],
    ["indigo", "Indigo", 0x4f46e5, 0x6366f1, [0x818cf8, 0x3b82f6]],
    ["purple", "Purple", 0x6d5ef5, 0x9d8cff, [0x6366f1, 0xa855f7]],
    ["fuchsia", "Fuchsia", 0xc026d3, 0xd946ef, [0xe879f9, 0x8b5cf6]],
    ["pink", "Hot pink", 0xdb2777, 0xf0469c, [0xf472b6, 0xa855f7]],
    ["slate", "Slate", 0x475569, 0x64748b, [0x64748b, 0x334155]],
    ["graphite", "Graphite", 0x27272a, 0x71717a, [0x52525b, 0x18181b]],
  ];
  const mix = (a, b, t) =>
    [16, 8, 0].reduce((out, shift) => {
      const x = (a >> shift) & 255, y = (b >> shift) & 255;
      return out | (Math.round(x + (y - x) * t) << shift);
    }, 0);
  const accents = ACCENTS.map(([name, label, light, dark, gradient]) => ({
    name,
    label,
    light: { accent: light, hover: mix(light, 0xffffff, 0.15), gradient },
    dark: { accent: dark, hover: mix(dark, 0xffffff, 0.15), gradient },
  }));
  const accent = (name) => accents.find((a) => a.name === name) ?? accents[6];

  const target = (id, name, playback, volume, running = true) => ({
    id, name, playback, track: null, running, viaWindow: false, volume, muted: false, icon: null, route: null,
  });

  const state = {
    targets: [
      target("smtc:Spotify", "Spotify", "playing", 0.8),
      target("smtc:Chrome", "Google Chrome", "paused", 1),
      Object.assign(target("exe:foobar2000.exe", "foobar2000", "unknown", null, false), { viaWindow: true }),
    ],
    mixer: [
      { id: "exe:Discord.exe", name: "Discord", volume: 0.7, muted: false, icon: null, route: null },
      { id: "exe:steam.exe", name: "Steam", volume: 0.5, muted: false, icon: null, route: null },
      { id: "system", name: "System sounds", volume: 0.4, muted: false, icon: null, route: null },
    ],
    audio: {
      devices: [
        { id: "spk", name: "Speakers (Realtek Audio)", short: "Speakers", kind: "speakers", default: true },
        { id: "hp", name: "Headphones (USB Headset)", short: "Headphones", kind: "headphones", default: false },
        { id: "tv", name: "Monitor (HDMI Audio)", short: "Monitor", kind: "display", default: false },
      ],
      volume: 0.62,
      muted: false,
    },
    selected: "smtc:Spotify",
    current: "smtc:Chrome",
    settings: {
      autostart: true, autoUpdate: true, dark: null, albumArt: true,
      trayStyle: "ring", trayMiddle: "speaker", pulseStyle: "dot", speakerPosition: false,
      volumeBars: "compact", tooltip: "nowPlaying", mediaButtons: "hover", mediaControls: "hover",
      levelMeters: "volume", deviceChip: "beside", showMediaKeys: true, showOtherApps: true, showVolume: true,
      accent: accent("cyan"),
    },
    update: { state: { state: "upToDate" }, checkedAt: Date.now(), summary: "Version 0.1.0 · Web · up to date, checked today" },
    iconOnTaskbar: true,
    speakerPositionActive: false,
  };
  state.mixer.find((m) => m.id === "exe:Discord.exe").route = "hp";

  // Tracks and icons follow the album art setting, like SMTC thumbnails in the core.
  function refreshTracks() {
    for (const t of state.targets) {
      const list = TRACKS[t.id];
      if (!list) continue;
      const [track, cover] = list[trackIndex[t.id]];
      t.track = track;
      t.icon = state.settings.albumArt ? { url: coverSvg(COVERS[cover]), art: true } : null;
    }
  }

  const find = (id) => state.targets.find((t) => t.id === id) ?? state.mixer.find((m) => m.id === id);
  const clamp01 = (v) => Math.min(1, Math.max(0, v));
  const step = (v) => Math.round(v * 100) / 100;

  // Events: `state` after each change, and `page` when the host reopens the flyout.
  const listeners = new Map();
  let queued = false;
  function emit(name, payload) {
    for (const fn of listeners.get(name) ?? []) fn({ event: name, payload });
  }
  function changed() {
    if (queued) return;
    queued = true;
    queueMicrotask(() => {
      queued = false;
      refreshTracks();
      emit("state", snapshot());
      host?.changed?.();
    });
  }
  const snapshot = () => JSON.parse(JSON.stringify(state));

  /** Where media keys go: the chosen app while it runs, else Windows' current session. */
  function keyTarget(id = null) {
    const chosen = id ?? state.selected;
    const t = state.targets.find((t) => t.id === chosen && t.running);
    return t ?? state.targets.find((t) => t.id === state.current) ?? null;
  }

  function press(key, id = null) {
    const t = keyTarget(id);
    if (!t || !TRACKS[t.id]) return null;
    const list = TRACKS[t.id];
    if (key === "playPause") t.playback = t.playback === "playing" ? "paused" : "playing";
    if (key === "stop") t.playback = "stopped";
    if (key === "next" || key === "previous") {
      trackIndex[t.id] = (trackIndex[t.id] + (key === "next" ? 1 : list.length - 1)) % list.length;
      t.playback = "playing";
    }
    // Windows' current session follows whatever last started playing.
    if (t.playback === "playing") state.current = t.id;
    changed();
    host?.pressed?.(key, t.name, t.playback);
    return t;
  }

  // A fake song: a beat at 118 BPM, bass on the beat, treble on the off-beat.
  function spectrum(now) {
    const playing = state.targets.filter((t) => t.running && t.playback === "playing" && !t.muted && t.volume);
    const loud = state.audio.muted ? 0 : Math.max(0, ...playing.map((t) => t.volume)) * Math.sqrt(state.audio.volume);
    const beat = (now / 1000) * (118 / 60);
    const kick = Math.pow(1 - (beat % 1), 3);
    const hat = Math.pow(1 - ((beat + 0.5) % 1), 6);
    const bands = Array.from({ length: 12 }, (_, i) => {
      const f = i / 11;
      const body = 0.35 + 0.25 * Math.sin(now / 700 + i * 1.7) * Math.sin(now / 1300 + i);
      const v = (1 - f) * (0.4 + 0.6 * kick) + f * 0.8 * hat + body * (0.6 - 0.3 * f);
      return Math.min(1, v * loud);
    });
    const level = Math.min(1, loud * (0.55 + 0.35 * kick + 0.1 * Math.random()));
    return { bands, bass: Math.min(1, loud * (0.3 + 0.7 * kick)), level };
  }

  function levelOf(id, now) {
    const t = find(id);
    if (!t || t.muted) return 0;
    const seed = [...id].reduce((h, c) => h + c.charCodeAt(0), 0);
    const wobble = 0.55 + 0.3 * Math.abs(Math.sin(now / 180 + seed)) + 0.15 * Math.random();
    let on = 0;
    if ("playback" in t) on = t.running && t.playback === "playing" ? 1 : 0;
    else if (id === "exe:Discord.exe") on = Math.sin(now / 900) > 0.35 ? 0.8 : 0;
    return state.audio.muted ? 0 : clamp01((t.volume ?? 0) * on * wobble);
  }

  /** The tray icon's look, as the host's taskbar and the settings swatches draw it. */
  function look(now, lightTaskbar) {
    const np = keyTarget();
    const cover = np && TRACKS[np.id] ? COVERS[TRACKS[np.id][trackIndex[np.id]][1]] : null;
    return {
      level: state.audio.volume,
      muted: state.audio.muted,
      style: state.settings.trayStyle,
      middle: state.settings.trayMiddle,
      pulse: state.settings.pulseStyle,
      lightTaskbar,
      spectrum: spectrum(now),
      playing: np && TRACKS[np.id] ? np.playback === "playing" : null,
      art: state.settings.albumArt && cover ? coverPainter(cover) : null,
      clock: now,
    };
  }

  const SAMPLE = { bands: [0.9, 0.8, 0.7, 0.75, 0.6, 0.55, 0.5, 0.45, 0.5, 0.4, 0.35, 0.3], bass: 0.7, level: 0.7 };

  const commands = {
    state: () => snapshot(),
    accents: () => accents,
    select: ({ id }) => {
      state.selected = id ?? null;
      changed();
    },
    press: ({ key }) => void press(key),
    press_for: ({ id, key }) => void press(key, id),
    check_update: () => {
      state.update = { state: { state: "checking" }, checkedAt: state.update.checkedAt, summary: "Version 0.1.0 · Web · checking for updates…" };
      changed();
      setTimeout(() => {
        state.update = { state: { state: "upToDate" }, checkedAt: Date.now(), summary: "Version 0.1.0 · Web · up to date, checked today" };
        changed();
      }, 1200);
    },
    levels: ({ ids }) => {
      const now = performance.now();
      return ids.map((id) => levelOf(id, now));
    },
    set_setting: ({ setting }) => {
      const [[name, value]] = Object.entries(setting);
      const before = state.settings[name];
      state.settings[name] = name === "accent" ? accent(value) : value;
      // Like the core, keep at least one part of the flyout showing.
      const s = state.settings;
      const card = s.mediaControls === "banner" || s.mediaControls === "dock";
      if (!(s.showMediaKeys || s.showOtherApps || s.showVolume || card)) state.settings[name] = before;
      if (name === "speakerPosition") state.speakerPositionActive = value;
      changed();
    },
    adjust_volume: ({ id, delta }) => {
      const t = find(id);
      if (t?.volume == null) return;
      t.volume = step(clamp01(t.volume + delta));
      changed();
    },
    set_volume: ({ id, level }) => {
      const t = find(id);
      if (!t) return;
      t.volume = step(clamp01(level));
      changed();
    },
    toggle_mute: ({ id }) => {
      const t = find(id);
      if (!t) return;
      t.muted = !t.muted;
      changed();
    },
    adjust_master: ({ delta }) => {
      state.audio.volume = step(clamp01(state.audio.volume + delta));
      state.audio.muted = false;
      changed();
    },
    set_master: ({ level }) => {
      state.audio.volume = step(clamp01(level));
      changed();
    },
    toggle_master_mute: () => {
      state.audio.muted = !state.audio.muted;
      changed();
    },
    set_device: ({ id }) => {
      for (const d of state.audio.devices) d.default = d.id === id;
      changed();
    },
    set_route: ({ app, device }) => {
      const t = find(app);
      if (!t) return;
      t.route = device ?? null;
      changed();
    },
    open_icon_page: () => host?.notice?.("Opens a Windows settings page"),
    open_app_volume_settings: () => host?.notice?.("Opens Windows' app volume settings"),
    set_icon_on_taskbar: ({ show }) => {
      state.iconOnTaskbar = show;
      changed();
    },
    tray_swatch: ({ style, middle, pulse, dark }) => {
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 32;
      const l = look(0, !dark);
      Object.assign(l, { style, middle, pulse, spectrum: SAMPLE, level: 0.62, muted: false, playing: true });
      window.TrayIcon.draw(canvas, l);
      return canvas.toDataURL();
    },
    quit: () => host?.quit?.(),
    set_pinned: ({ pinned }) => host?.pinned?.(pinned),
    start_drag: () => startDrag(),
    flyout_size: ({ flyout, panel }) => {
      host?.resize?.(flyout, panel);
      return [true, false, true];
    },
    ready: () => host?.ready?.(),
  };

  // Dragging a pinned flyout: the host moves the frame as the pointer moves.
  function startDrag() {
    let last = null;
    const move = (e) => {
      if (!(e.buttons & 1)) return stop();
      if (last) host?.drag?.(e.screenX - last.x, e.screenY - last.y);
      last = { x: e.screenX, y: e.screenY };
    };
    const stop = () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", stop);
    };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", stop);
  }

  window.__TAURI__ = {
    core: {
      invoke: (cmd, args = {}) => {
        const fn = commands[cmd];
        if (!fn) return Promise.reject(new Error(`Unknown command ${cmd}`));
        try {
          return Promise.resolve(fn(args));
        } catch (e) {
          return Promise.reject(e);
        }
      },
    },
    event: {
      listen: (name, fn) => {
        if (!listeners.has(name)) listeners.set(name, new Set());
        listeners.get(name).add(fn);
        return Promise.resolve(() => listeners.get(name).delete(fn));
      },
    },
  };

  // Real media keys work while the flyout has focus.
  const KEYS = { MediaPlayPause: "playPause", MediaTrackNext: "next", MediaTrackPrevious: "previous", MediaStop: "stop" };
  document.addEventListener("keydown", (e) => {
    if (!KEYS[e.key]) return;
    e.preventDefault();
    press(KEYS[e.key]);
  });

  refreshTracks();

  // What the host page drives: the tray icon, the wheel, media keys and reopening.
  window.__abDemo = {
    look,
    press: (key) => press(key),
    adjustMaster: (delta) => commands.adjust_master({ delta }),
    toggleMasterMute: () => commands.toggle_master_mute(),
    showPage: (page) => emit("page", page),
    themeChanged: () => {
      for (const fn of themeListeners) fn();
    },
    tooltip: () => {
      const np = keyTarget();
      const device = state.audio.devices.find((d) => d.default);
      const cover = np && TRACKS[np.id] ? coverSvg(COVERS[TRACKS[np.id][trackIndex[np.id]][1]]) : null;
      return {
        style: state.settings.tooltip,
        device: device?.name ?? "No output device",
        volume: state.audio.muted ? "Muted" : `${Math.round(state.audio.volume * 100)}%`,
        app: np?.name ?? null,
        track: np?.track ?? null,
        playing: np?.playback === "playing",
        cover: state.settings.albumArt ? cover : null,
      };
    },
  };
})();

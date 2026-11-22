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
  transform: translateY(0); transition: transform .15s cubic-bezier(.2,.8,.3,1); }
.abd-frame.closed iframe { transform: translateY(105%); transition-duration: .09s; pointer-events: none; }
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
.abd-tip { position: absolute; bottom: ${TASKBAR + 10}px; z-index: 3; background: var(--card); color: var(--card-fg); border-radius: 8px;
  box-shadow: 0 8px 24px rgba(0,0,0,.35); border: 1px solid var(--tb-line); padding: 10px 12px; display: flex; gap: 10px; align-items: center; max-width: 280px; pointer-events: none; }
.abd-tip img { width: 44px; height: 44px; border-radius: 6px; }
.abd-tip b { display: block; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 200px; }
.abd-tip span { display: block; color: var(--card-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 200px; }
.abd-tip.text { padding: 5px 8px; font-size: 12px; }
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
      frame.classList.toggle("closed", !open);
      hideTip();
      if (open) {
        demo()?.showPage("apps");
        iframe.focus();
      } else {
        drag = { x: 0, y: 0 };
        layout();
      }
    }

    function showNotice(text) {
      notice.textContent = text;
      notice.classList.add("show");
      clearTimeout(showNotice.timer);
      showNotice.timer = setTimeout(() => notice.classList.remove("show"), 1600);
    }

    // What the mock calls back into.
    window.__abDemoHost = {
      systemDark,
      tracks: opts.tracks ?? null,
      changed: () => (dirty = true),
      resize: (flyout, panel) => {
        size = { flyout, panel };
        layout();
      },
      ready: () => {
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
      pressed: (key, app, playback) => opts.onPress?.(key, app, playback),
    };

    shortcut.addEventListener("click", () => {
      running = true;
      pinned = false;
      open = true;
      drag = { x: 0, y: 0 };
      trayIcon.hidden = false;
      shortcut.hidden = true;
      iframe.src = iframe.src;
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
        showTip();
      },
      { passive: false },
    );

    // Hovering the icon shows the tooltip card after a moment.
    let tipTimer = 0;
    trayIcon.addEventListener("mouseenter", () => {
      clearTimeout(tipTimer);
      tipTimer = setTimeout(showTip, 450);
    });
    trayIcon.addEventListener("mouseleave", hideTip);
    function showTip() {
      const info = demo()?.tooltip();
      if (!info || open) return;
      tip.className = "abd-tip" + (info.style === "text" ? " text" : "");
      if (info.style === "text") tip.textContent = `${info.device}: ${info.volume}`;
      else {
        const cover = info.style === "nowPlaying" && info.cover ? `<img src="${info.cover}" alt="">` : "";
        const line = info.track ? `${info.playing ? "Playing" : "Paused"} · ${info.track}` : info.app ? info.app : "Nothing playing";
        tip.innerHTML = `${cover}<div><b>${info.device} · ${info.volume}</b><span>${info.app ? info.app + " · " : ""}${line}</span></div>`;
      }
      tip.hidden = false;
      const icon = trayIcon.getBoundingClientRect();
      const box = screen.getBoundingClientRect();
      const x = (icon.left + icon.width / 2 - box.left) / scale;
      tip.style.left = `${Math.min(x - tip.offsetWidth / 2, width - tip.offsetWidth - 8)}px`;
    }
    function hideTip() {
      clearTimeout(tipTimer);
      tip.hidden = true;
    }

    // A click anywhere else on the desktop closes an unpinned flyout.
    screen.addEventListener("pointerdown", (e) => {
      if (!open || pinned || trayIcon.contains(e.target)) return;
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
      dirty = true;
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

// The tray icon, drawn like crates/core/src/tray_icon.rs: each style is a shape on the
// unit square, sampled 4x4 per pixel. Segoe MDL2 glyphs become drawn speaker paths.

(function () {
  const TAU = Math.PI * 2;
  const RING_OUTER = 0.48;
  const RING_WIDTH = 0.12;
  const TRACK = 0.32;
  const WAVE_TRACK = 0.25;
  const DIAL_START = TAU * 0.625;
  const DIAL_SWEEP = TAU * 0.75;
  const PLAIN_SCALE = 1.55;
  const BANDS = 12;

  const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));
  const rem = (a, b) => ((a % b) + b) % b;

  function coverage(size, shape) {
    const out = new Float32Array(size * size);
    const n = 4;
    for (let py = 0; py < size; py++) {
      for (let px = 0; px < size; px++) {
        let sum = 0;
        for (let sy = 0; sy < n; sy++) {
          for (let sx = 0; sx < n; sx++) sum += shape((px + (sx + 0.5) / n) / size, (py + (sy + 0.5) / n) / size);
        }
        out[py * size + px] = sum / (n * n);
      }
    }
    return out;
  }

  const max = (a, b) => a.map((v, i) => Math.max(v, b[i]));

  function cap(dx, dy, mid, angle, level) {
    if (level <= 0) return false;
    const cx = mid * Math.sin(angle);
    const cy = -mid * Math.cos(angle);
    return (dx - cx) ** 2 + (dy - cy) ** 2 <= (RING_WIDTH / 2) ** 2;
  }

  function arcBand(x, y, start, sweep, level) {
    const dx = x - 0.5, dy = y - 0.5;
    const r = Math.hypot(dx, dy);
    const mid = RING_OUTER - RING_WIDTH / 2;
    if (Math.abs(r - mid) > RING_WIDTH / 2) return 0;
    const along = rem(Math.atan2(dx, -dy) - start, TAU);
    const fill = clamp(level, 0, 1) * sweep;
    if (along <= fill || cap(dx, dy, mid, start, level) || cap(dx, dy, mid, start + fill, level)) return 1;
    return along <= sweep ? TRACK : 0;
  }

  function signalSteps(x, y, level) {
    for (let i = 0; i < 4; i++) {
      const left = 0.06 + i * 0.235;
      const height = 0.25 + i * 0.2;
      if (x < left || x > left + 0.17 || y < 0.92 - height || y > 0.92) continue;
      const fill = clamp(level * 4 - i, 0, 1);
      return y >= 0.92 - height * fill ? 1 : TRACK;
    }
    return 0;
  }

  function bandAt(s, f) {
    const at = clamp(f, 0, 1) * (BANDS - 1);
    const low = Math.floor(at), part = at - low;
    const high = Math.min(low + 1, BANDS - 1);
    return s.bands[low] * (1 - part) + s.bands[high] * part;
  }

  const mirrored = (dx, dy) => Math.abs(Math.atan2(dx, -dy)) / Math.PI;
  const disc = (x, y, r) => ((x - 0.5) ** 2 + (y - 0.5) ** 2 <= r * r ? 1 : 0);
  const triangle = (x, y) => x >= 0.41 && x <= 0.67 && Math.abs(y - 0.5) <= ((0.67 - x) / 0.26) * 0.17;

  function bar(x, y, i, height) {
    const cx = 0.175 + i * 0.215, r = 0.075;
    const reach = Math.max(height / 2 - r, 0);
    const dy = Math.max(Math.abs(y - 0.5) - reach, 0);
    return (x - cx) ** 2 + dy * dy <= r * r ? 1 : 0;
  }

  function pulse(style, x, y, s, t) {
    const dx = x - 0.5, dy = y - 0.5;
    const r = Math.hypot(dx, dy);
    if (style === "dot") return r <= 0.08 + 0.09 * s.bass + 0.045 * bandAt(s, mirrored(dx, dy)) ? 1 : 0;
    if (style === "rays") {
      let best = disc(x, y, 0.05);
      for (let i = 0; i < 6; i++) {
        const a = (i / 6) * TAU - TAU / 4;
        const ux = Math.cos(a), uy = Math.sin(a);
        const len = 0.02 + 0.2 * bandAt(s, mirrored(ux, uy));
        const along = clamp(dx * ux + dy * uy, 0.12, 0.12 + len);
        if ((dx - ux * along) ** 2 + (dy - uy * along) ** 2 <= 0.045 ** 2) best = 1;
      }
      return best;
    }
    if (style === "orbit") {
      for (let i = 0; i < 6; i++) {
        const a = (i / 6) * TAU + t * 0.6;
        const size = 0.04 + 0.06 * bandAt(s, i / 5);
        if ((dx - 0.2 * Math.cos(a)) ** 2 + (dy - 0.2 * Math.sin(a)) ** 2 <= size * size) return 1;
      }
      return 0;
    }
    const slot = TAU / 16;
    const a = rem(Math.atan2(dx, -dy), TAU);
    const center = rem(Math.round(a / slot) * slot, TAU);
    const ux = Math.sin(center), uy = -Math.cos(center);
    const len = 0.03 + 0.18 * bandAt(s, mirrored(ux, uy));
    const along = clamp(dx * ux + dy * uy, 0.11, 0.11 + len);
    return (dx - ux * along) ** 2 + (dy - uy * along) ** 2 <= 0.028 ** 2 ? 1 : 0;
  }

  /** A canvas-drawn layer's alpha channel, for the speaker glyphs and the number. */
  function layer(size, paint) {
    const c = document.createElement("canvas");
    c.width = c.height = size;
    const g = c.getContext("2d");
    g.fillStyle = g.strokeStyle = "#000";
    paint(g, size);
    const data = g.getImageData(0, 0, size, size).data;
    const out = new Float32Array(size * size);
    for (let i = 0; i < out.length; i++) out[i] = data[i * 4 + 3] / 255;
    return out;
  }

  /** The speaker body, `scale` of the icon tall, its center `shift` right of the middle. */
  function speakerPath(g, size, scale, shift) {
    const u = (size * scale) / 16;
    const ox = size / 2 + shift * size - 5 * u;
    const oy = size / 2 - 8 * u;
    g.beginPath();
    g.moveTo(ox + 0 * u, oy + 5.5 * u);
    g.lineTo(ox + 3 * u, oy + 5.5 * u);
    g.lineTo(ox + 7 * u, oy + 2 * u);
    g.lineTo(ox + 7 * u, oy + 14 * u);
    g.lineTo(ox + 3 * u, oy + 10.5 * u);
    g.lineTo(ox + 0 * u, oy + 10.5 * u);
    g.closePath();
    g.fill();
    return { ox, oy, u };
  }

  function speaker(size, scale = 0.9, shift = -0.12) {
    return layer(size, (g) => speakerPath(g, size, scale, shift));
  }

  /** Wave `i` (0 to 2) beside a speaker drawn at the default scale. */
  function wave(size, i) {
    return layer(size, (g) => {
      const { ox, oy, u } = speakerPath(Object.assign(g, { fillStyle: "transparent" }), size, 0.9, -0.12);
      g.strokeStyle = "#000";
      g.lineWidth = 1.3 * u;
      g.lineCap = "round";
      g.beginPath();
      g.arc(ox + 7 * u, oy + 8 * u, (3 + i * 2.6) * u, -0.9, 0.9);
      g.stroke();
    });
  }

  function mute(size) {
    return layer(size, (g) => {
      const { ox, oy, u } = speakerPath(g, size, 0.9, -0.12);
      g.lineWidth = 1.4 * u;
      g.lineCap = "round";
      g.beginPath();
      g.moveTo(ox + 10 * u, oy + 5.5 * u);
      g.lineTo(ox + 15 * u, oy + 10.5 * u);
      g.moveTo(ox + 15 * u, oy + 5.5 * u);
      g.lineTo(ox + 10 * u, oy + 10.5 * u);
      g.stroke();
    });
  }

  function number(size, value, scale) {
    return layer(size, (g) => {
      g.font = `600 ${Math.round(size * scale)}px "Segoe UI", system-ui, sans-serif`;
      g.textAlign = "center";
      g.textBaseline = "middle";
      g.fillText(String(value), size / 2, size / 2 + size * 0.03);
    });
  }

  const steps = (level) => {
    const v = Math.round(level * 100);
    return v === 0 ? 0 : v <= 33 ? 1 : v <= 66 ? 2 : 3;
  };
  const hasMiddle = (style) => style === "ring" || style === "dial" || style === "plain";

  function frame(look, size) {
    const level = clamp(look.level, 0, 1);
    if (look.style === "ring") return coverage(size, (x, y) => arcBand(x, y, 0, TAU, level));
    if (look.style === "dial") return coverage(size, (x, y) => arcBand(x, y, DIAL_START, DIAL_SWEEP, level));
    return null;
  }

  function middle(look, size) {
    const plain = look.style === "plain";
    const k = plain ? PLAIN_SCALE : 1;
    const scaled = (shape) => coverage(size, (x, y) => shape((x - 0.5) / k + 0.5, (y - 0.5) / k + 0.5));
    const sp = () => (plain ? speaker(size, 1.1, 0.02) : speaker(size, 0.66, 0.02));
    switch (look.middle) {
      case "playState":
        if (look.playing === true) return scaled((x, y) => (triangle(x, y) ? 1 : 0));
        if (look.playing === false)
          return scaled((x, y) => ((x >= 0.37 && x <= 0.45) || (x >= 0.55 && x <= 0.63)) && y >= 0.34 && y <= 0.66 ? 1 : 0);
        return sp();
      case "number": {
        const value = Math.round(clamp(look.level, 0, 1) * 100);
        const scale = plain ? (value >= 100 ? 0.78 : 0.95) : value >= 100 ? 0.4 : 0.5;
        return number(size, value, scale);
      }
      case "pulse":
        return scaled((x, y) => pulse(look.pulse, x, y, look.spectrum, look.clock / 1000));
      default:
        return sp();
    }
  }

  function alpha(look, size) {
    if (look.muted) return mute(size);
    const level = clamp(look.level, 0, 1);
    const sound = clamp(look.spectrum.level, 0, 1);
    if (hasMiddle(look.style)) {
      const m = middle(look, size);
      const f = frame(look, size);
      return f ? max(f, m) : m;
    }
    if (look.style === "native" || look.style === "liveWaves") {
      let out = speaker(size);
      const shown = steps(level);
      for (let i = 0; i < 3; i++) {
        if (i >= shown) continue;
        const lit = look.style === "native" ? 1 : WAVE_TRACK + (1 - WAVE_TRACK) * clamp(sound * 3 - i, 0, 1);
        out = max(out, wave(size, i).map((a) => a * lit));
      }
      return out;
    }
    if (look.style === "signalSteps") return coverage(size, (x, y) => signalSteps(x, y, level));
    const heights = [0, 1, 2, 3].map((i) => 0.16 + 0.7 * bandAt(look.spectrum, i / 3) * Math.sqrt(level));
    return coverage(size, (x, y) => heights.reduce((best, h, i) => Math.max(best, bar(x, y, i, h)), 0));
  }

  /**
   * Draws `look` onto `canvas` (square, `size` px). `look.art` is a paint function
   * for the cover, used by the Art middle.
   */
  function draw(canvas, look) {
    const size = canvas.width;
    const g = canvas.getContext("2d");
    const fg = look.lightTaskbar ? [28, 28, 28] : [255, 255, 255];
    g.clearRect(0, 0, size, size);
    const a = alpha(look, size);
    const img = g.createImageData(size, size);
    for (let i = 0; i < a.length; i++) {
      img.data[i * 4] = fg[0];
      img.data[i * 4 + 1] = fg[1];
      img.data[i * 4 + 2] = fg[2];
      img.data[i * 4 + 3] = Math.round(clamp(a[i], 0, 1) * 255);
    }
    const art = !look.muted && hasMiddle(look.style) && look.middle === "albumArt" && look.art;
    if (!art) return g.putImageData(img, 0, 0);
    // The cover in color inside the ring or dial, dimmed while paused.
    const f = frame(look, size);
    if (f) {
      for (let i = 0; i < f.length; i++) img.data[i * 4 + 3] = Math.round(f[i] * 255);
      g.putImageData(img, 0, 0);
    }
    const radius = (look.style === "plain" ? 0.47 : 0.33) * size;
    g.save();
    g.beginPath();
    g.arc(size / 2, size / 2, radius, 0, TAU);
    g.clip();
    look.art(g, size / 2 - radius, size / 2 - radius, radius * 2);
    if (look.playing === false) {
      g.fillStyle = "rgba(0,0,0,.45)";
      g.fillRect(0, 0, size, size);
    }
    g.restore();
  }

  window.TrayIcon = { draw, hasMiddle };
})();

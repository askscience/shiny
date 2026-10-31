/**
 * fx.js — the Terminal's look: an animated cursor and command decorations.
 *
 * The user picks one of four styles (Settings → Appearance → Terminal). A
 * style decides SHAPE and MOTION only; every colour comes from the active
 * theme's tokens (`--terminal-*`, `--accent`), so each style follows Dusk,
 * Ink or any other theme.
 *
 *   smear   block cursor whose corners chase the target at different speeds
 *           (Ghostty/Neovide smear); bare surface, status pill on hover
 *   glass   accent beam that sheds sparks and drags a tail; translucent
 *           surface, status chips
 *   paper   ink beam that leaves a fading accent highlighter stroke; command
 *           marks in a left gutter, meta status line
 *   blocks  accent block on an underdamped spring with a ribbon tail; every
 *           command drawn as a card with its duration and exit status
 *
 * How it sits on xterm: the terminal is created with a transparent background
 * (the host paints `--terminal-bg`) and an invisible cursor. Each tab's host
 * gets two canvases — one UNDER the xterm element for the command cards,
 * bands and gutter marks, one OVER it for the cursor and its trail. Commands
 * are known from the shell integration's OSC 133 marks (A prompt, C output
 * starts, D;code finished) and the working directory from OSC 7; the server
 * injects the hooks for bash and zsh (see plugins/terminal/src/integration.rs).
 *
 * Power: the cursor loop runs only while something moves; at rest the canvas
 * blinks with a CSS animation. With reduced motion or the power saver the
 * cursor snaps and draws no trail.
 */

export const TERMINAL_STYLES = ['smear', 'glass', 'paper', 'blocks'];
export const DEFAULT_TERMINAL_STYLE = 'smear';

export function normalizeTerminalStyle(value) {
  return TERMINAL_STYLES.includes(value) ? value : DEFAULT_TERMINAL_STYLE;
}

/** The commands a tab keeps decorations for (older ones just lose them). */
const MAX_COMMANDS = 300;
/** Idle time before the resting cursor starts blinking. */
const BLINK_AFTER_MS = 650;

const rand = (a, b) => a + Math.random() * (b - a);

/* ── colours ──────────────────────────────────────────────────── */

let colorCtx = null;
/** Normalise any CSS colour to `#rrggbb` (alpha dropped); null if unparsable. */
export function toHex(color) {
  if (!color) return null;
  colorCtx ||= document.createElement('canvas').getContext('2d');
  colorCtx.fillStyle = '#000001';
  colorCtx.fillStyle = color;
  const v = colorCtx.fillStyle;
  if (v === '#000001' && color.trim().toLowerCase() !== '#000001') return null;
  if (v.startsWith('#')) return v.slice(0, 7);
  const m = v.match(/[\d.]+/g);
  if (!m || m.length < 3) return null;
  return `#${m.slice(0, 3).map((n) => Math.round(Number(n)).toString(16).padStart(2, '0')).join('')}`;
}

/**
 * xterm's background: the theme colour at zero alpha. Fully transparent lets
 * the host's paint (and the under-layer) show through, while keeping the RGB
 * matters — inverse video (`less`, `htop` bars) draws its text in the
 * background colour, and xterm makes that opaque.
 */
export function transparentBackground(color) {
  return `${toHex(color) || '#0b0b0c'}00`;
}

/* ── cursor renderers ─────────────────────────────────────────────
   update(dt, T, st) advances the motion and returns whether anything still
   moves; draw(ctx, T, st) paints. T = {x, y, w, h} is the target cell in CSS
   px relative to the host. st carries colours, focus, the glyph under the
   cursor and `calm` (no motion: snap, no trail). */

const corners = (T) => [
  { x: T.x, y: T.y }, { x: T.x + T.w, y: T.y },
  { x: T.x + T.w, y: T.y + T.h }, { x: T.x, y: T.y + T.h },
];

/** Redraw the glyph under a filled block cursor in the background colour —
 *  what a real terminal cursor does — clipped to the current path. */
function knockoutGlyph(ctx, T, st) {
  if (!st.char || !st.char.trim()) return;
  ctx.save();
  ctx.clip();
  ctx.globalAlpha = 1;
  ctx.fillStyle = st.bg;
  ctx.font = st.font;
  ctx.textBaseline = 'middle';
  ctx.fillText(st.char, T.x, T.y + T.h / 2 + 0.5);
  ctx.restore();
}

function hollow(ctx, T, st) {
  ctx.globalAlpha = 0.75;
  ctx.strokeStyle = st.cursor;
  ctx.lineWidth = 1;
  ctx.strokeRect(T.x + 0.5, T.y + 0.5, T.w - 1, T.h - 1);
}

/** smear — the four corners chase their targets: leading corners snap,
 *  trailing ones lag, so a jump stretches the block into a quad. */
function smearCursor() {
  const OFF = [[-1, -1], [1, -1], [1, 1], [-1, 1]];
  let P = null;
  const tau = [0.03, 0.03, 0.03, 0.03];
  let last = null;
  return {
    reset() { P = null; },
    update(dt, T, st) {
      const tg = corners(T);
      if (!P || st.calm) { P = tg.map((p) => ({ ...p })); last = { ...T }; return false; }
      if (last.x !== T.x || last.y !== T.y) {
        const dx = T.x - last.x;
        const dy = T.y - last.y;
        if (Math.abs(dy) < 1 && Math.abs(dx) <= T.w * 1.6) tau.fill(0.026); // typing: crisp
        else {
          const dist = Math.hypot(dx, dy);
          const nx = dx / dist; const ny = dy / dist;
          const dots = OFF.map(([ox, oy]) => ox * nx + oy * ny);
          const order = [0, 1, 2, 3].sort((a, b) => dots[a] - dots[b]);
          const L = [0.16, 0.13, 0.05, 0.035];
          order.forEach((ci, rank) => { tau[ci] = L[rank]; });
        }
        last = { ...T };
      }
      let moving = false;
      P.forEach((p, i) => {
        const k = 1 - Math.exp(-dt / tau[i]);
        p.x += (tg[i].x - p.x) * k;
        p.y += (tg[i].y - p.y) * k;
        if (Math.abs(tg[i].x - p.x) > 0.25 || Math.abs(tg[i].y - p.y) > 0.25) moving = true;
        else { p.x = tg[i].x; p.y = tg[i].y; }
      });
      return moving;
    },
    draw(ctx, T, st) {
      if (!st.focused) { hollow(ctx, T, st); return; }
      ctx.fillStyle = st.cursor;
      ctx.beginPath();
      P.forEach((p, i) => (i ? ctx.lineTo(p.x, p.y) : ctx.moveTo(p.x, p.y)));
      ctx.closePath();
      ctx.fill();
      if (st.shape === 'block') knockoutGlyph(ctx, T, st);
    },
  };
}

/** glass — a glowing accent beam that sheds sparks along its path and drags
 *  a short tapered tail. */
function cometCursor() {
  let p = null;
  let time = 0;
  const parts = [];
  const hist = [];
  return {
    reset() { p = null; parts.length = 0; hist.length = 0; },
    update(dt, T, st) {
      time += dt;
      if (!p || st.calm) { p = { x: T.x, y: T.y }; parts.length = 0; hist.length = 0; return false; }
      const ox = p.x; const oy = p.y;
      const k = 1 - Math.exp(-dt / 0.055);
      p.x += (T.x - p.x) * k;
      p.y += (T.y - p.y) * k;
      if (Math.abs(T.x - p.x) < 0.25 && Math.abs(T.y - p.y) < 0.25) { p.x = T.x; p.y = T.y; }
      const d = Math.hypot(p.x - ox, p.y - oy);
      if (d > 0.6) {
        const n = Math.min(30, Math.ceil(d / 3));
        for (let i = 0; i < n; i++) {
          const f = i / n;
          const life = rand(0.3, 0.65);
          parts.push({
            x: ox + (p.x - ox) * f + rand(-1, 1),
            y: oy + (p.y - oy) * f + rand(0.15, 0.85) * T.h,
            vx: rand(-18, 18) - (p.x - ox) * 0.6,
            vy: rand(-22, 10),
            r: rand(0.5, 1.8),
            life, max: life,
          });
        }
        hist.push({ x: p.x, y: p.y, t: time });
      }
      if (parts.length > 500) parts.splice(0, parts.length - 500);
      for (const q of parts) { q.life -= dt; q.x += q.vx * dt; q.y += q.vy * dt; q.vy += 30 * dt; }
      for (let i = parts.length - 1; i >= 0; i--) if (parts[i].life <= 0) parts.splice(i, 1);
      while (hist.length && time - hist[0].t > 0.14) hist.shift();
      return p.x !== T.x || p.y !== T.y || parts.length > 0 || hist.length > 0;
    },
    draw(ctx, T, st) {
      ctx.fillStyle = st.accent;
      ctx.globalCompositeOperation = st.light ? 'source-over' : 'lighter';
      for (const q of parts) {
        ctx.globalAlpha = (q.life / q.max) * 0.85;
        ctx.beginPath();
        ctx.arc(q.x, q.y, q.r * (0.4 + 0.6 * q.life / q.max), 0, Math.PI * 2);
        ctx.fill();
      }
      const trail = [...hist, { x: p.x, y: p.y }];
      for (let i = 1; i < trail.length; i++) {
        const a = trail[i - 1]; const b = trail[i];
        const f = i / trail.length;
        const hh = T.h * (0.15 + 0.6 * f);
        ctx.globalAlpha = 0.45 * f;
        ctx.beginPath();
        ctx.moveTo(a.x, a.y + (T.h - hh) / 2);
        ctx.lineTo(b.x + 2, b.y + (T.h - hh) / 2);
        ctx.lineTo(b.x + 2, b.y + (T.h + hh) / 2);
        ctx.lineTo(a.x, a.y + (T.h + hh) / 2);
        ctx.fill();
      }
      ctx.globalCompositeOperation = 'source-over';
      ctx.globalAlpha = st.focused ? 1 : 0.45;
      ctx.shadowColor = st.accent;
      ctx.shadowBlur = st.focused ? 12 : 0;
      ctx.fillRect(p.x, p.y + 1, 2.5, T.h - 2);
      ctx.shadowBlur = 0;
    },
  };
}

/** paper — an ink beam that glides, leaving a soft accent highlighter
 *  stroke that dries away behind it. */
function inkCursor() {
  let p = null;
  const hist = [];
  const LIFE = 0.6;
  return {
    reset() { p = null; hist.length = 0; },
    update(dt, T, st) {
      if (!p || st.calm) { p = { x: T.x, y: T.y }; hist.length = 0; return false; }
      const ox = p.x; const oy = p.y;
      const k = 1 - Math.exp(-dt / 0.06);
      p.x += (T.x - p.x) * k;
      p.y += (T.y - p.y) * k;
      if (Math.abs(T.x - p.x) < 0.25 && Math.abs(T.y - p.y) < 0.25) { p.x = T.x; p.y = T.y; }
      for (const h of hist) h.age += dt;
      while (hist.length && hist[0].age > LIFE) hist.shift();
      const moved = Math.hypot(p.x - ox, p.y - oy) > 0.05;
      if (moved) {
        if (!hist.length) hist.push({ x: ox, y: oy + T.h / 2, age: 0 });
        hist.push({ x: p.x, y: p.y + T.h / 2, age: 0 });
      }
      if (hist.length > 400) hist.splice(0, hist.length - 400);
      return p.x !== T.x || p.y !== T.y || hist.length > 0;
    },
    draw(ctx, T, st) {
      ctx.fillStyle = st.accent;
      const th = T.h * 0.78;
      for (let i = 1; i < hist.length; i++) {
        const a = hist[i - 1]; const b = hist[i];
        const dx = b.x - a.x; const dy = b.y - a.y;
        const len = Math.hypot(dx, dy);
        if (len < 0.01) continue;
        const nx = (-dy / len) * th / 2; const ny = (dx / len) * th / 2;
        const f = 1 - a.age / LIFE;
        ctx.globalAlpha = 0.26 * f * f;
        ctx.beginPath();
        ctx.moveTo(a.x + nx, a.y + ny);
        ctx.lineTo(b.x + nx, b.y + ny);
        ctx.lineTo(b.x - nx, b.y - ny);
        ctx.lineTo(a.x - nx, a.y - ny);
        ctx.fill();
      }
      ctx.globalAlpha = st.focused ? 1 : 0.4;
      ctx.fillStyle = st.cursor;
      ctx.beginPath();
      ctx.roundRect(p.x - 1, p.y + 1, 2, T.h - 2, 1);
      ctx.fill();
    },
  };
}

/** blocks — an accent block on an underdamped spring that squashes and
 *  stretches with its speed and leaves a ribbon. */
function elasticCursor() {
  let p = null;
  const v = { x: 0, y: 0 };
  let time = 0;
  const hist = [];
  return {
    reset() { p = null; hist.length = 0; },
    update(dt, T, st) {
      time += dt;
      if (!p || st.calm) { p = { x: T.x, y: T.y }; v.x = 0; v.y = 0; hist.length = 0; return false; }
      const K = 520; const C = 2 * Math.sqrt(K) * 0.58;
      const steps = Math.max(1, Math.ceil(dt / 0.004));
      const h = dt / steps;
      for (let i = 0; i < steps; i++) {
        v.x += (K * (T.x - p.x) - C * v.x) * h;
        v.y += (K * (T.y - p.y) - C * v.y) * h;
        p.x += v.x * h; p.y += v.y * h;
      }
      const settled = Math.abs(T.x - p.x) < 0.2 && Math.abs(T.y - p.y) < 0.2 && Math.hypot(v.x, v.y) < 4;
      if (settled) { p.x = T.x; p.y = T.y; v.x = 0; v.y = 0; } else hist.push({ x: p.x + T.w / 2, y: p.y + T.h / 2, t: time });
      while (hist.length && time - hist[0].t > 0.18) hist.shift();
      return !settled || hist.length > 0;
    },
    draw(ctx, T, st) {
      const sx = 1 + Math.min(1.4, Math.abs(v.x) / 1400);
      const sy = 1 + Math.min(0.6, Math.abs(v.y) / 2600);
      const w = (T.w * sx) / Math.sqrt(sy);
      const h = (T.h * sy) / Math.sqrt(sx);
      const cx = p.x + T.w / 2; const cy = p.y + T.h / 2;
      ctx.strokeStyle = st.accent;
      ctx.lineCap = 'round';
      for (let i = 1; i < hist.length; i++) {
        const f = i / hist.length;
        ctx.globalAlpha = 0.5 * f;
        ctx.lineWidth = T.h * 0.55 * f;
        ctx.beginPath();
        ctx.moveTo(hist[i - 1].x, hist[i - 1].y);
        ctx.lineTo(hist[i].x, hist[i].y);
        ctx.stroke();
      }
      ctx.beginPath();
      if (st.shape === 'bar') ctx.roundRect(cx - T.w / 2, cy - h / 2, 2.5, h, 1);
      else ctx.roundRect(cx - w / 2, cy - h / 2, w, h, Math.min(3, w / 3));
      if (!st.focused) { ctx.globalAlpha = 0.8; ctx.lineWidth = 1.2; ctx.stroke(); return; }
      ctx.globalAlpha = 1;
      ctx.fillStyle = st.accent;
      ctx.fill();
      if (st.shape === 'block') knockoutGlyph(ctx, T, st);
    },
  };
}

const RENDERERS = { smear: smearCursor, glass: cometCursor, paper: inkCursor, blocks: elasticCursor };
/** The cursor shape each style draws at rest (an app may ask for a bar). */
const RESTING_SHAPE = { smear: 'block', glass: 'bar', paper: 'bar', blocks: 'block' };

/* ── command decorations (under the text) ─────────────────────── */

function roundRect(ctx, x, y, w, h, r) {
  ctx.beginPath();
  ctx.roundRect(x, y, w, h, r);
}

function fmtDuration(ms) {
  if (!Number.isFinite(ms)) return '';
  if (ms < 1000) return `${Math.max(1, Math.round(ms))} ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)} s`;
  const m = Math.floor(ms / 60000);
  return `${m}m ${Math.round((ms % 60000) / 1000)}s`;
}

/** The text a finished command shows next to it: `4.2 s · ✓` / `12 ms · ✕ 127`. */
export function commandMeta(cmd) {
  if (!cmd || cmd.state !== 'done') return '';
  const status = cmd.code ? `✕ ${cmd.code}` : '✓';
  return `${fmtDuration(cmd.ms)} · ${status}`;
}

/* ── the per-tab controller ───────────────────────────────────── */

/**
 * Attach the look to one tab's terminal.
 *
 * @param {object} opts
 * @param {Terminal} opts.term   the opened xterm instance
 * @param {HTMLElement} opts.host the tab's host element (xterm's parent)
 * @param {string} opts.style    one of TERMINAL_STYLES
 * @param {() => boolean} opts.calm  true when motion should be suppressed
 * @param {(cwd: string) => void} [opts.onCwd]   OSC 7 working directory
 * @param {(cmd: object) => void} [opts.onCommand] a command finished
 */
export function createTerminalFx({ term, host, style, calm, onCwd, onCommand }) {
  let currentStyle = normalizeTerminalStyle(style);
  let renderer = RENDERERS[currentStyle]();
  let colors = null;
  let layout = null;
  let disposed = false;

  const under = document.createElement('canvas');
  under.className = 'terminal-fx terminal-fx--under';
  const over = document.createElement('canvas');
  over.className = 'terminal-fx terminal-fx--cursor';
  host.prepend(under);
  host.append(over);
  const uctx = under.getContext('2d');
  const octx = over.getContext('2d');

  /* — shell integration state — */
  const commands = [];
  let cursorHidden = false;
  let appShape = null; // 'bar' when an app asked for one (DECSCUSR 5/6)
  let focused = false;
  const disposables = [];

  const has25 = (params) => params.some((p) => p === 25 || (Array.isArray(p) && p[0] === 25));
  disposables.push(
    term.parser.registerCsiHandler({ prefix: '?', final: 'h' }, (params) => { if (has25(params)) { cursorHidden = false; kick(); } return false; }),
    term.parser.registerCsiHandler({ prefix: '?', final: 'l' }, (params) => { if (has25(params)) { cursorHidden = true; kick(); } return false; }),
    term.parser.registerCsiHandler({ intermediates: ' ', final: 'q' }, (params) => {
      const n = Number(params[0]) || 0;
      appShape = n === 5 || n === 6 ? 'bar' : null;
      kick();
      return false;
    }),
    term.parser.registerEscHandler({ final: 'c' }, () => { cursorHidden = false; appShape = null; return false; }),
    term.parser.registerOscHandler(133, (data) => { onPromptMark(data); return true; }),
    term.parser.registerOscHandler(7, (data) => {
      try {
        const path = decodeURIComponent(new URL(data).pathname);
        if (path) onCwd?.(path);
      } catch (_) { /* not a file:// URL */ }
      return true;
    }),
  );

  const lastCommand = () => commands[commands.length - 1] || null;

  function dropCommand(cmd) {
    for (const m of [cmd.start, cmd.end]) { try { m?.dispose(); } catch (_) { /* gone */ } }
    const i = commands.indexOf(cmd);
    if (i >= 0) commands.splice(i, 1);
  }

  function onPromptMark(data) {
    const [kind, arg] = String(data).split(';');
    if (term.buffer.active.type !== 'normal') return;
    const last = lastCommand();
    if (kind === 'A') {
      // A new prompt. A previous prompt that never ran anything (empty Enter,
      // Ctrl+C) leaves no decoration.
      if (last && last.state === 'prompt') dropCommand(last);
      else if (last && last.state === 'running') { last.state = 'done'; last.code = null; last.ms = NaN; }
      const start = term.registerMarker(0);
      if (!start) return;
      const cmd = { start, end: null, state: 'prompt', t0: 0, ms: NaN, code: null };
      start.onDispose(() => { const i = commands.indexOf(cmd); if (i >= 0) commands.splice(i, 1); scheduleDecor(); });
      commands.push(cmd);
      while (commands.length > MAX_COMMANDS) dropCommand(commands[0]);
    } else if (kind === 'C') {
      if (last && last.state === 'prompt') { last.state = 'running'; last.t0 = performance.now(); }
    } else if (kind === 'D') {
      if (last && last.state === 'running') {
        last.state = 'done';
        last.code = arg === undefined || arg === '' ? 0 : Number(arg) || 0;
        last.ms = performance.now() - last.t0;
        last.end = term.registerMarker(0) || null;
        onCommand?.({ code: last.code, ms: last.ms });
      }
    }
    scheduleDecor();
  }

  /* — layout & colours — */

  function readColors() {
    const cs = getComputedStyle(host);
    const v = (name) => cs.getPropertyValue(name).trim();
    const bg = toHex(v('--terminal-bg')) || '#0b0b0c';
    const fg = toHex(v('--terminal-fg')) || '#d8d8dc';
    const n = parseInt(bg.slice(1), 16);
    const lum = (0.2126 * (n >> 16) + 0.7152 * ((n >> 8) & 255) + 0.0722 * (n & 255)) / 255;
    colors = {
      bg, fg,
      cursor: v('--terminal-cursor') || fg,
      accent: v('--accent') || '#8ab4ff',
      red: v('--terminal-ansi-red') || '#e5534b',
      green: v('--terminal-ansi-green') || '#3fb950',
      light: lum > 0.55,
    };
  }

  function measure() {
    const screen = term.element?.querySelector('.xterm-screen');
    if (!screen || !host.isConnected) { layout = null; return; }
    const hr = host.getBoundingClientRect();
    const sr = screen.getBoundingClientRect();
    if (!hr.width || !sr.width || !term.cols || !term.rows) { layout = null; return; }
    const dpr = window.devicePixelRatio || 1;
    for (const c of [under, over]) {
      const w = Math.round(hr.width * dpr); const h = Math.round(hr.height * dpr);
      if (c.width !== w || c.height !== h) { c.width = w; c.height = h; }
    }
    layout = {
      dpr, w: hr.width, h: hr.height,
      ox: sr.left - hr.left, oy: sr.top - hr.top,
      cw: sr.width / term.cols, ch: sr.height / term.rows,
    };
  }

  function font(size) {
    const o = term.options;
    return `${o.fontWeight || 'normal'} ${size || o.fontSize}px ${o.fontFamily}`;
  }

  /* — cursor — */

  let raf = 0;
  let prevT = 0;
  let lastTarget = null;
  let lastViewportY = 0;
  let blinkTimer = 0;

  function target() {
    if (!layout || cursorHidden) return null;
    const buf = term.buffer.active;
    const row = buf.baseY + buf.cursorY - buf.viewportY;
    if (row < 0 || row >= term.rows) return null;
    const line = buf.getLine(buf.baseY + buf.cursorY);
    const cell = line?.getCell(buf.cursorX);
    const wide = cell?.getWidth() === 2;
    return {
      x: Math.round(layout.ox + buf.cursorX * layout.cw),
      y: Math.round(layout.oy + row * layout.ch),
      w: layout.cw * (wide ? 2 : 1),
      h: layout.ch,
      char: cell?.getChars() || '',
    };
  }

  function frame(now) {
    raf = 0;
    if (disposed) return;
    // The first frame after a rest has no previous timestamp: assume one
    // display frame, so the motion starts moving right away.
    const dt = prevT ? Math.min(0.05, (now - prevT) / 1000) : 1 / 60;
    prevT = now;
    if (!layout) measure();
    if (!colors) readColors();
    const T = target();
    octx.setTransform(1, 0, 0, 1, 0, 0);
    octx.clearRect(0, 0, over.width, over.height);
    if (!T) { renderer.reset(); lastTarget = null; prevT = 0; return; }

    const buf = term.buffer.active;
    if (buf.viewportY !== lastViewportY) { renderer.reset(); lastViewportY = buf.viewportY; } // scrolling: no trail
    if (!lastTarget || lastTarget.x !== T.x || lastTarget.y !== T.y) restartBlink();
    lastTarget = T;

    const st = {
      ...colors,
      focused,
      calm: calm(),
      char: T.char,
      font: font(),
      shape: appShape || RESTING_SHAPE[currentStyle],
    };
    const moving = renderer.update(dt, T, st);
    octx.setTransform(layout.dpr, 0, 0, layout.dpr, 0, 0);
    octx.save();
    renderer.draw(octx, T, st);
    octx.restore();
    if (moving) raf = requestAnimationFrame(frame);
    else prevT = 0;
  }

  function restartBlink() {
    over.classList.remove('is-blinking');
    window.clearTimeout(blinkTimer);
    blinkTimer = window.setTimeout(() => { if (focused && !disposed) over.classList.add('is-blinking'); }, BLINK_AFTER_MS);
  }

  function kick() {
    if (disposed || raf) return;
    raf = requestAnimationFrame(frame);
  }

  /* — decorations — */

  let decorRaf = 0;
  function scheduleDecor() {
    if (disposed || decorRaf) return;
    decorRaf = requestAnimationFrame(drawDecor);
  }

  function drawDecor() {
    decorRaf = 0;
    if (disposed) return;
    if (!layout) measure();
    if (!colors) readColors();
    uctx.setTransform(1, 0, 0, 1, 0, 0);
    uctx.clearRect(0, 0, under.width, under.height);
    if (!layout || (currentStyle !== 'paper' && currentStyle !== 'blocks')) return;
    const buf = term.buffer.active;
    if (buf.type !== 'normal') return; // full-screen apps own the screen
    uctx.setTransform(layout.dpr, 0, 0, layout.dpr, 0, 0);
    const { ox, oy, cw, ch } = layout;
    const top = buf.viewportY;
    const bottom = top + term.rows - 1;
    const cursorLine = buf.baseY + buf.cursorY;
    const width = term.cols * cw;
    const rowY = (line) => oy + (line - top) * ch;

    commands.forEach((cmd, i) => {
      if (cmd.start.isDisposed || cmd.start.line < 0) return;
      const first = cmd.start.line;
      let last;
      if (cmd.state === 'done') {
        const next = commands[i + 1];
        const endLine = cmd.end && !cmd.end.isDisposed ? cmd.end.line : (next ? next.start.line : cursorLine + 1);
        last = Math.max(first, endLine - 1);
      } else last = Math.max(first, cursorLine);
      if (last < top || first > bottom) return;
      const y0 = rowY(first);
      const y1 = rowY(last + 1);

      if (currentStyle === 'blocks') drawBlock(cmd, ox, y0, width, y1 - y0, ch);
      else drawMark(cmd, ox, y0, width, y1 - y0, ch, first >= top);
    });
  }

  function drawBlock(cmd, ox, y, width, h, ch) {
    const c = colors;
    const x = ox - 10;
    const w = width + 20;
    const pad = 3;
    roundRect(uctx, x, y + pad, w, h - pad * 2 + 2, 9);
    uctx.globalAlpha = cmd.state === 'done' && cmd.code ? 0.07 : 0.035;
    uctx.fillStyle = cmd.state === 'done' && cmd.code ? c.red : c.fg;
    uctx.fill();
    if (cmd.state === 'prompt') {
      // The live prompt reads as the input box: a quiet accent ring.
      uctx.globalAlpha = 0.45;
      uctx.strokeStyle = c.accent;
      uctx.lineWidth = 1;
      roundRect(uctx, x + 0.5, y + pad + 0.5, w - 1, h - pad * 2 + 1, 9);
      uctx.stroke();
    }
    // Left rule: accent while running, red on failure, a hairline otherwise.
    uctx.globalAlpha = cmd.state === 'running' ? 1 : cmd.state === 'done' && cmd.code ? 0.9 : 0.18;
    uctx.fillStyle = cmd.state === 'running' ? c.accent : cmd.state === 'done' && cmd.code ? c.red : c.fg;
    roundRect(uctx, x + 3, y + pad + 6, 2, Math.max(4, h - pad * 2 - 10), 1);
    uctx.fill();
    drawMeta(cmd, ox + width, y, ch);
  }

  function drawMark(cmd, ox, y, width, h, ch, firstVisible) {
    const c = colors;
    if (cmd.state === 'running') {
      // The running command's lines get a faint accent band.
      uctx.globalAlpha = 0.07;
      uctx.fillStyle = c.accent;
      uctx.fillRect(0, y, layout.w, h);
    }
    if (!firstVisible) return;
    const cx = ox - 15;
    const cy = y + ch / 2;
    uctx.beginPath();
    uctx.arc(cx, cy, 3.5, 0, Math.PI * 2);
    if (cmd.state === 'prompt') {
      uctx.globalAlpha = 0.5;
      uctx.strokeStyle = c.fg;
      uctx.lineWidth = 1.3;
      uctx.stroke();
    } else {
      uctx.globalAlpha = 1;
      uctx.fillStyle = cmd.state === 'running' ? c.accent : cmd.code ? c.red : cmd.code === null ? c.fg : c.green;
      uctx.fill();
    }
    drawMeta(cmd, ox + width, y, ch);
  }

  function drawMeta(cmd, right, y, ch) {
    const text = commandMeta(cmd);
    if (!text) return;
    uctx.globalAlpha = cmd.code ? 0.85 : 0.5;
    uctx.fillStyle = cmd.code ? colors.red : colors.fg;
    uctx.font = font(Math.max(9, Math.round(term.options.fontSize * 0.8)));
    uctx.textAlign = 'right';
    uctx.textBaseline = 'middle';
    uctx.fillText(text, right - 2, y + ch / 2);
    uctx.textAlign = 'left';
  }

  /* — wiring — */

  const onFocus = () => { focused = true; restartBlink(); kick(); };
  const onBlur = () => { focused = false; over.classList.remove('is-blinking'); kick(); };
  term.textarea?.addEventListener('focus', onFocus);
  term.textarea?.addEventListener('blur', onBlur);
  focused = document.activeElement === term.textarea;

  const relayout = () => { measure(); renderer.reset(); kick(); scheduleDecor(); };
  // The host for window resizes; xterm's screen element for cell-size
  // changes (font, line height) that keep the host's size.
  const resizeObserver = new ResizeObserver(relayout);
  resizeObserver.observe(host);
  const screenEl = term.element?.querySelector('.xterm-screen');
  if (screenEl) resizeObserver.observe(screenEl);

  disposables.push(
    term.onCursorMove(kick),
    term.onRender(() => { kick(); scheduleDecor(); }),
    term.onScroll(() => { kick(); scheduleDecor(); }),
    term.onResize(() => queueMicrotask(relayout)),
  );

  host.dataset.terminalStyle = currentStyle;
  kick();

  return {
    get style() { return currentStyle; },
    get commands() { return commands; },
    setStyle(next) {
      next = normalizeTerminalStyle(next);
      if (next === currentStyle) return;
      currentStyle = next;
      host.dataset.terminalStyle = next;
      renderer = RENDERERS[next]();
      queueMicrotask(relayout);
    },
    /** Theme or accent changed: re-read the tokens and repaint. */
    refreshColors() { readColors(); renderer.reset(); kick(); scheduleDecor(); },
    /** Layout changed from outside (fit, tab switch). */
    relayout,
    dispose() {
      disposed = true;
      cancelAnimationFrame(raf);
      cancelAnimationFrame(decorRaf);
      window.clearTimeout(blinkTimer);
      resizeObserver.disconnect();
      term.textarea?.removeEventListener('focus', onFocus);
      term.textarea?.removeEventListener('blur', onBlur);
      for (const d of disposables) { try { d.dispose(); } catch (_) { /* gone */ } }
      for (const cmd of [...commands]) dropCommand(cmd);
      under.remove();
      over.remove();
    },
  };
}

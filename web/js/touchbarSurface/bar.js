/**
 * bar.js — the Touch Bar surface (web/touchbar.html).
 *
 * The kiosk shell renders this page on the MacBook's Touch Bar and feeds it
 * the panel's touches (`window.__shinyBarTouch`) and the Fn key
 * (`window.__shinyBarFn`). The page keeps no state of its own: the desktop
 * page publishes a snapshot of what to show over a BroadcastChannel
 * (`js/touchbarHost.js`), and this page answers with what was touched.
 *
 * It imports one thing from the app: the assistant's dot renderer, so the bar
 * shows the same effect as the dock. Everything else arrives ready: labels
 * translated, icons as markup.
 *
 * The shell samples the page only while its picture changes. Changes it
 * cannot see coming (new state, a layer switch) are announced by bumping the
 * document title — see `bump()` and `bar_title_changed` in the shell.
 */

import { mountVoiceDots } from '../voiceDots.js';
import { CHANNEL, FN_KEYS, clamp01, sliderValue, scrollLimit, flingStep } from './barShared.js';

const bar = document.getElementById('bar');
const orb = document.getElementById('orb');
const strip = document.getElementById('strip');
const track = document.getElementById('strip-track');
const tray = document.getElementById('tray');
const fnLayer = document.getElementById('fn-layer');
const menuLayer = document.getElementById('menu-layer');

const channel = typeof BroadcastChannel === 'function' ? new BroadcastChannel(CHANNEL) : null;
const send = (message) => { try { channel?.postMessage(message); } catch (_) { /* closed */ } };

let state = null;
let asleep = false;
let seq = 0;

/** Tell the shell the picture is about to change. */
function bump() {
  seq += 1;
  document.title = `Touch Bar ${seq}`;
}

const unit = () => window.innerHeight / 60;

/* ── Rendering ────────────────────────────────────────────────── */

function glyph(icon) {
  const span = document.createElement('span');
  span.className = 'glyph';
  fillGlyph(span, icon);
  return span;
}

function fillGlyph(span, icon) {
  const key = icon ? (icon.svg || icon.img || '') : '';
  if (span.dataset.key === key) return;
  span.dataset.key = key;
  if (icon?.svg) {
    // Markup from the desktop page's own (already sanitised) icons.
    span.innerHTML = icon.svg;
  } else if (icon?.img) {
    const img = document.createElement('img');
    img.src = icon.img;
    img.alt = '';
    span.replaceChildren(img);
  } else {
    span.replaceChildren();
  }
}

function part(el, className, text) {
  let node = el.querySelector(`:scope > .${className}`);
  if (text == null || text === '') {
    node?.remove();
    return;
  }
  if (!node) {
    node = document.createElement('span');
    node.className = className;
    el.append(node);
  }
  if (node.textContent !== text) node.textContent = text;
}

/** Build or refresh one item's element from its description. */
function paint(el, item) {
  // A finger on the item keeps its pressed / sliding look through a repaint.
  const held = touch?.el === el
    ? ['is-pressed', 'is-sliding'].filter((name) => el.classList.contains(name))
    : [];
  el.className = `item item--${item.kind}`;
  el.classList.add(...held);
  el.classList.toggle('is-active', !!item.active);
  el.classList.toggle('is-focused', !!item.focused);
  el.classList.toggle('is-disabled', !!item.disabled);
  el.classList.toggle('is-on', !!item.on);
  el.classList.toggle('is-muted', !!item.muted);
  if (item.label) el.setAttribute('aria-label', item.label);

  if (item.kind === 'gap') {
    el.className = 'gap';
    return;
  }
  if (item.kind === 'spaces') {
    const dots = el.querySelectorAll('.dot');
    if (dots.length !== item.count) {
      el.replaceChildren(...Array.from({ length: item.count }, (_, i) => {
        const dot = document.createElement('span');
        dot.className = 'dot';
        dot.dataset.index = String(i);
        return dot;
      }));
    }
    el.querySelectorAll('.dot').forEach((dot, i) => dot.classList.toggle('is-current', i === item.active));
    // The dock's "new workspace" button.
    if (!el.querySelector('.add')) {
      const add = document.createElement('span');
      add.className = 'add';
      add.textContent = '+';
      el.append(add);
    }
    return;
  }
  if (item.kind === 'slider') {
    if (!el.querySelector('.fill')) {
      const fill = document.createElement('span');
      fill.className = 'fill';
      el.prepend(fill);
    }
    // A finger on the slider owns its value until it lifts.
    if (touch?.el !== el) el.style.setProperty('--value', String(clamp01(item.value)));
  }
  if (item.icon || el.querySelector(':scope > .glyph')) {
    let span = el.querySelector(':scope > .glyph');
    if (!span) {
      span = glyph(item.icon);
      el.append(span);
    } else {
      fillGlyph(span, item.icon);
    }
  }
  part(el, 'text', item.text);
  part(el, 'sub', item.sub);
}

/**
 * Bring `container` in line with `items`, keyed by id: elements that stay are
 * updated in place (so their transitions run), new ones animate in with a
 * stagger, and the ones that left fade out before they are removed.
 */
function sync(container, items) {
  const existing = new Map();
  for (const el of container.children) {
    if (!el.classList.contains('is-leaving')) existing.set(el.dataset.id, el);
  }
  let fresh = 0;
  let cursor = container.firstElementChild;
  while (cursor?.classList.contains('is-leaving')) cursor = cursor.nextElementSibling;
  for (const item of items) {
    let el = existing.get(item.id);
    if (el) {
      existing.delete(item.id);
    } else {
      el = document.createElement('div');
      el.dataset.id = item.id;
      el.style.setProperty('--i', String(fresh));
      fresh += 1;
    }
    el.dataset.kind = item.kind;
    paint(el, item);
    if (el !== cursor) container.insertBefore(el, cursor);
    else {
      cursor = cursor.nextElementSibling;
      while (cursor?.classList.contains('is-leaving')) cursor = cursor.nextElementSibling;
    }
  }
  for (const el of existing.values()) {
    el.classList.add('is-leaving');
    setTimeout(() => el.remove(), 220);
  }
}

/** The scrolling strip: the focused window's controls, then the dock's apps. */
function stripItems(next) {
  const items = [];
  const context = next.context;
  if (context?.items?.length) {
    if (context.title) items.push({ id: `title:${context.name}`, kind: 'title', text: context.title });
    for (const entry of context.items) {
      items.push({
        id: `ctx:${context.name}:${entry.id}`,
        kind: 'context',
        label: entry.label,
        // A button with an icon shows only the icon unless it asks for both.
        text: entry.icon && !entry.showLabel ? '' : entry.label,
        icon: entry.icon,
        active: entry.active,
        disabled: entry.disabled,
      });
    }
  }
  if (next.apps?.length) {
    if (items.length) items.push({ id: 'gap:apps', kind: 'gap' });
    for (const app of next.apps) {
      items.push({ ...app, kind: 'app', text: app.icon ? '' : app.label });
    }
  }
  return items;
}

/** The fixed right end: workspaces, sliders, the status chips, the clock. */
function trayItems(next) {
  const items = [];
  // The workspace switcher keeps its place: it does not scroll with the apps.
  if (next.spaces) {
    items.push({ id: 'spaces', kind: 'spaces', count: next.spaces.count, active: next.spaces.active });
  }
  for (const slider of next.sliders || []) items.push({ ...slider, kind: 'slider' });
  for (const chip of next.status || []) items.push({ ...chip, kind: chip.kind === 'clock' ? 'clock' : 'chip' });
  if (next.menu?.items?.length) {
    items.push({ id: 'more', kind: 'more', label: next.menu.label, icon: next.menu.icon, text: next.menu.icon ? '' : '…', on: next.menu.on });
  }
  return items;
}

function render(next) {
  state = next;
  bar.dataset.linked = '1';
  bar.dataset.ai = next.ai === false ? '0' : '1';
  if (next.accent) document.documentElement.style.setProperty('--accent', next.accent);
  setOrbPrefs(next.orb);
  setVoice(next.voice || 'idle');
  sync(track, stripItems(next));
  sync(tray, trayItems(next));
  const entries = (next.menu?.items || []).map((entry) => ({
    ...entry, kind: 'menu', text: entry.label,
  }));
  sync(menuLayer, [{ id: 'menu:close', kind: 'menu-close', text: '✕' }, ...entries]);
  for (const el of menuLayer.children) el.classList.toggle('is-danger', !!entries.find((e) => e.id === el.dataset.id)?.danger);
  if (!entries.length && bar.dataset.layer === 'menu') setLayer('main');
  setScroll(scroll);
  bump();
}

function buildFnLayer() {
  fnLayer.replaceChildren(...FN_KEYS.map((number, i) => {
    const key = document.createElement('div');
    key.className = 'item item--fn';
    key.dataset.id = `fn:${number}`;
    key.dataset.kind = 'fn';
    key.style.setProperty('--i', String(i));
    const text = document.createElement('span');
    text.className = 'text';
    text.textContent = `F${number}`;
    key.append(text);
    return key;
  }));
}

let menuTimer = 0;

function setLayer(layer) {
  clearTimeout(menuTimer);
  // The menu closes by itself when it is left alone.
  if (layer === 'menu') menuTimer = setTimeout(() => setLayer('main'), 8000);
  if (bar.dataset.layer === layer) return;
  bar.dataset.layer = layer;
  bump();
}

/* ── Scrolling ────────────────────────────────────────────────── */

let scroll = 0;
let fling = 0;

function setScroll(value) {
  const limit = scrollLimit(track.scrollWidth, strip.clientWidth);
  scroll = Math.max(0, Math.min(limit, value));
  track.style.transform = `translate3d(${-scroll}px, 0, 0)`;
}

function startFling(velocity) {
  cancelAnimationFrame(fling);
  let v = velocity;
  let last = performance.now();
  const step = (now) => {
    const moved = flingStep(v, now - last);
    last = now;
    v = moved.velocity;
    const before = scroll;
    setScroll(scroll + moved.distance);
    if (moved.done || scroll === before) return;
    fling = requestAnimationFrame(step);
  };
  fling = requestAnimationFrame(step);
}

/* ── Touch ────────────────────────────────────────────────────── */

/** The one contact the bar follows (the first finger down). */
let touch = null;
let slideSentAt = 0;

function ripple(el, x, y) {
  const rect = el.getBoundingClientRect();
  const drop = document.createElement('span');
  drop.className = 'ripple';
  drop.style.left = `${x - rect.left}px`;
  drop.style.top = `${y - rect.top}px`;
  el.append(drop);
  setTimeout(() => drop.remove(), 600);
}

function touchDown(id, x, y) {
  if (touch) return;
  send({ t: 'activity' });
  if (asleep) {
    // The first touch on a dark bar only wakes it.
    touch = { id, swallowed: true };
    return;
  }
  cancelAnimationFrame(fling);
  const hit = document.elementFromPoint(x, y);
  const el = hit?.closest('.item, .orb') || null;
  const inStrip = !!hit?.closest('#strip');
  touch = {
    id, el, inStrip, x0: x, x, t: performance.now(), v: 0,
    scroll0: scroll, moved: false, kind: el?.dataset.kind || (el === orb ? 'orb' : ''),
    value0: 0,
  };
  if (!el || el.classList.contains('is-disabled')) return;
  el.classList.add('is-pressed');
  ripple(el, x, y);
  // The assistant's button follows the dock's rules (one tap: chat, two:
  // listen, hold: wake word). The desktop page times them, so it gets the
  // press and the release as they happen.
  if (touch.kind === 'orb') send({ t: 'orb', phase: 'down' });
  if (touch.kind === 'slider') {
    touch.value0 = Number(el.style.getPropertyValue('--value')) || 0;
  }
}

function touchMove(id, x) {
  if (!touch || touch.id !== id || touch.swallowed) return;
  const now = performance.now();
  const dt = Math.max(1, now - touch.t);
  touch.v = 0.7 * touch.v + 0.3 * ((touch.x - x) / dt);
  touch.x = x;
  touch.t = now;
  const dx = x - touch.x0;
  const el = touch.el;

  if (touch.kind === 'slider' && el) {
    if (!touch.moved && Math.abs(dx) < unit() * 6) return;
    if (!touch.moved) {
      touch.moved = true;
      el.classList.add('is-sliding');
      el.classList.remove('is-pressed');
    }
    const value = sliderValue(touch.value0, dx, unit() * 300);
    el.style.setProperty('--value', String(value));
    if (now - slideSentAt > 50) {
      slideSentAt = now;
      send({ t: 'slide', id: el.dataset.id, value });
    }
    return;
  }

  if (!touch.moved && Math.abs(dx) > unit() * 12) {
    touch.moved = true;
    el?.classList.remove('is-pressed');
  }
  if (touch.moved && touch.inStrip && touch.kind !== 'spaces') setScroll(touch.scroll0 - dx);
}

function touchUp(id) {
  if (!touch || touch.id !== id) return;
  const done = touch;
  touch = null;
  if (done.swallowed) return;
  if (done.kind === 'orb') {
    done.el.classList.remove('is-pressed');
    send({ t: 'orb', phase: 'up' });
    return;
  }
  const el = done.el;
  el?.classList.remove('is-pressed');
  if (!el || el.classList.contains('is-disabled')) {
    if (done.moved && done.inStrip) startFling(done.v);
    return;
  }
  const dx = done.x - done.x0;

  if (done.kind === 'slider') {
    el.classList.remove('is-sliding');
    if (done.moved) {
      send({ t: 'slide', id: el.dataset.id, value: sliderValue(done.value0, dx, unit() * 300), final: true });
    } else {
      send({ t: 'tap', id: el.dataset.id });
    }
    bump();
    return;
  }
  if (done.kind === 'spaces') {
    // A swipe across the dots walks the workspaces; a tap picks one.
    if (done.moved) send({ t: 'space', dir: dx < 0 ? 'next' : 'prev' });
    else {
      const hit = document.elementFromPoint(done.x, window.innerHeight / 2);
      const dot = hit?.closest('.dot');
      if (hit?.closest('.add')) send({ t: 'space', add: true });
      else if (dot) send({ t: 'space', index: Number(dot.dataset.index) });
    }
    return;
  }
  if (done.moved) {
    if (done.inStrip) startFling(done.v);
    return;
  }
  if (done.kind === 'more') setLayer('menu');
  else if (done.kind === 'menu-close') setLayer('main');
  else if (done.kind === 'menu') {
    send({ t: 'tap', id: el.dataset.id });
    setLayer('main');
  } else if (done.kind === 'fn') send({ t: 'fkey', n: Number(el.dataset.id.slice(3)) });
  else send({ t: 'tap', id: el.dataset.id });
}

/** From the shell: `[[phase, id, x, y], …]`, positions as fractions of the bar. */
window.__shinyBarTouch = (batch) => {
  for (const [phase, id, fx, fy] of batch) {
    const x = fx * window.innerWidth;
    const y = fy * window.innerHeight;
    if (phase === 'down') touchDown(id, x, y);
    else if (phase === 'move') touchMove(id, x);
    else touchUp(id);
  }
};

/** From the shell: the physical Fn key. */
window.__shinyBarFn = (held) => setLayer(held ? 'fn' : 'main');
window.addEventListener('pointerdown', () => { if (bar.dataset.layer === 'menu') setLayer('menu'); }, true);

// A mouse does the same job, so the page can be tried in a browser tab.
window.addEventListener('pointerdown', (e) => touchDown(-1, e.clientX, e.clientY));
window.addEventListener('pointermove', (e) => { if (e.buttons) touchMove(-1, e.clientX); });
window.addEventListener('pointerup', () => touchUp(-1));
window.addEventListener('keydown', (e) => { if (e.key === 'Alt') setLayer('fn'); });
window.addEventListener('keyup', (e) => { if (e.key === 'Alt') setLayer('main'); });

/* ── The assistant's dots ─────────────────────────────────────── */

// The dock's own renderer, so the bar shows the effect picked in Settings →
// Appearance and reacts to the same things. It listens on `window` for the
// desktop's voice events, which arrive here over the channel and are
// dispatched again; the effect and what it reacts to come with the snapshot.
const orbPrefs = { effect: 'glyph', react: 'both' };
let dots = null;
let voiceState = 'idle';
let stepping = false;
let pulse = 0;

function mountDots() {
  if (dots) return;
  dots = mountVoiceDots(orb, { options: () => orbPrefs });
}

function setOrbPrefs(next) {
  if (!next || (next.effect === orbPrefs.effect && next.react === orbPrefs.react)) return;
  orbPrefs.effect = next.effect || 'glyph';
  orbPrefs.react = next.react || 'both';
  dots?.refresh();
}

/** While the assistant is doing something the dots move at frame rate, and
 *  the shell is told to keep up. At rest their shimmer is slow enough for
 *  the shell's own slower sampling. */
function keepPace() {
  const busy = voiceState !== 'idle' && voiceState !== 'disabled' && voiceState !== 'error';
  if ((busy || stepping) && !pulse) pulse = setInterval(bump, 300);
  else if (!busy && !stepping && pulse) { clearInterval(pulse); pulse = 0; }
}

function setVoice(next) {
  if (voiceState === next) return;
  voiceState = next;
  bar.dataset.voice = next;
  document.documentElement.dataset.voice = next;
  window.dispatchEvent(new CustomEvent('voice:state', { detail: { state: next } }));
  keepPace();
  bump();
}

/* ── The desktop page ─────────────────────────────────────────── */

if (channel) {
  channel.onmessage = ({ data }) => {
    if (!data || typeof data !== 'object') return;
    if (data.t === 'state') render(data.state);
    else if (data.t === 'voice') setVoice(data.state || 'idle');
    else if (data.t === 'level') {
      window.dispatchEvent(new CustomEvent('voice:level', { detail: { level: clamp01(data.level), pan: data.pan } }));
    } else if (data.t === 'step') {
      stepping = !!data.message;
      window.dispatchEvent(new CustomEvent('agent:step', { detail: { message: data.message, action: data.action } }));
      keepPace();
      bump();
    } else if (data.t === 'sleep') {
      asleep = !!data.on;
      bar.dataset.asleep = asleep ? '1' : '0';
      bump();
    } else if (data.t === 'bye') {
      bar.dataset.linked = '0';
      state = null;
      standby();
    }
  };
}

/** With no desktop page to mirror (sign-in screen, loading) the bar still
 *  tells the time. */
function standby() {
  if (state) return;
  const now = new Date();
  const text = now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const sub = now.toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short' });
  sync(track, []);
  sync(tray, [{ id: 'clock', kind: 'clock', text, sub }]);
  bump();
}

// Ask for a snapshot until the desktop page answers (it may still be booting,
// or sitting on the sign-in screen).
function hello() {
  if (state) return;
  send({ t: 'hello' });
  standby();
}
setInterval(hello, 2000);
window.addEventListener('resize', () => setScroll(scroll));

buildFnLayer();
mountDots();
hello();

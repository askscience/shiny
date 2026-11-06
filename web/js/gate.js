/**
 * gate.js — the stage shared by the sign-in screen and the lock screen.
 *
 * Both are the same place: the time set very large in the theme's display
 * face, the date as a mono kicker, the weather, the brand, an accent aura —
 * and a panel beside it (the profile index and the password form at sign-in,
 * the password alone at unlock). This module owns that stage:
 *
 *   - `gateChrome(host)` fills a `[data-gate-aside]` box with the brand, the
 *     clock, the date and the weather, and keeps them current;
 *   - `gateEnter` / `gateExit` / `gateReveal` / `gateShake` move it in the
 *     user's motion language (`<html data-motion>`, Settings → Appearance →
 *     Motion): kinetic wipes behind masks, fluid rides real springs, depth
 *     comes out of a blur and leaves through the viewer;
 *   - `gateLookFor(user)` / `applyGateLook(look)` dress the sign-in screen in
 *     the accent, gradient and motion a profile chose on this device, before
 *     that user is signed in.
 *
 * The weather is the top bar's (hudLeft.js, `weather:update`); the last copy
 * is kept unscoped (`gate.weather`) so the sign-in screen has one before any
 * session exists. Layout and look live in css/gate.css.
 */
import {
  motionStyle, SPRINGS, EASE, reducedMotion, setIcon, applyAppearance, getAccent, getGradient,
  getActiveTheme, setTheme,
} from '../ui/index.js';
import { getLocale, t } from './i18n.js';
import { getKnownUsers } from './userProfiles.js';
import { tempText, weatherIconStem, weatherLabel } from './weatherShared.js';

const WEATHER_KEY = 'gate.weather';
/** A cached reading older than this is not worth showing. */
const WEATHER_MAX_AGE_MS = 6 * 60 * 60 * 1000;

/* ── Weather cache ──────────────────────────────────────────── */

let liveWeather = null;

function readCachedWeather() {
  try {
    const w = JSON.parse(localStorage.getItem(WEATHER_KEY) || 'null');
    if (w?.current && Date.now() - (w.at || 0) < WEATHER_MAX_AGE_MS) return w;
  } catch (_) { /* nothing cached */ }
  return null;
}

if (typeof window !== 'undefined') {
  window.addEventListener('weather:update', (event) => {
    const w = event.detail;
    if (!w?.current) return;
    liveWeather = w;
    try {
      localStorage.setItem(WEATHER_KEY, JSON.stringify({
        timezone: w.timezone || null,
        current: { code: w.current.code, temp: w.current.temp },
        at: w.at || Date.now(),
      }));
    } catch (_) { /* storage full or off: the live copy still shows */ }
  });
}

function currentWeather() {
  return liveWeather || readCachedWeather();
}

/* ── Look (accent, gradient, motion) per profile ─────────────── */

function readJson(key) {
  try { return JSON.parse(localStorage.getItem(key) || 'null'); } catch (_) { return null; }
}

/**
 * The look a profile picked on this device: accent, gradient and motion, read
 * from that user's own scoped preferences. `user` is a known profile, or a
 * `{ username }` (Linux accounts carry no id); `null` → the last one signed in.
 */
export function gateLookFor(user) {
  // The machine's login screen: each account's look comes with the account
  // list, published by its own session (greeterLook.js) — theme included.
  if (user?.look) return { ...user.look };
  const known = getKnownUsers();
  const match = user
    ? known.find((u) => (user.id && u.id === user.id) || (user.username && u.username === user.username))
    : known[0];
  if (!match?.id) return null;
  const id = match.id;
  const surface = readJson(`desktop.surface.${id}`);
  return {
    accent: localStorage.getItem(`ui.accent.${id}`) || null,
    gradient: readJson(`ui.gradient.${id}`),
    motion: surface?.motion || null,
  };
}

/** Dress the stage in a look, without storing anything. */
export function applyGateLook(look) {
  const root = document.documentElement;
  if (look?.motion) root.dataset.motion = look.motion;
  const paint = () => {
    const gradient = look?.gradient?.stops?.length >= 2 ? look.gradient : getGradient();
    applyAppearance({ accent: look?.accent || getAccent(), gradient });
  };
  // Only a published look names a theme (the in-app sign-in keeps the
  // device's). The accent is painted again once the theme is in: whether it
  // reads on the canvas depends on the theme's mode.
  if (look?.theme && look.theme !== getActiveTheme()) {
    void setTheme(look.theme).then(paint, () => {});
  }
  paint();
}

/** Of the accounts the login screen lists, the look published last, or null. */
export function latestLook(users) {
  let best = null;
  for (const user of users || []) {
    if (user?.look && (!best || (user.look.updated || 0) > (best.updated || 0))) best = user.look;
  }
  return best;
}

/* ── Chrome: brand, clock, date, weather ─────────────────────── */

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

/**
 * The time as `{ digits, period }`: "09:41" set large, and the AM/PM of a
 * 12-hour locale kept apart as a small suffix. A 24-hour clock pads the hour
 * ("09:41"); a 12-hour one does not ("9:41").
 */
function timeParts(date, timeZone) {
  const make = (zone) => {
    const probe = new Intl.DateTimeFormat(getLocale(), { hour: 'numeric' }).resolvedOptions();
    const h12 = probe.hourCycle === 'h12' || probe.hourCycle === 'h11';
    const opts = { hour: h12 ? 'numeric' : '2-digit', minute: '2-digit' };
    if (zone) opts.timeZone = zone;
    return new Intl.DateTimeFormat(getLocale(), opts);
  };
  let fmt;
  try { fmt = make(timeZone); } catch (_) { fmt = make(undefined); }
  let digits = '';
  let period = '';
  for (const part of fmt.formatToParts(date)) {
    if (part.type === 'dayPeriod') period = part.value;
    else if (part.type === 'hour' || part.type === 'minute') digits += part.value;
    else if (part.type === 'literal' && digits && !period && /[:.]/.test(part.value)) digits += part.value.trim();
  }
  return { digits, period };
}

function dateText(date, timeZone) {
  const opts = { weekday: 'long', day: 'numeric', month: 'long' };
  if (timeZone) opts.timeZone = timeZone;
  try { return new Intl.DateTimeFormat(getLocale(), opts).format(date); } catch (_) {
    return date.toDateString();
  }
}

/** Swap one digit of the clock, moving it the way the language moves. */
function swapGlyph(cell, ch, animate) {
  const prev = cell.firstChild;
  if (prev?.textContent === ch) return;
  const next = el('span', 'gate-glyph', ch);
  cell.appendChild(next);
  if (!animate || reducedMotion() || typeof next.animate !== 'function') {
    prev?.remove();
    return;
  }
  const style = motionStyle();
  if (style === 'depth') {
    next.animate([{ opacity: 0, filter: 'blur(14px)', transform: 'scale(1.25)' }, { opacity: 1, filter: 'blur(0)', transform: 'none' }],
      { duration: 700, easing: EASE.expo });
    prev?.animate([{ opacity: 1 }, { opacity: 0, filter: 'blur(14px)', transform: 'scale(.8)' }],
      { duration: 500, easing: EASE.out, fill: 'forwards' }).finished.then(() => prev.remove(), () => prev.remove());
    return;
  }
  const sp = style === 'fluid' ? SPRINGS.bouncy : { easing: EASE.expo, duration: 760 };
  next.animate([{ transform: 'translateY(105%)' }, { transform: 'none' }], { duration: sp.duration, easing: sp.easing });
  prev?.animate([{ transform: 'none' }, { transform: 'translateY(-105%)' }],
    { duration: Math.min(sp.duration, 620), easing: style === 'fluid' ? sp.easing : EASE.cine, fill: 'forwards' })
    .finished.then(() => prev.remove(), () => prev.remove());
}

function renderTime(timeEl, text, animate) {
  const chars = [...text];
  while (timeEl.children.length > chars.length) timeEl.lastChild.remove();
  while (timeEl.children.length < chars.length) timeEl.appendChild(el('span', 'gate-cell'));
  chars.forEach((ch, i) => {
    const cell = timeEl.children[i];
    cell.classList.toggle('gate-cell--sep', !/\d/.test(ch));
    swapGlyph(cell, ch, animate);
  });
}

/**
 * Fill `host` (a `[data-gate-aside]`) with the brand, clock, date and weather.
 * Returns `{ refresh, destroy }`; the clock ticks on the minute until destroyed.
 */
export function gateChrome(host) {
  if (!host) return { refresh() {}, destroy() {} };
  if (host.__gate) return host.__gate;
  host.textContent = '';

  const brand = el('div', 'gate-brand');
  brand.setAttribute('data-gate-in', '');
  const logo = el('span', 'app-icon gate-brand-logo');
  logo.setAttribute('aria-hidden', 'true');
  brand.append(logo, el('span', 'gate-brand-name', "LiteAura"));

  const clock = el('div', 'gate-clock');
  const dateEl = el('p', 'gate-date');
  dateEl.setAttribute('data-gate-in', '');
  const timeRow = el('div', 'gate-time-row');
  const timeEl = el('div', 'gate-time');
  timeEl.setAttribute('role', 'timer');
  const periodEl = el('span', 'gate-period');
  timeRow.append(timeEl, periodEl);
  const weather = el('p', 'gate-weather hidden');
  weather.setAttribute('data-gate-in', '');
  const wIcon = el('span', 'gate-weather-icon');
  wIcon.setAttribute('aria-hidden', 'true');
  const wTemp = el('span', 'gate-weather-temp');
  const wLabel = el('span', 'gate-weather-label');
  weather.append(wIcon, wTemp, wLabel);
  clock.append(dateEl, timeRow, weather);
  host.append(brand, clock);

  let lastStem = '';
  function refresh({ animate = true } = {}) {
    const w = currentWeather();
    const zone = w?.timezone || undefined;
    const now = new Date();
    const { digits, period } = timeParts(now, zone);
    renderTime(timeEl, digits, animate && timeEl.children.length > 0);
    periodEl.textContent = period;
    periodEl.hidden = !period;
    timeEl.setAttribute('aria-label', period ? `${digits} ${period}` : digits);
    dateEl.textContent = dateText(now, zone);
    if (w?.current) {
      weather.classList.remove('hidden');
      const stem = weatherIconStem(w.current.code);
      if (stem !== lastStem) {
        lastStem = stem;
        void setIcon(wIcon, `insights/${stem}`, { size: 22 });
      }
      wTemp.textContent = tempText(w.current.temp);
      wLabel.textContent = weatherLabel(w.current.code);
      weather.setAttribute('aria-label', t('gate.weather', { temp: wTemp.textContent, label: wLabel.textContent }, `${wTemp.textContent}, ${wLabel.textContent}`));
    } else {
      weather.classList.add('hidden');
    }
  }

  let timer = 0;
  const tick = () => {
    refresh();
    const now = new Date();
    timer = setTimeout(tick, (60 - now.getSeconds()) * 1000 - now.getMilliseconds() + 30);
  };
  refresh({ animate: false });
  const nowAtStart = new Date();
  timer = setTimeout(tick, (60 - nowAtStart.getSeconds()) * 1000 - nowAtStart.getMilliseconds() + 30);
  const onWeather = () => refresh({ animate: false });
  window.addEventListener('weather:update', onWeather);
  window.addEventListener('locale:change', onWeather);

  const api = {
    refresh,
    destroy() {
      clearTimeout(timer);
      window.removeEventListener('weather:update', onWeather);
      window.removeEventListener('locale:change', onWeather);
      delete host.__gate;
    },
  };
  host.__gate = api;
  return api;
}

/* ── Motion ─────────────────────────────────────────────────── */

const still = () => reducedMotion() || typeof Element === 'undefined' || typeof Element.prototype.animate !== 'function';

/** The pieces that arrive one after the other, in document order. */
function arrivals(root) {
  return [...root.querySelectorAll('[data-gate-in]')].filter((n) => n.getClientRects().length);
}

function enterOne(node, i, style, base = 0) {
  const delay = base + i * (style === 'depth' ? 80 : style === 'fluid' ? 55 : 65);
  if (style === 'depth') {
    return node.animate([
      { opacity: 0, filter: 'blur(18px)', transform: 'scale(1.12)' },
      { opacity: 1, filter: 'blur(0)', transform: 'none' },
    ], { duration: 1000, delay, easing: EASE.expo, fill: 'backwards' });
  }
  if (style === 'fluid') {
    return node.animate([
      { opacity: 0, transform: 'translateY(36px) scale(.96)' },
      { opacity: 1, transform: 'none' },
    ], { duration: SPRINGS.bouncy.duration, delay, easing: SPRINGS.bouncy.easing, fill: 'backwards' });
  }
  return node.animate([
    { clipPath: 'inset(0 0 100% 0)', transform: 'translateY(28px)' },
    { clipPath: 'inset(0 0 0% 0)', transform: 'none' },
  ], { duration: 760, delay, easing: EASE.cine, fill: 'backwards' });
}

/** The whole stage arriving: backdrop, the clock's digits, then the rest. */
export function gateEnter(root) {
  if (!root || still()) return Promise.resolve();
  const style = motionStyle();
  const runs = [root.animate([{ opacity: 0 }, { opacity: 1 }], { duration: style === 'depth' ? 600 : 360, easing: EASE.out })];
  const glyphs = [...root.querySelectorAll('.gate-time .gate-glyph')];
  glyphs.forEach((g, i) => {
    const delay = 120 + i * 55;
    if (style === 'depth') {
      runs.push(g.animate([{ opacity: 0, filter: 'blur(20px)', transform: 'scale(1.4)' }, { opacity: 1, filter: 'blur(0)', transform: 'none' }],
        { duration: 1100, delay, easing: EASE.expo, fill: 'backwards' }));
    } else if (style === 'fluid') {
      runs.push(g.animate([{ transform: 'translateY(80%) scale(.85)', opacity: 0 }, { transform: 'none', opacity: 1 }],
        { duration: SPRINGS.bouncy.duration, delay, easing: SPRINGS.bouncy.easing, fill: 'backwards' }));
    } else {
      runs.push(g.animate([{ transform: 'translateY(108%)' }, { transform: 'none' }],
        { duration: 900, delay, easing: EASE.expo, fill: 'backwards' }));
    }
  });
  arrivals(root).forEach((n, i) => runs.push(enterOne(n, i, style, 260)));
  return Promise.all(runs.map((a) => a.finished.catch(() => {}))).then(() => {});
}

/** A panel step coming up (the password form after the profile index…). */
export function gateReveal(container) {
  if (!container || still()) return Promise.resolve();
  const style = motionStyle();
  const nodes = arrivals(container);
  return Promise.all(nodes.map((n, i) => enterOne(n, i, style).finished.catch(() => {}))).then(() => {});
}

/**
 * The stage leaving, uncovering what is behind it. `fromY` continues a swipe
 * from where the finger let go (px, negative = up).
 */
export function gateExit(root, { fromY = 0 } = {}) {
  if (!root || still()) return Promise.resolve();
  const style = motionStyle();
  const start = fromY ? `translateY(${fromY}px)` : 'none';
  let anim;
  if (fromY) {
    const sp = style === 'fluid' ? SPRINGS.soft : { easing: EASE.cine, duration: 560 };
    anim = root.animate([
      { transform: start, opacity: 1, filter: 'blur(0)' },
      { transform: 'translateY(-100%)', opacity: style === 'depth' ? 0 : 1, filter: style === 'depth' ? 'blur(16px)' : 'blur(0)' },
    ], { duration: sp.duration, easing: sp.easing, fill: 'forwards' });
  } else if (style === 'depth') {
    anim = root.animate([
      { opacity: 1, filter: 'blur(0)', transform: 'none' },
      { opacity: 0, filter: 'blur(22px)', transform: 'scale(1.16)' },
    ], { duration: 720, easing: EASE.expo, fill: 'forwards' });
  } else if (style === 'fluid') {
    anim = root.animate([
      { opacity: 1, transform: 'none' },
      { opacity: 0, transform: 'translateY(-6%) scale(.97)' },
    ], { duration: SPRINGS.soft.duration, easing: SPRINGS.soft.easing, fill: 'forwards' });
  } else {
    anim = root.animate([
      { clipPath: 'inset(0 0 0% 0)' },
      { clipPath: 'inset(0 0 100% 0)' },
    ], { duration: 760, easing: EASE.cine, fill: 'forwards' });
  }
  return anim.finished.catch(() => {}).then(() => {
    // The caller hides or removes the stage; drop the held frame after that.
    requestAnimationFrame(() => anim.cancel());
  });
}

/** A wrong password: the field says no in the language's own way. */
export function gateShake(node) {
  if (!node || still()) return;
  const style = motionStyle();
  if (style === 'depth') {
    node.animate([
      { transform: 'perspective(600px) rotateY(0)' },
      { transform: 'perspective(600px) rotateY(-14deg)' },
      { transform: 'perspective(600px) rotateY(10deg)' },
      { transform: 'perspective(600px) rotateY(-5deg)' },
      { transform: 'perspective(600px) rotateY(0)' },
    ], { duration: 560, easing: EASE.out });
  } else if (style === 'fluid') {
    node.animate([
      { transform: 'translateX(0)' }, { transform: 'translateX(-18px)' }, { transform: 'translateX(14px)' },
      { transform: 'translateX(-9px)' }, { transform: 'translateX(5px)' }, { transform: 'translateX(-2px)' },
      { transform: 'translateX(0)' },
    ], { duration: 640, easing: 'ease-out' });
  } else {
    node.animate([
      { transform: 'translateX(0)' }, { transform: 'translateX(-12px)' }, { transform: 'translateX(12px)' },
      { transform: 'translateX(-6px)' }, { transform: 'translateX(0)' },
    ], { duration: 360, easing: 'steps(8, end)' });
  }
}

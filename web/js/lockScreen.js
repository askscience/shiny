/**
 * lockScreen.js — the lock screen, on the desktop and on a phone.
 *
 * The same stage as the sign-in screen (gate.js, css/gate.css): the time, the
 * date and the weather, over the wallpaper, in the user's theme, accent and
 * motion language. The desktop behind it is hidden — `#app` is made inert and
 * invisible once the stage covers it, and the Browser's native page steps
 * aside (`body.lock-active`, see nativeShouldShow in the Browser plugin).
 *
 *   lock     after `idle` seconds without input — or never — (Settings →
 *            Account → Lock screen; see "Idle clock" for the input the DOM
 *            cannot see),
 *            before a suspend (powerMenu.js), from the power menu, on
 *            Super+L / Ctrl+Alt+L, or on a `lock:now` event;
 *   wake     a phone swipes up (the stage follows the finger); a desktop
 *            moves the mouse, clicks, scrolls or presses a key;
 *   unlock   straight away, or — when Settings asks for it — once the
 *            account password checks out (`POST /api/auth/verify-password`,
 *            the same check as the password vault: PAM or the stored hash,
 *            throttled server-side; no new session is opened).
 *
 * With the lock screen off in Settings nothing here runs.
 */
import { apiFetch, getTraveler } from './api.js';
import { getLockScreen } from './preferences.js';
import { renderAvatarEl } from './userProfiles.js';
import { t } from './i18n.js';
import { gateChrome, gateEnter, gateExit, gateReveal, gateShake } from './gate.js';

/** How often the idle clock is checked (ms). A suspend shows up as a jump. */
const CHECK_MS = 5000;
/** Back to the quiet clock when the password form is left alone this long. */
const AUTH_IDLE_MS = 30000;
/** Input right after locking is the gesture that locked, not a wake. */
const WAKE_GRACE_MS = 700;
/** A desktop wake needs a real move, not a jittery sensor. */
const WAKE_MOVE_PX = 6;

const ARROW_SVG = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12h14M13 6l6 6-6 6"/></svg>';
const CHEVRON_SVG = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 15l6-6 6 6"/></svg>';

let started = false;
let locked = false;
let lockedAt = 0;
let lastActivity = Date.now();
let checkTimer = 0;
let authTimer = 0;
let busy = false;

let root = null;
let refs = null;
let chrome = null;
/** The active swipe (phone), or the first pointer position (desktop wake). */
let drag = null;
let wakeOrigin = null;

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

/** A phone, or any touch-first screen: wake by swiping up. */
function touchFirst() {
  return document.documentElement.dataset.phone === '1'
    || (typeof matchMedia === 'function' && matchMedia('(pointer: coarse)').matches);
}

function signedIn() {
  const login = document.getElementById('login-overlay');
  return !!getTraveler() && (!login || login.classList.contains('hidden'));
}

/* ── Stage ──────────────────────────────────────────────────── */

function build() {
  root = el('div', 'gate gate--lock');
  root.id = 'lock-screen';
  root.tabIndex = -1;
  root.setAttribute('role', 'dialog');
  root.setAttribute('aria-modal', 'true');

  const aura = el('div', 'gate-aura');
  aura.setAttribute('aria-hidden', 'true');
  aura.append(el('span'), el('span'));

  const aside = el('aside', 'gate-aside');
  aside.setAttribute('data-gate-aside', '');

  const panel = el('main', 'gate-panel lock-panel');
  const step = el('div', 'gate-step');
  const kicker = el('p', 'gate-kicker');
  kicker.setAttribute('data-gate-in', '');
  const kickerText = el('span');
  kicker.append(el('span', 'gate-kicker-num', '01'), kickerText);

  const who = el('div', 'profile-selected');
  who.setAttribute('data-gate-in', '');
  const avatar = el('div', 'profile-avatar');
  const name = el('span', 'profile-selected-name');
  who.append(avatar, name);

  const form = el('form', 'gate-field');
  form.setAttribute('data-gate-in', '');
  const input = el('input', 'ui-input gate-input');
  input.type = 'password';
  input.autocomplete = 'current-password';
  const go = el('button', 'gate-go');
  go.type = 'submit';
  go.innerHTML = ARROW_SVG;
  form.append(input, go);

  const error = el('p', 'app-error gate-error hidden');
  error.setAttribute('role', 'alert');
  step.append(kicker, who, form, error);
  panel.appendChild(step);

  const hint = el('div', 'gate-hint');
  const hintIcon = el('span', 'gate-hint-icon');
  hintIcon.innerHTML = CHEVRON_SVG;
  const hintText = el('span', 'gate-hint-text');
  hint.append(hintIcon, hintText);

  root.append(aura, aside, panel, hint);

  form.addEventListener('submit', (e) => {
    e.preventDefault();
    void submit();
  });
  input.addEventListener('input', () => {
    error.classList.add('hidden');
    armAuthIdle();
  });
  root.addEventListener('pointerdown', onPointerDown);
  root.addEventListener('pointermove', onPointerMove);
  root.addEventListener('pointerup', onPointerUp);
  root.addEventListener('pointercancel', onPointerUp);
  root.addEventListener('wheel', () => wake(), { passive: true });
  // Keys typed here never reach the desktop's own shortcuts behind it.
  root.addEventListener('keydown', (e) => {
    e.stopPropagation();
    if (e.key === 'Escape' && root.dataset.state === 'auth' && !busy) {
      refs.input.value = '';
      refs.error.classList.add('hidden');
      setState('idle');
      root.focus({ preventScroll: true });
      void gateReveal(refs.aside);
    }
  });

  refs = { aside, panel, step, kickerText, avatar, name, input, go, error, hint, hintText, form };
  label();
}

function label() {
  if (!refs) return;
  refs.kickerText.textContent = t('lock.locked', null, 'Locked');
  refs.input.placeholder = t('login.password', null, 'Password');
  refs.input.setAttribute('aria-label', t('login.password', null, 'Password'));
  refs.go.setAttribute('aria-label', t('lock.unlock', null, 'Unlock'));
  refs.hintText.textContent = touchFirst()
    ? t('lock.swipeHint', null, 'Swipe up to unlock')
    : t('lock.wakeHint', null, 'Move the mouse or press a key');
  root?.setAttribute('aria-label', t('lock.title', null, 'Lock screen'));
}

function setState(state) {
  root.dataset.state = state;
  refs.panel.hidden = state !== 'auth';
  refs.hint.hidden = state !== 'idle';
}

/* ── Lock / unlock ──────────────────────────────────────────── */

/** Lock now (if the lock screen is on and someone is signed in). */
export function lockNow() {
  if (locked || !started || !getLockScreen().enabled || !signedIn()) return;
  locked = true;
  lockedAt = Date.now();
  if (!root) build();
  label();

  const traveler = getTraveler();
  renderAvatarEl(refs.avatar, { name: traveler?.name || traveler?.username || '', avatar: traveler?.avatar || null });
  refs.name.textContent = traveler?.name || traveler?.username || '';
  refs.input.value = '';
  refs.error.classList.add('hidden');
  setState('idle');

  // Nothing the desktop had in focus keeps taking keys.
  if (document.activeElement && document.activeElement !== document.body) document.activeElement.blur?.();
  document.body.classList.add('lock-active');
  document.body.appendChild(root);
  chrome = gateChrome(refs.aside);
  root.focus({ preventScroll: true });
  window.dispatchEvent(new CustomEvent('lock:change', { detail: { locked: true } }));
  // The kiosk shell waits for a locked screen to relaunch for its own update.
  window.ipc?.postMessage?.('liteaura:locked');

  const app = document.getElementById('app');
  void gateEnter(root).then(() => {
    if (!locked || !app) return;
    // Covered: now the desktop goes away for real (no peeking through a
    // translucent edge, no focus, no assistive tech reading it out).
    app.inert = true;
    app.style.visibility = 'hidden';
  });
}

function unlock({ fromY = 0, animate = true } = {}) {
  if (!locked) return;
  clearTimeout(authTimer);
  const app = document.getElementById('app');
  if (app) {
    app.inert = false;
    app.style.visibility = '';
  }
  const finish = () => {
    chrome?.destroy();
    chrome = null;
    root?.remove();
    if (root) root.style.transform = '';
    document.body.classList.remove('lock-active');
    locked = false;
    busy = false;
    lastActivity = Date.now();
    window.dispatchEvent(new CustomEvent('lock:change', { detail: { locked: false } }));
    window.ipc?.postMessage?.('liteaura:unlocked');
  };
  if (animate) void gateExit(root, { fromY }).then(finish);
  else finish();
}

/** The quiet clock woke up: unlock, or ask for the password. */
function wake({ fromY = 0 } = {}) {
  if (!locked || busy || Date.now() - lockedAt < WAKE_GRACE_MS) {
    snapBack();
    return;
  }
  if (root.dataset.state === 'auth') return;
  if (!getLockScreen().password) {
    unlock({ fromY });
    return;
  }
  snapBack();
  setState('auth');
  void gateReveal(refs.panel);
  refs.input.focus({ preventScroll: true });
  armAuthIdle();
}

function armAuthIdle() {
  clearTimeout(authTimer);
  authTimer = setTimeout(() => {
    if (!locked || busy) return;
    refs.input.value = '';
    refs.error.classList.add('hidden');
    refs.input.blur();
    setState('idle');
    void gateReveal(refs.aside);
  }, AUTH_IDLE_MS);
}

async function submit() {
  if (!locked || busy) return;
  const password = refs.input.value;
  if (!password) {
    refs.input.focus();
    return;
  }
  busy = true;
  refs.go.disabled = true;
  try {
    const res = await apiFetch('/api/auth/verify-password', {
      method: 'POST',
      body: JSON.stringify({ password }),
    });
    if (res?.data?.valid) {
      refs.input.value = '';
      busy = false;
      unlock();
      return;
    }
    fail(t('lock.wrongPassword', null, 'That password is not right'));
  } catch (e) {
    fail(e?.message || t('lock.checkFailed', null, 'Could not check the password'));
  } finally {
    busy = false;
    if (refs) refs.go.disabled = false;
  }
}

function fail(message) {
  refs.error.textContent = message;
  refs.error.classList.remove('hidden');
  gateShake(refs.form);
  refs.input.select();
  armAuthIdle();
}

/* ── Gestures ───────────────────────────────────────────────── */

function snapBack() {
  if (!root || !root.style.transform) return;
  const from = root.style.transform;
  root.style.transform = '';
  root.animate?.([{ transform: from }, { transform: 'none' }], { duration: 320, easing: 'cubic-bezier(0.2, 0.8, 0.2, 1)' });
}

function onPointerDown(e) {
  if (root.dataset.state !== 'idle') return;
  if (e.pointerType === 'mouse') {
    wake();
    return;
  }
  drag = { id: e.pointerId, y: e.clientY, t: performance.now(), dy: 0 };
  root.setPointerCapture?.(e.pointerId);
}

function onPointerMove(e) {
  if (!drag || e.pointerId !== drag.id) return;
  // Only upwards; a little give the other way.
  drag.dy = Math.min(24, e.clientY - drag.y);
  root.style.transform = `translateY(${drag.dy}px)`;
}

function onPointerUp(e) {
  if (!drag || e.pointerId !== drag.id) return;
  const { dy, t: t0 } = drag;
  drag = null;
  const speed = -dy / Math.max(1, performance.now() - t0);
  const enough = -dy > Math.min(140, window.innerHeight * 0.18) || (speed > 0.6 && -dy > 30);
  if (enough) wake({ fromY: dy });
  else snapBack();
}

/** Desktop: any real input on the quiet clock wakes it. */
function onGlobalInput(e) {
  if (!locked) {
    lastActivity = Date.now();
    return;
  }
  if (root?.dataset.state !== 'idle') {
    if (root?.dataset.state === 'auth') armAuthIdle();
    return;
  }
  if (e.type === 'keydown') {
    if (['Shift', 'Control', 'Alt', 'Meta'].includes(e.key) && e.repeat) return;
    // The key only wakes the clock: it is not typed, and nothing else sees it.
    e.preventDefault();
    e.stopPropagation();
    wake();
    return;
  }
  if (e.type === 'pointermove' && e.pointerType === 'mouse') {
    if (!wakeOrigin) { wakeOrigin = { x: e.clientX, y: e.clientY }; return; }
    if (Math.hypot(e.clientX - wakeOrigin.x, e.clientY - wakeOrigin.y) < WAKE_MOVE_PX) return;
    wakeOrigin = null;
    wake();
  }
}

function onShortcut(e) {
  if (locked) return;
  const key = (e.key || '').toLowerCase();
  if (key !== 'l') return;
  if ((e.metaKey && !e.ctrlKey && !e.altKey) || (e.ctrlKey && e.altKey && !e.metaKey)) {
    if (!getLockScreen().enabled) return;
    e.preventDefault();
    lockNow();
  }
}

/* ── Idle clock ─────────────────────────────────────────────── *
 * Input the app's own DOM never sees still counts where it can be seen:
 *  - a same-origin frame (a web app, a plugin's page) is hooked when the
 *    pointer enters it or it takes the focus;
 *  - the Browser's native page reports navigations (`user:activity`);
 *  - sound or video playing in the app is someone watching or listening.
 * Where nothing can be seen — the focus is in the native page or a
 * cross-origin frame — the desk waits three times as long before it locks,
 * rather than lock someone out while they are reading or typing there.
 */

const hookedFrames = new WeakSet();
const ACTIVITY_TYPES = ['pointermove', 'pointerdown', 'keydown', 'wheel', 'touchstart'];

function markActive() {
  if (!locked) lastActivity = Date.now();
}

function hookFrame(frame) {
  try {
    const win = frame?.contentWindow;
    if (!win || hookedFrames.has(win)) return;
    // Cross-origin frames throw here; there is nothing to hook in them.
    void win.document;
    for (const type of ACTIVITY_TYPES) win.addEventListener(type, markActive, { capture: true, passive: true });
    hookedFrames.add(win);
  } catch (_) { /* cross-origin */ }
}

function onFrameReach(e) {
  if (e.target?.tagName === 'IFRAME') hookFrame(e.target);
}

function onWindowBlur() {
  // The focus went into a frame: it is the active element once the blur is done.
  setTimeout(() => {
    const active = document.activeElement;
    if (active?.tagName === 'IFRAME') hookFrame(active);
  }, 0);
}

function mediaPlaying() {
  for (const m of document.querySelectorAll('video, audio')) {
    if (!m.paused && !m.ended && m.readyState > 2) return true;
  }
  return false;
}

function checkIdle() {
  if (locked) return;
  const s = getLockScreen();
  if (!s.enabled) return;
  if (mediaPlaying()) {
    lastActivity = Date.now();
    return;
  }
  const unseen = typeof document.hasFocus === 'function' && !document.hasFocus();
  if (!s.idle) return;
  const limit = s.idle * 1000 * (unseen ? 3 : 1);
  if (Date.now() - lastActivity >= limit) lockNow();
}

/** Start watching (once, after sign-in). */
export function initLockScreen() {
  if (started) return;
  started = true;
  lastActivity = Date.now();
  for (const type of ACTIVITY_TYPES) {
    window.addEventListener(type, onGlobalInput, { capture: true, passive: type !== 'keydown' });
  }
  document.addEventListener('pointerover', onFrameReach, true);
  window.addEventListener('blur', onWindowBlur);
  window.addEventListener('user:activity', markActive);
  window.addEventListener('keydown', onShortcut, true);
  window.addEventListener('lock:now', () => lockNow());
  window.addEventListener('locale:change', label);
  window.addEventListener('lock:settings', (e) => {
    // Switched off while locked: let the user in.
    if (locked && e.detail && !e.detail.enabled) unlock();
    lastActivity = Date.now();
  });
  // Signed out underneath (session expired): the sign-in screen takes over.
  window.addEventListener('auth:expired', () => unlock({ animate: false }));
  document.addEventListener('visibilitychange', () => {
    if (!document.hidden) checkIdle();
  });
  checkTimer = setInterval(checkIdle, CHECK_MS);
}

export function isLocked() {
  return locked;
}

/** For tests: stop the clock. */
export function stopLockScreen() {
  clearInterval(checkTimer);
  started = false;
}

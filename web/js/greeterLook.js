/**
 * greeterLook.js — publish the look this account picked (theme, accent,
 * gradient, motion) for the machine's login screen.
 *
 * The greeter is another account with its own server and web storage, so it
 * cannot see these preferences; the session server writes them where it can
 * (`PUT /api/greeter-look`, see `src/services/greeter_look.rs`). The values
 * are read from the account's own stored choices, never from what is on
 * screen: the sign-in stage paints other profiles' looks too.
 *
 * Published once the session has loaded its preferences, then on every
 * change of theme, accent/gradient or motion; an unchanged look is not sent
 * twice. Off the kiosk the server answers `published: false` and that is all.
 */
import { apiFetch, getTraveler } from './api.js';
import { getActiveTheme } from '../ui/index.js';

const DEBOUNCE_MS = 800;
let timer = null;
let lastSent = '';
let wired = false;

function readJson(key) {
  try { return JSON.parse(localStorage.getItem(key) || 'null'); } catch (_) { return null; }
}

/** The signed-in account's stored look, or null when no one is signed in. */
export function currentLook() {
  const id = getTraveler()?.id;
  if (!id) return null;
  const surface = readJson(`desktop.surface.${id}`);
  const gradient = readJson(`ui.gradient.${id}`);
  return {
    theme: localStorage.getItem('ui.theme.name') || getActiveTheme() || null,
    accent: localStorage.getItem(`ui.accent.${id}`) || null,
    gradient: gradient?.stops?.length >= 2
      ? { id: gradient.id || undefined, angle: Number(gradient.angle) || 0, stops: gradient.stops }
      : null,
    motion: surface?.motion || null,
  };
}

async function send() {
  timer = null;
  const look = currentLook();
  if (!look) return;
  const body = JSON.stringify(look);
  if (body === lastSent) return;
  try {
    await apiFetch('/api/greeter-look', { method: 'PUT', authRedirect: false, body });
    lastSent = body;
  } catch (_) {
    /* the next change tries again */
  }
}

/** Publish soon (changes come in bursts: a theme switch repaints the accent). */
export function publishGreeterLook() {
  if (timer) clearTimeout(timer);
  timer = setTimeout(send, DEBOUNCE_MS);
}

/** Publish now and on every later change of the look. */
export function initGreeterLook() {
  publishGreeterLook();
  if (wired) return;
  wired = true;
  for (const event of ['theme:change', 'appearance:change', 'desktop:surface']) {
    window.addEventListener(event, publishGreeterLook);
  }
}

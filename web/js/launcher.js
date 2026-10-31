import { controlButton } from '../ui/components/button.js';
/**
 * launcher.js — the plugin launcher (three-finger swipe down).
 *
 * A full-screen grid of every installed plugin, each drawn with its own icon,
 * over the blurred desktop — Launchpad / GNOME app grid, not a dialog. Tapping
 * an inactive plugin activates it; tapping an active one focuses (or opens) its
 * window. It is the top-bar plugin tray at full size — the same `/api/plugins`
 * list and the same activate/focus paths — so the gesture is a second door onto
 * the same room.
 *
 * The gesture is recognised natively (crates/peakd/src/gestures.rs) and mapped
 * here by gestures.js; Escape, an upward swipe and a click on the blurred
 * desktop all dismiss.
 *
 * Like the overview it is one progress value (0 = hidden, 1 = open), so a
 * swipe pulls the panel in with the fingers and a release completes or
 * springs back (`launcherTransition`). Themes read `--launcher-p` and
 * `html[data-launcher="opening|open|closing"]` on the document element.
 */
import { apiFetch } from './api.js';
import { createTransition, clamp01 } from './gestureTransition.js';
import { refreshActivePlugins, refreshPluginCatalog } from './activePlugins.js';
import { pluginIconEl } from './pluginIcon.js';
import { t } from './i18n.js';

let panel = null;
let grid = null;
let open = false;
/** On screen (opening, open or closing). */
let staged = false;
let running = null;
let active = new Set();
let plugins = [];

function label(name) {
  return name.charAt(0).toUpperCase() + name.slice(1);
}

async function refresh(force = false) {
  try {
    const [list, activeNames] = await Promise.all([
      refreshPluginCatalog({ force }),
      refreshActivePlugins({ force }),
    ]);
    plugins = list || [];
    active = new Set(activeNames || []);
  } catch (_) {
    // Keep the last known list on a transient error.
  }
  render();
}

async function choose(p) {
  if (active.has(p.name)) {
    window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: p.name } }));
    closeLauncher();
    return;
  }
  try {
    await apiFetch('/api/plugins/activate', {
      method: 'POST',
      body: JSON.stringify({ name: p.name }),
    });
  } catch (err) {
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: {
        message: err.message || t('launcher.activateFailed', { name: label(p.name) },
          `Could not activate ${label(p.name)}`),
        type: 'error',
      },
    }));
    return;
  }
  // The same signal the tray and the Plugins page send, so the desktop mounts
  // the new window; `opened` makes it come to the front.
  localStorage.setItem('plugins.changed', String(Date.now()));
  window.dispatchEvent(new CustomEvent('plugins:changed', { detail: { opened: p.name } }));
  closeLauncher();
}

function render() {
  if (!grid) return;
  grid.textContent = '';
  if (!plugins.length) {
    const empty = document.createElement('p');
    empty.className = 'launcher-empty';
    empty.textContent = t('launcher.noPlugins', null, 'No plugins installed');
    grid.appendChild(empty);
    return;
  }
  for (const p of plugins) {
    const isActive = active.has(p.name);
    const displayName = label(p.name);
    const btn = controlButton();
    btn.type = 'button';
    btn.className = 'launcher-item';
    btn.classList.toggle('is-active', isActive);
    btn.title = p.description || displayName;
    btn.setAttribute('aria-label', isActive
      ? t('launcher.openItem', { name: displayName }, `${displayName} (open)`)
      : displayName);

    const iconWrap = document.createElement('span');
    iconWrap.className = 'launcher-item-icon';
    iconWrap.appendChild(pluginIconEl(p.name, { size: 40, fallback: 'ui/puzzle' }));

    const name = document.createElement('span');
    name.className = 'launcher-item-name';
    name.textContent = label(p.name);

    // A running app is marked by the dot under its name, the way an app grid
    // does — no "Open/Activate" words.
    const dot = document.createElement('span');
    dot.className = 'launcher-item-dot';
    dot.setAttribute('aria-hidden', 'true');

    btn.append(iconWrap, name, dot);
    btn.addEventListener('click', () => void choose(p));
    grid.appendChild(btn);
  }
}

export function isLauncherOpen() {
  return open;
}

/** True from `beginLauncher()` until the launcher is fully hidden again. */
export function isLauncherActive() {
  return staged;
}

function setPhase(phase) {
  const root = document.documentElement;
  if (phase) root.dataset.launcher = phase;
  else delete root.dataset.launcher;
}

/** Put the launcher on screen at progress 0 (or mark an open one closing). */
export function beginLauncher() {
  if (!panel) return false;
  if (staged) {
    setPhase(open ? 'closing' : 'opening');
    return true;
  }
  staged = true;
  window.dispatchEvent(new CustomEvent('overlay:open', { detail: { which: 'launcher' } }));
  setLauncherProgress(0);
  setPhase('opening');
  panel.classList.remove('hidden');
  // Set before the panel paints, so native pages hide as the panel appears.
  document.body.classList.add('launcher-active');
  void refresh();
  return true;
}

/** Render the launcher at `p` (0 = hidden, 1 = open). */
export function setLauncherProgress(p) {
  document.documentElement.style.setProperty('--launcher-p', clamp01(p).toFixed(4));
  // Beyond the ends (a rubber band) only the panel's travel stretches.
  const over = Number(p) > 1 ? Number(p) - 1 : Number(p) < 0 ? Number(p) : 0;
  document.documentElement.style.setProperty('--launcher-over', over.toFixed(4));
}

/** Settle: `true` leaves the launcher open, `false` hides it. */
export function endLauncher(openAtEnd) {
  if (!staged) return;
  const root = document.documentElement;
  if (openAtEnd) {
    open = true;
    setLauncherProgress(1);
    setPhase('open');
    return;
  }
  open = false;
  staged = false;
  panel?.classList.add('hidden');
  document.body.classList.remove('launcher-active');
  setPhase(null);
  root.style.removeProperty('--launcher-p');
  root.style.removeProperty('--launcher-over');
}

/**
 * An interactive open (`wantOpen`) or close of the launcher, for a gesture to
 * drive or a discrete caller to `play()`. Null when there is nothing to do.
 */
export function launcherTransition(wantOpen) {
  running?.finishNow(true);
  if (wantOpen ? open : !staged) return null;
  if (!beginLauncher()) return null;
  setPhase(wantOpen ? 'opening' : 'closing');
  const transition = createTransition({
    apply: (k) => setLauncherProgress(wantOpen ? k : 1 - k),
    finish: (committed) => {
      if (running === transition) running = null;
      endLauncher(committed ? wantOpen : !wantOpen);
    },
  });
  running = transition;
  return transition;
}

export function openLauncher() {
  launcherTransition(true)?.play();
}

export function closeLauncher() {
  launcherTransition(false)?.play();
}

export function initLauncher() {
  if (panel) return;
  panel = document.getElementById('plugin-launcher');
  grid = document.getElementById('plugin-launcher-grid');
  if (!panel || !grid) return;

  // Like Launchpad, a click on the blurred desktop (outside every icon) leaves.
  panel.addEventListener('click', (e) => {
    if (e.target === panel) closeLauncher();
  });

  window.addEventListener('keydown', (e) => {
    if (!staged || e.key !== 'Escape') return;
    e.preventDefault();
    e.stopImmediatePropagation();
    closeLauncher();
  }, true);

  window.addEventListener('overlay:open', (e) => {
    if (staged && e.detail?.which !== 'launcher') closeLauncher();
  });
  window.addEventListener('plugins:changed', () => {
    if (staged) void refresh(true);
  });
}

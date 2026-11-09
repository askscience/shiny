/**
 * capture/hud.js — the recording chip in the top bar.
 *
 * Loaded whenever Capture is installed, so a recording or a live stream is
 * always visible and stoppable, even with the Capture window closed: a red
 * dot and the time while recording, LIVE and the time while streaming.
 * Clicking it opens a small menu (pause, marker, stop, open Capture).
 *
 * It also turns the assistant's requests into a confirmation: the agent can
 * only *ask* to record or go live (`capture_record_start`,
 * `capture_live_start`); nothing starts until the user says yes here.
 */
import { controlButton, menuSurface, menuItem, confirmDialog, formatTimecode, setQuiet } from '../../ui/index.js';
import { apiFetch } from '../../js/api.js';
import { activatePlugin, openPluginWindow } from '../../js/tiles.js';
import { t } from '../../js/i18n.js';
import { getFocus } from '../../js/desktop/workspaces.js';

const PLUGIN = 'capture';

let btn = null;
let dot = null;
let label = null;
let timer = 0;
let status = { active: false };
let menu = null;
let wasActive = false;

async function refresh() {
  try {
    const r = await apiFetch('/api/capture/status', { authRedirect: false });
    status = r?.data || { active: false };
  } catch (_) { /* server away */ }
  paint();
  timer = setTimeout(refresh, status.active ? 1000 : 4000);
}

function paint() {
  if (!btn) return;
  const active = !!status.active;
  btn.hidden = !active;
  btn.classList.toggle('is-live', !!status.live);
  btn.classList.toggle('is-paused', !!status.paused);
  const time = formatTimecode((status.elapsed_us || 0) / 1e6);
  label.textContent = status.live ? `${t('plugin.capture.hud.live', null, 'LIVE')} ${time}` : time;
  btn.title = status.live
    ? t('plugin.capture.hud.liveTitle', { names: (status.live_labels || []).join(', ') }, `Live on ${(status.live_labels || []).join(', ')}`)
    : t('plugin.capture.hud.recTitle', null, 'Recording');
  // Do not disturb for as long as anything records the screen.
  if (active !== wasActive) {
    setQuiet(active, `${PLUGIN}-hud`);
    wasActive = active;
  }
}

/* The recording's event log: which window was in front, and when. The
   editor's AI reads it to know what happens on screen without seeing it. */
let lastFocus = null;
function onDesktopChanged() {
  if (!status.active) { lastFocus = null; return; }
  const name = getFocus();
  if (!name || name === lastFocus) return;
  lastFocus = name;
  const tile = document.querySelector(`section.tile[data-plugin="${CSS.escape(name)}"]`);
  const title = tile?.querySelector('.tile-header-title')?.textContent?.trim() || name;
  apiFetch('/api/capture/event', { method: 'POST', body: JSON.stringify({ kind: 'window', detail: title }) }).catch(() => {});
}

function closeMenu() {
  menu?.remove();
  menu = null;
}

function openMenu() {
  if (menu) { closeMenu(); return; }
  menu = menuSurface({ className: 'capture-hud-menu', onEscape: closeMenu });
  const add = (label, fn, disabled = false) => menu.appendChild(menuItem({ label, disabled, onClick: () => { closeMenu(); fn(); } }));
  if (!status.live) {
    add(status.paused ? t('plugin.capture.action.resume', null, 'Resume') : t('plugin.capture.action.pause', null, 'Pause'),
      () => apiFetch('/api/capture/pause', { method: 'POST', body: '{}' }).then(refreshNow));
  }
  add(t('plugin.capture.action.marker', null, 'Add marker'), () => apiFetch('/api/capture/marker', { method: 'POST', body: '{}' }));
  add(t('plugin.capture.action.stop', null, 'Stop'), () => apiFetch('/api/capture/stop', { method: 'POST', body: '{}' }).then(refreshNow));
  add(t('plugin.capture.hud.open', null, 'Open Capture'), () => void openPluginWindow(PLUGIN, { focus: true }));
  document.body.appendChild(menu);
  const r = btn.getBoundingClientRect();
  menu.style.position = 'fixed';
  menu.style.top = `${r.bottom + 6}px`;
  menu.style.left = `${Math.max(8, r.right - 200)}px`;
  menu.focusFirst?.();
  setTimeout(() => document.addEventListener('pointerdown', function off(e) {
    if (!menu || menu.contains(e.target) || btn.contains(e.target)) return;
    closeMenu();
    document.removeEventListener('pointerdown', off, true);
  }, true));
}

function refreshNow() {
  clearTimeout(timer);
  refresh();
}

/** Open the Capture window and hand it an event once it is mounted. */
async function toWindow(type, detail) {
  if (!await openPluginWindow(PLUGIN, { focus: true })) {
    await activatePlugin(PLUGIN);
    await openPluginWindow(PLUGIN, { focus: true });
  }
  const t0 = Date.now();
  const tryIt = () => {
    if (document.querySelector('section.capture-tile .capture-body > :not(.ui-spinner)')) {
      window.dispatchEvent(new CustomEvent(type, { detail }));
    } else if (Date.now() - t0 < 8000) {
      setTimeout(tryIt, 150);
    }
  };
  tryIt();
}

async function onAgentActions(e) {
  for (const a of e.detail || []) {
    if (a?.result !== 'ok') continue;
    const data = a.data || {};
    if ((a.action === 'capture_record_start' || a.action === 'capture_live_start') && data.pending) {
      const req = data.request || {};
      const live = (req.live || []).map((d) => d.name).join(', ');
      const ok = await confirmDialog(live
        ? t('plugin.capture.confirm.live', { names: live }, `The assistant wants to go live on ${live}. Start?`)
        : t('plugin.capture.confirm.record', null, 'The assistant wants to record the screen. Start?'));
      if (ok) toWindow('capture:start-request', req);
    } else if (a.action === 'capture_project_create' && data.project && data.open) {
      toWindow('capture:open-project', { id: data.project });
    }
  }
}

export function install() {
  if (btn?.isConnected) return;
  const top = document.getElementById('hud-top');
  if (!top) return;
  btn = controlButton();
  btn.id = 'hud-capture';
  btn.className = 'hud-capture';
  btn.hidden = true;
  btn.setAttribute('aria-haspopup', 'menu');
  dot = document.createElement('span');
  dot.className = 'hud-capture-dot';
  dot.setAttribute('aria-hidden', 'true');
  label = document.createElement('span');
  label.className = 'hud-capture-time';
  btn.append(dot, label);
  btn.addEventListener('click', openMenu);
  const battery = document.getElementById('hud-battery');
  if (battery && battery.parentElement === top) top.insertBefore(btn, battery);
  else top.prepend(btn);
  window.addEventListener('agent:actions', onAgentActions);
  window.addEventListener('capture:changed', refreshNow);
  window.addEventListener('desktop:changed', onDesktopChanged);
  refresh();
}

export function uninstall() {
  clearTimeout(timer);
  closeMenu();
  window.removeEventListener('agent:actions', onAgentActions);
  window.removeEventListener('capture:changed', refreshNow);
  window.removeEventListener('desktop:changed', onDesktopChanged);
  if (wasActive) setQuiet(false, `${PLUGIN}-hud`);
  wasActive = false;
  btn?.remove();
  btn = null;
}

export default { install, uninstall };

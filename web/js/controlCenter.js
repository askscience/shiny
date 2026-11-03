/**
 * controlCenter.js — the phone's control centre.
 *
 * On a phone the top bar is a status bar: time, weather and the running apps.
 * The host's status chips (sound, network, Bluetooth, battery) and the bar's
 * actions (clipboard, theme, settings, plugins, power) do not fit a finger's
 * width there, so they move into a sheet that drops from the bar's control
 * button — like a mobile OS's control centre.
 *
 * The chips are not rebuilt: `#hud-top` itself moves into the sheet and back,
 * so every chip module keeps its element, its listeners and its menu (menus
 * are appended to <body> and placed under their chip wherever it is). The
 * bar's pill clips and contains its children, which is why the node moves
 * instead of being repositioned with CSS.
 */

import { MOBILE_QUERY } from './viewport.js';
import { t } from './i18n.js';
import { icon } from '../ui/index.js';

let sheet = null;
let button = null;
let hudTop = null;
/** Where `#hud-top` lives on the desktop, to put it back exactly there. */
let home = null;
let homeNext = null;

function isOpen() {
  return document.documentElement.dataset.controlCenter === 'open';
}

function setOpen(open) {
  if (!sheet) return;
  if (open === isOpen()) return;
  if (open) document.documentElement.dataset.controlCenter = 'open';
  else delete document.documentElement.dataset.controlCenter;
  sheet.setAttribute('aria-hidden', open ? 'false' : 'true');
  sheet.inert = !open;
  button?.setAttribute('aria-expanded', open ? 'true' : 'false');
  button?.classList.toggle('is-open', open);
}

export function closeControlCenter() { setOpen(false); }

/** A tap outside closes it — except inside one of its chips' menus, which
 *  are popups of their own on <body>. */
function onOutside(e) {
  if (!isOpen()) return;
  const target = e.target;
  if (!(target instanceof Element)) return;
  if (sheet.contains(target) || button?.contains(target)) return;
  if (target.closest('.ui-hud-menu-popup, .ui-popover, [role="dialog"], [role="menu"]')) return;
  setOpen(false);
}

/** Move the chips to where the screen wants them. */
function place() {
  if (!hudTop || !sheet) return;
  if (MOBILE_QUERY.matches) {
    if (hudTop.parentElement !== sheet) sheet.appendChild(hudTop);
  } else {
    setOpen(false);
    if (home && hudTop.parentElement !== home) {
      home.insertBefore(hudTop, homeNext && homeNext.parentElement === home ? homeNext : null);
    }
  }
}

export function initControlCenter() {
  if (sheet) return;
  hudTop = document.getElementById('hud-top');
  const main = document.querySelector('#hud-header .hud-header-main');
  if (!hudTop || !main) return;
  home = hudTop.parentElement;
  homeNext = hudTop.nextElementSibling;

  button = document.createElement('button');
  button.type = 'button';
  button.id = 'hud-control';
  button.className = 'icon-btn hud-control-btn';
  button.setAttribute('aria-haspopup', 'dialog');
  button.setAttribute('aria-expanded', 'false');
  button.setAttribute('aria-controls', 'control-center');
  const label = t('hud.controlCenter', null, 'Control centre');
  button.title = label;
  button.setAttribute('aria-label', label);
  button.appendChild(icon('ui/properties', { size: 20 }));
  main.appendChild(button);

  sheet = document.createElement('div');
  sheet.id = 'control-center';
  sheet.className = 'control-center';
  sheet.setAttribute('role', 'dialog');
  sheet.setAttribute('aria-label', label);
  sheet.setAttribute('aria-hidden', 'true');
  sheet.inert = true;
  document.body.appendChild(sheet);

  button.addEventListener('click', () => setOpen(!isOpen()));
  // Settings and Plugins open a window: the sheet steps out of its way. The
  // other actions (theme, menus) keep it open so they can be used in a row.
  sheet.addEventListener('click', (e) => {
    if (e.target instanceof Element && e.target.closest('#settings-btn, #plugins-btn')) setOpen(false);
  });
  document.addEventListener('pointerdown', onOutside, true);
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && isOpen()) setOpen(false);
  });
  window.addEventListener('locale:change', () => {
    const next = t('hud.controlCenter', null, 'Control centre');
    button.title = next;
    button.setAttribute('aria-label', next);
    sheet.setAttribute('aria-label', next);
  });
  MOBILE_QUERY.addEventListener('change', place);
  place();
}

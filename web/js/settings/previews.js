/**
 * previews.js — small illustrative previews for Settings → Appearance.
 *
 * Each preview is a fake, schematic miniature (never the real windows): a
 * few shapes drawn with the theme's own tokens, so it is easy to read at a
 * glance and always matches the theme and accent in use. None of them holds
 * state: they react to what the preferences already publish on <html>
 *
 *   style    — theme tokens, the accent and `data-motion` (a card that keeps
 *              arriving in the chosen motion language);
 *   window   — `--window-radius / -opacity / -shadow`, `--glass-blur`,
 *              `--tile-header-height`, `data-window-background`;
 *   bar      — `data-bar-pos`, `data-bar-style`, `data-orb-pos`,
 *              `data-orb-attached`;
 *   launcher — `data-launcher-style`, `data-launcher-recents`,
 *              `data-launcher-windows` (a looping "swipe down" demo).
 *
 * so changing a control repaints its preview through CSS alone
 * (css/settings-previews.css).
 */
import { el } from './controls.js';

function shapes(parent, className, count) {
  for (let i = 0; i < count; i += 1) parent.appendChild(el('i', className));
  return parent;
}

function frame(kind, screen) {
  const wrap = el('div', `sp sp--${kind}`);
  wrap.setAttribute('aria-hidden', 'true');
  wrap.appendChild(screen);
  return wrap;
}

/** A mini window: a head with its dots, then a few lines of content. */
function miniWindow(className = '') {
  const win = el('div', `sp-win ${className}`.trim());
  const head = el('div', 'sp-win-head');
  head.append(el('span', 'sp-win-title'), el('span', 'sp-sp'), el('i'), el('i'));
  const body = el('div', 'sp-win-body');
  body.append(el('b', 'sp-hero'), el('i', 'sp-line sp-line--70'), el('i', 'sp-line sp-line--50'));
  win.append(head, body);
  return win;
}

/** Theme, accent, type and motion on one tiny desktop. */
export function stylePreview() {
  const screen = el('div', 'sp-screen');
  screen.append(el('div', 'sp-wall'));
  const bar = el('div', 'sp-bar');
  shapes(bar, 'sp-dot', 4);
  const win = miniWindow('sp-win--style');
  const body = win.querySelector('.sp-win-body');
  body.prepend(el('span', 'sp-type', 'Aa'));
  const row = el('div', 'sp-row');
  row.append(el('span', 'sp-pill sp-pill--accent'), el('span', 'sp-pill'), el('span', 'sp-switch'));
  body.append(row);
  // The motion demo: a notification that keeps arriving the chosen way.
  const card = el('div', 'sp-toast');
  card.append(el('span', 'sp-toast-dot'), shapes(el('span', 'sp-toast-lines'), 'sp-line', 2));
  screen.append(bar, win, card);
  return frame('style', screen);
}

/** One window over a busy wallpaper, so radius, glass and shadow read. */
export function windowPreview() {
  const screen = el('div', 'sp-screen sp-screen--busy');
  screen.append(el('div', 'sp-blob sp-blob--a'), el('div', 'sp-blob sp-blob--b'), el('div', 'sp-blob sp-blob--c'));
  const win = miniWindow('sp-win--surface');
  win.prepend(el('div', 'sp-glow'));
  screen.append(win);
  return frame('window', screen);
}

/** The top bar on its edge, in its style, with the voice bar. */
export function barPreview() {
  const screen = el('div', 'sp-screen');
  screen.append(el('div', 'sp-wall'));
  const desk = el('div', 'sp-desk');
  desk.append(miniWindow(), miniWindow());
  const rail = el('div', 'sp-rail');
  const isle = (cls, n) => shapes(el('div', `sp-isle ${cls}`), 'sp-dot', n);
  rail.append(el('span', 'sp-voice sp-voice--ride'), isle('sp-isle--a', 2), isle('sp-isle--b', 4), isle('sp-isle--c', 3));
  screen.append(desk, rail, el('span', 'sp-voice sp-voice--free'));
  return frame('bar', screen);
}

/** The four launchers, the chosen one looping its swipe-down entrance. */
export function launcherPreview() {
  const screen = el('div', 'sp-screen');
  screen.append(el('div', 'sp-wall'));
  const desk = el('div', 'sp-desk');
  desk.append(miniWindow(), miniWindow());
  screen.append(desk, el('div', 'sp-veil'));

  const windows = () => shapes(el('div', 'spl-windows'), 'spl-window', 3);
  const recents = (n) => shapes(el('div', 'spl-recents'), 'spl-file', n);
  const icons = (n) => shapes(el('div', 'spl-icons'), 'spl-icon', n);
  const rows = (n) => shapes(el('div', 'spl-rows'), 'spl-row', n);
  const recentRows = (n) => shapes(el('div', 'spl-rows spl-recent-rows'), 'spl-row spl-row--file', n);

  const grid = el('div', 'spl spl--grid');
  grid.append(el('span', 'spl-search'), windows(), recents(5), icons(14));

  const sheet = el('div', 'spl spl--sheet');
  const sheetBody = el('div', 'spl-sheet-body');
  const aside = el('div', 'spl-aside');
  aside.append(windows(), recentRows(4));
  sheetBody.append(aside, rows(8));
  sheet.append(el('span', 'spl-title'), sheetBody);

  const command = el('div', 'spl spl--command');
  command.append(el('span', 'spl-input'), windows(), recentRows(2), rows(5));

  const mosaic = el('div', 'spl spl--mosaic');
  mosaic.append(el('span', 'spl-title'), windows(), recents(4), icons(10));

  screen.append(grid, sheet, command, mosaic);
  return frame('launcher', screen);
}

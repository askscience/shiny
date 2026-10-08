/**
 * clipboardMenu.js — the top-bar Clipboard menu.
 *
 * The popover lists what the central clipboard service (clipboard.js) has
 * recorded, newest first. Clicking a row puts that text back on the OS
 * clipboard and offers it to the focused window for a direct paste
 * (`clipboard:paste`, consumed by the Terminal and Browser surfaces); the
 * trailing × removes a single entry and "Clear all" empties the history.
 *
 * Ctrl/Cmd+Shift+V toggles the menu from anywhere — the keyboard route to the
 * history, alongside the icon.
 *
 * The skeleton follows audioMenu.js: the popup is built once, appended to
 * <body> (the HUD pill clips overflow) and repositioned under the trigger on
 * every open. `notifyMenuChange()` on open/close matters for the Browser
 * window: it uses the signal to punch a hole for open popups over its native
 * page (see menuState.js).
 */

import { emptyState, icon, iconButton, listItem } from '../ui/index.js';
import { history, removeHistoryEntry, clearHistory, copyText } from './clipboard.js';
import { previewText, relativeTime } from './clipboardShared.js';
import { notifyMenuChange } from './menuState.js';
import { getFocus } from './desktop.js';

let popup = null;
let bodyEl = null;
let headCountEl = null;
let clearBtn = null;
let open = false;
let trigger = null;

/* ── Open / close ──────────────────────────────────────────── */

export function isClipboardMenuOpen() {
  return open;
}

export function openClipboardMenu(chip) {
  trigger = chip || trigger || document.getElementById('hud-clipboard');
  ensurePopup();
  open = true;
  // The popup element is reused between opens (scroll position and listeners
  // survive); unhide it here so a second open is not a blank popup.
  popup.classList.remove('hidden');
  popup.style.animation = 'none';
  void popup.offsetHeight;
  popup.style.animation = '';
  render();
  document.addEventListener('pointerdown', onOutside, true);
  document.addEventListener('keydown', onKey, true);
  window.addEventListener('resize', reposition);
  trigger?.setAttribute('aria-expanded', 'true');
  trigger?.classList.add('is-open');
  reposition();
  notifyMenuChange();
}

export function closeClipboardMenu() {
  if (!open) return;
  open = false;
  popup?.classList.add('hidden');
  document.removeEventListener('pointerdown', onOutside, true);
  document.removeEventListener('keydown', onKey, true);
  window.removeEventListener('resize', reposition);
  trigger?.setAttribute('aria-expanded', 'false');
  trigger?.classList.remove('is-open');
  notifyMenuChange();
}

export function toggleClipboardMenu(chip) {
  if (open) closeClipboardMenu();
  else openClipboardMenu(chip);
}

function onOutside(event) {
  if (event.target.closest?.('.ui-modal')) return;
  if (popup?.contains(event.target)) return;
  if (trigger?.contains(event.target)) return;
  closeClipboardMenu();
}

function onKey(event) {
  if (event.key === 'Escape' && !document.querySelector('.ui-modal:not(.hidden)')) {
    closeClipboardMenu();
  }
}

function reposition() {
  if (!popup || !trigger) return;
  const rect = trigger.getBoundingClientRect();
  const width = popup.offsetWidth;
  const left = Math.max(12, Math.min(rect.right - width, window.innerWidth - width - 12));
  const top = rect.bottom + 8;
  popup.style.left = `${left}px`;
  popup.style.top = `${top}px`;
  popup.style.maxHeight = `${Math.max(220, window.innerHeight - top - 16)}px`;
}

/* ── Skeleton ──────────────────────────────────────────────── */

function ensurePopup() {
  if (popup) return;
  popup = document.createElement('div');
  popup.className = 'ui-hud-menu-popup clipboard-menu hidden';
  popup.setAttribute('role', 'dialog');
  popup.setAttribute('aria-label', 'Clipboard history');

  const head = document.createElement('div');
  head.className = 'clipboard-head';
  const meta = document.createElement('div');
  meta.className = 'clipboard-head-main';
  const title = document.createElement('div');
  title.className = 'clipboard-head-title';
  title.appendChild(icon('ui/clipboard', { size: 15 }));
  const label = document.createElement('span');
  label.textContent = 'Clipboard';
  title.appendChild(label);
  headCountEl = document.createElement('div');
  headCountEl.className = 'clipboard-head-hint';
  meta.append(title, headCountEl);
  clearBtn = iconButton({
    icon: 'ui/trash',
    label: 'Clear the clipboard history',
    variant: 'quiet',
    size: 'sm',
    onClick: () => clearHistory(),
  });
  head.append(meta, clearBtn);

  bodyEl = document.createElement('div');
  bodyEl.className = 'clipboard-body';

  popup.append(head, bodyEl);
  document.body.appendChild(popup);
  // The service announces every change; render only while visible.
  window.addEventListener('clipboard:changed', () => {
    if (open) render();
  });
}

/* ── Rendering ─────────────────────────────────────────────── */

function sourceLabel(source) {
  if (!source || source === 'app') return 'Desktop';
  return source.charAt(0).toUpperCase() + source.slice(1);
}

function render() {
  if (!bodyEl) return;
  const rows = history();

  headCountEl.textContent = rows.length
    ? `${rows.length} item${rows.length === 1 ? '' : 's'} · Ctrl+Shift+V`
    : 'Ctrl+Shift+V';
  clearBtn.disabled = !rows.length;

  bodyEl.textContent = '';
  if (!rows.length) {
    bodyEl.appendChild(emptyState({
      icon: 'ui/clipboard',
      title: 'Nothing copied yet',
      body: 'Text you copy anywhere in the app lands here.',
    }));
    return;
  }
  rows.forEach((entry, index) => bodyEl.appendChild(buildRow(entry, index)));
}

function buildRow(entry, index) {
  const remove = iconButton({
    icon: 'ui/close',
    label: 'Remove from history',
    variant: 'quiet',
    size: 'sm',
    onClick: (event) => {
      // Never let the delete click also copy the row.
      event?.stopPropagation();
      removeHistoryEntry(index);
    },
  });
  remove.classList.add('clipboard-row-remove');

  // A div, not listItem's button flavour: the row hosts its own remove
  // button, and a button inside a button is invalid HTML.
  const row = listItem({
    leading: icon('ui/clipboard', { size: 16, className: 'clipboard-leading' }),
    title: previewText(entry.text),
    subtitle: [sourceLabel(entry.source), relativeTime(entry.at)].filter(Boolean).join(' · '),
    trailing: remove,
  });
  row.classList.add('clipboard-row');
  row.setAttribute('role', 'button');
  row.tabIndex = 0;
  row.addEventListener('click', () => pick(entry));
  row.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      pick(entry);
    }
  });
  return row;
}

/**
 * Re-copy a history entry and, when a surface that can paste directly is
 * focused (the terminal, a browser page), hand it the text so the pick works
 * as "copy and paste" there.
 */
function pick(entry) {
  void copyText(entry.text, { source: entry.source });
  window.dispatchEvent(new CustomEvent('clipboard:paste', {
    detail: { text: entry.text, focused: getFocus() },
  }));
  closeClipboardMenu();
}

/* ── Wiring ────────────────────────────────────────────────── */

/**
 * Wire the top-bar button and the keyboard shortcut. Called once from app.js
 * (after installClipboard(), so the menu always has a history to show).
 */
export function initHudClipboard() {
  const chip = document.getElementById('hud-clipboard');
  if (!chip) return;
  chip.setAttribute('aria-haspopup', 'dialog');
  chip.setAttribute('aria-expanded', 'false');
  chip.addEventListener('click', () => toggleClipboardMenu(chip));
  document.addEventListener('keydown', onShortcut, true);
}

/** Ctrl/Cmd+Shift+V: the system-wide way to the clipboard history. */
function onShortcut(event) {
  if (!(event.ctrlKey || event.metaKey) || !event.shiftKey) return;
  if ((event.key || '').toLowerCase() !== 'v') return;
  // A dialog owns the keyboard while it is up.
  if (document.querySelector('.ui-modal:not(.hidden)')) return;
  event.preventDefault();
  toggleClipboardMenu();
}

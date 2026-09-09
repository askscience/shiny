/**
 * contextMenu.js — right-click context menus for the core desktop.
 *
 * The desktop is a Hyprland-style window manager (desktop.js + tiles.js), so
 * right-click follows the same conventions as a real WM:
 *
 *   • Right-click a window title bar  → window menu (focus / fullscreen /
 *     move-to-workspace / deactivate).
 *   • Right-click the empty desktop   → desktop menu (new / remove workspace,
 *     switch workspace, layout mode).
 *   • Right-click a workspace dot     → workspace menu (new / remove).
 *
 * The menu itself is a small, self-contained popup engine: `openMenu` takes a
 * list of entries (items, separators, headings, and one level of submenus) and
 * renders a body-level popup reusing the app's glass tokens and `hudMenuIn`
 * animation. All desktop state is read fresh from desktop.js at open time.
 */
import {
  createWorkspace, removeWorkspace, switchWorkspace, focusWindow,
  toggleFullscreen, moveWindow, moveWindowByIndex, getWorkspacesList,
  activeWorkspaceIndex, getLayout, setLayout, getFocus, getFullscreen,
} from './desktop.js';
import { deactivatePlugin } from './tiles.js';
import { setIcon } from '../ui/index.js';

/* ── Popup engine ─────────────────────────────────────────────── */

const MENU_CLS = 'ctx-menu';
const PAD = 8;

/** Open menu elements, outermost first. Submenus are appended on top. */
const menuStack = [];
let globalWired = false;

function wireGlobal() {
  if (globalWired) return;
  globalWired = true;
  document.addEventListener('pointerdown', onGlobalPointerDown, true);
  document.addEventListener('keydown', onGlobalKeyDown, true);
  window.addEventListener('blur', closeAllMenus);
  window.addEventListener('resize', closeAllMenus);
  window.addEventListener('scroll', closeAllMenus, true);
}

function unwireGlobal() {
  if (!globalWired) return;
  globalWired = false;
  document.removeEventListener('pointerdown', onGlobalPointerDown, true);
  document.removeEventListener('keydown', onGlobalKeyDown, true);
  window.removeEventListener('blur', closeAllMenus);
  window.removeEventListener('resize', closeAllMenus);
  window.removeEventListener('scroll', closeAllMenus, true);
}

function onGlobalPointerDown(e) {
  if (!menuStack.length) return;
  // A press outside any open menu dismisses it (but still passes through, so
  // e.g. clicking a tile also raises the window).
  if (!menuStack.some((m) => m.contains(e.target))) closeAllMenus();
}

function onGlobalKeyDown(e) {
  if (!menuStack.length) return;
  if (e.key === 'Escape') {
    e.preventDefault();
    e.stopPropagation();
    closeAllMenus();
  }
}

function pushMenu(el) {
  menuStack.push(el);
  if (menuStack.length === 1) wireGlobal();
}

function closeFrom(el) {
  const i = menuStack.indexOf(el);
  if (i < 0) return;
  const closing = menuStack.splice(i);
  closing.forEach((m) => m.remove());
  if (!menuStack.length) unwireGlobal();
}

export function closeAllMenus() {
  const closing = menuStack.splice(0);
  closing.forEach((m) => m.remove());
  unwireGlobal();
}

/** Position a just-appended menu at (x, y), clamped inside the viewport. */
function place(el, x, y) {
  const w = el.offsetWidth;
  const h = el.offsetHeight;
  const left = Math.max(PAD, Math.min(x, window.innerWidth - w - PAD));
  const top = Math.max(PAD, Math.min(y, window.innerHeight - h - PAD));
  el.style.left = `${left}px`;
  el.style.top = `${top}px`;
}

/** Open a root menu with the given entries at (x, y). */
function openMenu(entries, x, y) {
  const el = buildMenu(entries);
  document.body.appendChild(el);
  place(el, x, y);
  pushMenu(el);
  return el;
}

/** Shared row scaffold: [check] [icon] [label] (+ chevron for submenus). */
function rowShell(text, danger) {
  const item = document.createElement('button');
  item.type = 'button';
  item.className = MENU_CLS + '-item';
  if (danger) item.classList.add('is-danger');
  item.setAttribute('role', 'menuitem');

  const check = document.createElement('span');
  check.className = 'ctx-menu-check';
  const ic = document.createElement('span');
  ic.className = 'ctx-menu-icon';
  const lab = document.createElement('span');
  lab.className = 'ctx-menu-label';
  lab.textContent = text;
  item.append(check, ic, lab);
  return { item, check, ic, lab };
}

function buildItem(entry) {
  const { item, check, ic } = rowShell(entry.label, entry.danger);
  if (entry.checked) {
    item.classList.add('is-active');
    void setIcon(check, 'ui/check', { size: 14 });
  }
  if (entry.icon) void setIcon(ic, entry.icon, { size: 14 });
  if (entry.disabled) item.disabled = true;

  item.addEventListener('click', () => {
    if (entry.disabled) return;
    closeAllMenus();
    entry.onClick?.();
  });
  return item;
}

/** Submenus are limited to one level — the desktop never needs deeper. */
function buildSubmenuItem(entry, parentEl) {
  const { item, check, ic } = rowShell(entry.label, entry.danger);
  if (entry.checked) item.classList.add('is-active');
  if (entry.icon) void setIcon(ic, entry.icon, { size: 14 });

  const chev = document.createElement('span');
  chev.className = 'ctx-menu-chevron';
  void setIcon(chev, 'ui/chevron-right', { size: 14 });
  item.appendChild(chev);

  let subEl = null;

  const openSub = () => {
    // Already open (and still attached)? Nothing to do.
    if (subEl && subEl.isConnected) return;
    // Close any other open submenu of this menu first, and drop stale
    // bookkeeping: after a submenu was closed elsewhere, the old `subEl`
    // reference stayed set and silently blocked reopening it.
    if (parentEl.__openSub) {
      if (parentEl.__openSub.isConnected) closeFrom(parentEl.__openSub);
      parentEl.__openSub = null;
    }

    subEl = buildMenu(entry.items);
    document.body.appendChild(subEl);
    const r = item.getBoundingClientRect();
    placeSub(subEl, r);
    pushMenu(subEl);
    parentEl.__openSub = subEl;
  };

  item.addEventListener('pointerenter', openSub);
  item.addEventListener('click', (e) => {
    e.stopPropagation();
    openSub();
  });

  return item;
}

function placeSub(subEl, anchor) {
  const w = subEl.offsetWidth;
  const h = subEl.offsetHeight;
  let left = anchor.right + 4;
  if (left + w > window.innerWidth - PAD) left = anchor.left - w - 4;
  let top = anchor.top;
  if (top + h > window.innerHeight - PAD) top = window.innerHeight - h - PAD;
  subEl.style.left = `${Math.max(PAD, left)}px`;
  subEl.style.top = `${Math.max(PAD, top)}px`;
}

function buildMenu(entries) {
  const el = document.createElement('div');
  el.className = MENU_CLS;
  el.setAttribute('role', 'menu');
  // Suppress the native context menu over our own popup.
  el.addEventListener('contextmenu', (e) => e.preventDefault());

  for (const entry of entries) {
    if (!entry) continue;
    if (entry.type === 'separator') {
      const sep = document.createElement('div');
      sep.className = 'ctx-menu-sep';
      el.appendChild(sep);
    } else if (entry.type === 'heading') {
      const h = document.createElement('div');
      h.className = 'ctx-menu-heading';
      h.textContent = entry.label;
      el.appendChild(h);
    } else if (entry.type === 'submenu') {
      el.appendChild(buildSubmenuItem(entry, el));
    } else {
      el.appendChild(buildItem(entry));
    }
  }
  return el;
}

/* ── Menu definitions ─────────────────────────────────────────── */

function label(name) {
  return name ? name.charAt(0).toUpperCase() + name.slice(1) : name;
}

function moveToWorkspaceItems(name) {
  const items = getWorkspacesList().map((ws, i) => ({
    type: 'item',
    label: `Workspace ${i + 1}`,
    checked: ws.windows.includes(name),
    onClick: () => moveWindow(name, ws.id),
  }));
  items.push({ type: 'separator' });
  items.push({
    type: 'item',
    label: 'New workspace',
    icon: 'ui/plus',
    onClick: () => moveWindowByIndex(name, 'new'),
  });
  return items;
}

function windowMenu(name) {
  return [
    { type: 'heading', label: label(name) },
    {
      type: 'item',
      label: 'Focus',
      icon: 'ui/monitor',
      checked: getFocus() === name,
      onClick: () => focusWindow(name),
    },
    {
      type: 'item',
      label: 'Full screen',
      icon: 'ui/expand',
      checked: getFullscreen() === name,
      onClick: () => toggleFullscreen(name),
    },
    { type: 'separator' },
    {
      type: 'submenu',
      label: 'Move to workspace',
      icon: 'ui/grid',
      items: moveToWorkspaceItems(name),
    },
    { type: 'separator' },
    {
      type: 'item',
      label: 'Deactivate',
      icon: 'ui/close',
      danger: true,
      onClick: () => void deactivatePlugin(name),
    },
  ];
}

function switchWorkspaceItems() {
  return getWorkspacesList().map((ws, i) => ({
    type: 'item',
    label: `Workspace ${i + 1}`,
    checked: i === activeWorkspaceIndex(),
    onClick: () => switchWorkspace(i),
  }));
}

function layoutItems() {
  const mode = getLayout().mode;
  return [
    { type: 'item', label: 'Columns', checked: mode === 'columns', onClick: () => setLayout({ mode: 'columns' }) },
    { type: 'item', label: 'Master', checked: mode === 'master', onClick: () => setLayout({ mode: 'master' }) },
    { type: 'item', label: 'Windows', checked: mode === 'windows', onClick: () => setLayout({ mode: 'windows' }) },
  ];
}

function desktopMenu() {
  const wss = getWorkspacesList();
  return [
    { type: 'heading', label: 'Desktop' },
    { type: 'item', label: 'New workspace', icon: 'ui/plus', onClick: () => createWorkspace() },
    {
      type: 'item',
      label: 'Remove workspace',
      icon: 'ui/minus',
      disabled: wss.length <= 1,
      onClick: () => removeWorkspace(),
    },
    { type: 'separator' },
    { type: 'submenu', label: 'Switch to workspace', icon: 'ui/grid', items: switchWorkspaceItems() },
    { type: 'submenu', label: 'Layout', icon: 'ui/arranger', items: layoutItems() },
  ];
}

function workspaceDotMenu(index) {
  const wss = getWorkspacesList();
  return [
    { type: 'heading', label: `Workspace ${index + 1}` },
    { type: 'item', label: 'New workspace', icon: 'ui/plus', onClick: () => createWorkspace() },
    {
      type: 'item',
      label: 'Remove workspace',
      icon: 'ui/minus',
      disabled: wss.length <= 1,
      onClick: () => {
        // removeWorkspace() removes the ACTIVE workspace — switch to the
        // right-clicked one first so the dot's own workspace is removed.
        if (index !== activeWorkspaceIndex()) switchWorkspace(index);
        removeWorkspace();
      },
    },
  ];
}

/* ── Wiring ───────────────────────────────────────────────────── */

export function initContextMenu() {
  const app = document.getElementById('app');
  if (!app || app.__ctxMenuWired) return;
  app.__ctxMenuWired = true;

  app.addEventListener('contextmenu', (e) => {
    const target = e.target;

    // Window title bar → window menu.
    const header = target.closest?.('.tile-header');
    if (header) {
      const tile = header.closest('.tile');
      const name = tile?.dataset.plugin;
      if (!name) return;
      e.preventDefault();
      closeAllMenus();
      openMenu(windowMenu(name), e.clientX, e.clientY);
      return;
    }

    // Workspace dot → workspace menu.
    const dot = target.closest?.('.workspace-bar-dot');
    if (dot) {
      const index = parseInt(dot.textContent, 10) - 1;
      if (!Number.isInteger(index) || index < 0) return;
      e.preventDefault();
      closeAllMenus();
      openMenu(workspaceDotMenu(index), e.clientX, e.clientY);
      return;
    }

    // Empty desktop only — #app itself is the hit target because the tile
    // grid is pointer-events:none and every other layer is chrome with its
    // own pointer-events. Never hijack a right-click meant for another UI.
    if (target === app) {
      e.preventDefault();
      closeAllMenus();
      openMenu(desktopMenu(), e.clientX, e.clientY);
    }
  });
}

/**
 * paneStack — one window, several panes, one of them at a time when narrow.
 *
 * A wide window lays its panes side by side (a rail, a list, a detail) with
 * its own CSS grid. In a narrow window, or on a phone, the same panes become
 * a stack: the first pane is the floor, every other pane slides in over it
 * when shown and slides out on Back — the Plugins window's manifest panel,
 * for every window.
 *
 *   const nav = paneStack(shell, {
 *     panes: [
 *       { id: 'list',   el: listEl },
 *       { id: 'reader', el: readerEl },     // gets a back button in its [data-pane-bar]
 *     ],
 *   });
 *   nav.show('reader');   // narrow: slides in; wide: nothing to do
 *   nav.back();           // narrow: slides the top pane out
 *   nav.isStacked();      // the layout in force
 *   nav.open();           // the ids on the stack, floor first
 *   nav.destroy();
 *
 * The root carries `ui-panes` always and `is-stacked` while stacked; each
 * pane carries `ui-pane`, `data-pane="<id>"`, the floor `is-floor`, and
 * `is-open` while on the stack (ui/ui.css draws the slide). While stacked a pane other than the
 * floor gets an icon-only Back button at the start of its `[data-pane-bar]`
 * (a header the owner marks; `back: false` on a pane opts out), Escape pops
 * the top pane, Android's Back and the edge swipe pop it through
 * ui/backStack.js, and focus moves into the pane and comes back out with
 * it. The root dispatches `pane:change` `{ detail: { id, stacked } }` on
 * every change of the top pane or of the layout.
 *
 * Stacked when the document is a phone (`html[data-phone]`) or the window
 * is at most `COMPACT_MAX` wide — the same rule as compactToolbar — unless
 * `compactWhen({ width, phone })` says otherwise.
 */

import { iconButton } from './button.js';
import { COMPACT_MAX } from '../compact.js';
import { pushBack } from '../backStack.js';
import { t } from '../../js/i18n.js';

function isPhone() {
  return document.documentElement.dataset.phone === '1';
}

export function paneStack(root, { panes = [], compactWhen = null, onChange = null } = {}) {
  if (!root || root.__panes) return root?.__panes || null;
  root.classList.add('ui-panes');

  const list = panes.filter((p) => p && p.el);
  list.forEach((p, i) => {
    p.el.classList.add('ui-pane');
    // The floor is the first pane given, wherever it sits among the root's
    // children (a strip of sections may come before it).
    p.el.classList.toggle('is-floor', i === 0);
    p.el.dataset.pane = p.id;
    p.el.style.setProperty('--pane-depth', String(i));
  });

  /** Ids on the stack above the floor, in order. */
  const open = [];
  let stacked = false;
  let releaseBack = null;
  /** The element focused before the first pane was pushed. */
  let trigger = null;

  const byId = (id) => list.find((p) => p.id === id) || null;
  const floor = () => list[0] || null;
  const top = () => (open.length ? byId(open[open.length - 1]) : floor());

  function emit() {
    const detail = { id: top()?.id || null, stacked };
    root.dispatchEvent(new CustomEvent('pane:change', { detail }));
    onChange?.(detail);
  }

  function ensureBack(p) {
    if (p.back === false || p === floor() || p.__back) return;
    const bar = p.el.matches('[data-pane-bar]') ? p.el : p.el.querySelector('[data-pane-bar]');
    if (!bar) return;
    const back = iconButton({
      icon: 'ui/chevron-left', variant: 'ghost', size: 'sm',
      label: t('common.back', null, 'Back'), className: 'ui-pane-back',
      onClick: (e) => { e.stopPropagation(); api.back(); },
    });
    bar.prepend(back);
    p.__back = back;
  }

  function syncBackRegistry() {
    if (stacked && open.length && !releaseBack) {
      releaseBack = pushBack(() => api.back());
    } else if ((!stacked || !open.length) && releaseBack) {
      releaseBack();
      releaseBack = null;
    }
  }

  function paint() {
    root.classList.toggle('is-stacked', stacked);
    for (const p of list) {
      const on = open.includes(p.id);
      p.el.classList.toggle('is-open', on);
      if (p !== floor()) p.el.setAttribute('aria-hidden', stacked && !on ? 'true' : 'false');
    }
    syncBackRegistry();
  }

  function focusInto(p) {
    if (!stacked) return;
    const target = p.__back || p.el.querySelector('button,[href],input,select,textarea,[tabindex]:not([tabindex="-1"])') || p.el;
    if (target === p.el && !p.el.hasAttribute('tabindex')) p.el.tabIndex = -1;
    try { target.focus({ preventScroll: true }); } catch (_) { /* detached */ }
  }

  function measure() {
    const tile = root.closest('.tile');
    const width = tile?.clientWidth || root.clientWidth;
    const phone = isPhone();
    const want = compactWhen ? !!compactWhen({ width, phone }) : (phone || (width > 0 && width <= COMPACT_MAX));
    if (want === stacked) return;
    stacked = want;
    for (const p of list) ensureBack(p);
    paint();
    emit();
  }

  const api = {
    root,
    /** Bring `id` to the top of the stack (a no-op for the floor). */
    show(id) {
      const p = byId(id);
      if (!p) return false;
      if (p === floor()) { api.reset(); return true; }
      const at = open.indexOf(id);
      if (at !== -1) open.splice(at + 1);
      else {
        if (!open.length) trigger = document.activeElement;
        open.push(id);
      }
      ensureBack(p);
      paint();
      if (stacked) p.el.scrollTop = 0;
      focusInto(p);
      emit();
      return true;
    },
    /** Pop the top pane; true when one was open. */
    back() {
      if (!open.length) return false;
      open.pop();
      paint();
      const restore = open.length ? byId(open[open.length - 1]) : null;
      if (restore) focusInto(restore);
      else if (stacked && trigger?.isConnected) { try { trigger.focus({ preventScroll: true }); } catch (_) { /* gone */ } }
      if (!open.length) trigger = null;
      emit();
      return true;
    },
    /** Back to the floor. */
    reset() {
      if (!open.length) return false;
      open.length = 0;
      paint();
      emit();
      return true;
    },
    isStacked: () => stacked,
    isOpen: (id) => open.includes(id),
    open: () => [floor()?.id, ...open].filter(Boolean),
    top: () => top()?.id || null,
    refresh: measure,
    destroy() {
      ro?.disconnect();
      root.removeEventListener('keydown', onKey);
      releaseBack?.(); releaseBack = null;
      for (const p of list) {
        p.__back?.remove(); delete p.__back;
        p.el.classList.remove('ui-pane', 'is-floor', 'is-open');
        p.el.removeAttribute('aria-hidden');
      }
      root.classList.remove('ui-panes', 'is-stacked');
      delete root.__panes;
    },
  };

  function onKey(e) {
    if (e.key !== 'Escape' || !stacked || !open.length) return;
    // A menu, a select or a dialog on top takes its own Escape.
    if (e.target.closest?.('.ui-menu, .ui-select-menu, .ui-modal, .ui-sheet')) return;
    e.preventDefault();
    e.stopPropagation();
    api.back();
  }
  root.addEventListener('keydown', onKey);

  const ro = typeof ResizeObserver === 'function' ? new ResizeObserver(() => measure()) : null;
  ro?.observe(root);
  const tile = root.closest('.tile');
  if (tile && ro) ro.observe(tile);

  root.__panes = api;
  measure();
  paint();
  return api;
}

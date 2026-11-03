/**
 * compact.js — one toolbar, two sizes.
 *
 * A window toolbar is laid out for a wide window. In a narrow one (a phone,
 * a small tiled window) the same bar keeps every command but takes less room:
 *
 *   - compact: every `.ui-btn` that has an icon loses its text (the label
 *     becomes its tooltip and accessible name), captions and status text
 *     step back (ui/responsive.css, `.ui-compact-bar.is-compact`);
 *   - overflow: the trailing actions that still do not fit move, as the
 *     very same elements, into one shared "More" menu (⋯) at the end of
 *     the bar, so nothing is lost and no bar scrolls sideways.
 *
 * `compactToolbar(bar)` is applied by the tile manager to every
 * `data-window-bar` (web/js/tiles.js) and by a window to any secondary row
 * it owns (Word's formatting row). What can move: a `.ui-btn` that is a
 * direct child of the bar or of a group (a child holding only buttons);
 * menu triggers (`.ui-menu-trigger`), anything `[data-keep]`, inputs,
 * selects and segmented controls stay. Actions move from the end of the bar
 * backwards, so the front of the bar keeps the primary ones.
 *
 * Returns `{ refresh, destroy }`. The bar carries `ui-compact-bar` always and
 * `is-compact` while compact, and dispatches `compact:change` on the switch.
 */

import { controlButton } from './components/button.js';
import { menuSurface } from './components/menu.js';
import { placePopover } from './popover.js';
import { icon } from './icon.js';
import { t } from '../js/i18n.js';

/** Window width at or under which a bar is compact (keep in step with the
 *  `plugin-window` 700px container query in ui/responsive.css). */
export const COMPACT_MAX = 700;

const SKIP = 'input, textarea, select, .ui-select-wrap, .ui-segmented, .ui-search, .ui-menu-trigger, [data-keep], .ui-overflow-btn';

function isPhone() {
  return document.documentElement.dataset.phone === '1';
}

/** The buttons that may move to the menu, in bar order. */
function units(bar) {
  const out = [];
  for (const child of bar.children) {
    if (child.matches(SKIP) || child.hidden || child.classList.contains('hidden')) continue;
    if (child.matches('button.ui-btn')) { out.push(child); continue; }
    // A group: a plain box holding nothing but buttons (a cluster).
    const kids = [...child.children];
    if (kids.length && kids.every((k) => k.matches('button.ui-btn')) && !child.matches('[data-keep]')) {
      for (const k of kids) if (!k.matches(SKIP) && !k.hidden) out.push(k);
    }
  }
  return out;
}

/** The label becomes the tooltip once the text is hidden. */
function nameIconButtons(bar) {
  for (const btn of bar.querySelectorAll('button.ui-btn')) {
    if (btn.title || btn.getAttribute('aria-label')) continue;
    const text = [...btn.children].filter((c) => !c.classList.contains('ui-icon') && !c.classList.contains('ui-spinner') && !c.classList.contains('ui-btn-arrow'))
      .map((c) => c.textContent.trim()).filter(Boolean).join(' ');
    if (!text || !btn.querySelector('.ui-icon')) continue;
    btn.title = text;
    btn.setAttribute('aria-label', text);
  }
}

export function compactToolbar(bar, { compactWhen = null } = {}) {
  if (!bar || bar.__compact) return bar?.__compact || null;
  bar.classList.add('ui-compact-bar');

  const more = controlButton();
  more.className = 'ui-btn ui-btn--ghost ui-btn--sm ui-btn--icon ui-tool ui-overflow-btn';
  more.hidden = true;
  more.setAttribute('aria-haspopup', 'menu');
  more.setAttribute('aria-expanded', 'false');
  more.appendChild(icon('ui/more', { size: 16 }));
  const relabel = () => {
    const label = t('common.more', null, 'More');
    more.title = label;
    more.setAttribute('aria-label', label);
  };
  relabel();
  bar.appendChild(more);

  let menu = null;
  let list = null;
  let open = false;
  let busy = false;
  /** Set when refresh() moved nodes itself: that mutation batch is ours. */
  let selfMutated = false;
  let compact = false;
  /** Units moved into the menu, in the order they were moved. */
  const moved = [];

  function ensureMenu() {
    if (menu) return;
    menu = menuSurface({ className: 'ui-overflow-menu', onEscape: closeMenu });
    list = document.createElement('div');
    list.className = 'ui-overflow-list';
    menu.appendChild(list);
    // A plain action closes the menu once it ran; a trigger that opens its
    // own menu (export, AI) keeps this one up under it.
    menu.addEventListener('click', (e) => {
      const btn = e.target.closest('button');
      if (btn && !btn.getAttribute('aria-haspopup')) closeMenu();
    });
    document.body.appendChild(menu);
  }

  function onOutside(e) {
    if (!open) return;
    if (menu.contains(e.target) || more.contains(e.target)) return;
    // A menu opened from an overflowed trigger sits over this one.
    if (e.target.closest('.ui-menu, .ui-hud-menu-popup, .ui-modal, .ui-sheet')) return;
    closeMenu();
  }

  function openMenu() {
    if (open || !moved.length) return;
    ensureMenu();
    open = true;
    menu.classList.remove('hidden');
    more.setAttribute('aria-expanded', 'true');
    placePopover(menu, more);
    menu.focusFirst?.();
    document.addEventListener('pointerdown', onOutside, true);
  }

  function closeMenu() {
    if (!open) return;
    open = false;
    menu?.classList.add('hidden');
    more.setAttribute('aria-expanded', 'false');
    document.removeEventListener('pointerdown', onOutside, true);
  }

  function putBack(rec) {
    const { el, parent, next } = rec;
    if (!parent.isConnected && parent !== bar) { el.remove(); return; }
    parent.insertBefore(el, next && next.parentNode === parent ? next : null);
    if (parent !== bar && parent.dataset.compactEmpty) {
      delete parent.dataset.compactEmpty;
      setHidden(parent, false);
    }
  }

  function takeOut(el) {
    const parent = el.parentNode;
    const rec = { el, parent, next: el.nextSibling };
    ensureMenu();
    list.prepend(el);
    if (parent !== bar && ![...parent.children].some((c) => !c.hidden)) {
      parent.dataset.compactEmpty = '1';
      setHidden(parent, true);
    }
    moved.push(rec);
  }

  const overflowing = () => bar.scrollWidth > bar.clientWidth + 1;

  /** Writing an attribute always queues a mutation record, even unchanged:
   *  only write when it changes, and mark the write as ours. */
  function setHidden(el, hidden) {
    if (el.hidden === hidden) return;
    el.hidden = hidden;
    selfMutated = true;
  }

  function refresh() {
    if (busy || !bar.isConnected) return;
    busy = true;
    const movedBefore = moved.length;
    try {
      nameIconButtons(bar);
      const tile = bar.closest('.tile');
      const width = tile?.clientWidth || bar.clientWidth;
      const want = compactWhen ? !!compactWhen({ width, phone: isPhone() }) : (isPhone() || (width > 0 && width <= COMPACT_MAX));
      if (want !== compact) {
        compact = want;
        bar.classList.toggle('is-compact', compact);
        selfMutated = true;
        bar.dispatchEvent(new CustomEvent('compact:change', { detail: { compact } }));
      }
      // Everything back, then take from the end until the bar fits.
      closeMenu();
      while (moved.length) putBack(moved.pop());
      setHidden(more, true);
      if (!compact || !bar.clientWidth) return;
      if (!overflowing()) return;
      setHidden(more, false);
      const candidates = units(bar);
      while (overflowing() && candidates.length) takeOut(candidates.pop());
      if (!moved.length) setHidden(more, true);
    } finally {
      // Putting back / taking out and the More button's visibility are our
      // own mutations: the observer must not answer them with another pass.
      if (movedBefore || moved.length) selfMutated = true;
      busy = false;
    }
  }

  more.addEventListener('click', () => (open ? closeMenu() : openMenu()));

  const ro = typeof ResizeObserver === 'function' ? new ResizeObserver(() => refresh()) : null;
  ro?.observe(bar);
  const tile = bar.closest('.tile');
  if (tile && ro) ro.observe(tile);
  // The bar's own content changes (an action appears, a label changes).
  const mo = typeof MutationObserver === 'function'
    ? new MutationObserver(() => {
      if (selfMutated) { selfMutated = false; return; }
      if (!busy) refresh();
    })
    : null;
  mo?.observe(bar, { childList: true, subtree: true, attributes: true, attributeFilter: ['hidden', 'class'] });
  window.addEventListener('locale:change', relabel);

  const api = {
    refresh,
    isCompact: () => compact,
    destroy() {
      ro?.disconnect();
      mo?.disconnect();
      window.removeEventListener('locale:change', relabel);
      closeMenu();
      while (moved.length) putBack(moved.pop());
      menu?.remove();
      more.remove();
      bar.classList.remove('ui-compact-bar', 'is-compact');
      delete bar.__compact;
    },
  };
  bar.__compact = api;
  refresh();
  return api;
}

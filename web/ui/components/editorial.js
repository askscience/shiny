/**
 * editorial — the small pieces of the editorial window language, shared by
 * every window so none of them re-invents them:
 *
 *   eyebrow(text)          a small mono uppercase label ("OUTPUT", "LOOKS")
 *   metaLine(parts)        mono uppercase metadata joined by " · "
 *   largeTitle({ … })      a large display title + meta at the top of a
 *                          scrolling area, handing over to the window bar as
 *                          the area scrolls (the Mail pattern)
 *   hairlineList()         a list whose rows are separated by hairlines
 *
 * Styles live in /ui/ui.css (structure, tokens only); themes may skin them.
 */
import { decodeText } from '../motionStyle.js';

export function eyebrow(text = '', className = '') {
  const el = document.createElement('span');
  el.className = `ui-eyebrow${className ? ` ${className}` : ''}`;
  el.textContent = text;
  return el;
}

/** Mono metadata from parts; empty parts are skipped. Call `el.set(parts)` to update. */
export function metaLine(parts = [], className = '') {
  const el = document.createElement('span');
  el.className = `ui-meta${className ? ` ${className}` : ''}`;
  el.set = (next) => {
    el.textContent = (Array.isArray(next) ? next : [next]).filter((p) => p !== '' && p != null).join(' · ');
  };
  el.set(parts);
  return el;
}

/**
 * A large title for a window, at the top of `scroller` (the element that
 * scrolls). As it scrolls, `--sc` (0…1 over the first 56 px) is set on the
 * window (`.tile`): the large title fades out and the window bar's own title
 * and hairline fade in (`.tile[data-large-title]` in ui.css). Returns
 * `{ el, setTitle(text, { decode }), setMeta(parts), dispose() }`; insert
 * `el` as the scroller's first child.
 */
export function largeTitle({ title = '', meta = [], scroller = null, eyebrow: kicker = '' } = {}) {
  const el = document.createElement('header');
  el.className = 'ui-large-title';
  let kickerEl = null;
  if (kicker) {
    kickerEl = eyebrow(kicker);
    el.appendChild(kickerEl);
  }
  const row = document.createElement('div');
  row.className = 'ui-large-title-row';
  const h = document.createElement('h2');
  h.className = 'ui-large-title-text';
  h.textContent = title;
  const metaEl = metaLine(meta);
  row.append(h, metaEl);
  el.appendChild(row);

  let frame = 0;
  let tile = null;
  const sync = () => {
    frame = 0;
    tile = tile?.isConnected ? tile : el.closest('.tile');
    if (!tile || !scroller) return;
    tile.dataset.largeTitle = '';
    tile.style.setProperty('--sc', Math.min(1, Math.max(0, scroller.scrollTop / 56)).toFixed(3));
  };
  const onScroll = () => { if (!frame) frame = requestAnimationFrame(sync); };
  scroller?.addEventListener('scroll', onScroll, { passive: true });
  // The window element exists only once mounted: sync on the next frame too.
  if (typeof requestAnimationFrame === 'function') requestAnimationFrame(sync);

  return {
    el,
    setTitle(text, { decode = false } = {}) {
      if (h.textContent === text) return;
      if (decode) decodeText(h, text);
      else h.textContent = text;
    },
    setMeta(parts) { metaEl.set(parts); },
    setEyebrow(text) { if (kickerEl) kickerEl.textContent = text; },
    sync,
    dispose() {
      scroller?.removeEventListener('scroll', onScroll);
      cancelAnimationFrame(frame);
      if (tile) {
        delete tile.dataset.largeTitle;
        tile.style.removeProperty('--sc');
      }
    },
  };
}

/** A list whose rows (`.ui-hairline-row`) are separated by hairlines. */
export function hairlineList(className = '') {
  const el = document.createElement('div');
  el.className = `ui-hairline-list${className ? ` ${className}` : ''}`;
  el.setAttribute('role', 'list');
  return el;
}

/** One row for hairlineList: lead (icon), main text, mono meta, trailing node. */
export function hairlineRow({ lead = null, title = '', sub = '', meta = '', trail = null, onClick = null } = {}) {
  const el = document.createElement(onClick ? 'button' : 'div');
  if (onClick) { el.type = 'button'; el.dataset.uiControl = 'button'; el.addEventListener('click', onClick); }
  el.className = 'ui-hairline-row';
  el.setAttribute('role', 'listitem');
  if (lead) { const l = document.createElement('span'); l.className = 'ui-hairline-lead'; l.appendChild(lead); el.appendChild(l); }
  const main = document.createElement('span');
  main.className = 'ui-hairline-main';
  const t = document.createElement('span');
  t.className = 'ui-hairline-title';
  t.textContent = title;
  main.appendChild(t);
  if (sub) { const s = document.createElement('span'); s.className = 'ui-hairline-sub'; s.textContent = sub; main.appendChild(s); }
  el.appendChild(main);
  if (meta) { const m = document.createElement('span'); m.className = 'ui-meta'; m.textContent = meta; el.appendChild(m); }
  if (trail) el.appendChild(trail);
  return el;
}

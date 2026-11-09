/**
 * capture/picker.js — choose what to record: one window (click it) or a
 * region (drag it, with common aspect ratios for Shorts, Reels and posts).
 */

import { element as h, button, segmented } from '../../ui/index.js';
import { t } from '../../js/i18n.js';
import { toScreenRect } from './bubble.js';

export function tileRect(tile) {
  return tile.getBoundingClientRect();
}

export function tileTitle(tile) {
  if (!tile) return '';
  const title = tile.querySelector('.tile-header-title')?.textContent?.trim();
  const name = tile.dataset.plugin || '';
  return title || (name ? name.charAt(0).toUpperCase() + name.slice(1) : '');
}

function overlay(cls) {
  const o = h('div', `capture-pick ${cls}`);
  document.body.appendChild(o);
  return o;
}

/** Resolve with the tile the user clicks, or null (Escape, right click). */
export function pickWindow() {
  return new Promise((resolve) => {
    const o = overlay('capture-pick--window');
    const hint = h('div', 'capture-pick-hint', t('plugin.capture.pick.window', null, 'Click the window to record · Esc to cancel'));
    const box = h('div', 'capture-pick-box');
    const label = h('span', 'capture-pick-label');
    box.appendChild(label);
    box.hidden = true;
    o.append(box, hint);
    let current = null;
    const under = (x, y) => {
      o.style.pointerEvents = 'none';
      const tile = document.elementFromPoint(x, y)?.closest?.('section.tile');
      o.style.pointerEvents = '';
      return tile && tile.getClientRects().length ? tile : null;
    };
    const move = (e) => {
      current = under(e.clientX, e.clientY);
      if (!current) { box.hidden = true; return; }
      const r = current.getBoundingClientRect();
      box.hidden = false;
      Object.assign(box.style, { left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px` });
      label.textContent = tileTitle(current);
    };
    const done = (tile) => {
      o.remove();
      window.removeEventListener('keydown', key, true);
      resolve(tile);
    };
    const key = (e) => { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); done(null); } };
    o.addEventListener('pointermove', move);
    o.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      if (e.button !== 0) { done(null); return; }
      done(under(e.clientX, e.clientY));
    });
    o.addEventListener('contextmenu', (e) => e.preventDefault());
    window.addEventListener('keydown', key, true);
  });
}

const RATIOS = { free: null, '16:9': 16 / 9, '9:16': 9 / 16, '1:1': 1 };

/** Resolve with a region in screen pixels, or null. */
export function pickRegion() {
  return new Promise((resolve) => {
    const o = overlay('capture-pick--region');
    const box = h('div', 'capture-pick-box capture-pick-box--region');
    const size = h('span', 'capture-pick-label');
    box.appendChild(size);
    box.hidden = true;
    let ratio = null;
    const bar = h('div', 'capture-pick-bar');
    const seg = segmented({
      label: t('plugin.capture.pick.ratio', null, 'Shape'),
      value: 'free',
      options: [
        { value: 'free', label: t('plugin.capture.pick.free', null, 'Free') },
        { value: '16:9', label: '16:9' },
        { value: '9:16', label: '9:16' },
        { value: '1:1', label: '1:1' },
      ],
      onChange: (v) => { ratio = RATIOS[v]; if (rect) { fix(); paint(); } },
    });
    const ok = button({ icon: 'ui/check', label: t('plugin.capture.pick.use', null, 'Use this area'), size: 'sm', onClick: () => done(true) });
    const cancel = button({ icon: 'ui/close', label: t('plugin.capture.cancel', null, 'Cancel'), variant: 'ghost', size: 'sm', onClick: () => done(false) });
    bar.append(seg, cancel, ok);
    const hint = h('div', 'capture-pick-hint', t('plugin.capture.pick.region', null, 'Drag to draw the area · Enter to confirm · Esc to cancel'));
    o.append(box, bar, hint);
    let rect = null;
    let drag = null;
    const fix = () => {
      if (!ratio || !rect) return;
      rect.h = Math.round(rect.w / ratio);
      if (rect.y + rect.h > window.innerHeight) { rect.h = window.innerHeight - rect.y; rect.w = Math.round(rect.h * ratio); }
    };
    const paint = () => {
      if (!rect || rect.w < 4 || rect.h < 4) { box.hidden = true; return; }
      box.hidden = false;
      Object.assign(box.style, { left: `${rect.x}px`, top: `${rect.y}px`, width: `${rect.w}px`, height: `${rect.h}px` });
      const s = toScreenRect({ left: rect.x, top: rect.y, width: rect.w, height: rect.h });
      size.textContent = `${s.w} × ${s.h}`;
    };
    o.addEventListener('pointerdown', (e) => {
      if (e.target.closest('.capture-pick-bar')) return;
      e.preventDefault();
      o.setPointerCapture(e.pointerId);
      drag = { x0: e.clientX, y0: e.clientY };
      rect = { x: e.clientX, y: e.clientY, w: 0, h: 0 };
      paint();
    });
    o.addEventListener('pointermove', (e) => {
      if (!drag) return;
      const x = Math.min(drag.x0, e.clientX);
      const y = Math.min(drag.y0, e.clientY);
      rect = { x, y, w: Math.abs(e.clientX - drag.x0), h: Math.abs(e.clientY - drag.y0) };
      fix();
      paint();
    });
    o.addEventListener('pointerup', () => { drag = null; });
    o.addEventListener('dblclick', () => done(true));
    const key = (e) => {
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); done(false); }
      if (e.key === 'Enter') { e.preventDefault(); e.stopPropagation(); done(true); }
    };
    window.addEventListener('keydown', key, true);
    function done(use) {
      window.removeEventListener('keydown', key, true);
      seg.destroy?.();
      o.remove();
      if (!use || !rect || rect.w < 16 || rect.h < 16) { resolve(null); return; }
      resolve(toScreenRect({ left: rect.x, top: rect.y, width: rect.w, height: rect.h }));
    }
  });
}

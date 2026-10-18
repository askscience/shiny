import { element } from './element.js';

/** Selectable artwork card. Prefix classes are optional domain layout hooks. */
export function mediaCard({ title, subtitle, src, position, duration, onClick, className = '', prefix = 'ui-media-card' } = {}) {
  const cell = element('div', `ui-media-card ${className}`);
  cell.setAttribute('role', 'button');
  cell.tabIndex = 0;
  const num = element('span', `${prefix}-num`, String(position ?? '').padStart(2, '0'));
  const art = element('span', `${prefix}-art`);
  if (src) {
    const img = document.createElement('img');
    img.src = src; img.alt = ''; img.loading = 'lazy';
    img.onerror = () => img.remove();
    art.appendChild(img);
  }
  if (duration) art.appendChild(element('span', `${prefix}-dur`, duration));
  const meta = element('span', `${prefix}-meta`);
  meta.appendChild(element('span', `${prefix}-name`, title));
  if (subtitle) meta.appendChild(element('span', `${prefix}-channel`, subtitle));
  cell.append(num, art, meta);
  cell.addEventListener('click', onClick);
  cell.addEventListener('keydown', e => {
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onClick?.(e); }
  });
  return cell;
}

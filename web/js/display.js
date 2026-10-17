/**
 * display.js — the host interface scale (page zoom applied by the kiosk shell).
 *
 * Not a per-user preference: the scale belongs to the panel, and `peakd`
 * applies it before anyone signs in. The server persists the choice to a file
 * the shell watches, so changing it here re-zooms the page live — which also
 * re-fits the Terminal, since the logical viewport shrinks.
 *
 * `/api/display` returns the stored choice plus the value the shell last
 * applied, so "Auto" can show what it resolved to.
 */

import { apiFetch } from './api.js';
import { t } from './i18n.js';

/** One scale choice; `label` resolves in the active language when it is read. */
const scaleOption = (value, labelKey, english) => ({
  value,
  labelKey,
  get label() { return t(labelKey, null, english); },
});

/** Choices offered in Settings. `auto` follows the panel's pixel density. */
export const SCALE_OPTIONS = [
  scaleOption('auto', 'display.auto', 'Auto (match the display)'),
  scaleOption('1', 'display.scale.100', '100%'),
  scaleOption('1.25', 'display.scale.125', '125%'),
  scaleOption('1.5', 'display.scale.150', '150%'),
  scaleOption('1.75', 'display.scale.175', '175%'),
  scaleOption('2', 'display.scale.200', '200%'),
  scaleOption('2.5', 'display.scale.250', '250%'),
];

export async function getDisplayScale() {
  const res = await apiFetch('/api/display', { authRedirect: false });
  return res?.data || null;
}

export async function setDisplayScale(value) {
  const scale = value === 'auto' || value === '' ? 'auto' : Number(value);
  const res = await apiFetch('/api/display', {
    method: 'PUT',
    body: JSON.stringify({ scale }),
  });
  return res?.data || null;
}

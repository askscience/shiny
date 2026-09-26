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

/** Choices offered in Settings. `auto` follows the panel's pixel density. */
export const SCALE_OPTIONS = [
  { value: 'auto', label: 'Auto (match the display)' },
  { value: '1', label: '100%' },
  { value: '1.25', label: '125%' },
  { value: '1.5', label: '150%' },
  { value: '1.75', label: '175%' },
  { value: '2', label: '200%' },
  { value: '2.5', label: '250%' },
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

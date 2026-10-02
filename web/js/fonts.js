/**
 * fonts — the host's installed font families.
 *
 * The Settings "Global font" picker and the Writer's font menu both list what
 * fontconfig actually has, so a font the user installs (apt, a file in
 * ~/.local/share/fonts, …) appears with no code change. The list comes from
 * `GET /api/fonts` (the server runs `fc-list`; see src/api/fonts.rs) and is
 * cached for the session.
 */
import { apiFetch } from './api.js';

/** Shown only when the host list cannot be fetched (e.g. a remote client). */
const FALLBACK = [
  'Roboto', 'Inter', 'DM Sans', 'Space Grotesk', 'Instrument Serif',
  'DejaVu Sans', 'system-ui',
];

let cached = null;
let inflight = null;

/** Force the next `listFonts()` to re-query (e.g. after installing a font). */
export function clearFontCache() {
  cached = null;
}

/** Installed font family names, sorted. Falls back to a small built-in list. */
export async function listFonts({ force = false } = {}) {
  if (!force && cached) return cached;
  if (inflight) return inflight;
  inflight = apiFetch('/api/fonts', { authRedirect: false })
    .then((res) => {
      const data = Array.isArray(res?.data) ? res.data : [];
      const list = data.map((n) => String(n).trim()).filter(Boolean);
      cached = list.length ? list : FALLBACK.slice();
      return cached;
    })
    .catch(() => {
      cached = FALLBACK.slice();
      return cached;
    })
    .finally(() => { inflight = null; });
  return inflight;
}

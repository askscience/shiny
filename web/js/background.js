/**
 * background.js — the full-screen desktop background layer (#background).
 *
 * Modes (chosen in Settings → Background, stored per user in localStorage):
 *   none      — the default subtle accent/gradient mesh (layer stays blank)
 *   gradient  — a full-strength gradient, using the Appearance gradient
 *   image     — a built-in wallpaper preset, or an uploaded photo
 *               (served from /api/background), dimmed so the UI stays readable
 *   animated  — a CSS-animated preset ("aurora" drift or "shimmer" sweep)
 */

import { getGradient, gradientToCss, getThemeManifest } from '../ui/index.js';

const BG_KEY = 'ui.background';

/**
 * Built-in wallpapers, shipped in /backgrounds.
 *
 * `dim`   — strength of the black scrim painted over the artwork on a DARK
 *           theme: the dark pieces need almost none, the light "Paper" piece
 *           needs a heavy one to keep white UI text legible.
 * `tone`  — how bright the artwork is. On a LIGHT theme the scrim flips to
 *           white, and only the dark pieces need it, so the bright one is left
 *           alone rather than being greyed out.
 */
export const BACKGROUND_PRESETS = [
  { id: 'split',  label: 'Split',  src: '/backgrounds/split.svg',  dim: 0.12, tone: 'dark' },
  { id: 'grid',   label: 'Grid',   src: '/backgrounds/grid.svg',   dim: 0.12, tone: 'dark' },
  { id: 'halo',   label: 'Halo',   src: '/backgrounds/halo.svg',   dim: 0.05, tone: 'dark' },
  { id: 'offset', label: 'Offset', src: '/backgrounds/offset.svg', dim: 0.18, tone: 'dark' },
  { id: 'paper',  label: 'Paper',  src: '/backgrounds/paper.svg',  dim: 0.55, tone: 'light' },
];

/**
 * Which wallpaper a user gets before they choose one, per theme mode. The
 * default has to suit the theme it lands in: the dark pieces would swallow
 * dark UI text, so light themes start on Paper instead.
 */
const DEFAULT_BG_PRESETS = { dark: 'split', light: 'paper' };

const UPLOAD_DIM = 0.5;
const LIGHT_TONE_SCRIM = 0.82;
// `preset` stays null here on purpose: a user who never touched this setting
// (or who predates wallpapers, with only an uploaded photo stored) must not
// have the default injected over their choice. The default is resolved at
// read time instead — see activePreset().
const DEFAULT_BG = { mode: 'image', animation: null, url: null, preset: null };

export function presetById(id) {
  return BACKGROUND_PRESETS.find((p) => p.id === id) || null;
}

function themeMode() {
  const modes = getThemeManifest()?.modes;
  const light = Array.isArray(modes) && modes.includes('light') && !modes.includes('dark');
  return light ? 'light' : 'dark';
}

/** The scrim that keeps UI text readable over a wallpaper, by theme mode. */
function scrimFor(preset) {
  if (themeMode() === 'light') {
    return preset.tone === 'light' ? null : `rgba(255, 255, 255, ${LIGHT_TONE_SCRIM})`;
  }
  return preset.dim > 0 ? `rgba(0, 0, 0, ${preset.dim})` : null;
}

/** The built-in wallpaper that stands in when the user has chosen nothing. */
export function defaultPreset() {
  return presetById(DEFAULT_BG_PRESETS[themeMode()]) || BACKGROUND_PRESETS[0];
}

/**
 * The built-in wallpaper actually in effect for `bg`: the explicitly chosen
 * preset, the theme's default when the user has chosen nothing at all, or null
 * when an uploaded photo takes precedence.
 */
export function activePreset(bg = getBackground()) {
  if (bg.preset) return presetById(bg.preset);
  if (bg.url) return null;
  return defaultPreset();
}

let getScope = () => null;
function scopedKey(base) {
  const id = getScope();
  return id ? `${base}.${id}` : base;
}

let themeListenerBound = false;

export function initBackground({ getScope: scopeGetter } = {}) {
  if (typeof scopeGetter === 'function') getScope = scopeGetter;
  // An un-chosen background follows the theme mode, so it has to be re-resolved
  // when the user switches between a dark and a light theme.
  if (!themeListenerBound) {
    themeListenerBound = true;
    window.addEventListener('theme:change', () => applyBackground());
  }
  applyBackground();
}

export function getBackground() {
  try {
    const raw = localStorage.getItem(scopedKey(BG_KEY));
    if (raw) {
      const parsed = JSON.parse(raw);
      if (parsed && typeof parsed === 'object') {
        return { ...DEFAULT_BG, ...parsed };
      }
    }
  } catch (_) { /* fall through */ }
  return { ...DEFAULT_BG };
}

export function setBackground(patch) {
  const value = { ...getBackground(), ...patch };
  localStorage.setItem(scopedKey(BG_KEY), JSON.stringify(value));
  applyBackground();
  return value;
}

/** Re-apply after login / user switch (scope changed). */
export function refreshBackground() {
  applyBackground();
}

export function applyBackground() {
  const el = document.getElementById('background');
  if (!el) return;
  const bg = getBackground();

  el.classList.remove('bg-anim-aurora', 'bg-anim-shimmer');
  el.style.backgroundImage = '';
  el.style.backgroundSize = '';
  el.style.backgroundPosition = '';
  // Static by default — only the 'animated' mode adds a CSS animation class
  // (its `!important` animation overrides this inline `none`).
  el.style.animation = 'none';

  switch (bg.mode) {
    case 'gradient':
      el.style.backgroundImage = `linear-gradient(rgba(0, 0, 0, 0.42), rgba(0, 0, 0, 0.42)), ${gradientToCss(getGradient())}`;
      el.style.backgroundSize = '100% 100%';
      el.style.backgroundPosition = 'center';
      break;
    case 'image': {
      // A built-in wallpaper wins when one is selected; otherwise fall back to
      // the uploaded photo. Use the stored URL verbatim — it already carries a
      // one-time cache-buster from upload. Re-appending a fresh `Date.now()`
      // here would change the URL on every session refresh (reloadUserSession
      // runs every minute) and make the photo flicker.
      const preset = activePreset(bg);
      const src = preset ? preset.src : (bg.url || '/api/background');
      const scrim = preset ? scrimFor(preset) : `rgba(0, 0, 0, ${UPLOAD_DIM})`;
      el.style.backgroundImage = scrim
        ? `linear-gradient(${scrim}, ${scrim}), url("${src}")`
        : `url("${src}")`;
      el.style.backgroundSize = 'cover';
      el.style.backgroundPosition = 'center';
      break;
    }
    case 'animated':
      el.classList.add(bg.animation === 'shimmer' ? 'bg-anim-shimmer' : 'bg-anim-aurora');
      break;
    default:
      break; // none — static default mesh (no animation)
  }
}

/**
 * Fill `container` with the wallpaper picker: one tile per built-in preset
 * plus a "your own image" tile. Rebuilt on every call so selection state and
 * the uploaded thumbnail stay in sync.
 *
 *   onSelect(preset)  — a built-in wallpaper was picked
 *   onUpload()        — the upload tile was picked
 */
export function renderBackgroundPresets(container, { onSelect, onUpload } = {}) {
  if (!container) return;
  const bg = getBackground();
  const hasPhoto = !!bg.url;
  const activeId = hasPhoto ? null : activePreset(bg)?.id;
  container.innerHTML = '';

  const tile = (active) => {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.setAttribute('role', 'radio');
    btn.setAttribute('aria-checked', String(active));
    btn.className = `bg-preset${active ? ' is-active' : ''}`;
    btn.innerHTML = '<span class="bg-preset-label"></span>';
    return btn;
  };

  BACKGROUND_PRESETS.forEach((preset) => {
    const active = activeId === preset.id;
    const btn = tile(active);
    btn.dataset.preset = preset.id;
    btn.setAttribute('aria-label', `${preset.label} wallpaper`);
    btn.style.backgroundImage = `url("${preset.src}")`;
    btn.querySelector('.bg-preset-label').textContent = preset.label;
    btn.addEventListener('click', () => onSelect?.(preset));
    container.appendChild(btn);
  });

  // The uploaded photo, or an invitation to add one.
  const custom = tile(hasPhoto);
  custom.dataset.action = 'upload';
  custom.classList.add('bg-preset--upload');
  if (hasPhoto) {
    custom.setAttribute('aria-label', 'Your own image');
    custom.style.backgroundImage = `url("${bg.url}")`;
    custom.querySelector('.bg-preset-label').textContent = 'Your image';
  } else {
    custom.classList.add('bg-preset--empty');
    custom.setAttribute('aria-label', 'Use your own image');
    custom.querySelector('.bg-preset-label').textContent = 'Your image';
  }
  custom.addEventListener('click', () => onUpload?.());
  container.appendChild(custom);
}

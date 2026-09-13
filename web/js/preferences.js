import { apiFetch, getTraveler } from './api.js';
import { isMobilePortrait } from './viewport.js';

const AI_NAME_KEY = 'ai.name';
const AI_PROVIDER_KEY = 'ai.provider';
const OLLAMA_MODEL_KEY = 'ai.ollama_model';
const OPENAI_BASE_URL_KEY = 'ai.openai_base_url';
const OPENAI_API_KEY_KEY = 'ai.openai_api_key';
const OPENAI_MODEL_KEY = 'ai.openai_model';
const PLUGIN_LAYOUT_KEY = 'plugin.layout';
const DESKTOP_WORKSPACES_KEY = 'desktop.workspaces';
const DESKTOP_ACTIVE_KEY = 'desktop.active';
const DESKTOP_LAYOUT_KEY = 'desktop.layout';
const DESKTOP_WINDOWS_KEY = 'desktop.windows';
const DESKTOP_REMEMBER_KEY = 'session.remember';
const DESKTOP_SURFACE_KEY = 'desktop.surface';
const VOICE_TTS_VOICE_KEY = 'voice.tts_voice';
const VOICE_TTS_SPEED_KEY = 'voice.tts_speed';
const VOICE_SILENCE_KEY = 'voice.silence_timeout';
const VOICE_WAKE_KEY = 'voice.wake_word';
const VOICE_STT_ENGINE_KEY = 'voice.stt_engine';
const VOICE_WHISPER_MODEL_KEY = 'voice.whisper_model';
const ORB_STYLE_KEY = 'orb.style';
const DEFAULT_AI_NAME = "PEAK'D!";

function scopedKey(base) {
  const id = getTraveler()?.id;
  return id ? `${base}.${id}` : base;
}

/* ── Server persistence ────────────────────────────────────────
 * Preferences are the user's own space in the DATABASE. localStorage is only
 * a synchronous cache; every write is also flushed (debounced) to
 * /api/preferences, which stores rows keyed by (user_id, key).
 * ─────────────────────────────────────────────────────────────── */

let dirty = new Map();
let flushTimer = null;

function persist(base, rawValue) {
  dirty.set(base, rawValue);
  if (flushTimer) clearTimeout(flushTimer);
  flushTimer = setTimeout(flushPreferences, 400);
}

async function flushPreferences() {
  flushTimer = null;
  if (!dirty.size) return;
  const payload = {};
  for (const [k, v] of dirty) payload[k] = v;
  dirty = new Map();
  try {
    await apiFetch('/api/preferences', {
      method: 'PUT',
      authRedirect: false,
      body: JSON.stringify(payload),
    });
  } catch (_) {
    // Transient failure: put the keys back so the next change re-flushes
    // them (they used to be silently dropped).
    for (const [k, v] of Object.entries(payload)) {
      if (!dirty.has(k)) dirty.set(k, v);
    }
  }
}

/** Force an immediate flush (e.g. before navigating away on "Done"). */
export function flushPreferencesNow() {
  if (flushTimer) {
    clearTimeout(flushTimer);
    flushTimer = null;
  }
  return flushPreferences();
}

/** Load this user's saved preferences from the database into the local cache. */
export async function loadUserPreferences() {
  const id = getTraveler()?.id;
  if (!id) return;
  try {
    const res = await apiFetch('/api/preferences', { authRedirect: false });
    const data = res?.data || {};
    for (const [base, value] of Object.entries(data)) {
      localStorage.setItem(`${base}.${id}`, value);
    }
  } catch (_) {
    // Keep the existing local cache when the server is unreachable.
  }
  applyDesktopSurface();
  applyImmersive();
}

export function getAiName() {
  return localStorage.getItem(scopedKey(AI_NAME_KEY)) || DEFAULT_AI_NAME;
}

export function setAiName(name) {
  const trimmed = (name || '').trim();
  const key = scopedKey(AI_NAME_KEY);
  if (trimmed) localStorage.setItem(key, trimmed);
  else localStorage.removeItem(key);
  persist(AI_NAME_KEY, trimmed);
}

export function getOllamaModel() {
  return localStorage.getItem(scopedKey(OLLAMA_MODEL_KEY)) || '';
}

export function setOllamaModel(model) {
  const trimmed = (model || '').trim();
  const key = scopedKey(OLLAMA_MODEL_KEY);
  if (trimmed) localStorage.setItem(key, trimmed);
  else localStorage.removeItem(key);
  persist(OLLAMA_MODEL_KEY, trimmed);
}

/** Which AI provider answers the assistant: 'ollama' (default) or 'openai'. */
export function getAiProvider() {
  return localStorage.getItem(scopedKey(AI_PROVIDER_KEY)) || 'ollama';
}

export function setAiProvider(provider) {
  const trimmed = (provider || 'ollama').trim();
  const key = scopedKey(AI_PROVIDER_KEY);
  if (trimmed && trimmed !== 'ollama') localStorage.setItem(key, trimmed);
  else localStorage.removeItem(key);
  persist(AI_PROVIDER_KEY, trimmed === 'ollama' ? '' : trimmed);
}

export function getOpenAiBaseUrl() {
  return localStorage.getItem(scopedKey(OPENAI_BASE_URL_KEY)) || '';
}

export function setOpenAiBaseUrl(url) {
  const trimmed = (url || '').trim();
  const key = scopedKey(OPENAI_BASE_URL_KEY);
  if (trimmed) localStorage.setItem(key, trimmed);
  else localStorage.removeItem(key);
  persist(OPENAI_BASE_URL_KEY, trimmed);
}

export function getOpenAiApiKey() {
  return localStorage.getItem(scopedKey(OPENAI_API_KEY_KEY)) || '';
}

export function setOpenAiApiKey(keyValue) {
  const trimmed = (keyValue || '').trim();
  const key = scopedKey(OPENAI_API_KEY_KEY);
  if (trimmed) localStorage.setItem(key, trimmed);
  else localStorage.removeItem(key);
  persist(OPENAI_API_KEY_KEY, trimmed);
}

export function getOpenAiModel() {
  return localStorage.getItem(scopedKey(OPENAI_MODEL_KEY)) || '';
}

export function setOpenAiModel(model) {
  const trimmed = (model || '').trim();
  const key = scopedKey(OPENAI_MODEL_KEY);
  if (trimmed) localStorage.setItem(key, trimmed);
  else localStorage.removeItem(key);
  persist(OPENAI_MODEL_KEY, trimmed);
}

/**
 * "Remember my workspace" — controls only whether the desktop window
 * arrangement (workspaces, layout, fullscreen) is restored at sign-in. When
 * off, a sign-in starts with a clean desktop; plugin activation and chat are
 * the user's own and persist regardless of this switch.
 */
export function getRemember() {
  return localStorage.getItem(scopedKey(DESKTOP_REMEMBER_KEY)) === 'true';
}

export function setRemember(on) {
  const key = scopedKey(DESKTOP_REMEMBER_KEY);
  if (on) localStorage.setItem(key, 'true');
  else localStorage.removeItem(key);
  persist(DESKTOP_REMEMBER_KEY, on ? 'true' : '');
}

/**
 * Per-plugin window mode: 'tile' (right-rail tile) or 'full' (overlay
 * takeover). Stored per traveler, default 'tile'.
 */
export function getPluginLayout(name) {
  return localStorage.getItem(scopedKey(`${PLUGIN_LAYOUT_KEY}.${name}`)) || 'tile';
}

export function setPluginLayout(name, mode) {
  const key = scopedKey(`${PLUGIN_LAYOUT_KEY}.${name}`);
  if (mode === 'full') localStorage.setItem(key, 'full');
  else localStorage.removeItem(key);
  persist(`${PLUGIN_LAYOUT_KEY}.${name}`, mode === 'full' ? 'full' : '');
}

/* ── Desktop manager (workspaces + tiling layout) ───────────── */

const DEFAULT_DESKTOP_LAYOUT = {
  mode: 'master',        // 'master' | 'columns' | 'windows'
  master_ratio: 0.6,     // master fraction (0.25–0.85)
  orientation: 'left',   // 'left' | 'right' | 'top' | 'bottom'
  gap: 12,               // px between windows
};

function readJson(key, fallback) {
  try {
    const raw = localStorage.getItem(key);
    if (raw == null) return fallback;
    return JSON.parse(raw);
  } catch (_) {
    return fallback;
  }
}

/** Ordered workspace list: [{ id, windows: [pluginName] }]. */
export function getWorkspaces() {
  if (!getRemember()) return null; // fresh mode: never restore saved windows
  const ws = readJson(scopedKey(DESKTOP_WORKSPACES_KEY), null);
  return Array.isArray(ws) ? ws : null;
}

export function setWorkspaces(workspaces) {
  if (!getRemember()) return; // fresh mode: don't persist the desktop
  const raw = JSON.stringify(workspaces);
  localStorage.setItem(scopedKey(DESKTOP_WORKSPACES_KEY), raw);
  persist(DESKTOP_WORKSPACES_KEY, raw);
}

export function getActiveWorkspaceId() {
  if (!getRemember()) return null;
  return localStorage.getItem(scopedKey(DESKTOP_ACTIVE_KEY)) || null;
}

export function setActiveWorkspaceId(id) {
  if (!getRemember()) return;
  const key = scopedKey(DESKTOP_ACTIVE_KEY);
  if (id) localStorage.setItem(key, id);
  else localStorage.removeItem(key);
  persist(DESKTOP_ACTIVE_KEY, id || '');
}

/** Tiling layout config, merged over defaults. */
export function getDesktopLayout() {
  const stored = readJson(scopedKey(DESKTOP_LAYOUT_KEY), {});
  const out = { ...DEFAULT_DESKTOP_LAYOUT, ...(stored || {}) };
  out.mode = ['master', 'columns', 'windows'].includes(out.mode) ? out.mode : 'master';
  out.master_ratio = clamp(Number(out.master_ratio) || 0.6, 0.25, 0.85);
  out.orientation = ['left', 'right', 'top', 'bottom'].includes(out.orientation)
    ? out.orientation : 'left';
  out.gap = Math.round(clamp(Number(out.gap) ?? 12, 0, 40));
  // A vertical, phone-like screen has no room for a master/stack split or for
  // floating windows, so it always reads as a column — the stored choice is
  // left alone and comes back with the screen.
  if (isMobilePortrait()) out.mode = 'columns';
  return out;
}

export function setDesktopLayout(layout) {
  const raw = JSON.stringify(layout);
  localStorage.setItem(scopedKey(DESKTOP_LAYOUT_KEY), raw);
  persist(DESKTOP_LAYOUT_KEY, raw);
}

/**
 * Floating-window geometry (Windows layout mode): pluginName -> { x, y, w, h, z }.
 * Positions are stored in pixels relative to the tile grid and are only used
 * when the desktop layout is 'windows'.
 */
export function getWindowsGeom() {
  const stored = readJson(scopedKey(DESKTOP_WINDOWS_KEY), {});
  return (stored && typeof stored === 'object' && !Array.isArray(stored)) ? stored : {};
}

export function setWindowsGeom(geom) {
  const raw = JSON.stringify(geom || {});
  localStorage.setItem(scopedKey(DESKTOP_WINDOWS_KEY), raw);
  persist(DESKTOP_WINDOWS_KEY, raw);
}

/* ── Granular desktop surface (window chrome) ─────────────────
 * Fine-grained control over how every plugin window looks and feels.
 * Applied as CSS custom properties on <html> so both the desktop shell and
 * the settings page (live preview) share the same source of truth.
 * ───────────────────────────────────────────────────────────── */

const DEFAULT_DESKTOP_SURFACE = {
  window_radius: 28,     // px corner radius (matches --radius-lg default)
  window_opacity: 100,   // % window background opacity
  window_shadow: false,  // flat by default (noir hairline aesthetic)
  glass_blur: 12,        // px frosted-glass backdrop blur (panels/sheets/menus)
  title_height: 36,      // px window title bar height
};

function numOr(value, fallback) {
  const n = Number(value);
  return Number.isFinite(n) ? n : fallback;
}

/** Window-surface settings merged over defaults, each field clamped. */
export function getDesktopSurface() {
  const stored = readJson(scopedKey(DESKTOP_SURFACE_KEY), {});
  const src = (stored && typeof stored === 'object' && !Array.isArray(stored)) ? stored : {};
  return {
    window_radius: Math.round(clamp(numOr(src.window_radius, 28), 0, 32)),
    window_opacity: Math.round(clamp(numOr(src.window_opacity, 100), 60, 100)),
    window_shadow: src.window_shadow === true,
    glass_blur: Math.round(clamp(numOr(src.glass_blur, 12), 0, 24)),
    title_height: Math.round(clamp(numOr(src.title_height, 36), 28, 48)),
  };
}

export function setDesktopSurface(patch) {
  const merged = { ...getDesktopSurface(), ...patch };
  const raw = JSON.stringify(merged);
  localStorage.setItem(scopedKey(DESKTOP_SURFACE_KEY), raw);
  persist(DESKTOP_SURFACE_KEY, raw);
  applyDesktopSurface();
}

/** Write the surface settings onto :root as CSS custom properties. */
export function applyDesktopSurface() {
  const s = getDesktopSurface();
  const root = document.documentElement;
  root.style.setProperty('--window-radius', `${s.window_radius}px`);
  root.style.setProperty('--window-opacity', `${s.window_opacity}%`);
  root.style.setProperty('--window-shadow', s.window_shadow ? 'var(--shadow-glass)' : 'none');
  root.style.setProperty('--glass-blur-panel', `${s.glass_blur}px`);
  root.style.setProperty('--tile-header-height', `${s.title_height}px`);
}

/* ── Fullscreen immersion (autohide + bar position) ───────────
 * While a window is fullscreen the desktop chrome steps aside. These
 * preferences decide what steps aside, and from which edge the top bar comes
 * back: the top bar and the orb are independent, so the orb can stay up while
 * the bar hides (or nothing hides at all).
 * ───────────────────────────────────────────────────────────── */

const IMMERSIVE_KEY = 'desktop.immersive';

/** Where the fullscreen top bar docks and slides in from. */
export const BAR_POSITIONS = ['top', 'left', 'right', 'center'];

const DEFAULT_IMMERSIVE = {
  autohide_bar: true,
  bar_position: 'top',
  autohide_orb: true,
};

/** Fullscreen chrome behaviour, merged over defaults. */
export function getImmersive() {
  const stored = readJson(scopedKey(IMMERSIVE_KEY), {});
  const src = (stored && typeof stored === 'object' && !Array.isArray(stored)) ? stored : {};
  return {
    autohide_bar: src.autohide_bar !== false,
    bar_position: BAR_POSITIONS.includes(src.bar_position) ? src.bar_position : 'top',
    autohide_orb: src.autohide_orb !== false,
  };
}

export function setImmersive(patch) {
  const merged = { ...getImmersive(), ...patch };
  const raw = JSON.stringify(merged);
  localStorage.setItem(scopedKey(IMMERSIVE_KEY), raw);
  persist(IMMERSIVE_KEY, raw);
  applyImmersive();
}

/** Publish the immersion settings to CSS and the fullscreen controller. */
export function applyImmersive() {
  const s = getImmersive();
  const root = document.documentElement;
  root.dataset.barPos = s.bar_position;
  root.dataset.autohideBar = s.autohide_bar ? '1' : '0';
  root.dataset.autohideOrb = s.autohide_orb ? '1' : '0';
  window.dispatchEvent(new CustomEvent('desktop:immersive', { detail: s }));
}

/* ── Voice (per-user, server-backed) ───────────────────────── */

export function getTtsVoice() {
  return localStorage.getItem(scopedKey(VOICE_TTS_VOICE_KEY)) || '';
}

export function setTtsVoice(voice) {
  const trimmed = (voice || '').trim();
  const key = scopedKey(VOICE_TTS_VOICE_KEY);
  if (trimmed) localStorage.setItem(key, trimmed);
  else localStorage.removeItem(key);
  persist(VOICE_TTS_VOICE_KEY, trimmed);
}

export function getTtsSpeed() {
  const n = Number(localStorage.getItem(scopedKey(VOICE_TTS_SPEED_KEY)));
  return Number.isFinite(n) && n > 0 ? clamp(n, 0.7, 2.0) : 1.0;
}

export function setTtsSpeed(speed) {
  const n = clamp(numOr(speed, 1.0), 0.7, 2.0);
  localStorage.setItem(scopedKey(VOICE_TTS_SPEED_KEY), String(n));
  persist(VOICE_TTS_SPEED_KEY, String(n));
}

export function getSilenceTimeout() {
  const n = Number(localStorage.getItem(scopedKey(VOICE_SILENCE_KEY)));
  return Number.isFinite(n) && n >= 3000 ? Math.round(clamp(n, 3000, 30000)) : 8000;
}

export function setSilenceTimeout(ms) {
  const n = Math.round(clamp(numOr(ms, 8000), 3000, 30000));
  localStorage.setItem(scopedKey(VOICE_SILENCE_KEY), String(n));
  persist(VOICE_SILENCE_KEY, String(n));
}

export function getWakeWord() {
  return localStorage.getItem(scopedKey(VOICE_WAKE_KEY)) !== 'false';
}

export function setWakeWord(on) {
  const key = scopedKey(VOICE_WAKE_KEY);
  if (on) localStorage.removeItem(key); // default true
  else localStorage.setItem(key, 'false');
  persist(VOICE_WAKE_KEY, on ? 'true' : 'false');
}

/* ── Speech recognition engine (per-user, server-backed) ──── */

/** Which STT engine voice input uses: 'whisper' (default) or 'vosk'. */
export function getSttEngine() {
  return localStorage.getItem(scopedKey(VOICE_STT_ENGINE_KEY)) === 'vosk' ? 'vosk' : 'whisper';
}

export function setSttEngine(engine) {
  const value = engine === 'vosk' ? 'vosk' : 'whisper';
  const key = scopedKey(VOICE_STT_ENGINE_KEY);
  if (value === 'whisper') localStorage.removeItem(key); // default is implicit
  else localStorage.setItem(key, value);
  persist(VOICE_STT_ENGINE_KEY, value === 'whisper' ? '' : value);
}

/**
 * faster-whisper model size. `tiny` is bundled with the app; `small` is a
 * ~480 MB opt-in download.
 */
export function getWhisperModel() {
  return localStorage.getItem(scopedKey(VOICE_WHISPER_MODEL_KEY)) === 'small' ? 'small' : 'tiny';
}

export function setWhisperModel(model) {
  const value = model === 'small' ? 'small' : 'tiny';
  const key = scopedKey(VOICE_WHISPER_MODEL_KEY);
  if (value === 'tiny') localStorage.removeItem(key);
  else localStorage.setItem(key, value);
  persist(VOICE_WHISPER_MODEL_KEY, value === 'tiny' ? '' : value);
}

/* ── Voice orb style (per-user, server-backed) ─────────────── */

/**
 * The five orb looks. The ids are the contract with orbCanvas.js; every one is
 * a variant of the reference ring kept in `web/orbs/` (see that folder's
 * README).
 */
export const ORB_STYLES = [
  { id: 'ripple', label: 'Ripple', hint: 'Echoes spreading outward' },
  { id: 'corona', label: 'Corona', hint: 'A hairline ring with fine sparks' },
  { id: 'halo', label: 'Halo', hint: 'A ring that ripples with your voice' },
  { id: 'flare', label: 'Flare', hint: 'A crown of long spectral rays' },
  { id: 'aura', label: 'Aura', hint: 'Two rings twisting in and out' },
];

export const DEFAULT_ORB_STYLE = 'ripple';

const ORB_STYLE_IDS = new Set(ORB_STYLES.map((s) => s.id));

/**
 * Older stored ids still resolve. Everything that is gone was derived from the
 * ring, so it falls back to the plain ring rather than to a surprise.
 * (`ripple` is a current id now, so an old first-generation `ripple` resolves
 * to today's ripple.)
 */
const ORB_STYLE_LEGACY = {
  // Previous generation.
  nucleus: 'corona',
  nebula: 'corona',
  torus: 'corona',
  prism: 'corona',
  eclipse: 'corona',
  // Earlier generations.
  filament: 'corona',
  bubble: 'corona',
  marble: 'corona',
  orbit: 'corona',
  grid: 'corona',
  fluid: 'corona',
  pulse: 'corona',
};

export function getOrbStyle() {
  const stored = localStorage.getItem(scopedKey(ORB_STYLE_KEY));
  if (ORB_STYLE_IDS.has(stored)) return stored;
  if (ORB_STYLE_LEGACY[stored]) return ORB_STYLE_LEGACY[stored];
  return DEFAULT_ORB_STYLE;
}

export function setOrbStyle(id) {
  const style = ORB_STYLE_IDS.has(id) ? id : DEFAULT_ORB_STYLE;
  const key = scopedKey(ORB_STYLE_KEY);
  if (style === DEFAULT_ORB_STYLE) localStorage.removeItem(key); // default is implicit
  else localStorage.setItem(key, style);
  persist(ORB_STYLE_KEY, style === DEFAULT_ORB_STYLE ? '' : style);
}

function clamp(n, min, max) {
  return Math.min(max, Math.max(min, n));
}

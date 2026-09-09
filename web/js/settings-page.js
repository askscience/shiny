// Settings page controller — standalone page (no modal), built on the UI library.
// Same preferences as before: profile, appearance, assistant, voice.

import { apiFetch, getVoiceLang, getVoiceLangExplicit, setVoiceLang, clearVoiceLang, getTraveler, getToken, logoutSession, validateSession } from './api.js';
import {
  initThemeLoader, initAppearance, hydrateIcons, toast,
  listThemes, setTheme, getActiveTheme, getThemeManifest,
  applyAppearance, getAccent, setAccent, getGradient, setGradient,
  accentPresets, gradientPresets, gradientToCss,
} from '../ui/index.js';
import {
  getAiName, setAiName, getAiProvider, setAiProvider,
  getOllamaModel, setOllamaModel,
  getOpenAiBaseUrl, setOpenAiBaseUrl, getOpenAiApiKey, setOpenAiApiKey,
  getOpenAiModel, setOpenAiModel,
  getPluginLayout, setPluginLayout, getDesktopLayout, setDesktopLayout,
  loadUserPreferences, getRemember, setRemember, flushPreferencesNow,
  getDesktopSurface, setDesktopSurface,
  getTtsVoice, setTtsVoice, getTtsSpeed, setTtsSpeed,
  getSilenceTimeout, setSilenceTimeout, getWakeWord, setWakeWord,
} from './preferences.js';
import { saveKnownUser, renderAvatarEl, readAvatarFile } from './userProfiles.js';
import { initBackground, getBackground, setBackground } from './background.js';

const langSelect = document.getElementById('lang-select');
const doneBtn = document.getElementById('settings-done');
const logoutBtn = document.getElementById('settings-logout');
const themeSelect = document.getElementById('theme-select');
const accentSwatches = document.getElementById('accent-swatches');
const gradientSwatches = document.getElementById('gradient-swatches');
const gradientStopA = document.getElementById('gradient-stop-a');
const gradientStopB = document.getElementById('gradient-stop-b');
const gradientAngle = document.getElementById('gradient-angle');
const aiNameInput = document.getElementById('ai-name-input');
const aiNameHint = document.getElementById('ai-name-hint');
const profileNameInput = document.getElementById('profile-name-input');
const profileAvatarInput = document.getElementById('profile-avatar-input');
const profileAvatarPreview = document.getElementById('profile-avatar-preview');
const ollamaModelSelect = document.getElementById('ollama-model-select');
const ollamaModelHint = document.getElementById('ollama-model-hint');
const aiProviderSelect = document.getElementById('ai-provider-select');
const ollamaProviderFields = document.getElementById('ollama-provider-fields');
const openaiProviderFields = document.getElementById('openai-provider-fields');
const openaiBaseUrlInput = document.getElementById('openai-base-url-input');
const openaiApiKeyInput = document.getElementById('openai-api-key-input');
const openaiModelInput = document.getElementById('openai-model-input');
const openaiModelHint = document.getElementById('openai-model-hint');
const userAvatarEl = document.getElementById('settings-user-avatar');
const userNameEl = document.getElementById('settings-user-name');

const rememberToggle = document.getElementById('remember-toggle');
const bgModeSelect = document.getElementById('background-mode');
const bgGradientNote = document.getElementById('background-gradient-note');
const bgImageControls = document.getElementById('background-image-controls');
const bgImagePick = document.getElementById('background-image-pick');
const bgImageRemove = document.getElementById('background-image-remove');
const bgImageInput = document.getElementById('background-image-input');
const bgImagePreview = document.getElementById('background-image-preview');
const bgImageHint = document.getElementById('background-image-hint');
const bgAnimControls = document.getElementById('background-anim-controls');
const bgAnimSelect = document.getElementById('background-animation');

let pendingAvatar = undefined;
let serverDefaultModel = '';

/* ── Appearance ─────────────────────────────────────────────── */

function markActive(container, isActive) {
  container?.querySelectorAll('.swatch').forEach((el) => {
    el.classList.toggle('is-active', isActive(el));
  });
}

function pickAccent(hex) {
  setAccent(hex);
  applyAppearance();
  markActive(accentSwatches, (el) => el.dataset.accent === hex);
}

function pickGradient(gradient) {
  setGradient(gradient);
  applyAppearance();
  markActive(gradientSwatches, (el) => el.dataset.gradientId === gradient.id);
}

function buildAccentSwatches() {
  if (!accentSwatches) return;
  accentSwatches.textContent = '';
  const current = getAccent();
  const presets = accentPresets();

  for (const preset of presets) {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'swatch';
    b.dataset.accent = preset.value;
    b.style.background = preset.value;
    b.title = preset.label;
    b.setAttribute('role', 'radio');
    b.setAttribute('aria-label', preset.label);
    b.addEventListener('click', () => pickAccent(preset.value));
    accentSwatches.appendChild(b);
  }

  const custom = document.createElement('label');
  custom.className = 'swatch swatch--custom';
  custom.dataset.accent = 'custom';
  custom.title = 'Custom accent';
  const plus = document.createElement('span');
  plus.textContent = '+';
  const color = document.createElement('input');
  color.type = 'color';
  color.id = 'accent-custom';
  color.value = presets.some((p) => p.value === current) ? presets[0]?.value || '#ffffff' : current;
  color.setAttribute('aria-label', 'Custom accent color');
  color.addEventListener('input', () => pickAccent(color.value));
  custom.append(plus, color);
  accentSwatches.appendChild(custom);

  const isPreset = presets.some((p) => p.value === current);
  markActive(accentSwatches, (el) =>
    el.dataset.accent === current || (el.dataset.accent === 'custom' && !isPreset));
}

function buildGradientSwatches() {
  if (!gradientSwatches) return;
  gradientSwatches.textContent = '';
  const current = getGradient();
  const presets = gradientPresets();

  for (const preset of presets) {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'swatch swatch--gradient';
    b.dataset.gradientId = preset.id;
    b.style.background = gradientToCss(preset);
    b.title = preset.label;
    b.setAttribute('role', 'radio');
    b.setAttribute('aria-label', preset.label);
    b.addEventListener('click', () => pickGradient({ id: preset.id, angle: preset.angle, stops: preset.stops }));
    gradientSwatches.appendChild(b);
  }

  markActive(gradientSwatches, (el) => el.dataset.gradientId === current.id);
}

function customGradientFromInputs() {
  return {
    id: 'custom',
    stops: [gradientStopA?.value || '#ffffff', gradientStopB?.value || '#8a8a8a'],
    angle: Number(gradientAngle?.value ?? 135),
  };
}

function syncAppearanceUI() {
  buildAccentSwatches();
  buildGradientSwatches();

  const g = getGradient();
  if (g.id === 'custom' && g.stops?.length >= 2) {
    if (gradientStopA) gradientStopA.value = g.stops[0];
    if (gradientStopB) gradientStopB.value = g.stops[1];
    if (gradientAngle && Number.isFinite(g.angle)) gradientAngle.value = String(g.angle);
  }

  void listThemes().then((themes) => {
    if (!themeSelect) return;
    themeSelect.textContent = '';
    for (const name of themes) {
      const opt = document.createElement('option');
      opt.value = name;
      opt.textContent = name === getActiveTheme()
        ? (getThemeManifest()?.label || name)
        : name.charAt(0).toUpperCase() + name.slice(1);
      themeSelect.appendChild(opt);
    }
    themeSelect.value = getActiveTheme();
  });
}

function wireAppearance() {
  themeSelect?.addEventListener('change', async () => {
    await setTheme(themeSelect.value);
    buildAccentSwatches();
    buildGradientSwatches();
  });

  const applyCustomGradient = () => pickGradient(customGradientFromInputs());
  gradientStopA?.addEventListener('input', applyCustomGradient);
  gradientStopB?.addEventListener('input', applyCustomGradient);
  gradientAngle?.addEventListener('input', applyCustomGradient);
}

/* ── Profile / Assistant / Voice ────────────────────────────── */

function updateAiNameHint() {
  if (aiNameHint) aiNameHint.textContent = getAiName();
}

function renderUser() {
  const u = getTraveler();
  const name = u?.name || u?.username || 'Account';
  if (userNameEl) userNameEl.textContent = name;
  renderAvatarEl(userAvatarEl, { name, avatar: u?.avatar || null });
}

function loadProfileFields() {
  const traveler = getTraveler();
  if (profileNameInput) profileNameInput.value = traveler?.name || '';
  pendingAvatar = undefined;
  renderAvatarEl(profileAvatarPreview, {
    name: traveler?.name || traveler?.username || '',
    avatar: traveler?.avatar || null,
  });
}

function populateOllamaModelSelect(models, selected) {
  if (!ollamaModelSelect) return;
  ollamaModelSelect.innerHTML = '';

  const defaultOpt = document.createElement('option');
  defaultOpt.value = '';
  defaultOpt.textContent = serverDefaultModel
    ? `Default (${serverDefaultModel})`
    : 'Default (server)';
  ollamaModelSelect.appendChild(defaultOpt);

  models.forEach((name) => {
    const opt = document.createElement('option');
    opt.value = name;
    opt.textContent = name;
    ollamaModelSelect.appendChild(opt);
  });

  const stored = selected ?? getOllamaModel();
  if (stored && [...ollamaModelSelect.options].some((o) => o.value === stored)) {
    ollamaModelSelect.value = stored;
  } else {
    ollamaModelSelect.value = '';
  }
}

async function loadAiModels() {
  if (!ollamaModelSelect) return;
  try {
    const res = await apiFetch('/api/ai/models');
    const { provider, models = [], default: defaultModel, available } = res.data || {};
    serverDefaultModel = defaultModel || '';
    populateOllamaModelSelect(models);

    if (ollamaModelHint) {
      if (provider === 'openai') {
        ollamaModelHint.textContent = '';
      } else if (!available) {
        ollamaModelHint.textContent = 'Ollama is offline — using the server default model.';
      } else if (!models.length) {
        ollamaModelHint.textContent = 'No models found in Ollama. Pull one with: ollama pull llama3.2';
      } else {
        ollamaModelHint.textContent = 'Models from your local Ollama server.';
      }
    }

    if (openaiModelHint) {
      if (provider === 'openai') {
        openaiModelHint.textContent = available
          ? (models.length ? `${models.length} model(s) found at the endpoint.` : 'Endpoint reachable.')
          : 'Could not reach the endpoint yet — check the base URL and API key.';
      }
    }
  } catch (e) {
    populateOllamaModelSelect([]);
    if (ollamaModelHint) {
      ollamaModelHint.textContent = e.status === 404
        ? 'Model list unavailable — restart the server, then refresh the page.'
        : 'Could not load models — using the server default.';
    }
  }
}

function syncProviderUI() {
  const provider = getAiProvider();
  const isOpenAi = provider === 'openai';
  if (aiProviderSelect) aiProviderSelect.value = provider;
  ollamaProviderFields?.classList.toggle('hidden', isOpenAi);
  openaiProviderFields?.classList.toggle('hidden', !isOpenAi);
  if (openaiBaseUrlInput) openaiBaseUrlInput.value = getOpenAiBaseUrl();
  if (openaiApiKeyInput) openaiApiKeyInput.value = getOpenAiApiKey();
  if (openaiModelInput) openaiModelInput.value = getOpenAiModel();
}

async function loadLanguages() {
  try {
    const res = await apiFetch('/api/voice/languages');
    langSelect.innerHTML = '';

    // "Automatic" = follow the browser language (default until the user
    // picks a specific one). Selecting a concrete language pins both voice
    // (STT/TTS) and the assistant's reply language.
    const autoOpt = document.createElement('option');
    autoOpt.value = 'auto';
    autoOpt.textContent = 'Automatic (browser)';
    langSelect.appendChild(autoOpt);

    res.data.forEach((lang) => {
      const opt = document.createElement('option');
      opt.value = lang.code;
      opt.textContent = `${lang.code.toUpperCase()}${lang.vosk_available ? '' : ' (TTS only)'}`;
      langSelect.appendChild(opt);
    });

    const explicit = getVoiceLangExplicit();
    langSelect.value = explicit || 'auto';
  } catch (_) {
    langSelect.innerHTML = '<option value="auto">Automatic (browser)</option><option value="en">EN</option>';
  }
}

/* ── Plugin windows (tile vs full screen) ───────────────────── */

async function loadPluginLayouts() {
  const list = document.getElementById('plugin-layout-list');
  if (!list) return;
  try {
    const [all, activeRes] = await Promise.all([
      apiFetch('/api/plugins'),
      apiFetch('/api/plugins/active'),
    ]);
    const active = new Set(activeRes?.data || []);
    // The keyboard plugin is bottom chrome, not a window — no layout mode.
    const plugins = (all?.data || []).filter((p) => active.has(p.name) && p.name !== 'keyboard');
    list.innerHTML = '';
    if (!plugins.length) {
      const hint = document.createElement('p');
      hint.className = 'settings-hint';
      hint.textContent = 'No plugins active — activate one on the Plugins page.';
      list.appendChild(hint);
      return;
    }
    for (const p of plugins) {
      const row = document.createElement('div');
      row.className = 'plugin-layout-row';

      const name = document.createElement('span');
      name.className = 'plugin-layout-name';
      name.textContent = p.name.charAt(0).toUpperCase() + p.name.slice(1);
      name.title = p.description || p.summary || '';

      const wrap = document.createElement('div');
      wrap.className = 'ui-select-wrap';
      const sel = document.createElement('select');
      sel.className = 'ui-select';
      for (const [value, text] of [['tile', 'Tile'], ['full', 'Full screen']]) {
        const opt = document.createElement('option');
        opt.value = value;
        opt.textContent = text;
        sel.appendChild(opt);
      }
      sel.value = getPluginLayout(p.name);
      sel.addEventListener('change', () => {
        setPluginLayout(p.name, sel.value);
        toast(`${name.textContent}: opens as ${sel.value === 'full' ? 'full screen' : 'tile'}`, { type: 'info' });
      });
      wrap.appendChild(sel);

      row.append(name, wrap);
      list.appendChild(row);
    }
  } catch (_) {
    list.innerHTML = '';
    const hint = document.createElement('p');
    hint.className = 'settings-hint';
    hint.textContent = 'Could not load plugins.';
    list.appendChild(hint);
  }
}

/* ── Desktop / tiling ───────────────────────────────────────── */

function wireDesktopSection() {
  const modeSel = document.getElementById('desktop-layout-mode');
  const oriSel = document.getElementById('desktop-orientation');
  const ratio = document.getElementById('desktop-master-ratio');
  const ratioVal = document.getElementById('desktop-ratio-value');
  const gap = document.getElementById('desktop-gap');
  const gapVal = document.getElementById('desktop-gap-value');
  const tilingControls = document.getElementById('desktop-tiling-controls');

  const layout = getDesktopLayout();
  if (modeSel) modeSel.value = layout.mode;
  if (oriSel) oriSel.value = layout.orientation;
  if (ratio) {
    const pct = Math.round(layout.master_ratio * 100);
    ratio.value = String(pct);
    if (ratioVal) ratioVal.textContent = `${pct}%`;
  }
  if (gap) {
    gap.value = String(layout.gap);
    if (gapVal) gapVal.textContent = `${layout.gap}px`;
  }

  // The master/stack controls only apply to tiling layouts — hide them for
  // the floating "Windows" desktop experience.
  const syncDesktopControls = () => {
    const isWindows = modeSel?.value === 'windows';
    tilingControls?.classList.toggle('hidden', isWindows);
  };
  syncDesktopControls();

  modeSel?.addEventListener('change', () => {
    setDesktopLayout({ ...getDesktopLayout(), mode: modeSel.value });
    syncDesktopControls();
  });
  oriSel?.addEventListener('change', () =>
    setDesktopLayout({ ...getDesktopLayout(), orientation: oriSel.value }));
  ratio?.addEventListener('input', () => {
    const pct = Number(ratio.value);
    if (ratioVal) ratioVal.textContent = `${pct}%`;
    setDesktopLayout({ ...getDesktopLayout(), master_ratio: pct / 100 });
  });
  gap?.addEventListener('input', () => {
    const px = Number(gap.value);
    if (gapVal) gapVal.textContent = `${px}px`;
    setDesktopLayout({ ...getDesktopLayout(), gap: px });
  });

  // Title bar height — applies to every layout mode (not just tiling).
  const title = document.getElementById('desktop-title-height');
  const titleVal = document.getElementById('desktop-title-value');
  const titlePx = getDesktopSurface().title_height;
  if (title) title.value = String(titlePx);
  if (titleVal) titleVal.textContent = `${titlePx}px`;
  title?.addEventListener('input', () => {
    const px = Number(title.value);
    if (titleVal) titleVal.textContent = `${px}px`;
    setDesktopSurface({ title_height: px });
  });
}

/* ── Granular desktop surface (appearance → window chrome) ── */

function syncSurfaceUI() {
  const s = getDesktopSurface();
  const radius = document.getElementById('surface-radius');
  const radiusVal = document.getElementById('surface-radius-value');
  const opacity = document.getElementById('surface-opacity');
  const opacityVal = document.getElementById('surface-opacity-value');
  const blur = document.getElementById('surface-blur');
  const blurVal = document.getElementById('surface-blur-value');
  const shadow = document.getElementById('surface-shadow-toggle');

  if (radius) {
    radius.value = String(s.window_radius);
    if (radiusVal) radiusVal.textContent = `${s.window_radius}px`;
  }
  if (opacity) {
    opacity.value = String(s.window_opacity);
    if (opacityVal) opacityVal.textContent = `${s.window_opacity}%`;
  }
  if (blur) {
    blur.value = String(s.glass_blur);
    if (blurVal) blurVal.textContent = `${s.glass_blur}px`;
  }
  if (shadow) shadow.setAttribute('aria-checked', String(s.window_shadow));
}

function wireSurfaceSection() {
  const radius = document.getElementById('surface-radius');
  const radiusVal = document.getElementById('surface-radius-value');
  const opacity = document.getElementById('surface-opacity');
  const opacityVal = document.getElementById('surface-opacity-value');
  const blur = document.getElementById('surface-blur');
  const blurVal = document.getElementById('surface-blur-value');
  const shadow = document.getElementById('surface-shadow-toggle');

  syncSurfaceUI();

  radius?.addEventListener('input', () => {
    const px = Number(radius.value);
    if (radiusVal) radiusVal.textContent = `${px}px`;
    setDesktopSurface({ window_radius: px });
  });
  opacity?.addEventListener('input', () => {
    const pct = Number(opacity.value);
    if (opacityVal) opacityVal.textContent = `${pct}%`;
    setDesktopSurface({ window_opacity: pct });
  });
  blur?.addEventListener('input', () => {
    const px = Number(blur.value);
    if (blurVal) blurVal.textContent = `${px}px`;
    setDesktopSurface({ glass_blur: px });
  });
  shadow?.addEventListener('click', () => {
    const next = !getDesktopSurface().window_shadow;
    setDesktopSurface({ window_shadow: next });
    shadow.setAttribute('aria-checked', String(next));
  });
}

/* ── Voice extras (TTS voice, speed, wake word, silence) ──── */

function syncVoiceExtras() {
  const voiceSel = document.getElementById('tts-voice-select');
  const speed = document.getElementById('tts-speed');
  const speedVal = document.getElementById('tts-speed-value');
  const silenceSel = document.getElementById('silence-select');
  const wakeToggle = document.getElementById('wake-toggle');

  if (voiceSel) voiceSel.value = getTtsVoice() || '';
  if (speed) {
    const v = getTtsSpeed();
    speed.value = String(v);
    if (speedVal) speedVal.textContent = `${v.toFixed(1)}×`;
  }
  if (silenceSel) {
    const ms = getSilenceTimeout();
    if ([...silenceSel.options].some((o) => Number(o.value) === ms)) {
      silenceSel.value = String(ms);
    }
  }
  if (wakeToggle) wakeToggle.setAttribute('aria-checked', String(getWakeWord()));
}

function wireVoiceExtras() {
  const voiceSel = document.getElementById('tts-voice-select');
  const speed = document.getElementById('tts-speed');
  const speedVal = document.getElementById('tts-speed-value');
  const silenceSel = document.getElementById('silence-select');
  const wakeToggle = document.getElementById('wake-toggle');

  syncVoiceExtras();

  voiceSel?.addEventListener('change', () => setTtsVoice(voiceSel.value));
  speed?.addEventListener('input', () => {
    const v = Number(speed.value);
    if (speedVal) speedVal.textContent = `${v.toFixed(1)}×`;
    setTtsSpeed(v);
  });
  silenceSel?.addEventListener('change', () => setSilenceTimeout(Number(silenceSel.value)));
  wakeToggle?.addEventListener('click', () => {
    const next = !getWakeWord();
    setWakeWord(next);
    wakeToggle.setAttribute('aria-checked', String(next));
  });
}

/* ── Control panel (nav + background + session) ────────────── */

function wireNav() {
  const items = [...document.querySelectorAll('.settings-nav-item')];
  const panels = [...document.querySelectorAll('.settings-panel')];
  const show = (id) => {
    items.forEach((it) => it.classList.toggle('is-active', it.dataset.panelTarget === id));
    panels.forEach((p) => p.classList.toggle('is-active', p.dataset.panel === id));
  };
  items.forEach((it) => it.addEventListener('click', () => show(it.dataset.panelTarget)));
}

function syncBackgroundUI() {
  const bg = getBackground();
  if (bgModeSelect) bgModeSelect.value = bg.mode || 'none';
  bgGradientNote?.classList.toggle('hidden', bg.mode !== 'gradient');
  bgImageControls?.classList.toggle('hidden', bg.mode !== 'image');
  bgAnimControls?.classList.toggle('hidden', bg.mode !== 'animated');
  if (bgAnimSelect) bgAnimSelect.value = bg.animation || 'aurora';

  if (bgImagePreview) {
    if (bg.mode === 'image') {
      bgImagePreview.classList.remove('hidden');
      bgImagePreview.style.backgroundImage = `url("/api/background?v=${Date.now()}")`;
      if (bgImageHint) bgImageHint.textContent = 'Your photo is dimmed so the interface stays readable.';
    } else {
      bgImagePreview.classList.add('hidden');
      if (bgImageHint) bgImageHint.textContent = '';
    }
  }
}

async function uploadBackground(file) {
  if (!file) return;
  if (!file.type?.startsWith('image/')) {
    toast('Choose an image file', { type: 'error' });
    return;
  }
  if (file.size > 12 * 1024 * 1024) {
    toast('Image must be under 12 MB', { type: 'error' });
    return;
  }
  try {
    const form = new FormData();
    form.append('file', file);
    await apiFetch('/api/background', { method: 'POST', body: form });
    // One-time cache-buster lives in the stored URL so the fresh photo shows,
    // but later background re-applies reuse this stable URL (no flicker).
    setBackground({ mode: 'image', url: `/api/background?v=${Date.now()}` });
    syncBackgroundUI();
    toast('Background image updated', { type: 'info' });
  } catch (e) {
    toast(e.message || 'Could not upload image', { type: 'error' });
  }
}

async function removeBackgroundImage() {
  try {
    await apiFetch('/api/background', { method: 'DELETE' });
  } catch (_) { /* 404 or transient — proceed */ }
  setBackground({ mode: 'none', url: null });
  syncBackgroundUI();
  toast('Background image removed', { type: 'info' });
}

function wireBackground() {
  syncBackgroundUI();

  bgModeSelect?.addEventListener('change', () => {
    setBackground({ mode: bgModeSelect.value });
    syncBackgroundUI();
  });

  bgImagePick?.addEventListener('click', () => bgImageInput?.click());
  bgImageInput?.addEventListener('change', () => {
    const file = bgImageInput.files?.[0];
    if (file) uploadBackground(file);
    bgImageInput.value = '';
  });
  bgImageRemove?.addEventListener('click', removeBackgroundImage);

  bgAnimSelect?.addEventListener('change', () => {
    setBackground({ animation: bgAnimSelect.value });
  });
}

function wireSession() {
  if (!rememberToggle) return;
  const sync = () => rememberToggle.setAttribute('aria-checked', String(getRemember()));
  sync();
  rememberToggle.addEventListener('click', () => {
    setRemember(!getRemember());
    sync();
  });
}

/* ── Actions ────────────────────────────────────────────────── */

async function saveAndLeave() {
  setAiName(aiNameInput?.value || '');
  setAiProvider(aiProviderSelect?.value || 'ollama');
  setOllamaModel(ollamaModelSelect?.value || '');
  setOpenAiBaseUrl(openaiBaseUrlInput?.value || '');
  setOpenAiApiKey(openaiApiKeyInput?.value || '');
  setOpenAiModel(openaiModelInput?.value || '');
  if (langSelect) {
    // Persisted here; the sphere re-prepares voice on next load. "auto" keeps
    // following the browser language, a concrete code pins it per user.
    if (langSelect.value === 'auto') clearVoiceLang();
    else if (langSelect.value !== getVoiceLang()) setVoiceLang(langSelect.value);
  }

  const name = profileNameInput?.value.trim();
  const traveler = getTraveler();
  const body = {};
  if (name && name !== traveler?.name) body.name = name;
  if (pendingAvatar !== undefined) body.avatar = pendingAvatar;

  if (Object.keys(body).length) {
    try {
      const res = await apiFetch('/api/travelers/me', {
        method: 'PUT',
        body: JSON.stringify(body),
      });
      if (res?.data) {
        localStorage.setItem('traveler', JSON.stringify(res.data));
        saveKnownUser(res.data);
      }
    } catch (e) {
      toast(e.message || 'Could not save profile', { type: 'error' });
      return; // stay on the page so the user can retry
    }
  }

  // Flush any debounced preference writes (e.g. the "remember" toggle) to the
  // server before leaving, so the next boot reads the fresh values.
  await flushPreferencesNow();

  window.location.href = '/';
}

async function logout() {
  await logoutSession();
  window.location.href = '/';
}

/* ── Boot ───────────────────────────────────────────────────── */

async function boot() {
  await initThemeLoader();
  initAppearance({ getScope: () => getTraveler()?.id });
  initBackground({ getScope: () => getTraveler()?.id });
  hydrateIcons();

  if (!(await validateSession())) {
    window.location.href = '/';
    return;
  }

  await loadUserPreferences();
  renderUser();
  wireAppearance();
  syncAppearanceUI();
  loadProfileFields();
  updateAiNameHint();
  if (aiNameInput) aiNameInput.value = getAiName();
  syncProviderUI();

  await Promise.all([loadLanguages(), loadAiModels(), loadPluginLayouts()]);
  wireDesktopSection();
  wireSurfaceSection();
  wireVoiceExtras();
  wireNav();
  wireBackground();
  wireSession();

  aiNameInput?.addEventListener('input', () => {
    setAiName(aiNameInput.value);
    updateAiNameHint();
  });
  ollamaModelSelect?.addEventListener('change', async () => {
    setOllamaModel(ollamaModelSelect.value);
    // Persist immediately so the model switch takes effect even if the user
    // leaves the page without pressing "Done" (avoids the debounce race where
    // the old default would still be read on the next agent request).
    await flushPreferencesNow();
  });
  aiProviderSelect?.addEventListener('change', async () => {
    setAiProvider(aiProviderSelect.value);
    syncProviderUI();
    // The model list is resolved server-side from the stored provider, so
    // flush the change before fetching — otherwise a switch back to Ollama
    // would still be read as the previous provider (debounce lag).
    await flushPreferencesNow();
    loadAiModels();
  });

  profileAvatarInput?.addEventListener('change', async () => {
    const file = profileAvatarInput.files?.[0];
    if (!file) return;
    try {
      pendingAvatar = await readAvatarFile(file);
      renderAvatarEl(profileAvatarPreview, {
        name: profileNameInput?.value || getTraveler()?.name || '',
        avatar: pendingAvatar,
      });
    } catch (e) {
      toast(e.message, { type: 'error' });
      profileAvatarInput.value = '';
    }
  });

  doneBtn?.addEventListener('click', saveAndLeave);
  logoutBtn?.addEventListener('click', logout);
}

boot();

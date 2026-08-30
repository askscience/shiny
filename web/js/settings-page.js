// Settings page controller — standalone page (no modal), built on the UI library.
// Same preferences as before: profile, appearance, assistant, voice.

import { apiFetch, getVoiceLang, setVoiceLang, getTraveler, getToken, clearAuth, validateSession } from './api.js';
import {
  initThemeLoader, initAppearance, hydrateIcons, toast,
  listThemes, setTheme, getActiveTheme, getThemeManifest,
  applyAppearance, getAccent, setAccent, getGradient, setGradient,
  accentPresets, gradientPresets, gradientToCss,
} from '../ui/index.js';
import { getAiName, setAiName, getOllamaModel, setOllamaModel, getPluginLayout, setPluginLayout, getDesktopLayout, setDesktopLayout, loadUserPreferences } from './preferences.js';
import { saveKnownUser, renderAvatarEl, readAvatarFile } from './userProfiles.js';

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
const userAvatarEl = document.getElementById('settings-user-avatar');
const userNameEl = document.getElementById('settings-user-name');

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

async function loadOllamaModels() {
  if (!ollamaModelSelect) return;
  try {
    const res = await apiFetch('/api/ollama/models');
    const { models = [], default: defaultModel, available } = res.data || {};
    serverDefaultModel = defaultModel || '';
    populateOllamaModelSelect(models);

    if (ollamaModelHint) {
      if (!available) {
        ollamaModelHint.textContent = 'Ollama is offline — using the server default model.';
      } else if (!models.length) {
        ollamaModelHint.textContent = 'No models found in Ollama. Pull one with: ollama pull llama3.2';
      } else {
        ollamaModelHint.textContent = 'Models from your local Ollama server.';
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

async function loadLanguages() {
  try {
    const res = await apiFetch('/api/voice/languages');
    langSelect.innerHTML = '';
    res.data.forEach((lang) => {
      const opt = document.createElement('option');
      opt.value = lang.code;
      opt.textContent = `${lang.code.toUpperCase()}${lang.vosk_available ? '' : ' (TTS only)'}`;
      langSelect.appendChild(opt);
    });
    langSelect.value = getVoiceLang();
  } catch (_) {
    langSelect.innerHTML = '<option value="en">EN</option>';
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

  modeSel?.addEventListener('change', () =>
    setDesktopLayout({ ...getDesktopLayout(), mode: modeSel.value }));
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
}

/* ── Actions ────────────────────────────────────────────────── */

async function saveAndLeave() {
  setAiName(aiNameInput?.value || '');
  setOllamaModel(ollamaModelSelect?.value || '');
  if (langSelect && langSelect.value !== getVoiceLang()) {
    // Persisted here; the sphere re-prepares voice on next load.
    setVoiceLang(langSelect.value);
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

  window.location.href = '/';
}

function logout() {
  clearAuth();
  window.location.href = '/';
}

/* ── Boot ───────────────────────────────────────────────────── */

async function boot() {
  await initThemeLoader();
  initAppearance({ getScope: () => getTraveler()?.id });
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

  await Promise.all([loadLanguages(), loadOllamaModels(), loadPluginLayouts()]);
  wireDesktopSection();

  aiNameInput?.addEventListener('input', () => {
    setAiName(aiNameInput.value);
    updateAiNameHint();
  });
  ollamaModelSelect?.addEventListener('change', () => {
    setOllamaModel(ollamaModelSelect.value);
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

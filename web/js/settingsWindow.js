/**
 * settingsWindow.js — the Settings window (built-in desktop surface).
 *
 * A grouped control panel that lives in a tile like any plugin window. The
 * sections are fewer and broader than the old standalone page:
 *
 *   Account    — photo + display name
 *   Appearance — theme, accent, gradient, window surface, top bar / orb,
 *                orb style, background
 *   Desktop    — tiling layout
 *   Assistant  — name, provider, model
 *   Voice      — language, speech recognition, speech output
 *   System     — remember workspace, log out
 *
 * Everything writes straight to the per-user preference store; there is no
 * "Done" navigation because closing the window is the exit. The only explicit
 * save is the profile (name/avatar), which is a server round-trip.
 */
import {
  apiFetch, getVoiceLang, getVoiceLangExplicit, setVoiceLang, getTraveler,
  logoutSession,
} from './api.js';
import {
  toast, icon, button, input, select, slider, toggleRow,
  listThemes, setTheme, getActiveTheme, getThemeManifest,
  applyAppearance, getAccent, setAccent, getGradient, setGradient,
  accentPresets, gradientPresets, gradientToCss,
} from '../ui/index.js';
import {
  getAiName, setAiName, getAiProvider, setAiProvider,
  getOllamaModel, setOllamaModel,
  getOpenAiBaseUrl, setOpenAiBaseUrl, getOpenAiApiKey, setOpenAiApiKey,
  getOpenAiModel, setOpenAiModel,
  getDesktopLayout, setDesktopLayout,
  flushPreferencesNow,
  getDesktopSurface, setDesktopSurface,
  getImmersive, setImmersive,
  getTtsVoice, setTtsVoice, getTtsSpeed, setTtsSpeed,
  getSilenceTimeout, setSilenceTimeout, getWakeWord, setWakeWord,
  getSttEngine, setSttEngine, getWhisperModel, setWhisperModel,
  ORB_STYLES, getOrbStyle, setOrbStyle,
  getRemember, setRemember,
  getRemoteAllowTerminal, setRemoteAllowTerminal,
  getTouchBarEnabled, setTouchBarEnabled,
} from './preferences.js';
import { TOUCHBAR_ACTIONS } from './touchbarShared.js';
import { createOrbPreview } from './orbCanvas.js';
import { SCALE_OPTIONS, getDisplayScale, setDisplayScale } from './display.js';
import { saveKnownUser, renderAvatarEl, readAvatarFile } from './userProfiles.js';
import { getBackground, setBackground, renderBackgroundPresets } from './background.js';
import { pickFiles } from './files.js';

export const SETTINGS_WINDOW = 'settings';

const NEUMORPHIC_DARK = 'neumorphic';
const NEUMORPHIC_LIGHT = 'neumorphic-light';
const BASE_DARK = 'noir';
const BASE_LIGHT = 'light';
const WHISPER_MODEL_LABEL = { tiny: 'Tiny', small: 'Small' };

let tileEl = null;
let cleanups = [];

/* ── Small DOM helpers ──────────────────────────────────────── */

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

function on(target, event, handler, opts) {
  target?.addEventListener(event, handler, opts);
  cleanups.push(() => target?.removeEventListener(event, handler, opts));
}

function field(labelText, control, { hint, id } = {}) {
  const wrap = el('div', 'settings-field');
  if (labelText) {
    const l = el('label', 'settings-label', labelText);
    if (id) l.htmlFor = id;
    wrap.appendChild(l);
  }
  if (control) wrap.appendChild(control);
  if (hint) wrap.appendChild(el('p', 'settings-hint', hint));
  return wrap;
}

function sliderField(labelText, valueText, control) {
  const wrap = el('div', 'settings-field');
  const l = el('label', 'settings-label');
  l.append(document.createTextNode(labelText + ' '));
  const value = el('span', 'settings-value', valueText);
  l.appendChild(value);
  wrap.append(l, control);
  return { wrap, value };
}

function heading(text) {
  const h = el('h4', 'settings-subhead', text);
  return h;
}

function panel(id, iconName, title, children) {
  const section = el('section', 'settings-section settings-panel');
  section.id = `panel-${id}`;
  section.dataset.panel = id;
  const head = el('div', 'settings-panel-head');
  const ring = el('span', 'settings-panel-icon');
  ring.appendChild(icon(iconName, { size: 22 }));
  head.append(ring, el('h3', null, title));
  section.appendChild(head);
  for (const child of children) {
    if (child) section.appendChild(child);
  }
  return section;
}

/* ── Appearance ─────────────────────────────────────────────── */

let orbPreviews = [];

function isNeumorphicTheme(name) {
  return name === NEUMORPHIC_DARK || name === NEUMORPHIC_LIGHT;
}

function isLightTheme(name) {
  return name === NEUMORPHIC_LIGHT || name === BASE_LIGHT;
}

function buildOrbStyles(container) {
  container.textContent = '';
  for (const style of ORB_STYLES) {
    const card = el('button', 'orb-style');
    card.type = 'button';
    card.dataset.orbStyle = style.id;
    card.setAttribute('role', 'radio');
    card.setAttribute('aria-label', `${style.label} — ${style.hint}`);

    const canvas = el('canvas', 'orb-style-canvas');
    canvas.setAttribute('aria-hidden', 'true');

    card.append(canvas, el('span', 'orb-style-label', style.label), el('span', 'orb-style-hint', style.hint));
    on(card, 'click', () => {
      setOrbStyle(style.id);
      markActiveOrb(container);
      const found = ORB_STYLES.find((s) => s.id === style.id);
      if (found) toast(`Orb style: ${found.label}`, { type: 'info' });
    });
    container.appendChild(card);
  }
  requestAnimationFrame(() => {
    // The window may have closed between the paint request and this frame, and
    // this section can be rebuilt in place. Drop the previews we are about to
    // replace — each one owns an animation loop, so losing the reference
    // without destroying it would leak a running renderer.
    orbPreviews.forEach((p) => p.destroy?.());
    orbPreviews = [];
    if (!container.isConnected) return;
    const canvases = [...container.querySelectorAll('.orb-style-canvas')];
    orbPreviews = canvases
      .map((c, i) => (ORB_STYLES[i] ? createOrbPreview(c, ORB_STYLES[i].id, 64) : null))
      .filter(Boolean);
  });
}

function markActiveOrb(container) {
  const current = getOrbStyle();
  container.querySelectorAll('.orb-style').forEach((node) => {
    const active = node.dataset.orbStyle === current;
    node.classList.toggle('is-active', active);
    node.setAttribute('aria-checked', String(active));
  });
}

function buildAccentSwatches(container) {
  container.textContent = '';
  const current = getAccent();
  const presets = accentPresets();

  for (const preset of presets) {
    const b = el('button', 'swatch');
    b.type = 'button';
    b.dataset.accent = preset.value;
    b.style.background = preset.value;
    b.title = preset.label;
    b.setAttribute('role', 'radio');
    b.setAttribute('aria-label', preset.label);
    on(b, 'click', () => {
      setAccent(preset.value);
      applyAppearance();
      markActive(container, (n) => n.dataset.accent === preset.value);
    });
    container.appendChild(b);
  }

  const custom = el('label', 'swatch swatch--custom');
  custom.dataset.accent = 'custom';
  custom.title = 'Custom accent';
  custom.appendChild(el('span', null, '+'));
  const color = input({ type: 'color' });
  color.value = presets.some((p) => p.value === current) ? presets[0]?.value || '#ffffff' : current;
  color.setAttribute('aria-label', 'Custom accent color');
  on(color, 'input', () => {
    setAccent(color.value);
    applyAppearance();
    markActive(container, (n) => n.dataset.accent === current || n.dataset.accent === 'custom');
  });
  custom.appendChild(color);
  container.appendChild(custom);

  const isPreset = presets.some((p) => p.value === current);
  markActive(container, (n) =>
    n.dataset.accent === current || (n.dataset.accent === 'custom' && !isPreset));
}

function buildGradientSwatches(container) {
  container.textContent = '';
  const current = getGradient();
  for (const preset of gradientPresets()) {
    const b = el('button', 'swatch swatch--gradient');
    b.type = 'button';
    b.dataset.gradientId = preset.id;
    b.style.background = gradientToCss(preset);
    b.title = preset.label;
    b.setAttribute('role', 'radio');
    b.setAttribute('aria-label', preset.label);
    on(b, 'click', () => {
      setGradient({ id: preset.id, angle: preset.angle, stops: preset.stops });
      applyAppearance();
      markActive(container, (n) => n.dataset.gradientId === preset.id);
    });
    container.appendChild(b);
  }
  markActive(container, (n) => n.dataset.gradientId === current.id);
}

function markActive(container, isActive) {
  container?.querySelectorAll('.swatch').forEach((node) => {
    node.classList.toggle('is-active', isActive(node));
  });
}

/* ── Background ─────────────────────────────────────────────── */

function buildBackgroundControls() {
  const mode = select({
    options: [
      { value: 'none', label: 'None (default mesh)' },
      { value: 'gradient', label: 'Gradient' },
      { value: 'image', label: 'Image' },
      { value: 'animated', label: 'Animated' },
    ],
    onChange: (value) => {
      setBackground({ mode: value });
      syncBackground();
    },
  });
  const gradientNote = el('div', 'settings-hint hidden', 'Gradient uses your Appearance gradient — change it under Appearance.');

  const presetGrid = el('div', 'bg-preset-grid');
  presetGrid.setAttribute('role', 'radiogroup');
  const pickBackgroundImage = async () => {
    const [file] = await pickFiles({ accept: 'image/*' });
    if (file) void uploadBackground(file, syncBackground);
  };
  const uploadBtn = button({ label: 'Upload your own…', variant: 'quiet', size: 'sm' });
  const removeBtn = button({ label: 'Remove photo', variant: 'danger', size: 'sm' });
  removeBtn.classList.add('hidden');
  const imageRow = el('div', 'background-image-row');
  imageRow.append(uploadBtn, removeBtn);
  const imageHint = el('p', 'settings-hint');

  const imageControls = el('div', 'hidden');
  imageControls.append(presetGrid, imageRow, imageHint);

  const animSelect = select({
    options: [
      { value: 'aurora', label: 'Aurora drift' },
      { value: 'shimmer', label: 'Shimmer' },
    ],
    onChange: (value) => setBackground({ animation: value }),
  });
  const animControls = el('div', 'hidden');
  animControls.append(field('Animation', animSelect));

  const syncBackground = () => {
    const bg = getBackground();
    mode.select.value = bg.mode || 'none';
    gradientNote.classList.toggle('hidden', bg.mode !== 'gradient');
    imageControls.classList.toggle('hidden', bg.mode !== 'image');
    animControls.classList.toggle('hidden', bg.mode !== 'animated');
    animSelect.select.value = bg.animation || 'aurora';
    removeBtn.classList.toggle('hidden', !bg.url);
    imageHint.textContent = bg.mode === 'image'
      ? (bg.url
        ? 'Your photo is dimmed so the interface stays readable.'
        : 'Built-in wallpapers. Pick one, or upload your own image — it is dimmed so the interface stays readable.')
      : '';

    renderBackgroundPresets(presetGrid, {
      onSelect: (preset) => {
        setBackground({ mode: 'image', preset: preset.id, url: null });
        syncBackground();
      },
      onUpload: () => void pickBackgroundImage(),
    });
  };

  on(uploadBtn, 'click', () => void pickBackgroundImage());
  on(removeBtn, 'click', () => void removeBackgroundImage(syncBackground));

  syncBackground();
  return [
    field('Type', mode),
    gradientNote,
    imageControls,
    animControls,
  ];
}

async function uploadBackground(file, sync) {
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
    setBackground({ mode: 'image', url: `/api/background?v=${Date.now()}`, preset: null });
    sync();
    toast('Background image updated', { type: 'info' });
  } catch (e) {
    toast(e.message || 'Could not upload image', { type: 'error' });
  }
}

async function removeBackgroundImage(sync) {
  try {
    await apiFetch('/api/background', { method: 'DELETE' });
  } catch (_) { /* 404 or transient — proceed */ }
  setBackground({ mode: 'image', url: null, preset: null });
  sync();
  toast('Background image removed', { type: 'info' });
}

/* ── Account ────────────────────────────────────────────────── */

function buildAccount() {
  const traveler = getTraveler();
  let pendingAvatar;

  const avatarPreview = el('div', 'profile-avatar profile-avatar--large');
  renderAvatarEl(avatarPreview, {
    name: traveler?.name || traveler?.username || '',
    avatar: traveler?.avatar || null,
  });

  const avatarInput = input({ type: 'file' });
  avatarInput.accept = 'image/*';
  avatarInput.hidden = true;

  const upload = el('label', 'profile-avatar-upload');
  upload.append(avatarInput, avatarPreview, el('span', 'profile-avatar-label', 'Change photo'));

  const nameInput = input({
    value: traveler?.name || '',
    maxlength: 64,
    autocomplete: 'name',
  });

  on(avatarInput, 'change', async () => {
    const file = avatarInput.files?.[0];
    if (!file) return;
    try {
      pendingAvatar = await readAvatarFile(file);
      renderAvatarEl(avatarPreview, { name: nameInput.value || traveler?.name || '', avatar: pendingAvatar });
    } catch (e) {
      toast(e.message, { type: 'error' });
      avatarInput.value = '';
    }
  });

  const saveBtn = button({
    label: 'Save profile',
    variant: 'primary',
    size: 'sm',
    onClick: async () => {
      const name = nameInput.value.trim();
      const body = {};
      if (name && name !== traveler?.name) body.name = name;
      if (pendingAvatar !== undefined) body.avatar = pendingAvatar;
      if (!Object.keys(body).length) {
        toast('Nothing to save', { type: 'info' });
        return;
      }
      try {
        const res = await apiFetch('/api/travelers/me', { method: 'PUT', body: JSON.stringify(body) });
        if (res?.data) {
          localStorage.setItem('traveler', JSON.stringify(res.data));
          saveKnownUser(res.data);
          pendingAvatar = undefined;
          toast('Profile saved', { type: 'info' });
        }
      } catch (e) {
        toast(e.message || 'Could not save profile', { type: 'error' });
      }
    },
  });

  const row = el('div', 'settings-inline-actions');
  row.appendChild(saveBtn);

  return [
    upload,
    field('Display name', nameInput),
    row,
  ];
}

/* ── Assistant ──────────────────────────────────────────────── */

function buildAssistant() {
  const nameInput = input({
    value: getAiName(),
    maxlength: 32,
    autocomplete: 'off',
  });
  const nameHint = el('p', 'settings-hint');
  const syncNameHint = () => {
    nameHint.textContent = `Say “Hey, ${getAiName()}” to activate voice on long press.`;
  };
  syncNameHint();
  on(nameInput, 'input', () => {
    setAiName(nameInput.value);
    syncNameHint();
  });

  const provider = select({
    options: [
      { value: 'ollama', label: 'Ollama (local)' },
      { value: 'openai', label: 'OpenAI-compatible endpoint' },
    ],
    onChange: async (value) => {
      setAiProvider(value);
      syncProviderUI();
      await flushPreferencesNow();
      void loadAiModels();
    },
  });

  const model = select({ onChange: (value) => { setOllamaModel(value); void flushPreferencesNow(); } });
  const modelHint = el('p', 'settings-hint', 'Models from your local Ollama server.');

  const baseUrl = input({ value: getOpenAiBaseUrl(), placeholder: 'https://api.openai.com/v1', autocomplete: 'off' });
  const apiKey = input({ type: 'password', value: getOpenAiApiKey(), placeholder: 'sk-…', autocomplete: 'off' });
  const openaiModel = input({ value: getOpenAiModel(), placeholder: 'gpt-4o-mini', autocomplete: 'off' });
  const openaiHint = el('p', 'settings-hint', 'The exact model name your endpoint expects.');
  on(baseUrl, 'input', () => setOpenAiBaseUrl(baseUrl.value));
  on(apiKey, 'input', () => setOpenAiApiKey(apiKey.value));
  on(openaiModel, 'input', () => setOpenAiModel(openaiModel.value));

  const ollamaFields = el('div');
  ollamaFields.append(field('AI model', model, { id: model.select.id }), modelHint);

  const openaiFields = el('div', 'hidden');
  openaiFields.append(
    field('Endpoint base URL', baseUrl),
    field('API key', apiKey),
    field('Model', openaiModel),
    openaiHint,
  );

  function syncProviderUI() {
    const isOpenAi = getAiProvider() === 'openai';
    provider.select.value = getAiProvider();
    ollamaFields.classList.toggle('hidden', isOpenAi);
    openaiFields.classList.toggle('hidden', !isOpenAi);
  }
  syncProviderUI();

  let serverDefaultModel = '';
  let modelRequest = 0;

  async function loadAiModels() {
    const seq = ++modelRequest;
    try {
      const res = await apiFetch('/api/ai/models');
      if (seq !== modelRequest) return;
      const { provider: who, models = [], default: fallback, available } = res.data || {};
      serverDefaultModel = fallback || '';

      model.setOptions([
        { value: '', label: serverDefaultModel ? `Default (${serverDefaultModel})` : 'Default (server)' },
        ...models.map((n) => ({ value: n, label: n })),
      ]);
      const stored = getOllamaModel();
      model.select.value = stored && models.includes(stored) ? stored : '';

      if (who === 'openai') {
        modelHint.textContent = '';
        openaiHint.textContent = available
          ? (models.length ? `${models.length} model(s) found at the endpoint.` : 'Endpoint reachable.')
          : 'Could not reach the endpoint yet — check the base URL and API key.';
      } else {
        openaiHint.textContent = 'The exact model name your endpoint expects.';
        modelHint.textContent = !available
          ? 'Ollama is offline — using the server default model.'
          : (!models.length
            ? 'No models found in Ollama. Pull one with: ollama pull llama3.2'
            : 'Models from your local Ollama server.');
      }
    } catch (e) {
      if (seq !== modelRequest) return;
      model.setOptions([{ value: '', label: 'Default (server)' }]);
      modelHint.textContent = e.status === 404
        ? 'Model list unavailable — restart the server, then reopen Settings.'
        : 'Could not load models — using the server default.';
    }
  }
  void loadAiModels();

  return [
    field('Assistant name', nameInput),
    nameHint,
    field('AI provider', provider, {
      hint: 'Ollama runs entirely on this device. An OpenAI-compatible endpoint works with OpenAI, OpenRouter, LM Studio, vLLM and other servers.',
    }),
    ollamaFields,
    openaiFields,
  ];
}

/* ── Voice ──────────────────────────────────────────────────── */

function buildVoice() {
  const lang = select({ onChange: (value) => setVoiceLang(value) });
  const langHint = el('p', 'settings-hint', 'Controls speech recognition, spoken replies and the language the assistant writes in.');

  const engine = select({
    options: [
      { value: 'whisper', label: 'Faster Whisper (recommended)' },
      { value: 'vosk', label: 'Vosk (in-browser)' },
    ],
    onChange: (value) => {
      setSttEngine(value);
      updateWhisperUI();
      if (value === 'whisper' && whisperStatus?.whisper !== 'ready') {
        toast('Faster Whisper is not running — voice will use Vosk', { type: 'info' });
      }
    },
  });
  const engineHint = el('p', 'settings-hint', 'Faster Whisper runs on the machine hosting PEAK\u2019D! and streams words while you speak — noticeably more accurate than Vosk. Vosk runs entirely in this browser.');

  const whisperModel = select({
    options: [
      { value: 'tiny', label: 'Tiny — included (~75 MB)' },
      { value: 'small', label: 'Small — download (~480 MB)' },
    ],
    onChange: (value) => {
      setWhisperModel(value);
      updateWhisperUI();
    },
  });
  const whisperHint = el('p', 'settings-hint');
  const whisperDownload = button({ label: 'Download', variant: 'quiet', size: 'sm' });
  whisperDownload.classList.add('hidden');
  const whisperRow = el('div', 'whisper-model-row');
  whisperRow.append(whisperHint, whisperDownload);
  const whisperFields = el('div');
  whisperFields.append(field('Whisper model', whisperModel), whisperRow);

  let whisperStatus = null;
  let whisperStatusLoaded = false;
  let whisperPoll = null;

  function updateWhisperUI() {
    const current = getWhisperModel();
    whisperModel.select.value = current;
    whisperFields.classList.toggle('hidden', getSttEngine() !== 'whisper');

    const info = whisperStatus?.whisper_models?.[current];
    const download = whisperStatus?.whisper_downloads?.[current];
    const label = WHISPER_MODEL_LABEL[current] || current;

    if (whisperStatus && whisperStatus.whisper !== 'ready') {
      whisperHint.textContent = 'Faster Whisper is not running on this machine — voice falls back to Vosk until it is started (./voice/start_whisper.sh).';
      whisperDownload.classList.add('hidden');
      return;
    }
    if (!whisperStatus) {
      whisperHint.textContent = whisperStatusLoaded
        ? 'Could not check the speech service.'
        : 'Checking the speech service…';
      whisperDownload.classList.add('hidden');
      return;
    }
    if (info?.present) {
      whisperHint.textContent = `${label} model ready${info.loaded ? ' and loaded' : ''}${current === 'tiny' ? ' (default)' : ''}.`;
      whisperDownload.classList.add('hidden');
      return;
    }
    if (download?.status === 'downloading') {
      const pct = download.total ? Math.round((download.bytes / download.total) * 100) : 0;
      whisperHint.textContent = `Downloading the ${label.toLowerCase()} model… ${pct}%`;
      whisperDownload.classList.add('hidden');
      return;
    }
    if (download?.status === 'error') {
      whisperHint.textContent = download.error || 'Download failed.';
      whisperDownload.classList.remove('hidden');
      whisperDownload.disabled = false;
      return;
    }
    whisperHint.textContent = `${label} model is not downloaded yet — voice uses Tiny until you download it.`;
    whisperDownload.classList.remove('hidden');
    whisperDownload.disabled = false;
  }

  async function refreshWhisperStatus() {
    try {
      whisperStatus = await apiFetch(`/api/voice/status?lang=${encodeURIComponent(getVoiceLang())}`);
    } catch (_) {
      whisperStatus = null;
    }
    whisperStatusLoaded = true;
    updateWhisperUI();
    return whisperStatus;
  }

  function pollWhisperDownload() {
    clearInterval(whisperPoll);
    whisperPoll = setInterval(async () => {
      const status = await refreshWhisperStatus();
      const current = getWhisperModel();
      const state = status?.whisper_downloads?.[current]?.status;
      const present = status?.whisper_models?.[current]?.present === true;
      if (present || state === 'error') {
        clearInterval(whisperPoll);
        whisperPoll = null;
        if (present) toast(`${WHISPER_MODEL_LABEL[current] || current} model ready`, { type: 'info' });
      }
    }, 1500);
    cleanups.push(() => { clearInterval(whisperPoll); whisperPoll = null; });
  }

  on(whisperDownload, 'click', async () => {
    const current = getWhisperModel();
    whisperDownload.disabled = true;
    try {
      await apiFetch('/api/voice/whisper/download', {
        method: 'POST',
        body: JSON.stringify({ model: current }),
      });
      toast(`Downloading the ${current} model in the background…`, { type: 'info' });
      pollWhisperDownload();
    } catch (e) {
      toast(e.message || 'Could not start the download', { type: 'error' });
      whisperDownload.disabled = false;
    }
  });

  const ttsVoice = select({
    options: [
      { value: '', label: 'Default (server)' },
      ...['M1', 'M2', 'M3', 'M4', 'M5', 'F1', 'F2', 'F3', 'F4', 'F5'].map((v) => ({ value: v, label: v })),
    ],
    onChange: (value) => setTtsVoice(value),
  });

  const speedControl = slider({
    min: 0.7, max: 2, step: 0.1, value: getTtsSpeed(),
    onInput: (value) => {
      speed.value.textContent = `${value.toFixed(1)}×`;
      setTtsSpeed(value);
    },
  });
  const speed = sliderField('Speech speed', `${getTtsSpeed().toFixed(1)}×`, speedControl);

  const silence = select({
    options: [
      { value: '4000', label: '4 seconds' },
      { value: '6000', label: '6 seconds' },
      { value: '8000', label: '8 seconds' },
      { value: '12000', label: '12 seconds' },
      { value: '20000', label: '20 seconds' },
    ],
    onChange: (value) => setSilenceTimeout(Number(value)),
  });
  const silenceMs = getSilenceTimeout();
  if ([...silence.select.options].some((o) => Number(o.value) === silenceMs)) {
    silence.select.value = String(silenceMs);
  }

  const wake = toggleRow({
    label: 'Wake word',
    hint: 'Long-press listens for “Hey…” before a command; when off it just listens.',
    checked: getWakeWord(),
    onChange: (checked) => setWakeWord(checked),
  });

  ttsVoice.select.value = getTtsVoice() || '';

  void (async () => {
    try {
      const res = await apiFetch('/api/voice/languages');
      lang.setOptions(res.data.map((l) => ({
        value: l.code,
        label: `${l.code.toUpperCase()}${l.vosk_available ? '' : ' (TTS only)'}`,
      })));
      const explicit = getVoiceLangExplicit();
      const resolved = explicit || getVoiceLang();
      const codes = res.data.map((l) => l.code);
      const chosen = codes.includes(resolved) ? resolved : 'en';
      lang.select.value = chosen;
      if (!explicit) setVoiceLang(chosen);
    } catch (_) {
      lang.setOptions([{ value: 'en', label: 'EN' }]);
      lang.select.value = 'en';
    }
  })();

  updateWhisperUI();
  void refreshWhisperStatus();

  return [
    field('Language', lang),
    langHint,
    heading('Speech recognition'),
    field('Engine', engine),
    engineHint,
    whisperFields,
    heading('Speech output'),
    field('Voice', ttsVoice, { hint: 'The Supertonic voice used for spoken replies.' }),
    speed.wrap,
    field('Stop listening after', silence, { hint: 'How long to wait for speech before canceling a tap-to-talk input.' }),
    wake,
  ];
}

/* ── Desktop ────────────────────────────────────────────────── */

function buildDesktop() {
  const layout = getDesktopLayout();

  const mode = select({
    options: [
      { value: 'master', label: 'Master & stack' },
      { value: 'columns', label: 'Columns' },
      { value: 'windows', label: 'Windows' },
    ],
    onChange: (value) => {
      setDesktopLayout({ ...getDesktopLayout(), mode: value });
      syncTilingControls();
    },
  });
  mode.select.value = layout.mode;

  const orientation = select({
    options: [
      { value: 'left', label: 'Left' },
      { value: 'right', label: 'Right' },
      { value: 'top', label: 'Top' },
      { value: 'bottom', label: 'Bottom' },
    ],
    onChange: (value) => setDesktopLayout({ ...getDesktopLayout(), orientation: value }),
  });
  orientation.select.value = layout.orientation;

  const ratioControl = slider({
    min: 25, max: 85, step: 5, value: Math.round(layout.master_ratio * 100),
    onInput: (value) => {
      ratio.value.textContent = `${value}%`;
      setDesktopLayout({ ...getDesktopLayout(), master_ratio: value / 100 });
    },
  });
  const ratio = sliderField('Master size', `${Math.round(layout.master_ratio * 100)}%`, ratioControl);

  const gapControl = slider({
    min: 0, max: 40, step: 2, value: layout.gap,
    onInput: (value) => {
      gap.value.textContent = `${value}px`;
      setDesktopLayout({ ...getDesktopLayout(), gap: value });
    },
  });
  const gap = sliderField('Gap', `${layout.gap}px`, gapControl);

  const tilingControls = el('div', 'settings-tiling-controls');
  tilingControls.append(field('Master side', orientation), ratio.wrap, gap.wrap);

  const syncTilingControls = () => {
    tilingControls.classList.toggle('hidden', mode.select.value === 'windows');
  };
  syncTilingControls();

  const shortcuts = el('p', 'settings-hint');
  shortcuts.innerHTML = 'Shortcuts: <strong>Alt+Enter</strong> fullscreen · <strong>Alt+H/L</strong> focus · <strong>Alt+,/.</strong> workspace · <strong>Alt+1..9</strong> jump · <strong>Alt+N</strong> new · <strong>Alt+Shift+N</strong> remove.';

  return [
    field('Layout', mode, {
      hint: 'Tiling snaps windows into workspaces; Windows floats them as draggable, resizable desktop windows.',
    }),
    tilingControls,
    shortcuts,
  ];
}

/* ── Window surface & top bar (shown in the Appearance panel) ── */

function buildWindowSurfaceControls() {
  const surface = getDesktopSurface();
  const titleControl = slider({
    min: 28, max: 48, step: 1, value: surface.title_height,
    onInput: (value) => {
      title.value.textContent = `${value}px`;
      setDesktopSurface({ title_height: value });
    },
  });
  const title = sliderField('Title bar height', `${surface.title_height}px`, titleControl);

  const radiusControl = slider({
    min: 0, max: 32, step: 1, value: surface.window_radius,
    onInput: (value) => {
      radius.value.textContent = `${value}px`;
      setDesktopSurface({ window_radius: value });
    },
  });
  const radius = sliderField('Window corners', `${surface.window_radius}px`, radiusControl);

  const opacityControl = slider({
    min: 60, max: 100, step: 1, value: surface.window_opacity,
    onInput: (value) => {
      opacity.value.textContent = `${value}%`;
      setDesktopSurface({ window_opacity: value });
    },
  });
  const opacity = sliderField('Window opacity', `${surface.window_opacity}%`, opacityControl);

  const blurControl = slider({
    min: 0, max: 24, step: 1, value: surface.glass_blur,
    onInput: (value) => {
      blur.value.textContent = `${value}px`;
      setDesktopSurface({ glass_blur: value });
    },
  });
  const blur = sliderField('Glass blur', `${surface.glass_blur}px`, blurControl);

  const shadow = toggleRow({
    label: 'Window shadows',
    hint: 'Give windows a soft drop shadow instead of a flat hairline.',
    checked: surface.window_shadow,
    onChange: (checked) => setDesktopSurface({ window_shadow: checked }),
  });

  return [
    heading('Window surface'),
    title.wrap,
    radius.wrap,
    opacity.wrap,
    blur.wrap,
    shadow,
  ];
}

function buildTopBarControls() {
  const immersive = getImmersive();
  const autohideBar = toggleRow({
    label: 'Autohide the top bar',
    hint: 'Slide the top bar away until the pointer reaches the edge it docks to.',
    checked: immersive.autohide_bar,
    onChange: (checked) => setImmersive({ autohide_bar: checked }),
  });
  const barPosition = select({
    options: [
      { value: 'top', label: 'Top' },
      { value: 'left', label: 'Left' },
      { value: 'right', label: 'Right' },
      { value: 'center', label: 'Center' },
    ],
    onChange: (value) => setImmersive({ bar_position: value }),
  });
  barPosition.select.value = immersive.bar_position;
  const autohideOrb = toggleRow({
    label: 'Autohide the AI orb',
    hint: 'Slide the orb away until the pointer reaches the bottom centre.',
    checked: immersive.autohide_orb,
    onChange: (checked) => setImmersive({ autohide_orb: checked }),
  });

  return [
    heading('Top bar & orb'),
    autohideBar,
    field('Top bar position', barPosition),
    autohideOrb,
  ];
}

/* ── System ─────────────────────────────────────────────────── */

function buildSystem() {
  const remember = toggleRow({
    label: 'Remember my workspace',
    hint: 'Restore your windows, layout and workspaces from last time.',
    checked: getRemember(),
    onChange: (checked) => setRemember(checked),
  });

  const logoutBtn = button({
    label: 'Log out',
    variant: 'danger',
    onClick: async () => {
      try {
        await flushPreferencesNow();
      } catch (_) { /* proceed */ }
      await logoutSession();
      window.location.href = '/';
    },
  });
  const logoutHint = el('p', 'settings-hint', 'Logging out returns you to the sign-in screen. Your plugins and chats stay as you set them.');
  const actions = el('div', 'settings-inline-actions');
  actions.appendChild(logoutBtn);

  return [
    remember,
    el('p', 'settings-hint', 'On: your windows, layout and workspaces come back exactly as you left them. Off: each sign-in starts with a clean desktop.'),
    heading('Session'),
    actions,
    logoutHint,
  ];
}

/* ── Touch Bar ──────────────────────────────────────────────── */

function buildTouchBar() {
  const mode = select({
    options: [
      { value: 'auto', label: 'Automatic' },
      { value: 'on', label: 'Always on' },
      { value: 'off', label: 'Off' },
    ],
    onChange: (value) => {
      setTouchBarEnabled(value);
      refreshTouchBarHint();
    },
  });
  mode.select.value = getTouchBarEnabled();

  const state = el('p', 'settings-hint');
  const buttons = el('p', 'settings-hint');
  const install = el('p', 'settings-hint');

  function refreshTouchBarHint() {
    const host = document.documentElement.dataset.touchbarHost === '1';
    const setting = getTouchBarEnabled();
    if (host) {
      state.textContent = setting === 'off'
        ? 'A Touch Bar is present, but input is turned off.'
        : 'A Touch Bar is present and its buttons are active.';
    } else if (setting === 'on') {
      state.textContent = 'No Touch Bar detected, but the buttons are forced on — press Ctrl+Alt+Shift with 1–9 or the − = , . keys to try them.';
    } else {
      state.textContent = 'No Touch Bar detected on this machine. The buttons stay dormant, so a normal PC is unaffected.';
    }
    buttons.textContent = TOUCHBAR_ACTIONS
      .map((entry) => `${entry.label} (${entry.code})`)
      .join(' · ');
    install.textContent = host
      ? 'On a T2 Mac running Linux, scripts/touchbar/install-touchbar.sh installs this row through tiny-dfr.'
      : 'On a T2 Mac running Linux, scripts/touchbar/install-touchbar.sh puts this row on the bar through tiny-dfr.';
  }

  refreshTouchBarHint();
  // A change made elsewhere (or the host reporting detection) re-renders it.
  on(window, 'touchbar:settings', refreshTouchBarHint);

  return [
    field('Touch Bar', mode, { hint: 'Show the assistant controls on a MacBook Touch Bar. Automatic only uses the bar when this machine has one.' }),
    state,
    heading('Buttons'),
    buttons,
    install,
  ];
}

/* ── Remote (Iroh) ──────────────────────────────────────────── */

function formatBytes(n) {
  if (!n) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i += 1; }
  return `${v < 10 && i > 0 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

function buildRemote() {
  const state = { enabled: false, endpointId: null, ticket: null, connections: 0, bytes: 0, paired: 0, pairing: false };
  let revealed = false;

  const serverToggle = toggleRow({
    label: 'Server',
    hint: 'Let your other devices reach this desktop over Iroh — end-to-end encrypted, no port forwarding. The link changes every time you start or stop.',
    checked: false,
    onChange: setEnabled,
  });

  const terminalToggle = toggleRow({
    label: 'Allow Terminal from remote clients',
    hint: 'The Terminal is a real shell on this machine. Off by default; enable it only for devices you trust.',
    checked: getRemoteAllowTerminal(),
    onChange: (on) => { setRemoteAllowTerminal(on); },
  });

  const statusLine = el('p', 'settings-hint', 'Reading…');
  const ticketEl = el('code', 'remote-ticket');
  const qrEl = el('img', 'remote-qr');
  qrEl.alt = 'Connection QR code';

  const revealBtn = button({
    label: 'Reveal link',
    variant: 'ghost',
    onClick: () => { revealed = !revealed; render(); },
  });
  const copyBtn = button({
    label: 'Copy',
    variant: 'ghost',
    onClick: async () => {
      if (!state.ticket) return;
      try { await navigator.clipboard.writeText(state.ticket); toast('Link copied'); }
      catch (_) { toast('Could not copy the link', { type: 'error' }); }
    },
  });
  const rotateBtn = button({ label: 'Rotate key', variant: 'danger', onClick: rotate });
  const pairBtn = button({ label: 'Pair a new device', variant: 'ghost', onClick: pair });
  const unpairBtn = button({ label: 'Forget devices', variant: 'quiet', onClick: unpair });

  const actions = el('div', 'settings-inline-actions');
  actions.append(revealBtn, copyBtn, rotateBtn, pairBtn, unpairBtn);

  const linkBox = el('div', 'remote-link');
  linkBox.append(ticketEl, qrEl);
  const linkHint = el('p', 'settings-hint', 'On another device: peakd --iroh <link>, or run shiny-iroh-client --ticket <link> and open http://127.0.0.1:8080.');

  function apply(s) {
    if (!s) return;
    state.enabled = !!s.enabled;
    state.endpointId = s.endpoint_id || null;
    state.ticket = s.ticket || null;
    state.connections = s.connections || 0;
    state.bytes = s.bytes || 0;
    state.paired = s.paired || 0;
    state.pairing = !!s.pairing;
    render();
  }

  function render() {
    serverToggle.toggle.setChecked(state.enabled, { silent: true });
    if (!state.enabled) {
      statusLine.textContent = 'Off. This desktop is reachable only on this machine.';
      linkBox.classList.add('hidden');
      actions.classList.add('hidden');
      return;
    }
    const id = state.endpointId ? `${state.endpointId.slice(0, 12)}…` : '';
    let line = `On — ${id} · ${state.connections} connection${state.connections === 1 ? '' : 's'} · ${formatBytes(state.bytes)}`;
    if (state.paired > 0) line += ` · ${state.paired} paired device${state.paired === 1 ? '' : 's'}`;
    if (state.pairing) line += ' · pairing for the next device…';
    statusLine.textContent = line;
    linkBox.classList.remove('hidden');
    actions.classList.remove('hidden');
    revealBtn.textContent = revealed ? 'Hide link' : 'Reveal link';
    if (revealed) {
      ticketEl.textContent = state.ticket || '';
      qrEl.src = `/api/remote/qr?t=${Date.now()}`;
      qrEl.classList.remove('hidden');
    } else {
      ticketEl.textContent = '••••••••••••••••••••••••••••••';
      qrEl.removeAttribute('src');
      qrEl.classList.add('hidden');
    }
  }

  async function refresh() {
    try { apply(await apiFetch('/api/remote/status', { authRedirect: false })); } catch (_) {}
  }

  async function setEnabled(on) {
    try {
      const res = await apiFetch('/api/remote/enable', {
        method: 'POST',
        authRedirect: false,
        body: JSON.stringify({ enabled: on }),
      });
      if (on) {
        apply(res);
        // In the kiosk, hand the screen to the server-mode window (the shell
        // exits with a mode-switch code). A plain browser — or a remote client,
        // which cannot enable anyway — has no bridge and just shows the link.
        window.ipc?.postMessage?.('peakd:server-mode');
      } else {
        apply({ enabled: false });
      }
      toast(on ? 'Server mode on' : 'Server mode off');
    } catch (e) {
      serverToggle.toggle.setChecked(!on, { silent: true });
      toast(e.message || 'Could not change server mode', { type: 'error' });
    }
  }

  async function rotate() {
    try {
      apply(await apiFetch('/api/remote/rotate', { method: 'POST', authRedirect: false }));
      revealed = false;
      render();
      toast('New link generated');
    } catch (e) {
      toast(e.message || 'Could not rotate the link', { type: 'error' });
    }
  }

  async function pair() {
    try {
      const res = await apiFetch('/api/remote/pair', { method: 'POST', authRedirect: false });
      toast(`Pairing open for ${res?.seconds || 120}s — connect from the new device now`);
      refresh();
    } catch (e) {
      toast(e.message || 'Could not start pairing', { type: 'error' });
    }
  }

  async function unpair() {
    try {
      const res = await apiFetch('/api/remote/unpair', { method: 'POST', authRedirect: false });
      toast(`Forgot ${res?.removed ?? 0} paired device(s)`);
      refresh();
    } catch (e) {
      toast(e.message || 'Could not forget devices', { type: 'error' });
    }
  }

  // Poll only while the Remote panel is visible; stop once it is detached.
  const timer = setInterval(() => {
    if (!statusLine.isConnected) { clearInterval(timer); return; }
    const panel = statusLine.closest('.settings-panel');
    if (panel && panel.classList.contains('is-active')) refresh();
  }, 3000);
  refresh();

  return [
    serverToggle,
    statusLine,
    linkBox,
    linkHint,
    actions,
    heading('Remote control'),
    terminalToggle,
  ];
}

/* ── Surface ────────────────────────────────────────────────── */

function mountSettings() {
  if (tileEl) return tileEl;

  tileEl = el('section', 'tile settings-tile');
  tileEl.dataset.plugin = SETTINGS_WINDOW;

  const windowEl = el('div', 'settings-window');
  const nav = el('nav', 'settings-nav');
  nav.setAttribute('aria-label', 'Settings sections');
  const panels = el('div', 'settings-panels');

  const sections = [
    { id: 'account', label: 'Account', icon: 'ui/user', build: buildAccount },
    { id: 'appearance', label: 'Appearance', icon: 'ui/droplet', build: buildAppearancePanel },
    { id: 'desktop', label: 'Desktop', icon: 'ui/monitor', build: buildDesktop },
    { id: 'assistant', label: 'Assistant', icon: 'ui/message-circle', build: buildAssistant },
    { id: 'voice', label: 'Voice', icon: 'ui/mic', build: buildVoice },
    { id: 'touchbar', label: 'Touch Bar', icon: 'ui/keyboard', build: buildTouchBar },
    { id: 'remote', label: 'Remote', icon: 'ui/network', build: buildRemote },
    { id: 'system', label: 'System', icon: 'ui/power', build: buildSystem },
  ];

  const show = (id) => {
    nav.querySelectorAll('.settings-nav-item').forEach((item) => {
      item.classList.toggle('is-active', item.dataset.panelTarget === id);
    });
    panels.querySelectorAll('.settings-panel').forEach((p) => {
      p.classList.toggle('is-active', p.dataset.panel === id);
    });
  };

  for (const section of sections) {
    const btn = el('button', 'settings-nav-item');
    btn.type = 'button';
    btn.dataset.panelTarget = section.id;
    const ring = el('span', 'settings-nav-icon');
    ring.appendChild(icon(section.icon, { size: 20 }));
    btn.append(ring, el('span', 'settings-nav-label', section.label));
    on(btn, 'click', () => show(section.id));
    nav.appendChild(btn);

    panels.appendChild(panel(section.id, section.icon, section.label, section.build()));
  }

  windowEl.append(nav, panels);
  tileEl.appendChild(windowEl);
  show('account');
  return tileEl;
}

function buildAppearancePanel() {
  const theme = select({
    onChange: async (value) => {
      await setTheme(value);
      syncNeumorphic();
      buildAccentSwatches(accentSwatches);
      buildGradientSwatches(gradientSwatches);
    },
  });
  const neumorphic = toggleRow({
    label: 'Neumorphic soft UI',
    hint: 'Soft extruded-plastic surfaces with subtle dual shadows, in dark and light.',
    checked: isNeumorphicTheme(getActiveTheme()),
    onChange: () => void toggleNeumorphic(),
  });

  const accentSwatches = el('div', 'swatch-row');
  accentSwatches.setAttribute('role', 'radiogroup');
  const gradientSwatches = el('div', 'swatch-row swatch-row--gradient');
  gradientSwatches.setAttribute('role', 'radiogroup');

  const stopA = input({ type: 'color' });
  stopA.value = '#ffffff';
  const stopB = input({ type: 'color' });
  stopB.value = '#8a8a8a';
  const angle = slider({ min: 0, max: 360, step: 5, value: 135, onInput: () => applyCustomGradient() });
  const customControls = el('div', 'gradient-custom-controls');
  customControls.append(stopA, stopB, angle);
  const custom = el('details', 'gradient-custom');
  custom.appendChild(el('summary', null, 'Custom gradient'));
  custom.appendChild(customControls);

  function applyCustomGradient() {
    setGradient({ id: 'custom', stops: [stopA.value, stopB.value], angle: Number(angle.value) });
    applyAppearance();
    markActive(gradientSwatches, (n) => n.dataset.gradientId === 'custom');
  }
  on(stopA, 'input', applyCustomGradient);
  on(stopB, 'input', applyCustomGradient);
  on(angle, 'input', applyCustomGradient);

  const orbStyles = el('div', 'orb-style-row');
  orbStyles.setAttribute('role', 'radiogroup');
  orbStyles.setAttribute('aria-label', 'Orb style');

  const backgroundChildren = buildBackgroundControls();
  const windowSurfaceChildren = buildWindowSurfaceControls();
  const topBarChildren = buildTopBarControls();

  // Interface scale: a host setting, applied by the kiosk shell as page zoom.
  // Changing it re-lays-out the page, so the Terminal re-fits automatically.
  const scale = select({
    options: SCALE_OPTIONS,
    onChange: (value) => void applyScale(value),
  });
  const scaleField = field('Interface scale', scale);
  const scaleHint = el('p', 'settings-hint');
  scaleField.appendChild(scaleHint);

  async function refreshScale() {
    try {
      const data = await getDisplayScale();
      const stored = data?.scale ?? 'auto';
      scale.select.value = String(stored);
      const resolved = data?.resolved;
      const pct = typeof resolved === 'number' && Number.isFinite(resolved)
        ? Math.round(resolved * 100) : null;
      const dpi = data?.dpi;
      const applied = pct
        ? `The shell applies ${pct}%${typeof dpi === 'number' && dpi > 0 ? ` on this ${Math.round(dpi)}-DPI panel` : ''}. `
        : '';
      scaleHint.textContent = `${applied}Scales the whole desktop. Auto follows this panel's pixel density — the right default on a high-DPI screen, and it also cuts rendering cost.`;
    } catch (_) {
      scaleHint.textContent = 'Could not read the current scale.';
    }
  }

  async function applyScale(value) {
    try {
      await setDisplayScale(value);
      await refreshScale();
      toast('Interface scale updated');
    } catch (err) {
      toast(err?.message || 'Could not change the interface scale', { type: 'error' });
      void refreshScale();
    }
  }

  function syncNeumorphic() {
    neumorphic.toggle.setChecked(isNeumorphicTheme(getActiveTheme()), { silent: true });
  }

  async function toggleNeumorphic() {
    const current = getActiveTheme();
    const next = isNeumorphicTheme(current)
      ? (isLightTheme(current) ? BASE_LIGHT : BASE_DARK)
      : (isLightTheme(current) ? NEUMORPHIC_LIGHT : NEUMORPHIC_DARK);
    const ok = await setTheme(next);
    if (!ok) return;
    neumorphic.toggle.setChecked(isNeumorphicTheme(next), { silent: true });
    buildAccentSwatches(accentSwatches);
    buildGradientSwatches(gradientSwatches);
  }

  // Deferred so the swatches/orbs exist in the DOM before they are built.
  queueMicrotask(() => {
    buildAccentSwatches(accentSwatches);
    buildGradientSwatches(gradientSwatches);
    buildOrbStyles(orbStyles);
    custom.open = getGradient().id === 'custom';
    const g = getGradient();
    if (g.id === 'custom' && g.stops?.length >= 2) {
      stopA.value = g.stops[0];
      stopB.value = g.stops[1];
      if (Number.isFinite(g.angle)) angle.value = String(g.angle);
    }
    void listThemes().then((themes) => {
      theme.setOptions(themes.map((name) => ({
        value: name,
        label: name === getActiveTheme() ? (getThemeManifest()?.label || name) : name.charAt(0).toUpperCase() + name.slice(1),
      })));
      theme.select.value = getActiveTheme();
    });
    on(window, 'appearance:change', () => orbPreviews.forEach((p) => p.refreshPalette()));
    void refreshScale();
  });

  return [
    field('Theme', theme),
    neumorphic,
    field('Accent', accentSwatches),
    field('Gradient', gradientSwatches),
    custom,
    heading('Display'),
    scaleField,
    ...windowSurfaceChildren,
    ...topBarChildren,
    heading('Voice orb'),
    el('p', 'settings-hint', 'How the orb looks and moves. Every style reacts to your voice — louder grows the ring’s waves and sparks.'),
    orbStyles,
    heading('Background'),
    el('p', 'settings-hint', 'The full-screen backdrop behind the desktop — the default mesh, your gradient, a photo, or a subtle animation.'),
    ...backgroundChildren,
  ];
}

function unmountSettings() {
  for (const dispose of cleanups) {
    try { dispose(); } catch (_) { /* ignore */ }
  }
  cleanups = [];
  orbPreviews.forEach((p) => p.destroy?.());
  orbPreviews = [];
  tileEl?.remove();
  tileEl = null;
}

export function getSettingsElement() {
  return tileEl;
}

/* Minimal window menu additions (core adds window management around it). */
export function settingsContextMenu() {
  return [];
}

export default {
  name: SETTINGS_WINDOW,
  icon: 'ui/settings',
  mount: mountSettings,
  unmount: unmountSettings,
  getElement: getSettingsElement,
  contextMenu: settingsContextMenu,
};

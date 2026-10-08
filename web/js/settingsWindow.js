/**
 * settingsWindow.js — the Settings window (built-in desktop surface).
 *
 * A grouped control panel that lives in a tile like any plugin window. The
 * sections are fewer and broader than the old standalone page:
 *
 *   Account    — photo + display name
 *   Appearance — theme, accent, gradient, window surface, top bar, background
 *   Desktop    — tiling layout
 *   Browser    — default search engine, ad blocking, history
 *   Assistant  — name, provider, model
 *   Voice      — language, speech recognition, speech output
 *   Power      — power mode, low-power AI, screen/keyboard backlight
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
import { openCoreWindow } from './tiles.js';
import {
  toast, icon, button, input, select, slider, toggleRow,
  listThemes, setTheme, getActiveTheme, getThemeManifest,
  applyAppearance, getAccent, setAccent, getGradient, setGradient,
  accentPresets, gradientPresets, gradientToCss,
  getFont, setFont, FONT_THEME,
  iconsetOptions, getIconsetChoice, setIconset, tintEnabled, setTint, refreshIcons,
} from '../ui/index.js';
import { refreshPluginIcons } from './pluginIcon.js';
import { listFonts } from './fonts.js';
import {
  getAiName, setAiName, getAiProvider, setAiProvider,
  getOllamaModel, setOllamaModel,
  getOpenAiBaseUrl, setOpenAiBaseUrl, getOpenAiApiKey, setOpenAiApiKey,
  getOpenAiModel, setOpenAiModel,
  getDesktopLayout, setDesktopLayout,
  flushPreferencesNow,
  getDesktopSurface, setDesktopSurface,
  getImmersive, setImmersive,
  getHudChips, setHudChips,
  getTtsVoice, setTtsVoice, getTtsSpeed, setTtsSpeed,
  getSilenceTimeout, setSilenceTimeout, getWakeWord, setWakeWord,
  getSttEngine, setSttEngine, getWhisperModel, setWhisperModel,
  getTtsEngine, setTtsEngine, getQwenModel, setQwenModel,
  getRemember, setRemember,
  getRemoteAllowTerminal, setRemoteAllowTerminal,
  getTouchBarEnabled, setTouchBarEnabled,
  getPowerMode, setPowerMode, getAutoPowerSaver, setAutoPowerSaver,
  getLowPowerAi, setLowPowerAi, getEffectivePowerMode,
  getEffectiveSttEngine, getEffectiveTtsEngine, getPowerBatteryStatus,
} from './preferences.js';
import { POWER_MODES, powerModeLabel, powerModeHint, describeEngines } from './powerShared.js';
import { TOUCHBAR_ACTIONS } from './touchbarShared.js';
import { SCALE_OPTIONS, getDisplayScale, setDisplayScale } from './display.js';
import { saveKnownUser, renderAvatarEl, readAvatarFile } from './userProfiles.js';
import { getBackground, setBackground, renderBackgroundPresets } from './background.js';
import { pickFiles } from './files.js';
import { copyText } from './clipboard.js';

export const SETTINGS_WINDOW = 'settings';

const NEUMORPHIC_DARK = 'neumorphic';
const NEUMORPHIC_LIGHT = 'neumorphic-light';
const BASE_DARK = 'noir';
const BASE_LIGHT = 'light';
const WHISPER_MODEL_LABEL = { tiny: 'Tiny', small: 'Small' };
const QWEN_MODEL_LABEL = {
  '0.6b-customvoice': '0.6B CustomVoice',
  '1.7b-customvoice': '1.7B CustomVoice',
};
/** Supertonic voices (M1…F5) vs Qwen3-TTS CustomVoice speakers. */
const SUPERTONIC_VOICES = ['M1', 'M2', 'M3', 'M4', 'M5', 'F1', 'F2', 'F3', 'F4', 'F5'];
const QWEN_SPEAKERS = [
  'serena', 'vivian', 'uncle_fu', 'ryan', 'aiden', 'ono_anna', 'sohee', 'eric', 'dylan',
];

let tileEl = null;
let cleanups = [];
/** Section switcher of the mounted Settings window; lets other UI deep-link. */
let selectSectionFn = null;
let pendingSection = null;

/**
 * Ask the Settings window to show a section (e.g. the battery quick menu's
 * "Power settings"). Safe before the window has mounted: the request is
 * remembered and applied on mount.
 */
export function selectSettingsSection(id) {
  if (!id) return;
  if (selectSectionFn) {
    selectSectionFn(id);
    pendingSection = null;
  } else {
    pendingSection = id;
  }
}

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

function isNeumorphicTheme(name) {
  return name === NEUMORPHIC_DARK || name === NEUMORPHIC_LIGHT;
}

function isLightTheme(name) {
  return name === NEUMORPHIC_LIGHT || name === BASE_LIGHT;
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

  function pollWhisperDownload() {
    clearInterval(whisperPoll);
    whisperPoll = setInterval(async () => {
      const status = await refreshVoiceStatus();
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
      ...SUPERTONIC_VOICES.map((v) => ({ value: v, label: v })),
    ],
    onChange: (value) => {
      setTtsVoice(value);
      ttsVoiceValue = value;
    },
  });

  // ── Speech output engine ────────────────────────────────────
  const ttsEngine = select({
    options: [
      { value: 'supertonic', label: 'Supertonic (default)' },
      { value: 'qwen', label: 'Qwen3-TTS (experimental)' },
    ],
    onChange: (value) => {
      setTtsEngine(value);
      updateTtsUI();
      if (value === 'qwen' && qwenStatus && qwenStatus.qwen !== 'ready') {
        toast('Qwen3-TTS is not running — Supertonic will keep speaking', { type: 'info' });
      }
    },
  });

  const qwenModel = select({
    options: [
      { value: '0.6b-customvoice', label: '0.6B CustomVoice — recommended (~605 MB)' },
      { value: '1.7b-customvoice', label: '1.7B CustomVoice — better (~1.2 GB)' },
    ],
    onChange: (value) => {
      setQwenModel(value);
      updateTtsUI();
    },
  });
  const qwenHint = el('p', 'settings-hint');
  const qwenDownload = button({ label: 'Download', variant: 'quiet', size: 'sm' });
  qwenDownload.classList.add('hidden');
  const qwenRow = el('div', 'whisper-model-row');
  qwenRow.append(qwenHint, qwenDownload);
  const qwenFields = el('div');
  qwenFields.append(field('Qwen model', qwenModel), qwenRow);

  let qwenStatus = null;
  let qwenPoll = null;

  /**
   * The voice picker is engine-specific: Supertonic's M* / F* voices and the
   * Qwen3-TTS CustomVoice speakers share no names, so the options and the
   * chosen value are re-resolved whenever the engine changes.
   */
  function updateTtsUI() {
    const engine = getTtsEngine();
    ttsEngine.select.value = engine;
    qwenFields.classList.toggle('hidden', engine !== 'qwen');

    const voices = engine === 'qwen' ? QWEN_SPEAKERS : SUPERTONIC_VOICES;
    const current = getTtsVoice();
    ttsVoice.setOptions([
      { value: '', label: 'Default (server)' },
      ...voices.map((v) => ({ value: v, label: v })),
    ]);
    // Drop a stored voice that belongs to the other engine.
    ttsVoice.select.value = voices.includes(current) ? current : '';
    ttsVoiceValue = voices.includes(current) ? current : '';
  }
  let ttsVoiceValue = getTtsVoice();

  function updateQwenUI() {
    const current = getQwenModel();
    qwenModel.select.value = current;
    const info = qwenStatus?.qwen_models?.[current];
    const download = qwenStatus?.qwen_downloads?.[current];
    const label = QWEN_MODEL_LABEL[current] || current;

    if (!qwenStatus) {
      qwenHint.textContent = 'Checking the speech service…';
      qwenDownload.classList.add('hidden');
      return;
    }
    if (info?.present) {
      qwenHint.textContent =
        `${label} model ready${current === '0.6b-customvoice' ? ' (default)' : ''}`
        + (qwenStatus.qwen === 'ready' ? '.' : ' — start the engine to use it.');
      qwenDownload.classList.add('hidden');
      return;
    }
    if (download?.status === 'downloading') {
      const pct = download.total ? Math.round((download.bytes / download.total) * 100) : 0;
      qwenHint.textContent = `Downloading the ${label} model… ${pct}%`;
      qwenDownload.classList.add('hidden');
      return;
    }
    if (download?.status === 'error') {
      qwenHint.textContent = download.error || 'Download failed.';
      qwenDownload.classList.remove('hidden');
      qwenDownload.disabled = false;
      return;
    }
    qwenHint.textContent =
      `${label} model is not downloaded yet — download it to use Qwen3-TTS.`;
    qwenDownload.classList.remove('hidden');
    qwenDownload.disabled = false;
  }

  async function refreshVoiceStatus() {
    try {
      const status = await apiFetch(`/api/voice/status?lang=${encodeURIComponent(getVoiceLang())}`);
      whisperStatus = status;
      qwenStatus = status;
    } catch (_) {
      whisperStatus = null;
      qwenStatus = null;
    }
    whisperStatusLoaded = true;
    updateWhisperUI();
    updateTtsUI();
    updateQwenUI();
    return qwenStatus;
  }

  function pollQwenDownload() {
    clearInterval(qwenPoll);
    qwenPoll = setInterval(async () => {
      const status = await refreshVoiceStatus();
      const current = getQwenModel();
      const state = status?.qwen_downloads?.[current]?.status;
      const present = status?.qwen_models?.[current]?.present === true;
      if (present || state === 'error') {
        clearInterval(qwenPoll);
        qwenPoll = null;
        if (present) toast(`${QWEN_MODEL_LABEL[current] || current} model ready`, { type: 'info' });
      }
    }, 1500);
    cleanups.push(() => { clearInterval(qwenPoll); qwenPoll = null; });
  }

  on(qwenDownload, 'click', async () => {
    const current = getQwenModel();
    qwenDownload.disabled = true;
    try {
      await apiFetch('/api/voice/qwen/download', {
        method: 'POST',
        body: JSON.stringify({ model: current }),
      });
      toast(`Downloading ${QWEN_MODEL_LABEL[current] || current} in the background…`, { type: 'info' });
      pollQwenDownload();
    } catch (e) {
      toast(e.message || 'Could not start the download', { type: 'error' });
      qwenDownload.disabled = false;
    }
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

  if (ttsVoiceValue) ttsVoice.select.value = ttsVoiceValue;

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
  void refreshVoiceStatus();

  return [
    field('Language', lang),
    langHint,
    heading('Speech recognition'),
    field('Engine', engine),
    engineHint,
    whisperFields,
    heading('Speech output'),
    field('Engine', ttsEngine, {
      hint: 'Supertonic is bundled and always available. Qwen3-TTS sounds more natural, '
        + 'runs on your GPU through Vulkan when one is available, and needs a one-time '
        + 'build plus a model download.',
    }),
    qwenFields,
    field('Voice', ttsVoice, { hint: 'The voice used for spoken replies.' }),
    speed.wrap,
    field('Stop listening after', silence, { hint: 'How long to wait for speech before canceling a tap-to-talk input.' }),
    wake,
  ];
}

/* ── Power ──────────────────────────────────────────────────── */

function buildPower() {
  const mode = select({
    options: POWER_MODES.map((m) => ({ value: m, label: powerModeLabel(m) })),
    onChange: (value) => {
      setPowerMode(value);
      refreshPowerSummary();
    },
  });
  mode.select.value = getPowerMode();
  const modeField = field('Power mode', mode);
  const modeHint = el('p', 'settings-hint', powerModeHint(getPowerMode()));
  modeField.appendChild(modeHint);

  const auto = toggleRow({
    label: 'Automatic Power Saver',
    hint: 'In Balanced, switch to Power Saver when on battery or below 20%.',
    checked: getAutoPowerSaver(),
    onChange: (on) => {
      setAutoPowerSaver(on);
      refreshPowerSummary();
    },
  });

  const lowAi = toggleRow({
    label: 'Low-power AI',
    hint: 'Let Power Saver use Vosk for speech recognition and Supertonic for replies '
      + 'instead of faster-whisper and Qwen3-TTS.',
    checked: getLowPowerAi(),
    onChange: (on) => {
      setLowPowerAi(on);
      refreshPowerSummary();
    },
  });

  const summary = el('p', 'settings-hint');

  function refreshPowerSummary() {
    const status = getPowerBatteryStatus();
    const battery = status && status.available
      ? [
        status.percent != null ? `${status.percent}%` : 'Battery',
        status.state || null,
        status.ac_online === true ? 'plugged in' : null,
      ].filter(Boolean).join(' · ')
      : 'No battery on this machine';
    const effective = getEffectivePowerMode();
    const engines = describeEngines(getEffectiveSttEngine(), getEffectiveTtsEngine());
    modeHint.textContent = powerModeHint(getPowerMode());
    summary.textContent = effective === 'saver'
      ? `${battery}. Power Saver is in force — ${engines}.`
      : `${battery}. ${powerModeLabel(effective)} — full-quality AI (${engines}).`;
  }
  refreshPowerSummary();
  on(window, 'power:settings', refreshPowerSummary);
  on(window, 'battery:changed', refreshPowerSummary);

  return [
    summary,
    modeField,
    auto,
    lowAi,
    heading('Display'),
    backlightField(
      'Screen brightness',
      '/api/screen/brightness',
      'This machine does not expose a writable panel backlight to the app.',
    ),
    backlightField(
      'Keyboard backlight',
      '/api/keyboard/backlight',
      'This machine does not expose a writable keyboard backlight to the app.',
    ),
  ];
}

/**
 * A host backlight slider (screen or keyboard). Writes go straight to the
 * server's sysfs proxy, debounced so dragging does not flood it; a machine
 * without the device reports `available:false` and the control is disabled.
 */
function backlightField(label, endpoint, unavailableHint) {
  const value = el('span', 'settings-value', '—');
  const wrap = el('div', 'settings-field backlight-field');
  const l = el('label', 'settings-label');
  l.append(document.createTextNode(`${label} `), value);
  const control = slider({ min: 0, max: 100, value: 50 });
  wrap.append(l, control);

  let timer = null;
  control.addEventListener('input', () => {
    value.textContent = `${control.value}%`;
    clearTimeout(timer);
    timer = setTimeout(() => {
      apiFetch(endpoint, { method: 'POST', body: JSON.stringify({ percent: Number(control.value) }) })
        .catch(() => {});
    }, 150);
  });
  cleanups.push(() => clearTimeout(timer));

  void (async () => {
    try {
      const res = await apiFetch(endpoint, { authRedirect: false });
      const s = res?.data;
      if (!s || !s.available || s.percent == null) {
        control.disabled = true;
        value.textContent = 'Unavailable';
        wrap.appendChild(el('p', 'settings-hint', unavailableHint));
        return;
      }
      control.value = String(s.percent);
      value.textContent = `${s.percent}%`;
      if (s.writable === false) {
        control.disabled = true;
        wrap.appendChild(el('p', 'settings-hint', 'The app cannot write this backlight on this machine.'));
      }
    } catch (_) {
      control.disabled = true;
      value.textContent = 'Unavailable';
      wrap.appendChild(el('p', 'settings-hint', unavailableHint));
    }
  })();

  return wrap;
}

/* ── Desktop ────────────────────────────────────────────────── */

function buildDesktop() {
  const layout = getDesktopLayout();

  const mode = select({
    options: [
      { value: 'master', label: 'Tiling' },
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

  const background = toggleRow({
    label: 'Blurred window background',
    hint: 'The soft blurred mirror behind a window — its photo, PDF page, artwork or the accent glow. Turn off for flat, fully opaque-looking windows.',
    checked: surface.window_background,
    onChange: (checked) => setDesktopSurface({ window_background: checked }),
  });

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
    background,
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
    label: 'Autohide the voice bar',
    hint: 'Slide the voice bar away until the pointer reaches the bottom centre.',
    checked: immersive.autohide_orb,
    onChange: (checked) => setImmersive({ autohide_orb: checked }),
  });

  return [
    heading('Top bar & voice bar'),
    autohideBar,
    field('Top bar position', barPosition),
    autohideOrb,
  ];
}

/* Content of the top-bar host status chips: the device label and the numeric
 * percentage (volume / signal / battery) can each be hidden. Both off leaves
 * an icon-only button — see hudChipsShared.js + preferences.js. */
function buildHudChipControls() {
  const chips = getHudChips();
  const names = toggleRow({
    label: 'Device names',
    hint: 'Show the device or network name in the sound, network, Bluetooth and battery buttons.',
    checked: chips.name,
    onChange: (checked) => setHudChips({ name: checked }),
  });
  const percent = toggleRow({
    label: 'Percentages',
    hint: 'Show volume, signal and battery numbers. Turn both off for icon-only buttons.',
    checked: chips.percent,
    onChange: (checked) => setHudChips({ percent: checked }),
  });

  return [
    heading('Host status chips'),
    el('p', 'settings-hint', 'Choose what the sound, network, Bluetooth and battery buttons in the top bar show.'),
    names,
    percent,
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
  const state = { enabled: false, endpointId: null, ticket: null, connections: 0, bytes: 0, paired: 0, pairing: false, tailscale: null };
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
      // Copies go through the central service so they land in the history.
      const ok = await copyText(state.ticket, { source: 'settings' });
      toast(ok ? 'Link copied' : 'Could not copy the link', ok ? undefined : { type: 'error' });
    },
  });
  const rotateBtn = button({ label: 'Rotate key', variant: 'danger', onClick: rotate });
  const pairBtn = button({ label: 'Pair a new device', variant: 'ghost', onClick: pair });
  const unpairBtn = button({ label: 'Forget devices', variant: 'quiet', onClick: unpair });

  const actions = el('div', 'settings-inline-actions');
  actions.append(revealBtn, copyBtn, rotateBtn, pairBtn, unpairBtn);

  const linkBox = el('div', 'remote-link');
  linkBox.append(ticketEl, qrEl);
  const linkHint = el('p', 'settings-hint', 'On another device: peakd --iroh <link> opens the app in a window. To use a plain browser instead, turn on the public URL below.');

  // ── Tailscale Funnel: a normal https:// URL for any browser ──────────
  const tsToggle = toggleRow({
    label: 'Public URL (Tailscale Funnel)',
    hint: 'Serve the app at an https://…ts.net URL through Tailscale — open it in any browser, on any device, no app or port forwarding. Free; needs Tailscale installed and signed in.',
    checked: false,
    onChange: setTailscale,
  });
  const tsMissing = el('p', 'settings-hint', 'Tailscale is not installed on this machine.');
  const tsStatus = el('p', 'settings-hint', '');
  const tsUrlEl = el('code', 'remote-ticket');
  const tsQrEl = el('img', 'remote-qr');
  tsQrEl.alt = 'Tailscale URL QR code';
  const tsUrlBox = el('div', 'remote-link');
  tsUrlBox.append(tsUrlEl, tsQrEl);
  const tsCopyBtn = button({ label: 'Copy URL', variant: 'ghost', onClick: copyTailscale });
  const tsActions = el('div', 'settings-inline-actions');
  tsActions.append(tsCopyBtn);

  function apply(s) {
    if (!s) return;
    state.enabled = !!s.enabled;
    state.endpointId = s.endpoint_id || null;
    state.ticket = s.ticket || null;
    state.connections = s.connections || 0;
    state.bytes = s.bytes || 0;
    state.paired = s.paired || 0;
    state.pairing = !!s.pairing;
    state.tailscale = s.tailscale || null;
    render();
  }

  function render() {
    serverToggle.toggle.setChecked(state.enabled, { silent: true });
    renderTailscale();
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

  function renderTailscale() {
    const ts = state.tailscale;
    const installed = !!(ts && ts.installed);
    tsMissing.classList.toggle('hidden', installed);
    tsToggle.classList.toggle('hidden', !installed);
    if (!installed) {
      tsStatus.textContent = ts && ts.error ? `Tailscale: ${ts.error}` : '';
      tsUrlBox.classList.add('hidden');
      tsActions.classList.add('hidden');
      return;
    }
    const on = !!ts.funnel;
    tsToggle.toggle.setChecked(on, { silent: true });
    let line;
    if (!ts.up) line = 'Installed, but not connected — run `tailscale up` on this machine.';
    else if (on) line = `On — ${ts.hostname || 'this device'}`;
    else line = 'Off. The app is reachable only locally and over Iroh.';
    if (ts.error) line += ` (${ts.error})`;
    tsStatus.textContent = line;
    if (on && ts.url) {
      tsUrlEl.textContent = ts.url;
      tsUrlBox.classList.remove('hidden');
      tsActions.classList.remove('hidden');
      tsQrEl.src = `/api/remote/tailscale/qr?t=${Date.now()}`;
    } else {
      tsUrlBox.classList.add('hidden');
      tsActions.classList.add('hidden');
    }
  }

  async function setTailscale(on) {
    try {
      await apiFetch(`/api/remote/tailscale/${on ? 'enable' : 'disable'}`, {
        method: 'POST',
        authRedirect: false,
      });
      toast(on ? 'Tailscale Funnel on' : 'Tailscale Funnel off');
      await refresh();
    } catch (e) {
      tsToggle.toggle.setChecked(!on, { silent: true });
      toast(e.message || 'Could not change Tailscale Funnel', { type: 'error' });
    }
  }

  async function copyTailscale() {
    const url = state.tailscale && state.tailscale.url;
    if (!url) return;
    const ok = await copyText(url, { source: 'settings' });
    toast(ok ? 'URL copied' : 'Could not copy the URL', ok ? undefined : { type: 'error' });
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
    heading('Public URL (Tailscale)'),
    tsMissing,
    tsToggle,
    tsStatus,
    tsUrlBox,
    tsActions,
    heading('Remote control'),
    terminalToggle,
  ];
}

/* ── Browser ─────────────────────────────────────────────────── */

/**
 * The Browser window's settings, mirrored here so the search engine can be
 * chosen from Settings as well as from the window's own ⋯ menu. The values
 * live server-side, per user; this section reads them and writes them straight
 * back, like every other preference here.
 */
function buildBrowser() {
  const holder = el('div', 'settings-browser');
  holder.appendChild(el('p', 'settings-hint', 'Loading browser settings…'));

  void (async () => {
    let settings = { search_engine: 'duckduckgo', adblock: true, downloads_dir: '' };
    try {
      const res = await apiFetch('/api/browser/settings');
      settings = { ...settings, ...(res?.data || {}) };
    } catch (_) {
      /* the defaults stand in */
    }

    const engine = select({
      value: settings.search_engine,
      options: [
        { value: 'duckduckgo', label: 'DuckDuckGo (default)' },
        { value: 'brave', label: 'Brave Search' },
        { value: 'google', label: 'Google' },
        { value: 'bing', label: 'Bing' },
      ],
      onChange: (value) => {
        void apiFetch('/api/browser/settings', {
          method: 'POST',
          body: JSON.stringify({ search_engine: value }),
        })
          .then(() => toast('Search engine saved'))
          .catch(() => toast('Could not save the search engine', { type: 'error' }));
      },
    });

    const adblock = toggleRow({
      label: 'Block ads and trackers',
      hint: 'The Browser window filters page requests through the shell.',
      checked: settings.adblock !== false,
      onChange: (checked) => {
        void apiFetch('/api/browser/settings', {
          method: 'POST',
          body: JSON.stringify({ adblock: checked }),
        }).catch(() => {});
      },
    });

    const open = button({
      label: 'Open Browser',
      variant: 'ghost',
      onClick: () => openCoreWindow('browser'),
    });
    const clear = button({
      label: 'Clear history',
      variant: 'ghost',
      onClick: async () => {
        try {
          await apiFetch('/api/browser/history/clear', { method: 'POST' });
          toast('Browsing history cleared');
        } catch (_) {
          toast('Could not clear history', { type: 'error' });
        }
      },
    });
    const actions = el('div', 'settings-inline-actions');
    actions.append(open, clear);

    const storage = el(
      'p',
      'settings-hint',
      settings.downloads_dir
        ? `Downloads are saved to ${settings.downloads_dir}`
        : 'Downloads are saved to your Downloads folder.',
    );

    holder.replaceChildren(
      heading('Search'),
      engine,
      el('p', 'settings-hint', 'Used when you type words instead of an address. A self-hosted SearXNG (SEARXNG_URL) takes precedence when configured.'),
      heading('Privacy'),
      adblock,
      heading('Storage'),
      storage,
      heading('Actions'),
      actions,
    );
  })();

  return [holder];
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
    { id: 'browser', label: 'Browser', icon: 'ui/launcher', build: buildBrowser },
    { id: 'assistant', label: 'Assistant', icon: 'ui/message-circle', build: buildAssistant },
    { id: 'voice', label: 'Voice', icon: 'ui/mic', build: buildVoice },
    { id: 'power', label: 'Power', icon: 'ui/power', build: buildPower },
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
  selectSectionFn = show;
  show(pendingSection || 'account');
  pendingSection = null;
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

  const iconset = select({
    options: iconsetOptions().map(({ value, label }) => ({ value, label })),
    onChange: (value) => {
      setIconset(value);
      refreshIcons();
      refreshPluginIcons();
    },
  });
  iconset.select.value = getIconsetChoice();
  const iconsetField = field('Icon set', iconset, {
    hint: iconsetOptions().find((o) => o.value === getIconsetChoice())?.hint
      || 'Infinity is coloured; Slot-Beauty is the monochrome line set. Themes may still override individual icons.',
  });

  const tintIcons = toggleRow({
    label: 'Accent-tinted icons',
    hint: 'Recolour the set’s artwork (folders, app and file icons) to your accent. Off keeps each set’s native colours.',
    checked: tintEnabled(),
    onChange: (checked) => {
      setTint(checked);
      refreshIcons();
      refreshPluginIcons();
    },
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

  const font = select({
    options: [{ value: getFont(), label: getFont() }],
    onChange: (value) => setFont(value),
  });
  // Populated from the host's installed fonts (GET /api/fonts). The current
  // choice is kept even when it is not installed here, so a preference carried
  // from another machine never silently resets.
  void listFonts().then((families) => {
    const values = families.slice();
    const current = getFont();
    if (current !== FONT_THEME && !values.includes(current)) values.unshift(current);
    font.setOptions([
      ...values.map((f) => ({ value: f, label: f })),
      { value: FONT_THEME, label: 'Theme default' },
    ]);
    font.select.value = getFont();
  });
  font.select.value = getFont();
  const fontField = field('Global font', font, {
    hint: 'Every installed system font is listed. "Theme default" restores the active theme’s own fonts.',
  });

  const backgroundChildren = buildBackgroundControls();
  const windowSurfaceChildren = buildWindowSurfaceControls();
  const topBarChildren = buildTopBarControls();
  const hudChipChildren = buildHudChipControls();

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

  // Deferred so the swatches exist in the DOM before they are built.
  queueMicrotask(() => {
    buildAccentSwatches(accentSwatches);
    buildGradientSwatches(gradientSwatches);
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
    void refreshScale();
  });

  return [
    field('Theme', theme),
    iconsetField,
    tintIcons,
    neumorphic,
    field('Accent', accentSwatches),
    field('Gradient', gradientSwatches),
    custom,
    fontField,
    heading('Display'),
    scaleField,
    ...windowSurfaceChildren,
    ...topBarChildren,
    ...hudChipChildren,
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
  selectSectionFn = null;
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
  selectSection: selectSettingsSection,
};

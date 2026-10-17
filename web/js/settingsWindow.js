/**
 * settingsWindow.js — the Settings window (built-in desktop surface).
 *
 * A grouped control panel that lives in a tile like any plugin window. The
 * sections are fewer and broader than the old standalone page:
 *
 *   Account    — photo + display name
 *   Appearance — theme, accent, gradient, interface language, window surface,
 *                top bar, background
 *   Desktop    — tiling layout
 *   Browser    — default search engine, ad blocking, history
 *   Assistant  — name, provider, model
 *   Voice      — language, speech recognition, speech output
 *   Images     — local image generation model, quality + downloads
 *   Power      — power mode, low-power AI, screen/keyboard backlight
 *   System     — remember workspace, log out
 *
 * Everything writes straight to the per-user preference store; there is no
 * "Done" navigation because closing the window is the exit. The only explicit
 * save is the profile (name/avatar), which is a server round-trip.
 */
import {
  apiFetch, getVoiceLang, getVoiceLangExplicit, setVoiceLang, clearVoiceLang, getTraveler,
  logoutSession,
} from './api.js';
import { openCoreWindow } from './tiles.js';
import { setLayout } from './desktop.js';
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
import { getAvailableLocales, t, tp } from './i18n.js';
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
  getImageModel, setImageModel, getImageQuality, setImageQuality,
  getSongQuality, setSongQuality,
  getRemember, setRemember,
  getRemoteAllowTerminal, setRemoteAllowTerminal,
  getTouchBarEnabled, setTouchBarEnabled,
  getPowerMode, setPowerMode, getAutoPowerSaver, setAutoPowerSaver,
  getLowPowerAi, setLowPowerAi, getEffectivePowerMode,
  getEffectiveSttEngine, getEffectiveTtsEngine, getPowerBatteryStatus,
  getLinkPlugin, setLinkPlugin,
  getUiLocale, setUiLocale,
} from './preferences.js';
import { POWER_MODES, powerModeLabel, powerModeHint, describeEngines } from './powerShared.js';
import { TOUCHBAR_ACTIONS } from './touchbarShared.js';
import { voicesForEngine } from './voiceShared.js';
import { SCALE_OPTIONS, getDisplayScale, setDisplayScale } from './display.js';
import { saveKnownUser, renderAvatarEl, readAvatarFile } from './userProfiles.js';
import { getBackground, setBackground, renderBackgroundPresets } from './background.js';
import { pickFiles } from './files.js';
import { copyText } from './clipboard.js';
import { linkCatalog, linkPlugins, resolveLinkPlugin } from './linkRouter.js';

export const SETTINGS_WINDOW = 'settings';

const NEUMORPHIC_DARK = 'neumorphic';
const NEUMORPHIC_LIGHT = 'neumorphic-light';
const BASE_DARK = 'noir';
const BASE_LIGHT = 'light';
const WHISPER_MODEL_LABEL = {
  tiny: 'settings.voice.whisper.model.tiny',
  small: 'settings.voice.whisper.model.small',
};
const QWEN_MODEL_LABEL = {
  '0.6b-customvoice': 'settings.voice.qwen.model.06b',
  '1.7b-customvoice': 'settings.voice.qwen.model.17b',
};

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
  custom.title = t('settings.appearance.customAccent', null, 'Custom accent');
  custom.appendChild(el('span', null, '+'));
  const color = input({ type: 'color' });
  color.value = presets.some((p) => p.value === current) ? presets[0]?.value || '#ffffff' : current;
  color.setAttribute('aria-label', t('settings.appearance.customAccentColor', null, 'Custom accent color'));
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
      { value: 'none', label: t('settings.appearance.background.none', null, 'None (default mesh)') },
      { value: 'gradient', label: t('settings.appearance.background.gradient', null, 'Gradient') },
      { value: 'image', label: t('settings.appearance.background.image', null, 'Image') },
      { value: 'animated', label: t('settings.appearance.background.animated', null, 'Animated') },
    ],
    onChange: (value) => {
      setBackground({ mode: value });
      syncBackground();
    },
  });
  const gradientNote = el('div', 'settings-hint hidden', t('settings.appearance.background.gradientNote', null, 'Gradient uses your Appearance gradient — change it under Appearance.'));

  const presetGrid = el('div', 'bg-preset-grid');
  presetGrid.setAttribute('role', 'radiogroup');
  const pickBackgroundImage = async () => {
    const [file] = await pickFiles({ accept: 'image/*' });
    if (file) void uploadBackground(file, syncBackground);
  };
  const uploadBtn = button({ label: t('settings.appearance.background.upload', null, 'Upload your own…'), variant: 'quiet', size: 'sm' });
  const removeBtn = button({ label: t('settings.appearance.background.remove', null, 'Remove photo'), variant: 'danger', size: 'sm' });
  removeBtn.classList.add('hidden');
  const imageRow = el('div', 'background-image-row');
  imageRow.append(uploadBtn, removeBtn);
  const imageHint = el('p', 'settings-hint');

  const imageControls = el('div', 'hidden');
  imageControls.append(presetGrid, imageRow, imageHint);

  const animSelect = select({
    options: [
      { value: 'aurora', label: t('settings.appearance.background.aurora', null, 'Aurora drift') },
      { value: 'shimmer', label: t('settings.appearance.background.shimmer', null, 'Shimmer') },
    ],
    onChange: (value) => setBackground({ animation: value }),
  });
  const animControls = el('div', 'hidden');
  animControls.append(field(t('settings.appearance.background.animation', null, 'Animation'), animSelect));

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
        ? t('settings.appearance.background.photoHint', null, 'Your photo is dimmed so the interface stays readable.')
        : t('settings.appearance.background.wallpaperHint', null, 'Built-in wallpapers. Pick one, or upload your own image — it is dimmed so the interface stays readable.'))
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
    field(t('settings.appearance.background.type', null, 'Type'), mode),
    gradientNote,
    imageControls,
    animControls,
  ];
}

async function uploadBackground(file, sync) {
  if (!file.type?.startsWith('image/')) {
    toast(t('settings.appearance.background.chooseImage', null, 'Choose an image file'), { type: 'error' });
    return;
  }
  if (file.size > 12 * 1024 * 1024) {
    toast(t('settings.appearance.background.tooLarge', null, 'Image must be under 12 MB'), { type: 'error' });
    return;
  }
  try {
    const form = new FormData();
    form.append('file', file);
    await apiFetch('/api/background', { method: 'POST', body: form });
    setBackground({ mode: 'image', url: `/api/background?v=${Date.now()}`, preset: null });
    sync();
    toast(t('settings.appearance.background.updated', null, 'Background image updated'), { type: 'info' });
  } catch (e) {
    toast(e.message || t('settings.appearance.background.uploadFailed', null, 'Could not upload image'), { type: 'error' });
  }
}

async function removeBackgroundImage(sync) {
  try {
    await apiFetch('/api/background', { method: 'DELETE' });
  } catch (_) { /* 404 or transient — proceed */ }
  setBackground({ mode: 'image', url: null, preset: null });
  sync();
  toast(t('settings.appearance.background.removed', null, 'Background image removed'), { type: 'info' });
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
  upload.append(avatarInput, avatarPreview, el('span', 'profile-avatar-label', t('settings.account.changePhoto', null, 'Change photo')));

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
    label: t('settings.account.save', null, 'Save profile'),
    variant: 'primary',
    size: 'sm',
    onClick: async () => {
      const name = nameInput.value.trim();
      const body = {};
      if (name && name !== traveler?.name) body.name = name;
      if (pendingAvatar !== undefined) body.avatar = pendingAvatar;
      if (!Object.keys(body).length) {
        toast(t('settings.account.nothingToSave', null, 'Nothing to save'), { type: 'info' });
        return;
      }
      try {
        const res = await apiFetch('/api/travelers/me', { method: 'PUT', body: JSON.stringify(body) });
        if (res?.data) {
          localStorage.setItem('traveler', JSON.stringify(res.data));
          saveKnownUser(res.data);
          pendingAvatar = undefined;
          toast(t('settings.account.saved', null, 'Profile saved'), { type: 'info' });
        }
      } catch (e) {
        toast(e.message || t('settings.account.saveFailed', null, 'Could not save profile'), { type: 'error' });
      }
    },
  });

  const row = el('div', 'settings-inline-actions');
  row.appendChild(saveBtn);

  return [
    upload,
    field(t('settings.account.displayName', null, 'Display name'), nameInput),
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
    nameHint.textContent = t('settings.assistant.nameHint', { name: getAiName() }, `Say “Hey, ${getAiName()}” to activate voice on long press.`);
  };
  syncNameHint();
  on(nameInput, 'input', () => {
    setAiName(nameInput.value);
    syncNameHint();
  });

  const provider = select({
    options: [
      { value: 'ollama', label: t('settings.assistant.providerOllama', null, 'Ollama (local)') },
      { value: 'openai', label: t('settings.assistant.providerOpenai', null, 'OpenAI-compatible endpoint') },
    ],
    onChange: async (value) => {
      setAiProvider(value);
      syncProviderUI();
      await flushPreferencesNow();
      void loadAiModels();
    },
  });

  const model = select({ onChange: (value) => { setOllamaModel(value); void flushPreferencesNow(); } });
  const modelHint = el('p', 'settings-hint', t('settings.assistant.modelsHint', null, 'Models from your local Ollama server.'));

  const baseUrl = input({ value: getOpenAiBaseUrl(), placeholder: 'https://api.openai.com/v1', autocomplete: 'off' });
  const apiKey = input({ type: 'password', value: getOpenAiApiKey(), placeholder: 'sk-…', autocomplete: 'off' });
  const openaiModel = input({ value: getOpenAiModel(), placeholder: 'gpt-4o-mini', autocomplete: 'off' });
  const openaiHint = el('p', 'settings-hint', t('settings.assistant.openaiModelHint', null, 'The exact model name your endpoint expects.'));
  on(baseUrl, 'input', () => setOpenAiBaseUrl(baseUrl.value));
  on(apiKey, 'input', () => setOpenAiApiKey(apiKey.value));
  on(openaiModel, 'input', () => setOpenAiModel(openaiModel.value));

  const ollamaFields = el('div');
  ollamaFields.append(field(t('settings.assistant.aiModel', null, 'AI model'), model, { id: model.select.id }), modelHint);

  const openaiFields = el('div', 'hidden');
  openaiFields.append(
    field(t('settings.assistant.baseUrl', null, 'Endpoint base URL'), baseUrl),
    field(t('settings.assistant.apiKey', null, 'API key'), apiKey),
    field(t('settings.assistant.model', null, 'Model'), openaiModel),
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
        { value: '', label: serverDefaultModel ? t('settings.assistant.defaultModel', { model: serverDefaultModel }, `Default (${serverDefaultModel})`) : t('common.defaultServer', null, 'Default (server)') },
        ...models.map((n) => ({ value: n, label: n })),
      ]);
      const stored = getOllamaModel();
      model.select.value = stored && models.includes(stored) ? stored : '';

      if (who === 'openai') {
        modelHint.textContent = '';
        openaiHint.textContent = available
          ? (models.length
            ? tp('settings.assistant.modelsFound', models.length, null, `${models.length} model(s) found at the endpoint.`)
            : t('settings.assistant.endpointReachable', null, 'Endpoint reachable.'))
          : t('settings.assistant.endpointUnreachable', null, 'Could not reach the endpoint yet — check the base URL and API key.');
      } else {
        openaiHint.textContent = t('settings.assistant.openaiModelHint', null, 'The exact model name your endpoint expects.');
        modelHint.textContent = !available
          ? t('settings.assistant.ollamaOffline', null, 'Ollama is offline — using the server default model.')
          : (!models.length
            ? t('settings.assistant.noModels', null, 'No models found in Ollama. Pull one with: ollama pull llama3.2')
            : t('settings.assistant.modelsHint', null, 'Models from your local Ollama server.'));
      }
    } catch (e) {
      if (seq !== modelRequest) return;
      model.setOptions([{ value: '', label: t('common.defaultServer', null, 'Default (server)') }]);
      modelHint.textContent = e.status === 404
        ? t('settings.assistant.modelsUnavailable', null, 'Model list unavailable — restart the server, then reopen Settings.')
        : t('settings.assistant.modelsLoadFailed', null, 'Could not load models — using the server default.');
    }
  }
  void loadAiModels();

  return [
    field(t('settings.assistant.name', null, 'Assistant name'), nameInput),
    nameHint,
    field(t('settings.assistant.provider', null, 'AI provider'), provider, {
      hint: t('settings.assistant.providerHint', null, 'Ollama runs entirely on this device. An OpenAI-compatible endpoint works with OpenAI, OpenRouter, LM Studio, vLLM and other servers.'),
    }),
    ollamaFields,
    openaiFields,
  ];
}

/* ── Voice ──────────────────────────────────────────────────── */

function buildVoice() {
  // Empty value = follow the interface language (the default). Picking a code
  // stores an explicit voice language that then stays put across interface
  // language changes.
  const lang = select({
    onChange: (value) => (value ? setVoiceLang(value) : clearVoiceLang()),
  });
  const langHint = el('p', 'settings-hint', t('settings.voice.language.hint'));

  const engine = select({
    options: [
      { value: 'whisper', label: t('settings.voice.sttEngine.whisper', null, 'Faster Whisper (recommended)') },
      { value: 'vosk', label: t('settings.voice.sttEngine.vosk', null, 'Vosk (in-browser)') },
    ],
    onChange: (value) => {
      setSttEngine(value);
      updateWhisperUI();
      if (value === 'whisper' && whisperStatus?.whisper !== 'ready') {
        toast(t('settings.voice.whisperNotRunning', null, 'Faster Whisper is not running — voice will use Vosk'), { type: 'info' });
      }
    },
  });
  const engineHint = el('p', 'settings-hint', t('settings.voice.sttEngineHint', null, 'Faster Whisper runs on the machine hosting PEAK\u2019D! and streams words while you speak — noticeably more accurate than Vosk. Vosk runs entirely in this browser.'));

  const whisperModel = select({
    options: [
      { value: 'tiny', label: t('settings.voice.whisper.option.tiny', null, 'Tiny — included (~75 MB)') },
      { value: 'small', label: t('settings.voice.whisper.option.small', null, 'Small — download (~480 MB)') },
    ],
    onChange: (value) => {
      setWhisperModel(value);
      updateWhisperUI();
    },
  });
  const whisperHint = el('p', 'settings-hint');
  const whisperDownload = button({ label: t('common.download', null, 'Download'), variant: 'quiet', size: 'sm' });
  whisperDownload.classList.add('hidden');
  const whisperRow = el('div', 'whisper-model-row');
  whisperRow.append(whisperHint, whisperDownload);
  const whisperFields = el('div');
  whisperFields.append(field(t('settings.voice.whisperModel', null, 'Whisper model'), whisperModel), whisperRow);

  let whisperStatus = null;
  let whisperStatusLoaded = false;
  let whisperPoll = null;

  function updateWhisperUI() {
    const current = getWhisperModel();
    whisperModel.select.value = current;
    whisperFields.classList.toggle('hidden', getSttEngine() !== 'whisper');

    const info = whisperStatus?.whisper_models?.[current];
    const download = whisperStatus?.whisper_downloads?.[current];
    const label = t(WHISPER_MODEL_LABEL[current], null, current);

    if (whisperStatus && whisperStatus.whisper !== 'ready') {
      whisperHint.textContent = t('settings.voice.whisperMissing', null, 'Faster Whisper is not running on this machine — voice falls back to Vosk until it is started (./voice/start_whisper.sh).');
      whisperDownload.classList.add('hidden');
      return;
    }
    if (!whisperStatus) {
      whisperHint.textContent = whisperStatusLoaded
        ? t('settings.voice.speechCheckFailed', null, 'Could not check the speech service.')
        : t('settings.voice.speechChecking', null, 'Checking the speech service…');
      whisperDownload.classList.add('hidden');
      return;
    }
    if (info?.present) {
      const note = current === 'tiny' ? t('settings.voice.defaultNote', null, ' (default)') : '';
      whisperHint.textContent = info.loaded
        ? t('settings.voice.whisper.readyLoaded', { model: label, note }, `${label} model ready and loaded${note}.`)
        : t('settings.voice.whisper.ready', { model: label, note }, `${label} model ready${note}.`);
      whisperDownload.classList.add('hidden');
      return;
    }
    if (download?.status === 'downloading') {
      const pct = download.total ? Math.round((download.bytes / download.total) * 100) : 0;
      whisperHint.textContent = t('common.downloadingModel', { model: label.toLowerCase(), percent: pct }, `Downloading the ${label.toLowerCase()} model… ${pct}%`);
      whisperDownload.classList.add('hidden');
      return;
    }
    if (download?.status === 'error') {
      whisperHint.textContent = download.error || t('common.downloadFailed', null, 'Download failed.');
      whisperDownload.classList.remove('hidden');
      whisperDownload.disabled = false;
      return;
    }
    whisperHint.textContent = t('settings.voice.whisper.notDownloaded', { model: label }, `${label} model is not downloaded yet — voice uses Tiny until you download it.`);
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
        if (present) {
          const label = t(WHISPER_MODEL_LABEL[current], null, current);
          toast(t('settings.voice.whisper.readyToast', { model: label }, `${label} model ready`), { type: 'info' });
        }
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
      toast(t('settings.voice.whisper.downloadingBackground', { model: current }, `Downloading the ${current} model in the background…`), { type: 'info' });
      pollWhisperDownload();
    } catch (e) {
      toast(e.message || t('common.downloadStartFailed', null, 'Could not start the download'), { type: 'error' });
      whisperDownload.disabled = false;
    }
  });

  const ttsVoice = select({
    options: [
      { value: '', label: t('common.defaultServer', null, 'Default (server)') },
      ...voicesForEngine(getTtsEngine()).map((v) => ({ value: v, label: v })),
    ],
    onChange: (value) => {
      setTtsVoice(value, getTtsEngine());
      ttsVoiceValue = value;
    },
  });

  // ── Speech output engine ────────────────────────────────────
  const ttsEngine = select({
    options: [
      { value: 'supertonic', label: t('settings.voice.ttsEngine.supertonic', null, 'Supertonic (default)') },
      { value: 'qwen', label: t('settings.voice.ttsEngine.qwen', null, 'Qwen3-TTS (experimental)') },
    ],
    onChange: (value) => {
      setTtsEngine(value);
      updateTtsUI();
      if (value === 'qwen' && qwenStatus && qwenStatus.qwen !== 'ready') {
        toast(t('settings.voice.qwenNotRunning', null, 'Qwen3-TTS is not running — Supertonic will keep speaking'), { type: 'info' });
      }
    },
  });

  const qwenModel = select({
    options: [
      { value: '0.6b-customvoice', label: t('settings.voice.qwen.option.06b', null, '0.6B CustomVoice — recommended (~605 MB)') },
      { value: '1.7b-customvoice', label: t('settings.voice.qwen.option.17b', null, '1.7B CustomVoice — better (~1.2 GB)') },
    ],
    onChange: (value) => {
      setQwenModel(value);
      updateTtsUI();
    },
  });
  const qwenHint = el('p', 'settings-hint');
  const qwenDownload = button({ label: t('common.download', null, 'Download'), variant: 'quiet', size: 'sm' });
  qwenDownload.classList.add('hidden');
  // Reload the sidecar after any pause (and for troubleshooting): kills the
  // process holding the port and re-runs the idempotent launcher.
  const qwenReload = button({ label: t('settings.voice.reload', null, 'Reload'), variant: 'quiet', size: 'sm' });
  on(qwenReload, 'click', async () => {
    qwenReload.disabled = true;
    try {
      await apiFetch('/api/voice/qwen/restart', { method: 'POST' });
      toast(t('settings.voice.qwenRestarting', null, 'Restarting Qwen3-TTS…'), { type: 'info' });
    } catch (e) {
      toast(e.message || t('settings.voice.qwenRestartFailed', null, 'Could not restart Qwen3-TTS'), { type: 'error' });
    } finally {
      setTimeout(() => { qwenReload.disabled = false; }, 1500);
    }
  });
  const qwenRow = el('div', 'whisper-model-row');
  qwenRow.append(qwenHint, qwenDownload, qwenReload);
  const qwenFields = el('div');
  qwenFields.append(field(t('settings.voice.qwenModel', null, 'Qwen model'), qwenModel), qwenRow);

  let qwenStatus = null;
  let qwenPoll = null;

  /**
   * The voice picker is engine-specific: Supertonic's M* / F* voices and the
   * Qwen3-TTS CustomVoice speakers share no names, so the options and the
   * chosen value are re-resolved whenever the engine changes. The value comes
   * from getTtsVoice(engine), which only returns a voice the engine knows —
   * each engine's pick is stored under its own preference key, so switching
   * engines back and forth keeps both choices.
   */
  function updateTtsUI() {
    const engine = getTtsEngine();
    ttsEngine.select.value = engine;
    qwenFields.classList.toggle('hidden', engine !== 'qwen');

    ttsVoice.setOptions([
      { value: '', label: t('common.defaultServer', null, 'Default (server)') },
      ...voicesForEngine(engine).map((v) => ({ value: v, label: v })),
    ]);
    const current = getTtsVoice(engine);
    ttsVoice.select.value = current;
    ttsVoiceValue = current;
  }
  let ttsVoiceValue = getTtsVoice(getTtsEngine());

  function updateQwenUI() {
    const current = getQwenModel();
    qwenModel.select.value = current;
    const info = qwenStatus?.qwen_models?.[current];
    const download = qwenStatus?.qwen_downloads?.[current];
    const label = t(QWEN_MODEL_LABEL[current], null, current);

    if (!qwenStatus) {
      qwenHint.textContent = t('settings.voice.speechChecking', null, 'Checking the speech service…');
      qwenDownload.classList.add('hidden');
      return;
    }
    if (info?.present) {
      const note = current === '0.6b-customvoice' ? t('settings.voice.defaultNote', null, ' (default)') : '';
      qwenHint.textContent = qwenStatus.qwen === 'ready'
        ? t('settings.voice.qwen.ready', { model: label, note }, `${label} model ready${note}.`)
        : t('settings.voice.qwen.readyStopped', { model: label, note }, `${label} model ready${note} — start the engine to use it.`);
      qwenDownload.classList.add('hidden');
      return;
    }
    if (download?.status === 'downloading') {
      const pct = download.total ? Math.round((download.bytes / download.total) * 100) : 0;
      qwenHint.textContent = t('common.downloadingModel', { model: label, percent: pct }, `Downloading the ${label} model… ${pct}%`);
      qwenDownload.classList.add('hidden');
      return;
    }
    if (download?.status === 'error') {
      qwenHint.textContent = download.error || t('common.downloadFailed', null, 'Download failed.');
      qwenDownload.classList.remove('hidden');
      qwenDownload.disabled = false;
      return;
    }
    qwenHint.textContent = t('settings.voice.qwen.notDownloaded', { model: label }, `${label} model is not downloaded yet — download it to use Qwen3-TTS.`);
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
        if (present) {
          const label = t(QWEN_MODEL_LABEL[current], null, current);
          toast(t('settings.voice.qwen.readyToast', { model: label }, `${label} model ready`), { type: 'info' });
        }
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
      const label = t(QWEN_MODEL_LABEL[current], null, current);
      toast(t('settings.voice.qwen.downloadingBackground', { model: label }, `Downloading ${label} in the background…`), { type: 'info' });
      pollQwenDownload();
    } catch (e) {
      toast(e.message || t('common.downloadStartFailed', null, 'Could not start the download'), { type: 'error' });
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
  const speed = sliderField(t('settings.voice.speechSpeed', null, 'Speech speed'), `${getTtsSpeed().toFixed(1)}×`, speedControl);

  const silence = select({
    options: [
      { value: '4000', label: t('settings.voice.seconds', { count: 4 }, '4 seconds') },
      { value: '6000', label: t('settings.voice.seconds', { count: 6 }, '6 seconds') },
      { value: '8000', label: t('settings.voice.seconds', { count: 8 }, '8 seconds') },
      { value: '12000', label: t('settings.voice.seconds', { count: 12 }, '12 seconds') },
      { value: '20000', label: t('settings.voice.seconds', { count: 20 }, '20 seconds') },
    ],
    onChange: (value) => setSilenceTimeout(Number(value)),
  });
  const silenceMs = getSilenceTimeout();
  if ([...silence.select.options].some((o) => Number(o.value) === silenceMs)) {
    silence.select.value = String(silenceMs);
  }

  const wake = toggleRow({
    label: t('settings.voice.wakeWord', null, 'Wake word'),
    hint: t('settings.voice.wakeWordHint', null, 'Long-press listens for “Hey…” before a command; when off it just listens.'),
    checked: getWakeWord(),
    onChange: (checked) => setWakeWord(checked),
  });

  if (ttsVoiceValue) ttsVoice.select.value = ttsVoiceValue;

  void (async () => {
    try {
      const res = await apiFetch('/api/voice/languages');
      lang.setOptions([
        { value: '', label: t('settings.voice.language.auto') },
        ...res.data.map((l) => ({
          value: l.code,
          label: `${l.code.toUpperCase()}${l.vosk_available ? '' : t('settings.voice.ttsOnly', null, ' (TTS only)')}`,
        })),
      ]);
      // The stored value is the *override*; the empty option means "follow the
      // interface language", which getVoiceLang() resolves at use time.
      const explicit = getVoiceLangExplicit();
      const codes = res.data.map((l) => l.code);
      if (explicit && codes.includes(explicit)) lang.select.value = explicit;
      else lang.select.value = codes.includes(getVoiceLang()) ? '' : 'en';
    } catch (_) {
      lang.setOptions([{ value: '', label: t('settings.voice.language.auto') }, { value: 'en', label: 'EN' }]);
      lang.select.value = '';
    }
  })();

  updateWhisperUI();
  void refreshVoiceStatus();

  return [
    field(t('settings.voice.language'), lang),
    langHint,
    heading(t('settings.voice.sttHeading', null, 'Speech recognition')),
    field(t('settings.voice.sttEngine', null, 'Engine'), engine),
    engineHint,
    whisperFields,
    heading(t('settings.voice.ttsHeading', null, 'Speech output')),
    field(t('settings.voice.ttsEngine', null, 'Engine'), ttsEngine, {
      hint: t('settings.voice.ttsEngineHint', null, 'Supertonic is bundled and always available. Qwen3-TTS sounds more natural, '
        + 'runs on your GPU through Vulkan when one is available, and needs a one-time '
        + 'build plus a model download.'),
    }),
    qwenFields,
    field(t('settings.voice.voice', null, 'Voice'), ttsVoice, {
      hint: t('settings.voice.voiceHint', null, 'The voice used for spoken replies.'),
    }),
    speed.wrap,
    field(t('settings.voice.stopListening', null, 'Stop listening after'), silence, {
      hint: t('settings.voice.stopListeningHint', null, 'How long to wait for speech before canceling a tap-to-talk input.'),
    }),
    wake,
  ];
}

/* ── Power ──────────────────────────────────────────────────── */

/* ── Images (local image generation) ────────────────────────── */

const IMAGE_QUALITY_BLURB = {
  fast: 'settings.images.quality.fast.blurb',
  balanced: 'settings.images.quality.balanced.blurb',
  high: 'settings.images.quality.high.blurb',
  max: 'settings.images.quality.max.blurb',
};

const IMAGE_MODELS = [
  {
    value: 'z-image',
    labelKey: 'settings.images.model.zImage',
    hintKey: 'settings.images.model.zImage.hint',
  },
  {
    value: 'qwen-image-2.1',
    labelKey: 'settings.images.model.qwenImage',
    hintKey: 'settings.images.model.qwenImage.hint',
  },
];

const IMAGE_QUALITY_OPTIONS = [
  { value: 'fast', labelKey: 'settings.images.quality.fast' },
  { value: 'balanced', labelKey: 'settings.images.quality.balanced' },
  { value: 'high', labelKey: 'settings.images.quality.high' },
  { value: 'max', labelKey: 'settings.images.quality.max' },
];

function buildImage() {
  const model = select({
    options: IMAGE_MODELS.map((m) => ({ value: m.value, label: t(m.labelKey) })),
    onChange: (value) => {
      setImageModel(value);
      updateImageUI();
    },
  });
  model.select.value = getImageModel();

  const quality = select({
    options: IMAGE_QUALITY_OPTIONS.map((o) => ({ value: o.value, label: t(o.labelKey) })),
    onChange: (value) => {
      setImageQuality(value);
      updateImageUI();
    },
  });

  const modelHint = el('p', 'settings-hint');
  const hint = el('p', 'settings-hint');
  const download = button({ label: t('common.download', null, 'Download'), variant: 'quiet', size: 'sm' });
  download.classList.add('hidden');
  const row = el('div', 'whisper-model-row');
  row.append(hint, download);

  let status = null;
  let statusLoaded = false;
  let poll = null;

  const modelDef = (id) => IMAGE_MODELS.find((m) => m.value === id) || IMAGE_MODELS[0];
  const familyInfo = (id) => status?.inventory?.families?.[id] || null;
  const infoFor = (id, key) => familyInfo(id)?.qualities?.[key] || null;
  const downloadFor = (id, key) => status?.downloads?.[id]?.[key] || null;
  const baseFor = (id) => status?.downloads?.[id]?.base || null;
  const describe = (id, key) => {
    const info = infoFor(id, key);
    const blurb = IMAGE_QUALITY_BLURB[key];
    return info ? `${info.quant} · ${info.size} · ${blurb ? t(blurb) : ''}` : '';
  };

  function updateImageUI() {
    const currentModel = getImageModel();
    const current = getImageQuality();
    model.select.value = currentModel;
    quality.select.value = current;
    modelHint.textContent = t(modelDef(currentModel).hintKey);
    const info = infoFor(currentModel, current);
    const base = baseFor(currentModel);

    if (!status) {
      hint.textContent = statusLoaded
        ? t('settings.images.checkFailed', null, 'Could not check the image models.')
        : t('settings.images.checking', null, 'Checking the image models…');
      download.classList.add('hidden');
      return;
    }
    if (!status.inventory?.sd_cli) {
      hint.textContent = t('settings.images.unavailable', null, 'Local image generation is unavailable: the sd-cli binary was not found (set Z_IMAGE_SD_CLI).');
      download.classList.add('hidden');
      return;
    }
    if (base?.status === 'downloading') {
      const pct = base.total ? Math.round((base.bytes / base.total) * 100) : 0;
      hint.textContent = t('common.downloadingShared', { percent: pct }, `Downloading the shared model files… ${pct}%`);
      download.classList.add('hidden');
      return;
    }
    const dl = downloadFor(currentModel, current);
    if (dl?.status === 'downloading') {
      const pct = dl.total ? Math.round((dl.bytes / dl.total) * 100) : 0;
      hint.textContent = t('common.downloadingModel', { model: current, percent: pct }, `Downloading the ${current} model… ${pct}%`);
      download.classList.add('hidden');
      return;
    }
    if (dl?.status === 'error') {
      hint.textContent = dl.error || t('common.downloadFailed', null, 'Download failed.');
      download.classList.remove('hidden');
      download.disabled = false;
      return;
    }
    if (info?.ready) {
      hint.textContent = t('settings.images.ready', { details: describe(currentModel, current) }, `Ready — ${describe(currentModel, current)}. Images run locally on this machine; larger sizes take minutes.`);
      download.classList.add('hidden');
      return;
    }
    const missing = [];
    for (const shared of familyInfo(currentModel)?.shared || []) {
      if (!shared.present) missing.push(shared.label);
    }
    if (!info?.present) missing.push(describe(currentModel, current) || current);
    hint.textContent = t('common.notDownloaded', { missing: missing.join(', ') }, `Not downloaded yet — ${missing.join(', ')}. The assistant asks before fetching.`);
    download.classList.remove('hidden');
    download.disabled = false;
  }

  async function refreshImageStatus() {
    try {
      const res = await apiFetch('/api/imagegen/status');
      status = res?.data || null;
    } catch (_) {
      status = null;
    }
    statusLoaded = true;
    updateImageUI();
    return status;
  }

  function pollDownload() {
    clearInterval(poll);
    poll = setInterval(async () => {
      await refreshImageStatus();
      const currentModel = getImageModel();
      const current = getImageQuality();
      const state = downloadFor(currentModel, current)?.status;
      if (infoFor(currentModel, current)?.ready || state === 'error') {
        clearInterval(poll);
        poll = null;
        if (infoFor(currentModel, current)?.ready) toast(t('settings.images.modelReady', null, 'Image model ready'), { type: 'info' });
      }
    }, 1500);
  }

  on(download, 'click', async () => {
    const currentModel = getImageModel();
    const current = getImageQuality();
    download.disabled = true;
    try {
      await apiFetch('/api/imagegen/download', {
        method: 'POST',
        body: JSON.stringify({ model: currentModel, quality: current }),
      });
      toast(t('settings.images.downloadingBackground', null, 'Downloading image models in the background…'), { type: 'info' });
      pollDownload();
    } catch (e) {
      toast(e.message || t('common.downloadStartFailed', null, 'Could not start the download'), { type: 'error' });
      download.disabled = false;
    }
  });

  refreshImageStatus();
  cleanups.push(() => { clearInterval(poll); poll = null; });

  const modelField = field(t('settings.images.model', null, 'Model'), model);
  modelField.appendChild(modelHint);
  return [
    modelField,
    field(t('common.qualityModel', null, 'Quality model'), quality),
    row,
  ];
}

/* ── Music (local song generation) ──────────────────────────── */

const SONG_QUALITY_BLURB = {
  q4: 'settings.music.quality.q4.blurb',
  q8: 'settings.music.quality.q8.blurb',
};

const SONG_QUALITY_OPTIONS = [
  { value: 'q4', labelKey: 'settings.music.quality.q4' },
  { value: 'q8', labelKey: 'settings.music.quality.q8' },
];

function buildMusic() {
  const quality = select({
    options: SONG_QUALITY_OPTIONS.map((o) => ({ value: o.value, label: t(o.labelKey) })),
    onChange: (value) => {
      setSongQuality(value);
      updateMusicUI();
    },
  });

  const hint = el('p', 'settings-hint');
  const download = button({ label: t('common.download', null, 'Download'), variant: 'quiet', size: 'sm' });
  download.classList.add('hidden');
  const row = el('div', 'whisper-model-row');
  row.append(hint, download);

  let status = null;
  let statusLoaded = false;
  let poll = null;

  const infoFor = (key) => status?.inventory?.qualities?.[key] || null;
  const downloadFor = (key) => status?.downloads?.[key] || null;
  const baseFor = () => status?.downloads?.base || null;
  const describe = (key) => {
    const info = infoFor(key);
    const blurb = SONG_QUALITY_BLURB[key];
    return info ? `${info.quant} · ${info.size} · ${blurb ? t(blurb) : ''}` : '';
  };

  function updateMusicUI() {
    const current = getSongQuality();
    quality.select.value = current;
    const info = infoFor(current);
    const base = baseFor();

    if (!status) {
      hint.textContent = statusLoaded
        ? t('settings.music.checkFailed', null, 'Could not check the song models.')
        : t('settings.music.checking', null, 'Checking the song models…');
      download.classList.add('hidden');
      return;
    }
    if (!status.inventory?.engine) {
      hint.textContent = t('settings.music.unavailable', null, 'Local song generation is unavailable: the ace-server engine was not found (set YUE2_ENGINE or run scripts/install-yue2-engine.sh).');
      download.classList.add('hidden');
      return;
    }
    if (base?.status === 'downloading') {
      const pct = base.total ? Math.round((base.bytes / base.total) * 100) : 0;
      hint.textContent = t('common.downloadingShared', { percent: pct }, `Downloading the shared model files… ${pct}%`);
      download.classList.add('hidden');
      return;
    }
    const dl = downloadFor(current);
    if (dl?.status === 'downloading') {
      const pct = dl.total ? Math.round((dl.bytes / dl.total) * 100) : 0;
      hint.textContent = t('common.downloadingModel', { model: current, percent: pct }, `Downloading the ${current} model… ${pct}%`);
      download.classList.add('hidden');
      return;
    }
    if (dl?.status === 'error') {
      hint.textContent = dl.error || t('common.downloadFailed', null, 'Download failed.');
      download.classList.remove('hidden');
      download.disabled = false;
      return;
    }
    if (info?.ready) {
      hint.textContent = t('settings.music.ready', { details: describe(current) }, `Ready — ${describe(current)}. Songs render locally and take several minutes.`);
      download.classList.add('hidden');
      return;
    }
    const missing = [];
    for (const shared of status.inventory?.shared || []) {
      if (!shared.present) missing.push(shared.label);
    }
    if (!info?.present) missing.push(describe(current) || current);
    hint.textContent = t('common.notDownloaded', { missing: missing.join(', ') }, `Not downloaded yet — ${missing.join(', ')}. The assistant asks before fetching.`);
    download.classList.remove('hidden');
    download.disabled = false;
  }

  async function refreshMusicStatus() {
    try {
      const res = await apiFetch('/api/songgen/status');
      status = res?.data || null;
    } catch (_) {
      status = null;
    }
    statusLoaded = true;
    updateMusicUI();
    return status;
  }

  function pollDownload() {
    clearInterval(poll);
    poll = setInterval(async () => {
      await refreshMusicStatus();
      const current = getSongQuality();
      const state = downloadFor(current)?.status;
      if (infoFor(current)?.ready || state === 'error') {
        clearInterval(poll);
        poll = null;
        if (infoFor(current)?.ready) toast(t('settings.music.modelReady', null, 'Song model ready'), { type: 'info' });
      }
    }, 1500);
  }

  on(download, 'click', async () => {
    const current = getSongQuality();
    download.disabled = true;
    try {
      await apiFetch('/api/songgen/download', {
        method: 'POST',
        body: JSON.stringify({ quality: current }),
      });
      toast(t('settings.music.downloadingBackground', null, 'Downloading the song models in the background…'), { type: 'info' });
      pollDownload();
    } catch (e) {
      toast(e.message || t('common.downloadStartFailed', null, 'Could not start the download'), { type: 'error' });
      download.disabled = false;
    }
  });

  refreshMusicStatus();
  cleanups.push(() => { clearInterval(poll); poll = null; });

  const qualityField = field(t('common.qualityModel', null, 'Quality model'), quality);
  qualityField.appendChild(el('p', 'settings-hint', t('settings.music.hint', null, 'YuE2 turns style + lyrics into a 48 kHz stereo song on this machine.')));
  return [qualityField, row];
}

function buildPower() {
  const mode = select({
    options: POWER_MODES.map((m) => ({ value: m, label: powerModeLabel(m) })),
    onChange: (value) => {
      setPowerMode(value);
      refreshPowerSummary();
    },
  });
  mode.select.value = getPowerMode();
  const modeField = field(t('power.mode'), mode);
  const modeHint = el('p', 'settings-hint', powerModeHint(getPowerMode()));
  modeField.appendChild(modeHint);

  const auto = toggleRow({
    label: t('power.autoSaver'),
    hint: t('settings.power.autoSaverHint', null, 'In Balanced, switch to Power Saver when on battery or below 20%.'),
    checked: getAutoPowerSaver(),
    onChange: (on) => {
      setAutoPowerSaver(on);
      refreshPowerSummary();
    },
  });

  const lowAi = toggleRow({
    label: t('power.lowAi'),
    hint: t('settings.power.lowAiHint', null, 'Let Power Saver use Vosk for speech recognition and Supertonic for replies '
      + 'instead of faster-whisper and Qwen3-TTS.'),
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
        status.percent != null ? `${status.percent}%` : t('battery.label'),
        status.state || null,
        status.ac_online === true ? t('settings.power.pluggedIn', null, 'plugged in') : null,
      ].filter(Boolean).join(' · ')
      : t('battery.none');
    const effective = getEffectivePowerMode();
    const engines = describeEngines(getEffectiveSttEngine(), getEffectiveTtsEngine());
    modeHint.textContent = powerModeHint(getPowerMode());
    summary.textContent = effective === 'saver'
      ? t('settings.power.summary.saver', { battery, engines }, `${battery}. Power Saver is in force — ${engines}.`)
      : t('settings.power.summary.mode', { battery, mode: powerModeLabel(effective), engines }, `${battery}. ${powerModeLabel(effective)} — full-quality AI (${engines}).`);
  }
  refreshPowerSummary();
  on(window, 'power:settings', refreshPowerSummary);
  on(window, 'battery:changed', refreshPowerSummary);

  return [
    summary,
    modeField,
    auto,
    lowAi,
    heading(t('common.display', null, 'Display')),
    backlightField(
      t('settings.power.screenBrightness', null, 'Screen brightness'),
      '/api/screen/brightness',
      t('settings.power.screenBacklightUnavailable', null, 'This machine does not expose a writable panel backlight to the app.'),
    ),
    backlightField(
      t('settings.power.keyboardBacklight', null, 'Keyboard backlight'),
      '/api/keyboard/backlight',
      t('settings.power.keyboardBacklightUnavailable', null, 'This machine does not expose a writable keyboard backlight to the app.'),
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
        value.textContent = t('settings.power.unavailable', null, 'Unavailable');
        wrap.appendChild(el('p', 'settings-hint', unavailableHint));
        return;
      }
      control.value = String(s.percent);
      value.textContent = `${s.percent}%`;
      if (s.writable === false) {
        control.disabled = true;
        wrap.appendChild(el('p', 'settings-hint', t('settings.power.backlightReadOnly', null, 'The app cannot write this backlight on this machine.')));
      }
    } catch (_) {
      control.disabled = true;
      value.textContent = t('settings.power.unavailable', null, 'Unavailable');
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
      { value: 'master', label: t('settings.desktop.mode.tiling', null, 'Tiling') },
      { value: 'columns', label: t('settings.desktop.mode.columns', null, 'Columns') },
      { value: 'windows', label: t('settings.desktop.mode.windows', null, 'Windows') },
    ],
    onChange: (value) => {
      setDesktopLayout({ ...getDesktopLayout(), mode: value });
      syncTilingControls();
    },
  });
  mode.select.value = layout.mode;

  const orientation = select({
    options: [
      { value: 'left', label: t('common.left', null, 'Left') },
      { value: 'right', label: t('common.right', null, 'Right') },
      { value: 'top', label: t('common.top', null, 'Top') },
      { value: 'bottom', label: t('common.bottom', null, 'Bottom') },
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
  const ratio = sliderField(t('settings.desktop.masterSize', null, 'Master size'), `${Math.round(layout.master_ratio * 100)}%`, ratioControl);

  const gapControl = slider({
    min: 0, max: 40, step: 2, value: layout.gap,
    onInput: (value) => {
      gap.value.textContent = `${value}px`;
      setLayout({ gap: value });
    },
  });
  const gap = sliderField(t('settings.desktop.gap', null, 'Gap'), `${layout.gap}px`, gapControl);
  gap.wrap.appendChild(el('p', 'settings-hint', t('settings.desktop.gapHint', null, 'Window gaps — and the frame around them: the top bar\u2019s margins, the space to the workspace switcher and the window-to-screen edges.')));

  const tilingControls = el('div', 'settings-tiling-controls');
  tilingControls.append(field(t('settings.desktop.masterSide', null, 'Master side'), orientation), ratio.wrap, gap.wrap);

  const syncTilingControls = () => {
    tilingControls.classList.toggle('hidden', mode.select.value === 'windows');
  };
  syncTilingControls();

  const shortcuts = el('p', 'settings-hint');
  shortcuts.innerHTML = t('settings.desktop.shortcuts', null, 'Shortcuts: <strong>Alt+Enter</strong> fullscreen · <strong>Alt+H/L</strong> focus · <strong>Alt+,/.</strong> workspace · <strong>Alt+1..9</strong> jump · <strong>Alt+N</strong> new · <strong>Alt+Shift+N</strong> remove.');

  return [
    field(t('settings.desktop.layout', null, 'Layout'), mode, {
      hint: t('settings.desktop.layoutHint', null, 'Tiling snaps windows into workspaces; Windows floats them as draggable, resizable desktop windows.'),
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
  const title = sliderField(t('settings.appearance.titleBarHeight', null, 'Title bar height'), `${surface.title_height}px`, titleControl);

  const radiusControl = slider({
    min: 0, max: 32, step: 1, value: surface.window_radius,
    onInput: (value) => {
      radius.value.textContent = `${value}px`;
      setDesktopSurface({ window_radius: value });
    },
  });
  const radius = sliderField(t('settings.appearance.windowCorners', null, 'Window corners'), `${surface.window_radius}px`, radiusControl);

  const opacityControl = slider({
    min: 60, max: 100, step: 1, value: surface.window_opacity,
    onInput: (value) => {
      opacity.value.textContent = `${value}%`;
      setDesktopSurface({ window_opacity: value });
    },
  });
  const opacity = sliderField(t('settings.appearance.windowOpacity', null, 'Window opacity'), `${surface.window_opacity}%`, opacityControl);

  const blurControl = slider({
    min: 0, max: 24, step: 1, value: surface.glass_blur,
    onInput: (value) => {
      blur.value.textContent = `${value}px`;
      setDesktopSurface({ glass_blur: value });
    },
  });
  const blur = sliderField(t('settings.appearance.glassBlur', null, 'Glass blur'), `${surface.glass_blur}px`, blurControl);

  const background = toggleRow({
    label: t('settings.appearance.windowBackground', null, 'Blurred window background'),
    hint: t('settings.appearance.windowBackgroundHint', null, 'The soft blurred mirror behind a window — its photo, PDF page, artwork or the accent glow. Turn off for flat, fully opaque-looking windows.'),
    checked: surface.window_background,
    onChange: (checked) => setDesktopSurface({ window_background: checked }),
  });

  const shadow = toggleRow({
    label: t('settings.appearance.windowShadows', null, 'Window shadows'),
    hint: t('settings.appearance.windowShadowsHint', null, 'Give windows a soft drop shadow instead of a flat hairline.'),
    checked: surface.window_shadow,
    onChange: (checked) => setDesktopSurface({ window_shadow: checked }),
  });

  return [
    heading(t('settings.appearance.windowSurface', null, 'Window surface')),
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
    label: t('settings.appearance.autohideBar', null, 'Autohide the top bar'),
    hint: t('settings.appearance.autohideBarHint', null, 'Slide the top bar away until the pointer reaches the edge it docks to.'),
    checked: immersive.autohide_bar,
    onChange: (checked) => setImmersive({ autohide_bar: checked }),
  });
  const barPosition = select({
    options: [
      { value: 'top', label: t('common.top', null, 'Top') },
      { value: 'left', label: t('common.left', null, 'Left') },
      { value: 'right', label: t('common.right', null, 'Right') },
      { value: 'center', label: t('common.center', null, 'Center') },
    ],
    onChange: (value) => setImmersive({ bar_position: value }),
  });
  barPosition.select.value = immersive.bar_position;
  const autohideOrb = toggleRow({
    label: t('settings.appearance.autohideOrb', null, 'Autohide the voice bar'),
    hint: t('settings.appearance.autohideOrbHint', null, 'Slide the voice bar away until the pointer reaches the bottom centre.'),
    checked: immersive.autohide_orb,
    onChange: (checked) => setImmersive({ autohide_orb: checked }),
  });

  return [
    heading(t('settings.appearance.topBarHeading', null, 'Top bar & voice bar')),
    autohideBar,
    field(t('settings.appearance.topBarPosition', null, 'Top bar position'), barPosition),
    autohideOrb,
  ];
}

/* Content of the top-bar host status chips: the device label and the numeric
 * percentage (volume / signal / battery) can each be hidden. Both off leaves
 * an icon-only button — see hudChipsShared.js + preferences.js. */
function buildHudChipControls() {
  const chips = getHudChips();
  const names = toggleRow({
    label: t('settings.appearance.deviceNames', null, 'Device names'),
    hint: t('settings.appearance.deviceNamesHint', null, 'Show the device or network name in the sound, network, Bluetooth and battery buttons.'),
    checked: chips.name,
    onChange: (checked) => setHudChips({ name: checked }),
  });
  const percent = toggleRow({
    label: t('settings.appearance.percentages', null, 'Percentages'),
    hint: t('settings.appearance.percentagesHint', null, 'Show volume, signal and battery numbers. Turn both off for icon-only buttons.'),
    checked: chips.percent,
    onChange: (checked) => setHudChips({ percent: checked }),
  });

  return [
    heading(t('settings.appearance.hostStatusChips', null, 'Host status chips')),
    el('p', 'settings-hint', t('settings.appearance.hostStatusChipsHint', null, 'Choose what the sound, network, Bluetooth and battery buttons in the top bar show.')),
    names,
    percent,
  ];
}

/* ── System ─────────────────────────────────────────────────── */

function buildSystem() {
  // Interface language. Applied live: the window re-renders itself through its
  // `locale:change` listener, so the new language is visible on the spot.
  const locale = select({
    options: [
      { value: '', label: t('settings.system.interfaceLanguage.auto', null, 'Automatic (system)') },
      ...getAvailableLocales().map((l) => ({ value: l.code, label: l.name || l.code })),
    ],
    value: getUiLocale(),
    onChange: (value) => setUiLocale(value),
  });
  const localeField = field(t('settings.system.interfaceLanguage', null, 'Interface language'), locale, {
    hint: t('settings.system.interfaceLanguage.hint', null, 'Applies to the interface — menus, window chrome and labels — and to the voice (speech recognition and replies) unless a different one is chosen under Voice.'),
  });

  const remember = toggleRow({
    label: t('settings.system.remember', null, 'Remember my workspace'),
    hint: t('settings.system.rememberHint', null, 'Restore your windows, layout and workspaces from last time.'),
    checked: getRemember(),
    onChange: (checked) => setRemember(checked),
  });

  const logoutBtn = button({
    label: t('settings.system.logOut', null, 'Log out'),
    variant: 'danger',
    onClick: async () => {
      try {
        await flushPreferencesNow();
      } catch (_) { /* proceed */ }
      await logoutSession();
      // In the kiosk, hand the seat back to the machine's login screen: the
      // shell exits with its sign-out status and the session supervisor starts
      // the greeter. A plain browser has no bridge and reloads to the app's
      // own login screen.
      if (window.ipc?.postMessage) {
        window.ipc.postMessage('peakd:logout');
        return;
      }
      window.location.href = '/';
    },
  });
  const logoutHint = el('p', 'settings-hint', t('settings.system.logOutHint', null, 'Logging out returns you to the sign-in screen. Your plugins and chats stay as you set them.'));
  const actions = el('div', 'settings-inline-actions');
  actions.appendChild(logoutBtn);

  return [
    localeField,
    remember,
    el('p', 'settings-hint', t('settings.system.rememberBody', null, 'On: your windows, layout and workspaces come back exactly as you left them. Off: each sign-in starts with a clean desktop.')),
    heading(t('settings.system.session', null, 'Session')),
    actions,
    logoutHint,
  ];
}

/* ── Touch Bar ──────────────────────────────────────────────── */

function buildTouchBar() {
  const mode = select({
    options: [
      { value: 'auto', label: t('settings.touchbar.mode.auto', null, 'Automatic') },
      { value: 'on', label: t('settings.touchbar.mode.on', null, 'Always on') },
      { value: 'off', label: t('settings.touchbar.mode.off', null, 'Off') },
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
        ? t('settings.touchbar.presentOff', null, 'A Touch Bar is present, but input is turned off.')
        : t('settings.touchbar.presentOn', null, 'A Touch Bar is present and its buttons are active.');
    } else if (setting === 'on') {
      state.textContent = t('settings.touchbar.forcedOn', null, 'No Touch Bar detected, but the buttons are forced on — press Ctrl+Alt+Shift with 1–9 or the − = , . keys to try them.');
    } else {
      state.textContent = t('settings.touchbar.notDetected', null, 'No Touch Bar detected on this machine. The buttons stay dormant, so a normal PC is unaffected.');
    }
    buttons.textContent = TOUCHBAR_ACTIONS
      .map((entry) => `${entry.label} (${entry.code})`)
      .join(' · ');
    install.textContent = host
      ? t('settings.touchbar.installHost', null, 'On a T2 Mac running Linux, scripts/touchbar/install-touchbar.sh installs this row through tiny-dfr.')
      : t('settings.touchbar.install', null, 'On a T2 Mac running Linux, scripts/touchbar/install-touchbar.sh puts this row on the bar through tiny-dfr.');
  }

  refreshTouchBarHint();
  // A change made elsewhere (or the host reporting detection) re-renders it.
  on(window, 'touchbar:settings', refreshTouchBarHint);

  return [
    field(t('settings.touchbar.setting', null, 'Touch Bar'), mode, {
      hint: t('settings.touchbar.settingHint', null, 'Show the assistant controls on a MacBook Touch Bar. Automatic only uses the bar when this machine has one.'),
    }),
    state,
    heading(t('settings.touchbar.buttons', null, 'Buttons')),
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
    label: t('settings.remote.server', null, 'Server'),
    hint: t('settings.remote.serverHint', null, 'Let your other devices reach this desktop over Iroh — end-to-end encrypted, no port forwarding. The link changes every time you start or stop.'),
    checked: false,
    onChange: setEnabled,
  });

  const terminalToggle = toggleRow({
    label: t('settings.remote.allowTerminal', null, 'Allow Terminal from remote clients'),
    hint: t('settings.remote.allowTerminalHint', null, 'The Terminal is a real shell on this machine. Off by default; enable it only for devices you trust.'),
    checked: getRemoteAllowTerminal(),
    onChange: (on) => { setRemoteAllowTerminal(on); },
  });

  const statusLine = el('p', 'settings-hint', t('settings.remote.reading', null, 'Reading…'));
  const ticketEl = el('code', 'remote-ticket');
  const qrEl = el('img', 'remote-qr');
  qrEl.alt = t('settings.remote.qrAlt', null, 'Connection QR code');

  const revealBtn = button({
    label: t('settings.remote.revealLink', null, 'Reveal link'),
    variant: 'ghost',
    onClick: () => { revealed = !revealed; render(); },
  });
  const copyBtn = button({
    label: t('common.copy'),
    variant: 'ghost',
    onClick: async () => {
      if (!state.ticket) return;
      // Copies go through the central service so they land in the history.
      const ok = await copyText(state.ticket, { source: 'settings' });
      toast(ok ? t('settings.remote.linkCopied', null, 'Link copied') : t('settings.remote.copyFailed', null, 'Could not copy the link'), ok ? undefined : { type: 'error' });
    },
  });
  const rotateBtn = button({ label: t('settings.remote.rotateKey', null, 'Rotate key'), variant: 'danger', onClick: rotate });
  const pairBtn = button({ label: t('settings.remote.pairDevice', null, 'Pair a new device'), variant: 'ghost', onClick: pair });
  const unpairBtn = button({ label: t('settings.remote.forgetDevices', null, 'Forget devices'), variant: 'quiet', onClick: unpair });

  const actions = el('div', 'settings-inline-actions');
  actions.append(revealBtn, copyBtn, rotateBtn, pairBtn, unpairBtn);

  const linkBox = el('div', 'remote-link');
  linkBox.append(ticketEl, qrEl);
  const linkHint = el('p', 'settings-hint', t('settings.remote.linkHint', null, 'On another device: peakd --iroh <link> opens the app in a window. To use a plain browser instead, turn on the public URL below.'));

  // ── Tailscale Funnel: a normal https:// URL for any browser ──────────
  const tsToggle = toggleRow({
    label: t('settings.remote.publicUrl', null, 'Public URL (Tailscale Funnel)'),
    hint: t('settings.remote.publicUrlHint', null, 'Serve the app at an https://…ts.net URL through Tailscale — open it in any browser, on any device, no app or port forwarding. Free; needs Tailscale installed and signed in.'),
    checked: false,
    onChange: setTailscale,
  });
  const tsMissing = el('p', 'settings-hint', t('settings.remote.tailscaleMissing', null, 'Tailscale is not installed on this machine.'));
  const tsStatus = el('p', 'settings-hint', '');
  const tsUrlEl = el('code', 'remote-ticket');
  const tsQrEl = el('img', 'remote-qr');
  tsQrEl.alt = t('settings.remote.tailscaleQrAlt', null, 'Tailscale URL QR code');
  const tsUrlBox = el('div', 'remote-link');
  tsUrlBox.append(tsUrlEl, tsQrEl);
  const tsCopyBtn = button({ label: t('settings.remote.copyUrl', null, 'Copy URL'), variant: 'ghost', onClick: copyTailscale });
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
      statusLine.textContent = t('settings.remote.offLocalOnly', null, 'Off. This desktop is reachable only on this machine.');
      linkBox.classList.add('hidden');
      actions.classList.add('hidden');
      return;
    }
    const id = state.endpointId ? `${state.endpointId.slice(0, 12)}…` : '';
    const connections = tp('settings.remote.connections', state.connections, null, `${state.connections} connection${state.connections === 1 ? '' : 's'}`);
    let line = t('settings.remote.statusOn', { id, connections, bytes: formatBytes(state.bytes) }, `On — ${id} · ${connections} · ${formatBytes(state.bytes)}`);
    if (state.paired > 0) line += ` · ${tp('settings.remote.paired', state.paired, null, `${state.paired} paired device${state.paired === 1 ? '' : 's'}`)}`;
    if (state.pairing) line += ` · ${t('settings.remote.pairing', null, 'pairing for the next device…')}`;
    statusLine.textContent = line;
    linkBox.classList.remove('hidden');
    actions.classList.remove('hidden');
    revealBtn.textContent = revealed ? t('settings.remote.hideLink', null, 'Hide link') : t('settings.remote.revealLink', null, 'Reveal link');
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
      tsStatus.textContent = ts && ts.error ? t('settings.remote.tailscaleError', { error: ts.error }, `Tailscale: ${ts.error}`) : '';
      tsUrlBox.classList.add('hidden');
      tsActions.classList.add('hidden');
      return;
    }
    const on = !!ts.funnel;
    tsToggle.toggle.setChecked(on, { silent: true });
    let line;
    if (!ts.up) line = t('settings.remote.tailscaleDown', null, 'Installed, but not connected — run `tailscale up` on this machine.');
    else if (on) line = t('settings.remote.tailscaleOn', { host: ts.hostname || t('settings.remote.thisDevice', null, 'this device') }, `On — ${ts.hostname || 'this device'}`);
    else line = t('settings.remote.tailscaleOff', null, 'Off. The app is reachable only locally and over Iroh.');
    if (ts.error) line += t('settings.remote.errorNote', { error: ts.error }, ` (${ts.error})`);
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
      toast(on ? t('settings.remote.funnelOn', null, 'Tailscale Funnel on') : t('settings.remote.funnelOff', null, 'Tailscale Funnel off'));
      await refresh();
    } catch (e) {
      tsToggle.toggle.setChecked(!on, { silent: true });
      toast(e.message || t('settings.remote.funnelFailed', null, 'Could not change Tailscale Funnel'), { type: 'error' });
    }
  }

  async function copyTailscale() {
    const url = state.tailscale && state.tailscale.url;
    if (!url) return;
    const ok = await copyText(url, { source: 'settings' });
    toast(ok ? t('settings.remote.urlCopied', null, 'URL copied') : t('settings.remote.urlCopyFailed', null, 'Could not copy the URL'), ok ? undefined : { type: 'error' });
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
      toast(on ? t('settings.remote.serverOn', null, 'Server mode on') : t('settings.remote.serverOff', null, 'Server mode off'));
    } catch (e) {
      serverToggle.toggle.setChecked(!on, { silent: true });
      toast(e.message || t('settings.remote.serverFailed', null, 'Could not change server mode'), { type: 'error' });
    }
  }

  async function rotate() {
    try {
      apply(await apiFetch('/api/remote/rotate', { method: 'POST', authRedirect: false }));
      revealed = false;
      render();
      toast(t('settings.remote.newLink', null, 'New link generated'));
    } catch (e) {
      toast(e.message || t('settings.remote.rotateFailed', null, 'Could not rotate the link'), { type: 'error' });
    }
  }

  async function pair() {
    try {
      const res = await apiFetch('/api/remote/pair', { method: 'POST', authRedirect: false });
      toast(t('settings.remote.pairingOpen', { seconds: res?.seconds || 120 }, `Pairing open for ${res?.seconds || 120}s — connect from the new device now`));
      refresh();
    } catch (e) {
      toast(e.message || t('settings.remote.pairFailed', null, 'Could not start pairing'), { type: 'error' });
    }
  }

  async function unpair() {
    try {
      const res = await apiFetch('/api/remote/unpair', { method: 'POST', authRedirect: false });
      toast(tp('settings.remote.forgotDevices', res?.removed ?? 0, null, `Forgot ${res?.removed ?? 0} paired device(s)`));
      refresh();
    } catch (e) {
      toast(e.message || t('settings.remote.forgetFailed', null, 'Could not forget devices'), { type: 'error' });
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
    heading(t('settings.remote.publicUrlHeading', null, 'Public URL (Tailscale)')),
    tsMissing,
    tsToggle,
    tsStatus,
    tsUrlBox,
    tsActions,
    heading(t('settings.remote.remoteControl', null, 'Remote control')),
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
  holder.appendChild(el('p', 'settings-hint', t('settings.browser.loading', null, 'Loading browser settings…')));

  void (async () => {
    let settings = { search_engine: 'duckduckgo', adblock: true, downloads_dir: '' };
    // The link handler is a core preference, not a browser-plugin setting, and
    // its candidates come from the installed plugins — fetch both alongside.
    const [settingsRes, catalog] = await Promise.all([
      apiFetch('/api/browser/settings').catch(() => null),
      linkCatalog(),
    ]);
    settings = { ...settings, ...(settingsRes?.data || {}) };

    const engine = select({
      value: settings.search_engine,
      options: [
        { value: 'duckduckgo', label: t('settings.browser.engine.duckduckgo', null, 'DuckDuckGo (default)') },
        { value: 'brave', label: t('settings.browser.engine.brave', null, 'Brave Search') },
        { value: 'google', label: t('settings.browser.engine.google', null, 'Google') },
        { value: 'bing', label: t('settings.browser.engine.bing', null, 'Bing') },
      ],
      onChange: (value) => {
        void apiFetch('/api/browser/settings', {
          method: 'POST',
          body: JSON.stringify({ search_engine: value }),
        })
          .then(() => toast(t('settings.browser.engineSaved', null, 'Search engine saved')))
          .catch(() => toast(t('settings.browser.engineSaveFailed', null, 'Could not save the search engine'), { type: 'error' }));
      },
    });

    const adblock = toggleRow({
      label: t('settings.browser.adblock', null, 'Block ads and trackers'),
      hint: t('settings.browser.adblockHint', null, 'The Browser window filters page requests through the shell.'),
      checked: settings.adblock !== false,
      onChange: (checked) => {
        void apiFetch('/api/browser/settings', {
          method: 'POST',
          body: JSON.stringify({ adblock: checked }),
        }).catch(() => {});
      },
    });

    const open = button({
      label: t('settings.browser.open', null, 'Open Browser'),
      variant: 'ghost',
      onClick: () => openCoreWindow('browser'),
    });
    const clear = button({
      label: t('settings.browser.clearHistory', null, 'Clear history'),
      variant: 'ghost',
      onClick: async () => {
        try {
          await apiFetch('/api/browser/history/clear', { method: 'POST' });
          toast(t('settings.browser.historyCleared', null, 'Browsing history cleared'));
        } catch (_) {
          toast(t('settings.browser.clearFailed', null, 'Could not clear history'), { type: 'error' });
        }
      },
    });
    const actions = el('div', 'settings-inline-actions');
    actions.append(open, clear);

    const storage = el(
      'p',
      'settings-hint',
      settings.downloads_dir
        ? t('settings.browser.downloadsDir', { path: settings.downloads_dir }, `Downloads are saved to ${settings.downloads_dir}`)
        : t('settings.browser.downloadsDefault', null, 'Downloads are saved to your Downloads folder.'),
    );

    // Which plugin opens links: the browser (default) or any installed plugin
    // declaring the `open-urls` capability. The stored choice is shown only
    // while that plugin is installed — the router falls back to the default.
    const label = (name) => name.charAt(0).toUpperCase() + name.slice(1);
    const names = linkPlugins(catalog).map((p) => p.name);
    const fallback = resolveLinkPlugin(catalog);
    const stored = getLinkPlugin();
    const links = names.length
      ? select({
        value: names.includes(stored) ? stored : fallback,
        options: names.map((name) => ({
          value: name,
          label: name === fallback ? t('settings.browser.linkPluginDefault', { name: label(name) }, `${label(name)} (default)`) : label(name),
        })),
        onChange: (value) => {
          setLinkPlugin(value);
          toast(t('settings.browser.linkHandlerSaved', null, 'Link handler saved'));
        },
      })
      : null;
    const linksHint = names.length
      ? t('settings.browser.linksHint', null, 'Links clicked anywhere — chat, mail, documents — open in this plugin.')
      : t('settings.browser.noLinkPlugin', null, 'No plugin that can open links is installed.');

    holder.replaceChildren(
      heading(t('settings.browser.search', null, 'Search')),
      engine,
      el('p', 'settings-hint', t('settings.browser.searchHint', null, 'Used when you type words instead of an address. A self-hosted SearXNG (SEARXNG_URL) takes precedence when configured.')),
      heading(t('settings.browser.links', null, 'Links')),
      ...(links ? [links] : []),
      el('p', 'settings-hint', linksHint),
      heading(t('settings.browser.privacy', null, 'Privacy')),
      adblock,
      heading(t('settings.browser.storage', null, 'Storage')),
      storage,
      heading(t('settings.browser.actions', null, 'Actions')),
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
  nav.setAttribute('aria-label', t('settings.nav.sections', null, 'Settings sections'));
  const panels = el('div', 'settings-panels');

  const sections = [
    { id: 'account', labelKey: 'settings.section.account', icon: 'ui/user', build: buildAccount },
    { id: 'appearance', labelKey: 'settings.section.appearance', icon: 'ui/droplet', build: buildAppearancePanel },
    { id: 'desktop', labelKey: 'settings.section.desktop', icon: 'ui/monitor', build: buildDesktop },
    { id: 'browser', labelKey: 'settings.section.browser', icon: 'ui/launcher', build: buildBrowser },
    { id: 'assistant', labelKey: 'settings.section.assistant', icon: 'ui/message-circle', build: buildAssistant },
    { id: 'voice', labelKey: 'settings.section.voice', icon: 'ui/mic', build: buildVoice },
    { id: 'images', labelKey: 'settings.section.images', icon: 'ui/image', build: buildImage },
    { id: 'music', labelKey: 'settings.section.music', icon: 'ui/music', build: buildMusic },
    { id: 'power', labelKey: 'settings.section.power', icon: 'ui/power', build: buildPower },
    { id: 'touchbar', labelKey: 'settings.section.touchbar', icon: 'ui/keyboard', build: buildTouchBar },
    { id: 'remote', labelKey: 'settings.section.remote', icon: 'ui/network', build: buildRemote },
    { id: 'system', labelKey: 'settings.section.system', icon: 'ui/power', build: buildSystem },
  ];

  let activePanel = pendingSection || 'account';
  pendingSection = null;

  const show = (id) => {
    activePanel = id;
    nav.querySelectorAll('.settings-nav-item').forEach((item) => {
      item.classList.toggle('is-active', item.dataset.panelTarget === id);
    });
    panels.querySelectorAll('.settings-panel').forEach((p) => {
      p.classList.toggle('is-active', p.dataset.panel === id);
    });
  };

  // Picking another interface language re-renders the whole window in it. The
  // builders register their listeners through on(), which appends to the
  // module-wide cleanups list; the mark lets a re-render drop exactly its own
  // entries again, so listeners never stack up across languages.
  const onLocaleChange = () => render();
  window.addEventListener('locale:change', onLocaleChange);
  cleanups.push(() => window.removeEventListener('locale:change', onLocaleChange));
  const mark = cleanups.length;

  function render() {
    for (const dispose of cleanups.splice(mark)) {
      try { dispose(); } catch (_) { /* ignore */ }
    }
    nav.textContent = '';
    panels.textContent = '';
    for (const section of sections) {
      const btn = el('button', 'settings-nav-item');
      btn.type = 'button';
      btn.dataset.panelTarget = section.id;
      const ring = el('span', 'settings-nav-icon');
      ring.appendChild(icon(section.icon, { size: 20 }));
      const label = t(section.labelKey);
      btn.append(ring, el('span', 'settings-nav-label', label));
      on(btn, 'click', () => show(section.id));
      nav.appendChild(btn);

      panels.appendChild(panel(section.id, section.icon, label, section.build()));
    }
    show(activePanel);
  }

  render();
  windowEl.append(nav, panels);
  tileEl.appendChild(windowEl);
  selectSectionFn = show;
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
    label: t('settings.appearance.neumorphic', null, 'Neumorphic soft UI'),
    hint: t('settings.appearance.neumorphicHint', null, 'Soft extruded-plastic surfaces with subtle dual shadows, in dark and light.'),
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
  const iconsetField = field(t('settings.appearance.iconset', null, 'Icon set'), iconset, {
    hint: iconsetOptions().find((o) => o.value === getIconsetChoice())?.hint
      || t('settings.appearance.iconsetHint', null, 'Infinity is coloured; Slot-Beauty is the monochrome line set. Themes may still override individual icons.'),
  });

  const tintIcons = toggleRow({
    label: t('settings.appearance.tintedIcons', null, 'Accent-tinted icons'),
    hint: t('settings.appearance.tintedIconsHint', null, 'Recolour the set’s artwork (folders, app and file icons) to your accent. Off keeps each set’s native colours.'),
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
  custom.appendChild(el('summary', null, t('settings.appearance.customGradient', null, 'Custom gradient')));
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
      { value: FONT_THEME, label: t('settings.appearance.themeDefault', null, 'Theme default') },
    ]);
    font.select.value = getFont();
  });
  font.select.value = getFont();
  const fontField = field(t('settings.appearance.globalFont', null, 'Global font'), font, {
    hint: t('settings.appearance.globalFontHint', null, 'Every installed system font is listed. "Theme default" restores the active theme’s own fonts.'),
  });

  const backgroundChildren = buildBackgroundControls();
  const windowSurfaceChildren = buildWindowSurfaceControls();
  const topBarChildren = buildTopBarControls();
  const hudChipChildren = buildHudChipControls();

  // Interface scale: a host setting, applied by the kiosk shell as page zoom.
  // Changing it re-lays-out the page, so the Terminal re-fits automatically.
  const scale = select({
    options: SCALE_OPTIONS.map((o) => ({
      value: o.value,
      label: o.value === 'auto' ? t('settings.appearance.scale.auto', null, o.label) : o.label,
    })),
    onChange: (value) => void applyScale(value),
  });
  const scaleField = field(t('settings.appearance.interfaceScale', null, 'Interface scale'), scale);
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
      const panel = typeof dpi === 'number' && dpi > 0
        ? t('settings.appearance.scale.panel', { dpi: Math.round(dpi) }, ` on this ${Math.round(dpi)}-DPI panel`)
        : '';
      scaleHint.textContent = pct
        ? t('settings.appearance.scale.applied', { percent: pct, panel }, `The shell applies ${pct}%${panel}. Scales the whole desktop. Auto follows this panel's pixel density — the right default on a high-DPI screen, and it also cuts rendering cost.`)
        : t('settings.appearance.scale.hint', null, `Scales the whole desktop. Auto follows this panel's pixel density — the right default on a high-DPI screen, and it also cuts rendering cost.`);
    } catch (_) {
      scaleHint.textContent = t('settings.appearance.scale.readFailed', null, 'Could not read the current scale.');
    }
  }

  async function applyScale(value) {
    try {
      await setDisplayScale(value);
      await refreshScale();
      toast(t('settings.appearance.scale.updated', null, 'Interface scale updated'));
    } catch (err) {
      toast(err?.message || t('settings.appearance.scale.updateFailed', null, 'Could not change the interface scale'), { type: 'error' });
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
    field(t('settings.appearance.theme', null, 'Theme'), theme),
    iconsetField,
    tintIcons,
    neumorphic,
    field(t('settings.appearance.accent', null, 'Accent'), accentSwatches),
    field(t('settings.appearance.gradient', null, 'Gradient'), gradientSwatches),
    custom,
    fontField,
    heading(t('common.display', null, 'Display')),
    scaleField,
    ...windowSurfaceChildren,
    ...topBarChildren,
    ...hudChipChildren,
    heading(t('settings.appearance.background.heading', null, 'Background')),
    el('p', 'settings-hint', t('settings.appearance.background.hint', null, 'The full-screen backdrop behind the desktop — the default mesh, your gradient, a photo, or a subtle animation.')),
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

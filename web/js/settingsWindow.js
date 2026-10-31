/**
 * settingsWindow.js — the Settings window (built-in desktop surface).
 *
 * The surface mirrors the Installer's three-column grammar without being a
 * plugin: a numbered section rail on the left, the active panel on the middle
 * stage, and a live "at a glance" manifest on the right. The manifest and the
 * footer are context, not a second set of controls — every preference writes
 * straight to the store as it changes, so closing the window is still the exit.
 *
 *   Account    — photo + display name
 *   Appearance — theme, accent, gradient, interface language, window surface,
 *                top bar, background
 *   Desktop    — tiling layout
 *   Assistant  — name, provider, model
 *   Voice      — language, speech recognition, speech output
 *   Images     — local image generation model, quality + downloads
 *   Music      — local song generation model + quality
 *   Touch Bar  — only on hosts that expose one
 *   Remote     — Iroh + Tailscale reachability
 *   System     — interface language, session memory, link handling
 *
 * Browser settings live in the Browser window's own menu; power management
 * (modes, switches and backlights) lives in the battery quick menu.
 */
import {
  avatar, disposeMotion, eyebrow, FONT_THEME, getAccent, getActiveTheme, getFont,
  getThemeManifest, listReveal,
} from '../ui/index.js';
import { t, getAvailableLocales, getLocale } from './i18n.js';
import { getTraveler, getVoiceLang } from './api.js';
import {
  getAiName, getAiProvider, getDesktopLayout, getImageModel, getImageQuality,
  getOllamaModel, getOpenAiModel, getRemember, getSongQuality, getSttEngine,
  getTouchBarEnabled, getTtsEngine, getUiLocale, getWakeWord,
} from './preferences.js';
import { el, panel } from './settings/controls.js';
import { createResources } from './resources.js';
import { createAccountSection } from './settings/account.js';
import { createAssistantSection } from './settings/assistant.js';
import { createVoiceSection } from './settings/voice.js';
import { createImageSection } from './settings/image.js';
import { createMusicSection } from './settings/music.js';
import { createDesktopSection } from './settings/desktop.js';
import { createSystemSection } from './settings/system.js';
import { createTouchBarSection } from './settings/touchbar.js';
import { createRemoteSection } from './settings/remote.js';
import { createAppearancePanelSection } from './settings/appearance.js';

export const SETTINGS_WINDOW = 'settings';

const SECTION_DEFS = [
  {
    id: 'account',
    labelKey: 'settings.section.account',
    descKey: 'settings.section.account.desc',
    icon: 'ui/user',
    build: createAccountSection,
  },
  {
    id: 'appearance',
    labelKey: 'settings.section.appearance',
    descKey: 'settings.section.appearance.desc',
    icon: 'ui/droplet',
    build: createAppearancePanelSection,
  },
  {
    id: 'desktop',
    labelKey: 'settings.section.desktop',
    descKey: 'settings.section.desktop.desc',
    icon: 'ui/monitor',
    build: createDesktopSection,
  },
  {
    id: 'assistant',
    labelKey: 'settings.section.assistant',
    descKey: 'settings.section.assistant.desc',
    icon: 'ui/message-circle',
    build: createAssistantSection,
  },
  {
    id: 'voice',
    labelKey: 'settings.section.voice',
    descKey: 'settings.section.voice.desc',
    icon: 'ui/mic',
    build: createVoiceSection,
  },
  {
    id: 'images',
    labelKey: 'settings.section.images',
    descKey: 'settings.section.images.desc',
    icon: 'ui/image',
    build: createImageSection,
  },
  {
    id: 'music',
    labelKey: 'settings.section.music',
    descKey: 'settings.section.music.desc',
    icon: 'ui/music',
    build: createMusicSection,
  },
  {
    id: 'touchbar',
    labelKey: 'settings.section.touchbar',
    descKey: 'settings.section.touchbar.desc',
    icon: 'ui/keyboard',
    build: createTouchBarSection,
  },
  {
    id: 'remote',
    labelKey: 'settings.section.remote',
    descKey: 'settings.section.remote.desc',
    icon: 'ui/network',
    build: createRemoteSection,
  },
  {
    id: 'system',
    labelKey: 'settings.section.system',
    descKey: 'settings.section.system.desc',
    icon: 'ui/power',
    build: createSystemSection,
  },
];

let tileEl = null;
let windowResources = null;
let sectionResources = null;
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

function clear(node) {
  if (node) node.textContent = '';
}

function manifestSection(label) {
  const section = el('section', 'settings-manifest-section');
  section.appendChild(eyebrow(label, 'settings-manifest-label'));
  return section;
}

function manifestRow(label, value, { swatch = '' } = {}) {
  const row = el('div', 'settings-manifest-row');
  row.appendChild(el('span', 'settings-manifest-key', label));
  const val = el('span', 'settings-manifest-val');
  if (swatch) {
    const dot = el('span', 'settings-manifest-swatch');
    dot.style.background = swatch;
    dot.setAttribute('aria-hidden', 'true');
    val.appendChild(dot);
  }
  val.appendChild(el('span', 'settings-manifest-val-text', value));
  row.appendChild(val);
  return row;
}

function localeName(code) {
  const entry = getAvailableLocales().find((locale) => locale.code === code);
  return entry?.name || String(code || '').toUpperCase();
}

function themeLabel() {
  const name = getThemeManifest()?.label || getActiveTheme() || '';
  return name ? name.charAt(0).toUpperCase() + name.slice(1) : '—';
}

function interfaceLanguageLabel() {
  const name = localeName(getLocale());
  return getUiLocale() ? name : `${name} · ${t('settings.manifest.auto', null, 'Automatic')}`;
}

function providerLabel() {
  return getAiProvider() === 'openai'
    ? t('settings.assistant.providerOpenai', null, 'OpenAI-compatible endpoint')
    : t('settings.assistant.providerOllama', null, 'Ollama (local)');
}

function assistantModelLabel() {
  const model = getAiProvider() === 'openai' ? getOpenAiModel() : getOllamaModel();
  return model || t('common.defaultServer', null, 'Default (server)');
}

function layoutLabel(mode = getDesktopLayout().mode) {
  if (mode === 'columns') return t('settings.desktop.mode.columns', null, 'Columns');
  if (mode === 'windows') return t('settings.desktop.mode.windows', null, 'Windows');
  return t('settings.desktop.mode.tiling', null, 'Tiling');
}

function imageModelLabel() {
  return getImageModel() === 'qwen-image-2.1'
    ? t('settings.images.model.qwenImage', null, 'Qwen-Image 2.1 Turbo')
    : t('settings.images.model.zImage', null, 'Z-Image Turbo');
}

function imageQualityLabel() {
  const key = {
    balanced: 'settings.images.quality.balanced',
    high: 'settings.images.quality.high',
    max: 'settings.images.quality.max',
  }[getImageQuality()] || 'settings.images.quality.fast';
  return t(key);
}

function songQualityLabel() {
  return getSongQuality() === 'q8'
    ? t('settings.music.quality.q8', null, 'Studio')
    : t('settings.music.quality.q4', null, 'Compact — recommended');
}

function sttEngineLabel() {
  return getSttEngine() === 'vosk'
    ? t('settings.voice.sttEngine.vosk', null, 'Vosk (in-browser)')
    : t('settings.voice.sttEngine.whisper', null, 'Faster Whisper (recommended)');
}

function ttsEngineLabel() {
  return getTtsEngine() === 'qwen'
    ? t('settings.voice.ttsEngine.qwen', null, 'Qwen3-TTS (experimental)')
    : t('settings.voice.ttsEngine.supertonic', null, 'Supertonic (default)');
}

function touchBarModeLabel() {
  const mode = getTouchBarEnabled();
  if (mode === 'on') return t('settings.touchbar.mode.on', null, 'Always on');
  if (mode === 'off') return t('settings.touchbar.mode.off', null, 'Off');
  return t('settings.touchbar.mode.auto', null, 'Automatic');
}

function sectionDetails(id) {
  if (id === 'appearance') {
    return [
      manifestRow(t('settings.manifest.theme', null, 'Theme'), themeLabel()),
      manifestRow(t('settings.manifest.accent', null, 'Accent'), getAccent(), { swatch: getAccent() }),
      manifestRow(t('settings.manifest.font', null, 'Font'), getFont() === FONT_THEME
        ? t('settings.appearance.themeDefault', null, 'Theme default')
        : getFont()),
    ];
  }
  if (id === 'desktop') {
    const layout = getDesktopLayout();
    return [
      manifestRow(t('settings.manifest.layout', null, 'Layout'), layoutLabel(layout.mode)),
      manifestRow(t('settings.desktop.masterSize', null, 'Master size'), `${Math.round(layout.master_ratio * 100)}%`),
      manifestRow(t('settings.manifest.gap', null, 'Gap'), `${layout.gap}px`),
    ];
  }
  if (id === 'assistant') {
    return [
      manifestRow(t('settings.manifest.name', null, 'Name'), getAiName()),
      manifestRow(t('settings.manifest.provider', null, 'Provider'), providerLabel()),
      manifestRow(t('settings.manifest.model', null, 'Model'), assistantModelLabel()),
    ];
  }
  if (id === 'voice') {
    return [
      manifestRow(t('settings.manifest.voiceLanguage', null, 'Voice language'), getVoiceLang().toUpperCase()),
      manifestRow(t('settings.manifest.recognition', null, 'Recognition'), sttEngineLabel()),
      manifestRow(t('settings.manifest.speech', null, 'Speech'), ttsEngineLabel()),
      manifestRow(t('settings.voice.wakeWord', null, 'Wake word'), getWakeWord()
        ? t('settings.manifest.on', null, 'On')
        : t('settings.manifest.off', null, 'Off')),
    ];
  }
  if (id === 'images') {
    return [
      manifestRow(t('settings.images.model', null, 'Model'), imageModelLabel()),
      manifestRow(t('settings.manifest.quality', null, 'Quality'), imageQualityLabel()),
    ];
  }
  if (id === 'music') {
    return [
      manifestRow(t('settings.manifest.quality', null, 'Quality'), songQualityLabel()),
    ];
  }
  if (id === 'touchbar') {
    return [
      manifestRow(t('settings.section.touchbar', null, 'Touch Bar'), touchBarModeLabel()),
    ];
  }
  if (id === 'system') {
    return [
      manifestRow(t('settings.system.interfaceLanguage', null, 'Interface language'), interfaceLanguageLabel()),
      manifestRow(t('settings.system.remember', null, 'Remember my workspace'), getRemember()
        ? t('settings.manifest.on', null, 'On')
        : t('settings.manifest.off', null, 'Off')),
    ];
  }
  return [];
}

function mountSettings() {
  if (tileEl) return tileEl;

  windowResources = createResources();
  windowResources.add(() => sectionResources?.dispose());
  tileEl = el('section', 'tile settings-tile');
  tileEl.dataset.plugin = SETTINGS_WINDOW;

  const windowEl = el('div', 'settings-window');
  const chrome = el('div', 'settings-chrome');
  chrome.dataset.windowBar = '';
  const layout = el('div', 'settings-layout');
  const nav = el('nav', 'settings-nav');
  const main = el('main', 'settings-main');
  const scroll = el('div', 'settings-scroll');
  const panels = el('div', 'settings-panels');
  const footer = el('footer', 'settings-footer');
  const manifest = el('aside', 'settings-manifest');

  nav.setAttribute('aria-label', t('settings.nav.sections', null, 'Settings sections'));
  manifest.setAttribute('aria-label', t('settings.manifest.navLabel', null, 'Settings overview'));

  const sections = SECTION_DEFS.filter((section) => section.id !== 'touchbar'
    || document.documentElement.dataset.touchbarHost === '1');

  let activePanel = (pendingSection && sections.some((section) => section.id === pendingSection))
    ? pendingSection
    : 'account';
  pendingSection = null;
  let manifestQueued = false;

  function activeSection() {
    return sections.find((section) => section.id === activePanel) || sections[0];
  }

  function sectionNumber() {
    return Math.max(0, sections.findIndex((section) => section.id === activePanel)) + 1;
  }

  function scheduleManifest() {
    if (manifestQueued) return;
    manifestQueued = true;
    queueMicrotask(() => {
      manifestQueued = false;
      if (tileEl) renderManifest();
    });
  }

  function sectionCounter() {
    return `${String(sectionNumber()).padStart(2, '0')} / ${String(sections.length).padStart(2, '0')}`;
  }

  // The bar keeps one mono status line; the panel head carries the title.
  function renderChrome() {
    clear(chrome);
    const status = el('div', 'settings-chrome-status ui-meta');
    status.append(
      el('span', 'settings-status-dot'),
      el('span', null, t('settings.chrome.saved', null, 'Saved automatically')),
    );
    chrome.append(
      el('span', 'settings-chrome-spacer'),
      el('span', 'settings-chrome-title ui-meta', t('settings.rail.title', null, 'Settings')),
      el('span', 'settings-chrome-spacer'),
      status,
    );
  }

  function renderFooter() {
    clear(footer);
    const step = el('div', 'settings-footer-step ui-meta', sectionCounter());
    step.setAttribute('aria-label', t('settings.manifest.section', {
      current: sectionNumber(),
      total: sections.length,
    }, 'Section {current} of {total}'));
    footer.append(step, el('div', 'settings-footer-note ui-meta', t('settings.footer.note', null, 'Changes apply immediately')));
  }

  function renderManifest() {
    const section = activeSection();
    if (!section) return;
    clear(manifest);

    const head = el('div', 'settings-manifest-head');
    head.append(
      eyebrow(t('settings.manifest.eyebrow', null, 'Active section')),
      el('h2', 'settings-manifest-title', t('settings.manifest.title', null, 'At a glance')),
    );
    manifest.appendChild(head);

    const current = manifestSection(t('settings.manifest.section', {
      current: sectionNumber(),
      total: sections.length,
    }, 'Section {current} of {total}'));
    const currentCopy = el('div', 'settings-manifest-current');
    currentCopy.append(
      el('strong', 'settings-manifest-value', t(section.labelKey)),
      el('span', 'settings-manifest-sub', t(section.descKey)),
    );
    current.appendChild(currentCopy);
    manifest.appendChild(current);

    const details = sectionDetails(section.id);
    if (details.length) {
      const values = manifestSection(t('settings.manifest.values', null, 'Current values'));
      for (const row of details) values.appendChild(row);
      manifest.appendChild(values);
    }

    const profile = manifestSection(t('settings.manifest.profile', null, 'Profile'));
    const traveler = getTraveler();
    if (traveler) {
      const row = el('div', 'settings-manifest-profile');
      row.appendChild(avatar({
        name: traveler.name || traveler.username || '?',
        src: traveler.avatar || '',
        size: 42,
      }));
      const copy = el('div', 'settings-manifest-profile-copy');
      copy.append(
        el('strong', 'settings-manifest-value', traveler.name || traveler.username || '—'),
        el('span', 'settings-manifest-sub', traveler.username ? `@${traveler.username}` : '—'),
      );
      row.appendChild(copy);
      profile.appendChild(row);
    } else {
      profile.append(
        el('strong', 'settings-manifest-value', t('settings.manifest.noProfile', null, 'No profile')),
        el('span', 'settings-manifest-sub', t('settings.manifest.noProfileSub', null, 'Not signed in')),
      );
    }
    manifest.appendChild(profile);

    const iface = manifestSection(t('settings.manifest.interface', null, 'Interface'));
    iface.append(
      manifestRow(t('settings.manifest.theme', null, 'Theme'), themeLabel()),
      manifestRow(t('settings.manifest.language', null, 'Language'), interfaceLanguageLabel()),
    );
    manifest.appendChild(iface);

    manifest.appendChild(el('p', 'settings-manifest-note', t('settings.manifest.note', null,
      'Every change is saved as you make it. Close the window when you are done.')));
  }

  function show(id, animate = true) {
    if (!sections.some((section) => section.id === id)) return;
    activePanel = id;
    nav.querySelectorAll('.settings-nav-item').forEach((item) => {
      const selected = item.dataset.panelTarget === id;
      item.classList.toggle('is-active', selected);
      if (selected) item.setAttribute('aria-current', 'page');
      else item.removeAttribute('aria-current');
    });
    panels.querySelectorAll('.settings-panel').forEach((p) => {
      const selected = p.dataset.panel === id;
      disposeMotion(p);
      p.hidden = !selected;
      p.classList.toggle('is-active', selected);
      // The panel's head and controls arrive in a cascade (the motion language).
      if (selected && animate) {
        listReveal(p.querySelectorAll(':scope > .settings-panel-head, .ui-control-group > h4, .ui-control-grid > .ui-control-card'));
      }
    });
    renderChrome();
    renderFooter();
    renderManifest();
  }

  windowResources.on(window, 'locale:change', () => render());
  windowResources.on(window, 'profile:change', renderManifest);
  windowResources.on(window, 'theme:change', renderManifest);
  windowResources.on(window, 'appearance:change', renderManifest);
  windowResources.on(window, 'ai:settings', renderManifest);
  windowResources.on(window, 'touchbar:settings', renderManifest);

  function render() {
    sectionResources?.dispose();
    sectionResources = createResources();
    const context = {
      resources: sectionResources,
      on: sectionResources.on,
      cleanups: { push: sectionResources.add },
    };
    nav.textContent = '';
    panels.textContent = '';

    const navHead = el('div', 'settings-nav-head');
    navHead.append(
      eyebrow(t('settings.rail.eyebrow', null, 'Preferences')),
      el('h2', 'settings-nav-title', t('settings.rail.title', null, 'Settings')),
    );
    nav.appendChild(navHead);

    const list = el('div', 'settings-nav-list');
    sections.forEach((section, index) => {
      const btn = el('button', 'settings-nav-item');
      btn.type = 'button';
      btn.dataset.panelTarget = section.id;
      const label = t(section.labelKey);
      const desc = t(section.descKey);
      // Flat rail: a mono number and the label; the active row wears the grey pill.
      btn.append(
        el('span', 'settings-nav-kicker', String(index + 1).padStart(2, '0')),
        el('span', 'settings-nav-label', label),
      );
      btn.title = label;
      btn.setAttribute('aria-label', `${label} — ${desc}`);
      sectionResources.on(btn, 'click', () => show(section.id));
      list.appendChild(btn);

      const sectionPanel = panel(section.id, section.icon, label, section.build(context), {
        index: index + 1,
        total: sections.length,
        description: desc,
      });
      panels.appendChild(sectionPanel);
    });
    nav.appendChild(list);

    sectionResources.on(panels, 'input', scheduleManifest);
    sectionResources.on(panels, 'change', scheduleManifest);
    sectionResources.on(panels, 'click', scheduleManifest);

    show(activePanel, false);
  }

  render();
  scroll.appendChild(panels);
  main.append(scroll, footer);
  layout.append(nav, main, manifest);
  windowEl.appendChild(layout);
  // The bar is a direct child of the tile so `tiles.js` merges it into the
  // core window title row (`data-window-bar`). The layout below is a sibling.
  tileEl.append(chrome, windowEl);
  windowResources.add(() => disposeMotion(tileEl));
  selectSectionFn = show;
  return tileEl;
}

function unmountSettings() {
  windowResources?.dispose();
  windowResources = null;
  sectionResources = null;
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

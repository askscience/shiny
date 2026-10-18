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
import { icon, transition, disposeMotion } from '../ui/index.js';
import { t } from './i18n.js';

import { el, panel } from './settings/controls.js';
import { createResources } from './resources.js';
import { createAccountSection } from './settings/account.js';
import { createAssistantSection } from './settings/assistant.js';
import { createVoiceSection } from './settings/voice.js';
import { createImageSection } from './settings/image.js';
import { createMusicSection } from './settings/music.js';
import { createPowerSection } from './settings/power.js';
import { createDesktopSection } from './settings/desktop.js';
import { createSystemSection } from './settings/system.js';
import { createTouchBarSection } from './settings/touchbar.js';
import { createRemoteSection } from './settings/remote.js';
import { createBrowserSection } from './settings/browser.js';
import { createAppearancePanelSection } from './settings/appearance.js';

export const SETTINGS_WINDOW = 'settings';
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

function mountSettings() {
  if (tileEl) return tileEl;

  windowResources = createResources();
  windowResources.add(() => sectionResources?.dispose());
  tileEl = el('section', 'tile settings-tile');
  tileEl.dataset.plugin = SETTINGS_WINDOW;

  const windowEl = el('div', 'settings-window');
  const nav = el('nav', 'settings-nav');
  const indicator = el('div', 'ui-selection-indicator');
  nav.setAttribute('aria-label', t('settings.nav.sections', null, 'Settings sections'));
  const panels = el('div', 'settings-panels');

  const sections = [
    { id: 'account', labelKey: 'settings.section.account', icon: 'ui/user', build: createAccountSection },
    { id: 'appearance', labelKey: 'settings.section.appearance', icon: 'ui/droplet', build: createAppearancePanelSection },
    { id: 'desktop', labelKey: 'settings.section.desktop', icon: 'ui/monitor', build: createDesktopSection },
    { id: 'browser', labelKey: 'settings.section.browser', icon: 'ui/launcher', build: createBrowserSection },
    { id: 'assistant', labelKey: 'settings.section.assistant', icon: 'ui/message-circle', build: createAssistantSection },
    { id: 'voice', labelKey: 'settings.section.voice', icon: 'ui/mic', build: createVoiceSection },
    { id: 'images', labelKey: 'settings.section.images', icon: 'ui/image', build: createImageSection },
    { id: 'music', labelKey: 'settings.section.music', icon: 'ui/music', build: createMusicSection },
    { id: 'power', labelKey: 'settings.section.power', icon: 'ui/power', build: createPowerSection },
    { id: 'touchbar', labelKey: 'settings.section.touchbar', icon: 'ui/keyboard', build: createTouchBarSection },
    { id: 'remote', labelKey: 'settings.section.remote', icon: 'ui/network', build: createRemoteSection },
    { id: 'system', labelKey: 'settings.section.system', icon: 'ui/power', build: createSystemSection },
  ];

  let activePanel = pendingSection || 'account';
  pendingSection = null;

  const show = (id, animate = true) => {
    if (!sections.some(section => section.id === id)) return;
    const oldTop = indicator.offsetTop || 0;
    activePanel = id;
    nav.querySelectorAll('.settings-nav-item').forEach((item) => {
      const selected = item.dataset.panelTarget === id;
      item.classList.toggle('is-active', selected);
      if (selected) {
        item.setAttribute('aria-current', 'page');
        indicator.style.top = `${item.offsetTop || 0}px`;
        indicator.style.height = `${item.offsetHeight || 44}px`;
        if (animate) transition(indicator, [{ transform: `translateY(${oldTop - (item.offsetTop || 0)}px)` }, { transform: 'none' }], { kind: 'connect' });
      } else item.removeAttribute('aria-current');
    });
    panels.querySelectorAll('.settings-panel').forEach((p) => {
      const selected = p.dataset.panel === id;
      disposeMotion(p);
      p.hidden = !selected;
      p.classList.toggle('is-active', selected);
      if (selected && animate) transition(p, [{ opacity: 0, transform: 'translateY(6px)' }, { opacity: 1, transform: 'none' }]);
    });
  };

  windowResources.on(window, 'locale:change', () => render());

  function render() {
    sectionResources?.dispose();
    sectionResources = createResources();
    const context = {
      resources: sectionResources,
      on: sectionResources.on,
      cleanups: { push: sectionResources.add },
    };
    nav.textContent = '';
    nav.appendChild(indicator);
    panels.textContent = '';
    for (const section of sections) {
      const btn = el('button', 'settings-nav-item');
      btn.type = 'button';
      btn.dataset.panelTarget = section.id;
      const ring = el('span', 'settings-nav-icon');
      ring.appendChild(icon(section.icon, { size: 20 }));
      const label = t(section.labelKey);
      btn.append(ring, el('span', 'settings-nav-label', label));
      // The label is hidden in the narrow icon-only strip (see flow.css), so the
      // item carries its own name and tooltip.
      btn.title = label;
      btn.setAttribute('aria-label', label);
      sectionResources.on(btn, 'click', () => show(section.id));
      nav.appendChild(btn);

      panels.appendChild(panel(section.id, section.icon, label, section.build(context)));
    }
    show(activePanel, false);
  }

  render();
  const layout = el('div', 'settings-layout');
  layout.append(nav, panels);
  windowEl.appendChild(layout);
  tileEl.appendChild(windowEl);
  windowResources.add(() => disposeMotion(tileEl));
  if (typeof ResizeObserver !== 'undefined') {
    const observer = new ResizeObserver(() => show(activePanel, false));
    observer.observe(nav);
    windowResources.add(() => observer.disconnect());
  }
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

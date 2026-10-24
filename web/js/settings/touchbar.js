import { select } from '../../ui/index.js';
import { t } from '../i18n.js';
import { getTouchBarEnabled, setTouchBarEnabled } from '../preferences.js';
import { TOUCHBAR_ACTIONS } from '../touchbarShared.js';
import { el, field, heading } from './controls.js';

/** Build the touchbar section with resources owned by its current mount. */
export function createTouchBarSection({ on, cleanups, resources }) {
  if (document.documentElement.dataset.touchbarHost !== '1') return [];

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


  return buildTouchBar();
}

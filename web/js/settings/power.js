import { apiFetch } from '../api.js';
import { select, slider, toggleRow } from '../../ui/index.js';
import { t } from '../i18n.js';
import { getPowerMode, setPowerMode, getAutoPowerSaver, setAutoPowerSaver, getLowPowerAi, setLowPowerAi, getEffectivePowerMode, getEffectiveSttEngine, getEffectiveTtsEngine, getPowerBatteryStatus } from '../preferences.js';
import { POWER_MODES, powerModeLabel, powerModeHint, describeEngines } from '../powerShared.js';
import { el, field, heading } from './controls.js';

/** Build the power section with resources owned by its current mount. */
export function createPowerSection({ on, cleanups, resources }) {
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


  return buildPower();
}

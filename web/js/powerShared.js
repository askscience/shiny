/**
 * powerShared.js — pure helpers for Shiny's power management: the three power
 * modes, how the battery status selects the *effective* mode, and which "light"
 * AI engines Power Saver prefers.
 *
 * Kept import-free on purpose: `web/js/tests/powerShared.test.mjs` loads it in
 * Node with no DOM. `preferences.js` owns the stored values; this module only
 * decides what they mean.
 *
 * Power modes (the classic laptop choice):
 *   performance — never drop to low power; the AI uses whatever you configured.
 *   balanced    — full quality on wall power, low power on battery / below 20%.
 *   saver       — always prefer low power.
 *
 * "Low power AI" is the lever Power Saver pulls: Vosk (in-browser recognition,
 * no faster-whisper sidecar) and Supertonic (bundled, no GPU) instead of
 * faster-whisper and Qwen3-TTS.
 */

/** The three modes, best-experience first. */
export const POWER_MODES = ['performance', 'balanced', 'saver'];

/** Percentage at or below which "low battery" counts, whatever the AC state. */
export const LOW_BATTERY_PERCENT = 20;

/** Coerce a stored value into a known mode; anything odd means `balanced`. */
export function normalizePowerMode(value) {
  return POWER_MODES.includes(value) ? value : 'balanced';
}

/** The short label for a mode. */
export function powerModeLabel(mode) {
  switch (normalizePowerMode(mode)) {
    case 'performance': return 'Performance';
    case 'saver': return 'Power Saver';
    default: return 'Balanced';
  }
}

/** One line explaining a mode. */
export function powerModeHint(mode) {
  switch (normalizePowerMode(mode)) {
    case 'performance':
      return 'Full quality. Never switches to low power, even on battery.';
    case 'saver':
      return 'Always prefer low power — lighter AI and less draining.';
    default:
      return 'Full quality on wall power, low power on battery or below 20%.';
  }
}

/**
 * True while the machine is actually drawing from the battery. A kernel that
 * reports `discharging` even though wall power is present (some firmware
 * pulses it) must not trip Power Saver, so AC wins when it is known.
 */
export function isOnBattery(status) {
  return !!status
    && status.available === true
    && status.state === 'discharging'
    && status.ac_online !== true;
}

/** True when the pack is low (< = 20%), charging or not. */
export function isLowBattery(status) {
  return !!status
    && status.available === true
    && status.percent != null
    && status.percent <= LOW_BATTERY_PERCENT;
}

/** The condition that makes Balanced drop to Power Saver. */
export function batteryTriggersSaver(status) {
  return isOnBattery(status) || isLowBattery(status);
}

/**
 * Resolve the mode actually in force from the stored mode, the
 * "automatic power saver" switch and the current battery status.
 * Performance never downgrades; Power Saver is unconditional; Balanced
 * follows the battery when automatic saving is on.
 */
export function effectivePowerMode(mode, autoSaver, status) {
  const base = normalizePowerMode(mode);
  if (base === 'performance') return 'performance';
  if (base === 'saver') return 'saver';
  return autoSaver && batteryTriggersSaver(status) ? 'saver' : 'balanced';
}

/**
 * The engines low-power mode uses. Returns `{ stt, tts }` when the effective
 * mode is `saver` and the "Low power AI" switch is on, else `null` (leave the
 * user's Voice choices alone).
 */
export function lowPowerEngines(mode, lowPowerAi) {
  return effectiveModeSaver(mode) && lowPowerAi ? { stt: 'vosk', tts: 'supertonic' } : null;
}

function effectiveModeSaver(mode) {
  return normalizePowerMode(mode) === 'saver';
}

/** A short description of the live AI engines, for the settings hint. */
export function describeEngines(stt, tts) {
  const sttLabel = stt === 'vosk' ? 'Vosk' : 'Faster Whisper';
  const ttsLabel = tts === 'qwen' ? 'Qwen3-TTS' : 'Supertonic';
  return `${sttLabel} + ${ttsLabel}`;
}

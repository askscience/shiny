/**
 * voiceGlyph.js — the one glyph the voice bar wears, and its swap fade.
 *
 * Settings → Appearance → "Tool status" decides what the bar shows: nothing
 * (`off`), the status line as `text`, or the running tool's icon (`icons`,
 * swapped per tool with a short fade — the animation lives in sphere.css as
 * `.is-fading`). The app's own mark is the resting state of the icon mode: it
 * is what the bar wears with no tool — thinking, or between turns — so the bar
 * always carries a mark, swaps to the tool's while one runs, and comes back.
 *
 * dockStep.js paints the bar's glyph with this; the Settings preview paints its
 * own with the same look, so the option shows what it does.
 */

import { setIcon, decodeText } from '../ui/index.js';
import { toolIcon } from './toolIcons.js';
import { t } from './i18n.js';

/** The voice states (sphere.js) and the word the bar wears for each. */
const STATE_LABELS = {
  idle: ['voice.state.ready', 'Ready'],
  listening: ['voice.state.listening', 'Listening'],
  conversation: ['voice.state.listening', 'Listening'],
  processing: ['voice.state.thinking', 'Thinking'],
  speaking: ['voice.state.speaking', 'Speaking'],
  warming: ['voice.state.warming', 'Starting'],
  disabled: ['voice.state.off', 'Voice off'],
  error: ['voice.state.error', 'Error'],
};

/** The word for a voice state, in the interface language. */
export function stateLabel(state) {
  const [key, fallback] = STATE_LABELS[state] || STATE_LABELS.idle;
  return t(key, null, fallback);
}

/**
 * Paint the state label: the new word decodes in place (random glyphs settle
 * left to right), so every change of state reads as one small event.
 */
export function paintStateLabel(el, state) {
  const text = stateLabel(state);
  if (el.dataset.stateText === text) return;
  if (el.dataset.iconName || el.classList.contains(BRAND)) clearGlyph(el);
  el.dataset.stateText = text;
  decodeText(el, text);
}

/** The brand mark: the PEAK'D! favicon, masked and painted in the current ink. */
const BRAND = 'app-icon';

/** Drop whatever the glyph was showing (text, icon, mark) and its state. */
export function clearGlyph(el) {
  el.textContent = '';
  delete el.dataset.stateText;
  el.classList.remove('ui-icon', BRAND, 'is-fading', 'ui-icon--missing');
  delete el.dataset.iconName;
  delete el.dataset.tintable;
}

/** Run the swap fade once. Clearing the class first restarts the animation. */
export function fadeGlyph(el) {
  el.classList.remove('is-fading');
  void el.offsetWidth;
  el.classList.add('is-fading');
}

/**
 * Paint the glyph for a mode: the text, the icon of `action`'s tool, or the app
 * mark when that step has no tool. The caller decides whether there is anything
 * to show at all (`off`, or no line yet) and hides the element instead.
 */
export function paintGlyph(el, { mode, text = '', action = '' } = {}) {
  clearGlyph(el);
  if (mode === 'text') {
    el.textContent = text;
    return;
  }
  const name = toolIcon(action);
  if (name) setIcon(el, name);
  else el.classList.add(BRAND);
  fadeGlyph(el);
}

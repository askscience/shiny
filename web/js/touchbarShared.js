/**
 * touchbarShared.js — the Touch Bar action vocabulary.
 *
 * The Touch Bar reaches Shiny over two very different transports, and both
 * have to speak the same action names:
 *
 *  - **macOS** — `peakd` puts a native `NSTouchBar` on the kiosk window; each
 *    button evaluates a `touchbar:action` CustomEvent in the page
 *    (`crates/peakd/src/touchbar.rs`).
 *  - **Linux T2** — the `tiny-dfr` daemon draws the bar and its buttons emit a
 *    key combo through uinput; the page maps that combo back to the same
 *    actions (`web/js/touchbar.js`, config in `scripts/touchbar/`).
 *
 * Why a combo rather than the obvious F13–F24: tiny-dfr emits *key codes*, and
 * the X keymap decides the keysym. On a `us`/`es` layout the FK13–FK21 codes
 * are bound to `XF86Tools`/`XF86Launch5…` (from the `inet` symbols), so the
 * page would never see `F13`. `Ctrl+Alt+Shift+<digit>` is layout-independent:
 * digits are mapped everywhere, and `event.code` (`Digit1`…) is physical, not
 * the shifted character.
 *
 * Keeping the mapping in one dependency-free module is deliberate: it lets the
 * runtime, the Settings preview, the `tiny-dfr` TOML and the Node tests be
 * checked against a single source of truth (`tests/touchbar.test.mjs`).
 *
 * Nothing here touches the DOM, so Node can import it directly.
 */

/** The modifier set every Touch Bar combo carries. */
export const TOUCHBAR_MODIFIERS = { ctrlKey: true, altKey: true, shiftKey: true };

/**
 * The buttons, in Touch Bar order.
 *
 *  - `action` — the name both transports dispatch.
 *  - `label`  — the Settings label (the bar shows an icon).
 *  - `icon`   — the tiny-dfr icon name (`shiny_*` is ours, shipped in
 *               `scripts/touchbar/icons/`; anything else is a tiny-dfr built-in).
 *  - `code`   — the DOM `KeyboardEvent.code` the page listens for.
 *  - `combo`  — the uinput keys tiny-dfr presses (names from input-linux).
 */
export const TOUCHBAR_ACTIONS = [
  { action: 'talk', label: 'Ask', icon: 'shiny_talk', code: 'Digit1', combo: ['LeftCtrl', 'LeftAlt', 'LeftShift', 'Num1'] },
  { action: 'stop', label: 'Stop', icon: 'shiny_stop', code: 'Digit2', combo: ['LeftCtrl', 'LeftAlt', 'LeftShift', 'Num2'] },
  { action: 'mute', label: 'Mute', icon: 'volume_off', code: 'Digit3', combo: ['LeftCtrl', 'LeftAlt', 'LeftShift', 'Num3'] },
  { action: 'volume-down', label: 'Vol-', icon: 'volume_down', code: 'Digit4', combo: ['LeftCtrl', 'LeftAlt', 'LeftShift', 'Num4'] },
  { action: 'volume-up', label: 'Vol+', icon: 'volume_up', code: 'Digit5', combo: ['LeftCtrl', 'LeftAlt', 'LeftShift', 'Num5'] },
  { action: 'screen-down', label: 'Scr-', icon: 'brightness_low', code: 'Comma', combo: ['LeftCtrl', 'LeftAlt', 'LeftShift', 'Comma'] },
  { action: 'screen-up', label: 'Scr+', icon: 'brightness_high', code: 'Period', combo: ['LeftCtrl', 'LeftAlt', 'LeftShift', 'Dot'] },
  { action: 'workspace-prev', label: 'Prev', icon: 'fast_rewind', code: 'Digit6', combo: ['LeftCtrl', 'LeftAlt', 'LeftShift', 'Num6'] },
  { action: 'workspace-next', label: 'Next', icon: 'fast_forward', code: 'Digit7', combo: ['LeftCtrl', 'LeftAlt', 'LeftShift', 'Num7'] },
  { action: 'kbd-backlight-down', label: 'Kbd-', icon: 'backlight_low', code: 'Minus', combo: ['LeftCtrl', 'LeftAlt', 'LeftShift', 'Minus'] },
  { action: 'kbd-backlight-up', label: 'Kbd+', icon: 'backlight_high', code: 'Equal', combo: ['LeftCtrl', 'LeftAlt', 'LeftShift', 'Equal'] },
  { action: 'keyboard', label: 'Keys', icon: 'shiny_keyboard', code: 'Digit8', combo: ['LeftCtrl', 'LeftAlt', 'LeftShift', 'Num8'] },
  { action: 'settings', label: 'Setup', icon: 'shiny_settings', code: 'Digit9', combo: ['LeftCtrl', 'LeftAlt', 'LeftShift', 'Num9'] },
];

const BY_CODE = new Map(TOUCHBAR_ACTIONS.map((entry) => [entry.code, entry.action]));
const BY_ACTION = new Map(TOUCHBAR_ACTIONS.map((entry) => [entry.action, entry]));

/** The three user-selectable modes for the Touch Bar feature. */
export const TOUCHBAR_SETTINGS = ['auto', 'on', 'off'];

export function normalizeTouchBarSetting(value) {
  return TOUCHBAR_SETTINGS.includes(value) ? value : 'auto';
}

/**
 * Action name for a keydown event, or null when it is not a Touch Bar combo.
 * The modifiers must match exactly (Super also held does not count), and the
 * lookup is on `code`, so the shifted character is irrelevant.
 */
export function actionForEvent(event) {
  if (!event || event.metaKey) return null;
  if (!TOUCHBAR_MODIFIERS.ctrlKey || !event.ctrlKey) return null;
  if (!TOUCHBAR_MODIFIERS.altKey || !event.altKey) return null;
  if (!TOUCHBAR_MODIFIERS.shiftKey || !event.shiftKey) return null;
  return BY_CODE.get(event.code) || null;
}

/** The DOM code an action listens for, or null for an unknown action. */
export function codeForAction(action) {
  return BY_ACTION.get(action)?.code || null;
}

/** The uinput combo an action sends, or null for an unknown action. */
export function comboForAction(action) {
  return BY_ACTION.get(action)?.combo || null;
}

/** Human label for an action (falls back to the raw name). */
export function actionLabel(action) {
  return BY_ACTION.get(action)?.label || action;
}

/**
 * Whether the page should act on Touch Bar input.
 *
 * `auto` follows the host: `peakd` sets `window.__shinyTouchBar` when it has
 * put a bar on screen (macOS) or found the T2 hardware (Linux), and the server
 * answers the same question at `GET /api/touchbar`. On any other machine both
 * say no, so `auto` is inert and a normal PC never reacts to the combo. `on`
 * forces the mapping (useful to try the buttons on a normal keyboard); `off`
 * disables it outright.
 */
export function isTouchBarActive(setting, hostAvailable) {
  const mode = normalizeTouchBarSetting(setting);
  if (mode === 'off') return false;
  if (mode === 'on') return true;
  return hostAvailable === true;
}

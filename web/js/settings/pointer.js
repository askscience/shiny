import { segmented, slider, toggleRow } from '../../ui/index.js';
import { apiFetch } from '../api.js';
import { t } from '../i18n.js';
import { el, heading, settingsGroups, sliderField } from './controls.js';

/**
 * Mouse & touchpad — the libinput options of every pointer on this machine
 * (`/api/pointer`, see src/services/pointer.rs). The server reads the values
 * back from the X server, so the controls show what the pointer really does,
 * driver defaults included; a change applies at once and again whenever a
 * device is plugged in.
 */

const POLL_MS = 5000;

async function fetchPointer() {
  const res = await apiFetch('/api/pointer', { authRedirect: false });
  return res?.data || null;
}

async function putPointer(patch) {
  const res = await apiFetch('/api/pointer', { method: 'PUT', body: JSON.stringify(patch) });
  return res?.data || null;
}

/** libinput speed (-1…1) as the 0–100% the slider shows. */
const speedPercent = (v) => Math.round(((Number(v) || 0) + 1) * 50);

/** Build the pointer section with resources owned by its current mount. */
export function createPointerSection({ cleanups }) {
  const root = el('div', 'settings-raw settings-pointer');
  let state = null;
  let signature = '';
  let disposed = false;
  // Controls of the current render, disposed when the devices change.
  let renderCleanups = [];

  const send = async (patch) => {
    try {
      const data = await putPointer(patch);
      if (data && !disposed) state = data;
    } catch (_) {
      // The next poll brings the controls back in line with the device.
    }
  };

  function speedControl(cls, value) {
    const initial = speedPercent(value);
    const control = slider({
      min: 0, max: 100, step: 5, value: initial,
      onInput: (v) => { field.value.textContent = `${v}%`; },
    });
    control.addEventListener('change', () => {
      void send({ [cls]: { speed: Number(control.value) / 50 - 1 } });
    });
    const field = sliderField(t('settings.pointer.speed', null, 'Speed'), `${initial}%`, control);
    return field.wrap;
  }

  function switchRow(cls, key, label, checked) {
    if (checked == null) return null;
    return toggleRow({ label, checked, onChange: (on) => void send({ [cls]: { [key]: on } }) });
  }

  function choice(cls, key, label, value, options) {
    const offered = options.filter((o) => o.available);
    if (value == null || offered.length < 2) return null;
    const control = segmented({
      label,
      value,
      options: offered,
      onChange: (v) => void send({ [cls]: { [key]: v } }),
    });
    renderCleanups.push(() => control.destroy?.());
    const wrap = el('div', 'settings-field');
    wrap.append(el('label', 'settings-label', label), control);
    return wrap;
  }

  function touchpadChildren(tp) {
    if (!tp?.present) return [];
    const clicks = tp.click_methods || [];
    const scrolls = tp.scroll_methods || [];
    return [
      heading(t('settings.pointer.touchpad', null, 'Touchpad')),
      switchRow('touchpad', 'tap_to_click', t('settings.pointer.tapToClick', null, 'Tap to click'), tp.tap_to_click),
      tp.speed == null ? null : speedControl('touchpad', tp.speed),
      switchRow('touchpad', 'natural_scroll', t('settings.pointer.naturalScroll', null, 'Natural scrolling'), tp.natural_scroll),
      switchRow('touchpad', 'disable_while_typing', t('settings.pointer.disableWhileTyping', null, 'Disable while typing'), tp.disable_while_typing),
      choice('touchpad', 'click_method', t('settings.pointer.secondaryClick', null, 'Secondary click'), tp.click_method, [
        { value: 'fingers', label: t('settings.pointer.click.fingers', null, 'Two fingers'), available: clicks.includes('fingers') },
        { value: 'areas', label: t('settings.pointer.click.corner', null, 'Corner'), available: clicks.includes('areas') },
      ]),
      choice('touchpad', 'scroll_method', t('settings.pointer.scrolling', null, 'Scrolling'), tp.scroll_method, [
        { value: 'two_finger', label: t('settings.pointer.scroll.twoFinger', null, 'Two fingers'), available: scrolls.includes('two_finger') },
        { value: 'edge', label: t('settings.pointer.scroll.edge', null, 'Edge'), available: scrolls.includes('edge') },
      ]),
    ];
  }

  function mouseChildren(mouse) {
    if (!mouse?.present) return [];
    return [
      heading(t('settings.pointer.mouse', null, 'Mouse')),
      mouse.speed == null ? null : speedControl('mouse', mouse.speed),
      mouse.accel_profile == null ? null : toggleRow({
        label: t('settings.pointer.acceleration', null, 'Mouse acceleration'),
        checked: mouse.accel_profile !== 'flat',
        onChange: (on) => void send({ mouse: { accel_profile: on ? 'adaptive' : 'flat' } }),
      }),
      switchRow('mouse', 'natural_scroll', t('settings.pointer.naturalScroll', null, 'Natural scrolling'), mouse.natural_scroll),
      switchRow('mouse', 'left_handed', t('settings.pointer.leftHanded', null, 'Left-handed'), mouse.left_handed),
    ];
  }

  function disposeRender() {
    for (const fn of renderCleanups) fn();
    renderCleanups = [];
  }

  function render() {
    disposeRender();
    root.textContent = '';
    let children;
    if (!state) {
      children = [el('p', 'settings-hint', t('settings.pointer.loading', null, 'Looking for pointers…'))];
    } else if (!state.available) {
      children = [el('p', 'settings-hint', t('settings.pointer.unavailable', null, 'Mouse and touchpad settings are available on the machine itself, in the LiteAura session.'))];
    } else {
      children = [...touchpadChildren(state.touchpad), ...mouseChildren(state.mouse)].filter(Boolean);
      if (!children.length) {
        children = [el('p', 'settings-hint', t('settings.pointer.none', null, 'No mouse or touchpad found.'))];
      }
    }
    root.append(...settingsGroups(children));
  }

  // Redraw only when the devices change (a mouse plugged in or out); a value
  // change the user just made is already on screen.
  const deviceSignature = (s) => JSON.stringify([
    s?.available, s?.touchpad?.devices || [], s?.mouse?.devices || [],
  ]);

  async function poll() {
    let data = null;
    try { data = await fetchPointer(); } catch (_) { data = null; }
    if (disposed) return;
    const next = deviceSignature(data);
    state = data || { available: false };
    if (next !== signature) {
      signature = next;
      render();
    }
  }

  render();
  void poll();
  const timer = setInterval(() => { if (root.isConnected && !root.closest('[hidden]')) void poll(); }, POLL_MS);
  cleanups.push(() => { disposed = true; clearInterval(timer); disposeRender(); });

  return [root];
}

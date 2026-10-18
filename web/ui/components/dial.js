/** Rotary numeric control shared by image adjustments and audio parameters. */
export function dial({ label = '', min = 0, max = 100, step = 1, value = 0,
  resetValue = 0, format = String, onInput, onChange, onCancel } = {}) {
  const make = (tag, cls, text) => {
    const node = document.createElement(tag);
    node.className = cls;
    if (text != null) node.textContent = text;
    return node;
  };
  const root = make('div', 'ui-dial');
  root.tabIndex = 0;
  root.setAttribute('role', 'slider');
  root.setAttribute('aria-label', label);
  root.setAttribute('aria-valuemin', String(min));
  root.setAttribute('aria-valuemax', String(max));
  const knob = make('span', 'ui-dial-knob');
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 44 44');
  svg.setAttribute('aria-hidden', 'true');
  svg.classList.add('ui-dial-svg');
  const circle = (cls) => {
    const node = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
    for (const [key, val] of Object.entries({ cx: 22, cy: 22, r: 18, fill: 'none', pathLength: 100, transform: 'rotate(135 22 22)' })) node.setAttribute(key, String(val));
    node.classList.add(cls);
    return node;
  };
  const track = circle('ui-dial-track');
  track.setAttribute('stroke-dasharray', '75 100');
  const arc = circle('ui-dial-arc');
  svg.append(track, arc);
  const readout = make('span', 'ui-dial-value');
  knob.append(svg, readout);
  root.append(knob, make('span', 'ui-dial-label', label));
  const span = max - min || 1;
  const neutral = Math.max(0, Math.min(1, (resetValue - min) / span));
  const snap = v => {
    const numeric = Number(v) || 0;
    const rounded = step > 0 ? Number((Math.round(numeric / step) * step).toFixed(8)) : numeric;
    return Math.min(max, Math.max(min, rounded));
  };
  let current = snap(value), drag = null, wheelTimer = null, destroyed = false;
  const paint = () => {
    const fraction = (current - min) / span;
    arc.setAttribute('stroke-dasharray', `${Math.round(Math.abs(fraction - neutral) * 750) / 10} 100`);
    arc.setAttribute('stroke-dashoffset', `${-Math.round(Math.min(fraction, neutral) * 750) / 10}`);
    readout.textContent = format(current);
    root.dataset.value = String(current);
    root.classList.toggle('is-set', current !== resetValue);
    root.setAttribute('aria-valuenow', String(current));
    root.setAttribute('aria-valuetext', format(current));
  };
  const set = next => { current = snap(next); paint(); };
  const commit = () => { window.clearTimeout(wheelTimer); wheelTimer = null; if (!destroyed) onChange?.(current); };
  root.setValue = set;
  root.dial = knob;
  Object.defineProperty(root, 'value', { get: () => current });
  root.addEventListener('pointerdown', e => {
    if (e.button != null && e.button !== 0) return;
    e.preventDefault(); root.focus?.({ preventScroll: true });
    window.clearTimeout(wheelTimer);
    drag = { y: e.clientY, value: current, id: e.pointerId };
    root.setPointerCapture?.(e.pointerId);
    root.classList.add('is-dragging');
  });
  root.addEventListener('pointermove', e => {
    if (!drag) return;
    const next = snap(drag.value + (drag.y - e.clientY) / (160 * (e.shiftKey ? 5 : 1)) * span);
    if (next === current) return;
    set(next); onInput?.(current);
  });
  const finish = e => {
    if (!drag) return;
    drag = null; root.classList.remove('is-dragging');
    try { root.releasePointerCapture?.(e.pointerId); } catch { /* already released */ }
    commit();
  };
  root.addEventListener('pointerup', finish);
  root.addEventListener('pointercancel', finish);
  root.addEventListener('lostpointercapture', finish);
  root.addEventListener('wheel', e => {
    e.preventDefault();
    set(current + (e.deltaY < 0 ? 1 : -1) * (step || span / 100) * (e.shiftKey ? 1 : 5));
    onInput?.(current); window.clearTimeout(wheelTimer);
    wheelTimer = window.setTimeout(commit, 350);
  }, { passive: false });
  root.addEventListener('keydown', e => {
    const delta = (step || span / 100) * (e.shiftKey ? 10 : 1);
    let next;
    if (['ArrowUp','ArrowRight'].includes(e.key)) next = current + delta;
    else if (['ArrowDown','ArrowLeft'].includes(e.key)) next = current - delta;
    else if (e.key === 'Home') next = min;
    else if (e.key === 'End') next = max;
    else if (e.key === 'Delete' || e.key === 'Backspace') next = resetValue;
    else if (e.key === 'Escape' && onCancel) { e.preventDefault(); window.clearTimeout(wheelTimer); drag = null; root.classList.remove('is-dragging'); onCancel(); return; }
    if (next == null) return;
    e.preventDefault(); set(next); if (!onChange) onInput?.(current); commit();
  });
  root.addEventListener('dblclick', () => { set(resetValue); if (!onChange) onInput?.(current); commit(); });
  root.destroy = () => { destroyed = true; drag = null; window.clearTimeout(wheelTimer); };
  paint();
  return root;
}

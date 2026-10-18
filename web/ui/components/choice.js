import { controlButton } from './button.js';
import { transition, disposeMotion } from '../motion.js';
import { createResources } from '../../js/resources.js';

/** Small labeled choices with a moving indicator and roving keyboard focus. */
export function segmented({ label, options = [], value, onChange } = {}) {
  const node = document.createElement('div');
  node.className = 'ui-segmented';
  node.setAttribute('role', 'radiogroup');
  if (label) node.setAttribute('aria-label', label);
  const indicator = document.createElement('span');
  indicator.className = 'ui-selection-indicator';
  indicator.setAttribute('aria-hidden', 'true');
  node.appendChild(indicator);
  const resources = createResources();
  let selected = value ?? options[0]?.value;
  const buttons = options.map(option => {
    const button = controlButton();
    button.textContent = option.label;
    button.setAttribute('role', 'radio');
    button.disabled = !!option.disabled;
    resources.on(button, 'click', () => { if (!button.disabled) node.setValue(option.value); });
    node.appendChild(button);
    return button;
  });
  function paint(animate = false) {
    const previous = indicator.getBoundingClientRect();
    buttons.forEach((button, index) => {
      const active = options[index].value === selected;
      button.setAttribute('aria-checked', String(active));
      button.tabIndex = active ? 0 : -1;
    });
    const target = buttons[options.findIndex(option => option.value === selected)];
    if (!target) return;
    indicator.style.left = `${target.offsetLeft || 0}px`;
    indicator.style.top = `${target.offsetTop || 0}px`;
    indicator.style.width = `${target.offsetWidth || 0}px`;
    indicator.style.height = `${target.offsetHeight || 0}px`;
    if (animate && previous.width) {
      const current = indicator.getBoundingClientRect();
      transition(indicator, [{ transform: `translateX(${previous.left - current.left}px) scaleX(${previous.width / (current.width || 1)})` }, { transform: 'none' }], { kind: 'connect' });
    }
  }
  node.setValue = (next, { silent = false } = {}) => {
    if (!options.some(option => option.value === next && !option.disabled)) return;
    const changed = next !== selected;
    selected = next; paint(changed);
    if (changed && !silent) onChange?.(next);
  };
  Object.defineProperty(node, 'value', { get: () => selected });
  resources.on(node, 'keydown', event => {
    if (!['ArrowLeft','ArrowRight','ArrowUp','ArrowDown','Home','End'].includes(event.key)) return;
    event.preventDefault();
    const enabled = options.map((option,index) => ({ option,index })).filter(({ option }) => !option.disabled);
    if (!enabled.length) return;
    const position = enabled.findIndex(({ option }) => option.value === selected);
    const rtl = document.documentElement.dir === 'rtl';
    const forward = event.key === 'ArrowDown' || event.key === (rtl ? 'ArrowLeft' : 'ArrowRight');
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? enabled.length - 1 : (position + (forward ? 1 : -1) + enabled.length) % enabled.length;
    node.setValue(enabled[next].option.value); buttons[enabled[next].index].focus();
  });
  if (typeof ResizeObserver !== 'undefined') {
    const observer = new ResizeObserver(() => paint()); observer.observe(node);
    resources.add(() => observer.disconnect());
  }
  paint();
  resources.add(() => disposeMotion(node));
  node.destroy = () => resources.dispose();
  return node;
}

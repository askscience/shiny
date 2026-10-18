import { controlButton } from './button.js';
import { input, textarea, selectControl } from './field.js';

/** Small DOM builder: native form controls always use the shared UI contract. */
export function element(tag, className, text) {
  const node = tag === 'button' ? controlButton()
    : tag === 'input' ? input()
      : tag === 'textarea' ? textarea()
        : tag === 'select' ? selectControl() : document.createElement(tag);
  if (className) node.classList.add(...className.split(/\s+/).filter(Boolean));
  if (text != null) node.textContent = text;
  return node;
}

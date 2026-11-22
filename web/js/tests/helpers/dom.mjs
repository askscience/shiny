/**
 * The minimum DOM the card renderers use, for tests that run in Node: class
 * names, text, attributes, children, clicks and key presses. Import after
 * `./i18n.mjs` (it extends the globals that one installs) and before the
 * module under test.
 *
 *   import './helpers/i18n.mjs';
 *   import { dispatched } from './helpers/dom.mjs';
 */

export class FakeText {
  constructor(text) { this.nodeType = 3; this.data = String(text); }
  get textContent() { return this.data; }
}

export class FakeElement {
  constructor(tag) {
    this.tagName = tag.toUpperCase();
    this.nodeType = 1;
    this.children = [];
    this.attributes = {};
    this.dataset = {};
    this.style = {};
    this.listeners = {};
    this.className = '';
    this.parentElement = null;
    this.disabled = false;
    this.value = '';
    const self = this;
    this.classList = {
      add: (...names) => { self.className = [...new Set([...self.classes(), ...names])].join(' '); },
      remove: (...names) => { self.className = self.classes().filter((c) => !names.includes(c)).join(' '); },
      contains: (name) => self.classes().includes(name),
      toggle: (name, force) => {
        const on = force === undefined ? !self.classes().includes(name) : force;
        if (on) self.classList.add(name); else self.classList.remove(name);
        return on;
      },
    };
  }

  classes() { return this.className.split(/\s+/).filter(Boolean); }
  append(...nodes) {
    for (const n of nodes) {
      const node = typeof n === 'string' ? new FakeText(n) : n;
      if (node instanceof FakeElement) node.parentElement = this;
      this.children.push(node);
    }
  }
  appendChild(node) { this.append(node); return node; }
  remove() {
    if (!this.parentElement) return;
    this.parentElement.children = this.parentElement.children.filter((c) => c !== this);
    this.parentElement = null;
  }
  setAttribute(k, v) { this.attributes[k] = String(v); }
  getAttribute(k) { return this.attributes[k] ?? null; }
  addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); }
  removeEventListener() {}
  focus() {}
  /** Fire `type` listeners with a plain event object. */
  fire(type, init = {}) {
    const event = { stopPropagation() {}, preventDefault() {}, target: this, ...init };
    for (const fn of this.listeners[type] || []) fn(event);
  }
  click() { this.fire('click'); }
  set textContent(v) { this.children = v === '' || v == null ? [] : [new FakeText(v)]; }
  get textContent() { return this.children.map((c) => c.textContent).join(''); }

  /** All descendants (depth first) matching `.class`, `tag`, or a list of them. */
  querySelectorAll(selector) {
    const wanted = selector.split(',').map((s) => s.trim());
    const match = (el) => wanted.some((s) => (s.startsWith('.')
      ? el.classes().includes(s.slice(1))
      : el.tagName === s.toUpperCase()));
    const out = [];
    const walk = (el) => {
      for (const c of el.children) {
        if (!(c instanceof FakeElement)) continue;
        if (match(c)) out.push(c);
        walk(c);
      }
    };
    walk(this);
    return out;
  }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
}

/** Every event the code under test dispatched on `window`. */
export const dispatched = [];

Object.defineProperty(globalThis, 'document', {
  configurable: true,
  writable: true,
  value: {
    ...globalThis.document,
    documentElement: { lang: 'en', dir: 'ltr', setAttribute() {}, style: {}, dataset: {} },
    createElement: (tag) => new FakeElement(tag),
    createTextNode: (text) => new FakeText(text),
    querySelectorAll: () => [],
  },
});
Object.defineProperty(globalThis, 'window', {
  configurable: true,
  writable: true,
  value: {
    ...globalThis.window,
    addEventListener() {},
    removeEventListener() {},
    dispatchEvent: (event) => { dispatched.push(event); return true; },
  },
});
if (typeof globalThis.CustomEvent === 'undefined') {
  globalThis.CustomEvent = class CustomEvent {
    constructor(type, init = {}) { this.type = type; this.detail = init.detail; }
  };
}
globalThis.getComputedStyle = () => ({ getPropertyValue: () => '' });

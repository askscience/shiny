// Exercise the actual settings window and all section builders with a small
// DOM harness. No server, provider, microphone or host mutation is involved.
import './helpers/i18n.mjs';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

class Target extends EventTarget {
  listeners = new Map();
  addEventListener(type, handler, options) {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set());
    this.listeners.get(type).add(handler);
    super.addEventListener(type, handler, options);
  }
  removeEventListener(type, handler, options) {
    this.listeners.get(type)?.delete(handler);
    super.removeEventListener(type, handler, options);
  }
  count() { return [...this.listeners.values()].reduce((sum, handlers) => sum + handlers.size, 0); }
}
class Element extends Target {
  constructor(tag) {
    super();
    this.tagName = tag.toUpperCase();
    this.dataset = {};
    this.children = [];
    this.attributes = new Map();
    this.style = { setProperty() {}, removeProperty() {}, getPropertyValue: () => '' };
    this.className = '';
    const names = () => new Set(this.className.split(/\s+/).filter(Boolean));
    this.classList = {
      contains: (name) => names().has(name),
      add: (...add) => { this.className = [...new Set([...names(), ...add])].join(' '); },
      remove: (...remove) => { this.className = [...names()].filter((name) => !remove.includes(name)).join(' '); },
      toggle: (name, force) => {
        const enabled = force ?? !names().has(name);
        this.classList[enabled ? 'add' : 'remove'](name);
        return enabled;
      },
    };
  }
  set textContent(text) { this.text = String(text); this.children = []; }
  get textContent() { return this.text || ''; }
  set innerHTML(html) {
    this.textContent = html;
    for (const match of String(html).matchAll(/<([a-z][\w-]*)([^>]*)>/gi)) {
      const child = new Element(match[1]);
      child.className = match[2].match(/class=["']([^"']*)["']/)?.[1] || '';
      this.appendChild(child);
    }
  }
  get innerHTML() { return this.textContent; }
  appendChild(child) { this.children.push(child); child.parentElement = this; return child; }
  append(...children) { children.forEach((child) => this.appendChild(typeof child === 'string' ? new Element('text') : child)); }
  replaceChildren(...children) { this.children = []; this.append(...children); }
  remove() { if (this.parentElement) this.parentElement.children = this.parentElement.children.filter((child) => child !== this); }
  setAttribute(name, value) { this.attributes.set(name, String(value)); }
  getAttribute(name) { return this.attributes.get(name) ?? null; }
  removeAttribute(name) { this.attributes.delete(name); }
  get isConnected() { return true; }
  get options() { return this.children; }
  querySelectorAll(selector) {
    const matches = (node) => selector.startsWith('.') ? node.classList.contains(selector.slice(1)) : node.tagName === selector.toUpperCase();
    return this.children.flatMap((child) => [...(matches(child) ? [child] : []), ...child.querySelectorAll(selector)]);
  }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  getBoundingClientRect() { return { top: 0, left: 0, width: 1000, height: 800, right: 1000, bottom: 800 }; }
}
globalThis.CustomEvent = class extends Event { constructor(type, options = {}) { super(type); this.detail = options.detail; } };
globalThis.window = Object.assign(new Target(), {
  matchMedia: () => Object.assign(new Target(), { matches: false }),
  location: { origin: 'http://localhost', href: 'http://localhost/' },
  innerWidth: 1000, innerHeight: 800, setTimeout, clearTimeout,
});
const html = new Element('html');
const body = new Element('body');
globalThis.document = Object.assign(new Target(), {
  documentElement: html, body,
  createElement: (tag) => new Element(tag), createElementNS: (_ns, tag) => new Element(tag),
  createTextNode: (text) => Object.assign(new Element('text'), { textContent: text }),
  getElementById: () => null, querySelector: () => null, querySelectorAll: () => [],
});
globalThis.getComputedStyle = () => ({ getPropertyValue: () => '', fontFamily: 'sans-serif' });
globalThis.requestAnimationFrame = () => 1;
globalThis.cancelAnimationFrame = () => {};
globalThis.fetch = async (path, options = {}) => {
  assert.ok(!options.method || options.method === 'GET', `mount must not mutate ${path}`);
  if (!String(path).startsWith('/api/')) {
    try { return new Response(readFileSync(new URL(`../../${String(path).replace(/^\//, '')}`, import.meta.url))); }
    catch { return new Response('', { status: 404 }); }
  }
  const data = path === '/api/plugins' || path === '/api/fonts' ? [] : {};
  return new Response(JSON.stringify({ success: true, data }), { headers: { 'Content-Type': 'application/json' } });
};
const timers = new Set();
const nativeInterval = globalThis.setInterval;
const nativeClearInterval = globalThis.clearInterval;
globalThis.setInterval = (...args) => { const timer = nativeInterval(...args); timer.unref(); timers.add(timer); return timer; };
globalThis.clearInterval = (timer) => { timers.delete(timer); nativeClearInterval(timer); };

const { default: settings, selectSettingsSection } = await import('../settingsWindow.js');
const baseline = window.count();
const expectedSections = ['account', 'appearance', 'desktop', 'assistant', 'voice', 'images', 'music', 'remote', 'system'];
for (let mount = 0; mount < 3; mount++) {
  selectSettingsSection('music');
  const element = settings.mount();
  const panels = element.querySelectorAll('.settings-panel');
  assert.deepEqual(panels.map((panel) => panel.dataset.panel), expectedSections);
  assert.equal(panels.find((panel) => panel.dataset.panel === 'music').classList.contains('is-active'), true);
  const listeners = window.count();
  window.dispatchEvent(new CustomEvent('locale:change'));
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(window.count(), listeners, 'locale rerender does not accumulate window listeners');
  assert.equal(element.querySelectorAll('.settings-panel').length, expectedSections.length);
  assert.equal(element.querySelectorAll('.settings-panel').find((panel) => panel.dataset.panel === 'music').classList.contains('is-active'), true);
  settings.unmount();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(window.count(), baseline, 'unmount restores the original listener count');
  assert.equal(timers.size, 0, 'unmount clears every settings poll');
  assert.equal(settings.getElement(), null);
}
settings.mount();
settings.unmount();
await new Promise((resolve) => setImmediate(resolve));
assert.equal(timers.size, 0, 'late async replies cannot leave polls behind after unmount');
assert.equal(window.count(), baseline);
console.log('All settings sections mount, rerender and dispose cleanly');

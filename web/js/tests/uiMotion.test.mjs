// Shared motion is behavioral: cancellation, renderer stalls, focus, and nesting.
import './helpers/i18n.mjs';
import assert from 'node:assert/strict';
import { transition, revealRegion, disposeMotion, motionDuration, reducedMotion } from '../../ui/motion.js';

const media = Object.assign(new EventTarget(), { matches: false });
globalThis.window = new EventTarget();
window.matchMedia = () => media;
globalThis.getComputedStyle = () => ({ getPropertyValue: () => '' });
class Element extends EventTarget {
  constructor(tag) {
    super(); this.tagName = tag.toUpperCase(); this.children = []; this.className = ''; this.attributes = new Map();
    this.inert = false; this.hidden = false; this.tabIndex = 0; this.style = {}; this.scrollHeight = 80; this.animations = [];
    const names = () => this.className.split(/\s+/).filter(Boolean);
    this.classList = {
      contains: name => names().includes(name),
      add: (...items) => { this.className = [...new Set([...names(), ...items])].join(' '); },
      remove: (...items) => { this.className = names().filter(name => !items.includes(name)).join(' '); },
      toggle: (name, value) => { const enabled = value ?? !names().includes(name); this.classList[enabled ? 'add' : 'remove'](name); return enabled; },
    };
  }
  appendChild(child) { child.remove(); this.children.push(child); child.parentElement = this; return child; }
  append(...children) { children.forEach(child => this.appendChild(child)); }
  remove() { if (this.parentElement) this.parentElement.children = this.parentElement.children.filter(child => child !== this); this.parentElement = null; }
  contains(child) { return child === this || this.children.some(node => node.contains(child)); }
  setAttribute(name, value) { this.attributes.set(name, String(value)); }
  getAttribute(name) { return this.attributes.get(name) ?? null; }
  removeAttribute(name) { this.attributes.delete(name); }
  get isConnected() { return this === document.body || !!this.parentElement?.isConnected; }
  getClientRects() { return this.hidden || this.classList.contains('hidden') ? [] : [{}]; }
  getBoundingClientRect() { return { height: this.hidden ? 0 : 80 }; }
  querySelectorAll() { return this.children.flatMap(node => [...(['BUTTON','INPUT','SELECT'].includes(node.tagName) ? [node] : []), ...node.querySelectorAll()]); }
  closest(selector) {
    if (selector.includes('inert') && this.inert) return this;
    if (selector.includes('hidden') && (this.hidden || this.classList.contains('hidden'))) return this;
    return this.parentElement?.closest(selector) || null;
  }
  focus() { document.activeElement = this; }
  animate() {
    let resolve, reject;
    const animation = { finished: new Promise((yes,no) => { resolve = yes; reject = no; }), cancel: () => reject(new Error('cancel')), complete: () => resolve() };
    this.animations.push(animation); return animation;
  }
}
globalThis.HTMLElement = Element;
globalThis.document = { createElement: tag => new Element(tag), documentElement: new Element('html'), activeElement: null };
document.body = new Element('body');
const { modal, sheet } = await import('../../ui/components/overlay.js');
const flush = async () => { await Promise.resolve(); await Promise.resolve(); };
const target = document.createElement('div'); document.body.append(target);
const finished = [];
transition(target, [], { done: () => finished.push('stale') });
transition(target, [], { done: () => finished.push('current') });
target.animations.forEach(animation => animation.complete()); await flush();
assert.deepEqual(finished, ['current']);

revealRegion(target, false); const closing = target.animations.at(-1);
revealRegion(target, true); closing.complete(); target.animations.at(-1).complete(); await flush();
assert.equal(target.hidden, false, 'reopening cancels stale collapse');
const count = target.animations.length; revealRegion(target, true);
assert.equal(target.animations.length, count, 'status polling does not replay a settled reveal');
revealRegion(target, false); media.matches = true; media.dispatchEvent(new Event('change'));
assert.equal(target.hidden, true, 'reduced motion completes pending semantic state');
assert.equal(reducedMotion(), true); media.matches = false;
assert.equal(motionDuration('connect'), 280);
globalThis.getComputedStyle = () => ({ getPropertyValue: () => '0.18s' });
assert.equal(motionDuration('reveal'), 180, 'theme timing is read at interaction time');
globalThis.getComputedStyle = () => ({ getPropertyValue: () => '' });
let destroyed = false;
transition(target, [], { done: () => { destroyed = true; } }); disposeMotion(target);
target.animations.at(-1).complete(); await flush(); assert.equal(destroyed, false);
let stalled = false;
transition(target, [], { duration: 1, done: () => { stalled = true; } });
await new Promise(resolve => setTimeout(resolve, 130)); assert.equal(stalled, true);

// Real overlay implementations, with a small DOM model rather than copied code.
const background = document.createElement('main'); document.body.append(background);
const trigger = document.createElement('button'); background.append(trigger); trigger.focus();
const action = document.createElement('button');
let closes = 0;
const first = modal({ label: 'First', body: [action], onClose: () => { closes++; } });
first.open(); assert.equal(background.inert, true); assert.equal(document.activeElement, action);
first.close(); first.open();
first.card.animations.forEach(animation => animation.complete()); await flush();
assert.equal(first.isOpen(), true); assert.equal(first.el.hidden, false, 'stale exit cannot hide a reopened modal');
const second = sheet({ label: 'Second', body: [document.createElement('button')] }); second.open();
assert.equal(first.el.inert, true); assert.equal(second.el.inert, false);
first.close();
assert.equal(background.inert, true, 'out-of-order dismissal preserves the top dialog isolation');
second.close(); assert.equal(background.inert, false, 'last dialog restores original inert state');
first.open(); first.close(); first.close(); assert.equal(closes, 3, 'duplicate close is inert');
first.destroy(); second.destroy();
assert.equal(first.el.isConnected, false); assert.equal(second.el.isConnected, false);
assert.equal(background.inert, false);
const once = modal({ label: 'One shot', removeOnClose: true });
once.open(); once.close(); once.card.animations.at(-1).complete(); await flush();
assert.equal(once.el.isConnected, false, 'one-shot overlays clean up after shared exit timing');
console.log('Shared motion cancellation, reduced motion, renderer fallback and overlay lifecycle checks passed');

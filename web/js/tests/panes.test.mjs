/**
 * paneStack + backStack — the phone's one-pane-at-a-time layout and the one
 * "go back" registry. Run: node web/js/tests/panes.test.mjs
 *
 * A small DOM shim is enough: the stack only reads widths and classes, moves
 * focus and listens for Escape. The back button (an icon) is exercised with
 * the icon factory against the same shim.
 */
import './helpers/i18n.mjs';
import assert from 'node:assert/strict';

/* ── DOM shim ────────────────────────────────────────────────── */

class El extends EventTarget {
  constructor(tag) {
    super();
    this.tagName = tag.toUpperCase();
    this.children = [];
    this.parentNode = null;
    this.dataset = {};
    this.attrs = {};
    this.style = { setProperty(k, v) { this[k] = v; }, removeProperty(k) { delete this[k]; } };
    this.clientWidth = 0;
    this.scrollTop = 0;
    this.hidden = false;
    this.isConnected = true;
    this.textContent = '';
    this.innerHTML = '';
    this._classes = new Set();
    const self = this;
    this.classList = {
      add: (...n) => n.forEach((c) => self._classes.add(c)),
      remove: (...n) => n.forEach((c) => self._classes.delete(c)),
      toggle: (c, on) => { const want = on === undefined ? !self._classes.has(c) : !!on; if (want) self._classes.add(c); else self._classes.delete(c); return want; },
      contains: (c) => self._classes.has(c),
    };
  }
  get className() { return [...this._classes].join(' '); }
  set className(v) { this._classes = new Set(String(v).split(/\s+/).filter(Boolean)); }
  setAttribute(k, v) { this.attrs[k] = String(v); if (k === 'tabindex') this.tabIndex = Number(v); }
  getAttribute(k) { return this.attrs[k] ?? null; }
  hasAttribute(k) { return k in this.attrs; }
  removeAttribute(k) { delete this.attrs[k]; }
  append(...nodes) { for (const n of nodes) { n.parentNode = this; this.children.push(n); } }
  appendChild(n) { this.append(n); return n; }
  prepend(...nodes) { for (const n of nodes.reverse()) { n.parentNode = this; this.children.unshift(n); } }
  remove() { if (this.parentNode) this.parentNode.children = this.parentNode.children.filter((c) => c !== this); this.parentNode = null; this.isConnected = false; }
  matches(sel) { return sel.split(',').some((s) => this._match(s.trim())); }
  _match(s) {
    if (s.startsWith('[')) return this.hasAttribute(s.slice(1, -1));
    if (s.startsWith('.')) return this.classList.contains(s.slice(1));
    return this.tagName === s.toUpperCase();
  }
  closest(sel) { for (let n = this; n; n = n.parentNode) if (n.matches && n.matches(sel)) return n; return null; }
  querySelector(sel) {
    const parts = sel.split(',').map((s) => s.trim());
    const walk = (n) => { for (const c of n.children) { if (parts.some((p) => c._match(p))) return c; const d = walk(c); if (d) return d; } return null; };
    return walk(this);
  }
  focus() { document.activeElement = this; }
}

globalThis.document = {
  documentElement: new El('html'),
  body: new El('body'),
  activeElement: null,
  createElement: (t) => new El(t),
  createElementNS: (_ns, t) => new El(t),
};
globalThis.window = { setTimeout, clearTimeout, addEventListener() {}, removeEventListener() {} };
globalThis.CustomEvent = class CustomEvent extends Event {
  constructor(type, init = {}) { super(type, init); this.detail = init.detail; }
};
globalThis.navigator = { vibrate() {} };

const { paneStack } = await import('../../ui/components/panes.js');
const { pushBack, popBack, backDepth } = await import('../../ui/backStack.js');

/* ── backStack ───────────────────────────────────────────────── */

{
  const calls = [];
  const a = pushBack(() => { calls.push('a'); return true; });
  const b = pushBack(() => { calls.push('b'); return false; });
  assert.equal(backDepth(), 2);
  assert.equal(popBack(), true, 'the newest handler is asked first, the first consumer wins');
  assert.deepEqual(calls, ['b', 'a']);
  b(); a();
  assert.equal(backDepth(), 0);
  assert.equal(popBack(), false, 'nothing registered: nothing consumed');
}

/* ── paneStack: narrow window ────────────────────────────────── */

{
  const tile = new El('section'); tile.classList.add('tile'); tile.clientWidth = 480;
  const root = new El('div'); tile.append(root);
  const list = new El('div');
  const detail = new El('aside');
  const bar = new El('header'); bar.setAttribute('data-pane-bar', ''); detail.append(bar);
  const editor = new El('div');
  root.append(list, detail, editor);
  const opener = new El('button'); list.append(opener); opener.focus();

  const changes = [];
  const nav = paneStack(root, { panes: [{ id: 'list', el: list }, { id: 'detail', el: detail }, { id: 'editor', el: editor }], onChange: (d) => changes.push(d.id) });
  assert.ok(root.classList.contains('ui-panes') && root.classList.contains('is-stacked'), 'a 480px window stacks');
  assert.equal(list.dataset.pane, 'list');
  assert.equal(detail.attrs['aria-hidden'], 'true', 'a closed pane is hidden from assistive tech while stacked');

  assert.ok(nav.show('detail'));
  assert.ok(detail.classList.contains('is-open'));
  assert.equal(nav.top(), 'detail');
  const back = bar.children[0];
  assert.ok(back && back.classList.contains('ui-pane-back'), 'the pane bar gets a back button first');
  assert.equal(document.activeElement, back, 'focus moves into the pane, onto its back button');
  assert.equal(backDepth(), 1, 'an open pane waits for the phone’s Back');

  nav.show('editor');
  assert.deepEqual(nav.open(), ['list', 'detail', 'editor']);
  assert.equal(popBack(), true, 'Back pops the top pane');
  assert.equal(nav.top(), 'detail');
  assert.ok(!editor.classList.contains('is-open'));

  // Dispatched on the pane: the shim has no bubbling, so the root's listener
  // is called with the pane as the target the way the browser would.
  const esc = new Event('keydown', { cancelable: true }); esc.key = 'Escape';
  Object.defineProperty(esc, 'target', { value: detail });
  root.dispatchEvent(esc);
  assert.equal(nav.top(), 'list', 'Escape pops too');
  assert.equal(document.activeElement, opener, 'focus returns to the opener once the stack is empty');
  assert.equal(backDepth(), 0);
  assert.deepEqual(changes, ['list', 'detail', 'editor', 'detail', 'list'], 'the first change is the layout stacking');

  nav.destroy();
  assert.ok(!root.classList.contains('ui-panes'));
  assert.equal(bar.children.length, 0, 'destroy removes the back button');
}

/* ── paneStack: wide window, then a phone ────────────────────── */

{
  const tile = new El('section'); tile.classList.add('tile'); tile.clientWidth = 1200;
  const root = new El('div'); tile.append(root);
  const a = new El('div'); const b = new El('div');
  root.append(a, b);
  const nav = paneStack(root, { panes: [{ id: 'a', el: a }, { id: 'b', el: b }] });
  assert.ok(!nav.isStacked(), 'a wide window lays its panes side by side');
  nav.show('b');
  assert.ok(b.classList.contains('is-open'), 'the selection is remembered even while wide');
  assert.equal(backDepth(), 0, 'nothing to go back from while wide');

  document.documentElement.dataset.phone = '1';
  nav.refresh();
  assert.ok(nav.isStacked(), 'a phone stacks whatever the width');
  assert.equal(backDepth(), 1, 'the open pane now waits for Back');
  delete document.documentElement.dataset.phone;
  nav.refresh();
  assert.equal(backDepth(), 0);
  nav.destroy();
}

console.log('paneStack and backStack checks passed');

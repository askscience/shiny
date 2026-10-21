/**
 * HUD menu lifecycle test.
 *
 * The repo's JS tests have no DOM, so this file brings the minimum stub the
 * factory needs: elements, class lists, document/window listener plumbing and
 * `placeMenu`'s geometry reads. It covers the behaviour every top-bar popover
 * shares: build once, unhide under the chip, close on outside click/Escape,
 * and stay open while a modal is on screen.
 *
 * Run:  node web/js/tests/menuFactory.test.mjs
 */

import { createMenu } from '../menuFactory.js';

let failures = 0;
function check(name, condition, detail = '') {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

function classList(initial = []) {
  const set = new Set(initial);
  return {
    add: (...names) => names.forEach((n) => set.add(n)),
    remove: (...names) => names.forEach((n) => set.delete(n)),
    contains: (name) => set.has(name),
    toggle: (name, force) => {
      const on = force === undefined ? !set.has(name) : force;
      if (on) set.add(name); else set.delete(name);
      return on;
    },
  };
}

function makeEl(tag = 'div') {
  return {
    tagName: tag.toUpperCase(),
    style: {},
    classList: classList(),
    children: [],
    attributes: {},
    offsetWidth: 200,
    offsetHeight: 100,
    appendChild(child) { this.children.push(child); return child; },
    setAttribute(key, value) { this.attributes[key] = value; },
    getAttribute(key) { return this.attributes[key]; },
    contains(node) { return node === this || this.children.includes(node); },
    getBoundingClientRect() { return { left: 1000, right: 1100, top: 20, bottom: 40 }; },
    addEventListener() {},
    removeEventListener() {},
  };
}

function listenerBook() {
  const map = new Map();
  return {
    add(type, fn) {
      const list = map.get(type) || [];
      if (!list.includes(fn)) map.set(type, [...list, fn]);
    },
    remove(type, fn) { map.set(type, (map.get(type) || []).filter((f) => f !== fn)); },
    get(type) { return map.get(type) || []; },
  };
}

const documentListeners = listenerBook();
const windowListeners = listenerBook();
let modalOpen = false;

globalThis.document = {
  body: makeEl(),
  createElement: (tag) => makeEl(tag),
  querySelector: (selector) => (modalOpen && selector === '.ui-modal:not(.hidden)' ? makeEl() : null),
  addEventListener: (type, fn) => documentListeners.add(type, fn),
  removeEventListener: (type, fn) => documentListeners.remove(type, fn),
};

globalThis.window = {
  innerWidth: 1280,
  innerHeight: 800,
  addEventListener: (type, fn) => windowListeners.add(type, fn),
  removeEventListener: (type, fn) => windowListeners.remove(type, fn),
  dispatchEvent: () => true,
};

const popup = makeEl();
popup.className = 'ui-hud-menu-popup test-menu hidden';
popup.classList.add('hidden');
const chip = makeEl('button');
let built = 0;
let opens = 0;
let closes = 0;

const menu = createMenu({
  build: () => {
    built += 1;
    return popup;
  },
  align: 'right',
  onOpen: () => { opens += 1; },
  onClose: () => { closes += 1; },
});

check('popup is built lazily', built === 0);
check('starts closed', !menu.isOpen());

menu.show(chip);

check('popup is built once', built === 1);
check('popup is attached to body', document.body.children.includes(popup));
check('popup is visible', !popup.classList.contains('hidden'));
check('chip reports expanded', chip.getAttribute('aria-expanded') === 'true');
check('chip gets the open class', chip.classList.contains('is-open'));
check('menu reports open', menu.isOpen());
check('onOpen runs once', opens === 1);
check('menu is positioned', popup.style.left === '900px' && popup.style.top === '48px');
check('document pointerdown listener attached', documentListeners.get('pointerdown').length === 1);
check('document keydown listener attached', documentListeners.get('keydown').length === 1);
check('resize listener attached', windowListeners.get('resize').length === 1);

menu.show(chip);
check('reopening reuses the popup', built === 1 && opens === 2);

const outside = makeEl();
documentListeners.get('pointerdown')[0]({ target: outside });
check('outside click closes', !menu.isOpen());

menu.show(chip);
const inside = makeEl();
popup.children.push(inside);
documentListeners.get('pointerdown')[0]({ target: inside });
check('inside click keeps it open', menu.isOpen());

const closesBeforeEscape = closes;
documentListeners.get('keydown')[0]({ key: 'Escape' });
check('Escape closes', !menu.isOpen());
check('onClose runs once per close', closes === closesBeforeEscape + 1);
check('chip reports collapsed', chip.getAttribute('aria-expanded') === 'false');
check('chip open class removed', !chip.classList.contains('is-open'));
check('listeners removed', documentListeners.get('pointerdown').length === 0
  && documentListeners.get('keydown').length === 0
  && windowListeners.get('resize').length === 0);

menu.show(chip);
modalOpen = true;
documentListeners.get('keydown')[0]({ key: 'Escape' });
check('Escape is ignored while a modal is open', menu.isOpen());
modalOpen = false;

menu.close();
check('close hides the popup', popup.classList.contains('hidden'));
const closesAfterModal = closes;
menu.close();
check('closing twice is a no-op', closes === closesAfterModal);

menu.toggle(chip);
check('toggle opens', menu.isOpen());
menu.toggle(chip);
check('toggle closes', !menu.isOpen());
check('popup is built exactly once', built === 1);

if (failures) {
  console.error(`\n${failures} menu factory check(s) failed`);
  process.exit(1);
}
console.log('\nall menu factory checks passed');

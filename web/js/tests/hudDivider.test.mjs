/**
 * Unit checks for hudDivider.js against a minimal DOM stub. Run:
 *   node web/js/tests/hudDivider.test.mjs
 */

import { hudDivider, initHudDividers } from '../hudDivider.js';

let failures = 0;
function check(name, condition) {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}`);
  }
}

/** Just enough of an element for the factory's after/before mounting. */
class FakeEl {
  constructor(tag = 'div') {
    this.tagName = tag.toUpperCase();
    this.className = '';
    this.attributes = {};
    this.parentNode = null;
    this.childNodes = [];
  }

  setAttribute(name, value) { this.attributes[name] = String(value); }

  get classList() {
    return {
      add: (name) => {
        const parts = this.className.split(' ').filter(Boolean);
        if (!parts.includes(name)) this.className = [...parts, name].join(' ');
      },
    };
  }

  append(...nodes) {
    for (const node of nodes) {
      node.parentNode = this;
      this.childNodes.push(node);
    }
    return this;
  }

  after(node) { insert(this, node, this.parentNode.childNodes.indexOf(this) + 1); }
  before(node) { insert(this, node, this.parentNode.childNodes.indexOf(this)); }
}

function insert(anchor, node, index) {
  node.parentNode = anchor.parentNode;
  anchor.parentNode.childNodes.splice(index, 0, node);
}

const clock = new FakeEl('button');
const meteo = new FakeEl('button');
const top = new FakeEl('div');
const clipboard = new FakeEl('button');
top.append(clipboard);
const bar = new FakeEl('div');
bar.append(clock, meteo, top);

globalThis.document = {
  createElement: (tag) => new FakeEl(tag),
  querySelector: (sel) => (sel === '.hud-clock' ? clock : null),
  getElementById: (id) => ({ 'hud-meteo': meteo, 'hud-top': top, 'hud-clipboard': clipboard }[id] ?? null),
};

const a = hudDivider();
const b = hudDivider();
check('each call builds a fresh element', a !== b);
check('separator is a div', a.tagName === 'DIV');
check('class is hud-divider', a.className === 'hud-divider');
check('separator is aria-hidden', a.attributes['aria-hidden'] === 'true');

initHudDividers();

const [c, d1, m, d2, push, t] = bar.childNodes;
check('row order: clock, divider, weather, divider, push divider, hud-top',
  c === clock && m === meteo && t === top && bar.childNodes.length === 6);
check('divider between clock and weather', d1.className === 'hud-divider');
check('divider between weather and the tray', d2.className === 'hud-divider');
check('push divider carries the modifier', push.className === 'hud-divider hud-divider--push');
check('push divider is not plain', d1.className !== push.className);
check('divider before the first action', top.childNodes[0].className === 'hud-divider'
  && top.childNodes[1] === clipboard);

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall hudDivider checks passed');

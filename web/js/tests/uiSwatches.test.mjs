import assert from 'node:assert/strict';

/* A row of colour swatches: one is chosen, the arrows move the choice, and an
   option without a colour is the "none" choice. */
class Node extends EventTarget {
  constructor(tag) {
    super(); this.tagName = tag.toUpperCase(); this.children = []; this.attributes = {}; this.className = '';
    this.style = { setProperty: (key, value) => { this.style[key] = value; } };
    this.focused = false;
  }
  setAttribute(key, value) { this.attributes[key] = String(value); }
  appendChild(node) { this.children.push(node); return node; }
  focus() { this.focused = true; }
}
globalThis.document = { createElement: tag => new Node(tag), documentElement: { dir: 'ltr' } };

const { swatches } = await import('../../ui/components/choice.js');

const seen = [];
const row = swatches({
  label: 'Scarf',
  value: 2,
  options: [{ value: 0, label: 'None' }, { value: 1, color: '#f9aab4', label: 'Rose' }, { value: 2, color: '#96c4f0', label: 'Sky' }],
  onChange: value => seen.push(value),
});
assert.equal(row.attributes.role, 'radiogroup');
assert.equal(row.attributes['aria-label'], 'Scarf');
const [none, rose, sky] = row.children;
assert.equal(none.className, 'ui-swatch ui-swatch--none', 'no colour is the "none" choice');
assert.equal(rose.style['--swatch'], '#f9aab4');
assert.equal(rose.attributes['aria-label'], 'Rose');
assert.equal(sky.attributes['aria-checked'], 'true');
assert.equal(sky.tabIndex, 0, 'only the chosen swatch is in the tab order');
assert.equal(rose.tabIndex, -1);

rose.dispatchEvent(new Event('click'));
assert.equal(row.value, 1);
assert.deepEqual(seen, [1]);
assert.equal(rose.attributes['aria-checked'], 'true');
assert.equal(sky.attributes['aria-checked'], 'false');

rose.dispatchEvent(new Event('click'));
assert.deepEqual(seen, [1], 'choosing the chosen one again is not a change');

const key = name => Object.assign(new Event('keydown'), { key: name, preventDefault() {} });
row.dispatchEvent(key('ArrowRight'));
assert.equal(row.value, 2);
assert.ok(sky.focused, 'the arrows move the focus with the choice');
row.dispatchEvent(key('ArrowRight'));
assert.equal(row.value, 0, 'and wrap round');
row.dispatchEvent(key('End'));
assert.equal(row.value, 2);

row.setValue(1, { silent: true });
assert.deepEqual(seen, [1, 2, 0, 2], 'a silent change tells nobody');
row.setValue(99);
assert.equal(row.value, 1, 'a value that is not on offer is ignored');
row.destroy();
console.log('ui swatches: ok');

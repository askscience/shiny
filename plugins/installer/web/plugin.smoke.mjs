/**
 * Installer timeline smoke test.
 *
 * Run: node --experimental-vm-modules plugins/installer/web/plugin.smoke.mjs
 * It renders the real timeline against a synthetic disk. No helper socket,
 * no fetch, no /dev access.
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
import { GIB, MIB, placementForPlan } from './timeline-model.js';

const here = dirname(fileURLToPath(import.meta.url));

class StubNode {
  constructor(tag) {
    this.tagName = String(tag).toUpperCase();
    this.children = [];
    this.attrs = {};
    this.listeners = {};
    this.style = {};
    this.dataset = {};
    this.className = '';
    this.textContentValue = '';
    this.value = '';
    this.checked = false;
    this.disabled = false;
    this.type = '';
    this.title = '';
    this.tabIndex = 0;
    this.open = false;
    this.dir = '';
    this.parentNode = null;
    this.classList = {
      add: (...names) => {
        const current = new Set(String(this.className).split(/\s+/).filter(Boolean));
        for (const name of names) current.add(name);
        this.className = [...current].join(' ');
      },
      remove: (...names) => {
        const current = new Set(String(this.className).split(/\s+/).filter(Boolean));
        for (const name of names) current.delete(name);
        this.className = [...current].join(' ');
      },
      toggle: (name, force) => {
        const current = new Set(String(this.className).split(/\s+/).filter(Boolean));
        const on = force === undefined ? !current.has(name) : !!force;
        if (on) current.add(name); else current.delete(name);
        this.className = [...current].join(' ');
        return on;
      },
      contains: (name) => String(this.className).split(/\s+/).includes(name),
    };
  }

  get textContent() { return this.textContentValue; }
  set textContent(value) {
    this.textContentValue = String(value);
    if (this.textContentValue === '') this.children = [];
  }

  appendChild(child) {
    if (!child) return child;
    child.parentNode = this;
    this.children.push(child);
    return child;
  }

  append(...children) {
    for (const child of children) {
      if (typeof child === 'string') this.appendChild(new StubNode('span'));
      else this.appendChild(child);
    }
  }

  setAttribute(name, value) { this.attrs[name] = String(value); }
  getAttribute(name) { return this.attrs[name]; }
  addEventListener(type, handler) {
    if (!this.listeners[type]) this.listeners[type] = [];
    this.listeners[type].push(handler);
  }
  dispatch(type, extra = {}) {
    const event = {
      type,
      target: this,
      currentTarget: this,
      preventDefault() {},
      stopPropagation() {},
      ...extra,
    };
    for (const handler of this.listeners[type] || []) handler(event);
    return event;
  }
  getBoundingClientRect() {
    return { width: 1000, height: 44, left: 0, top: 0, right: 1000, bottom: 44 };
  }
  setPointerCapture() {}
  releasePointerCapture() {}
}

const documentStub = {
  activeElement: null,
  createElement: (tag) => new StubNode(tag),
};
const windowStub = {
  listeners: {},
  addEventListener(type, handler) {
    if (!this.listeners[type]) this.listeners[type] = [];
    this.listeners[type].push(handler);
  },
  removeEventListener(type, handler) {
    this.listeners[type] = (this.listeners[type] || []).filter((item) => item !== handler);
  },
  dispatch(type, extra = {}) {
    const event = { type, preventDefault() {}, ...extra };
    for (const handler of [...(this.listeners[type] || [])]) handler(event);
  },
};

globalThis.document = documentStub;
globalThis.window = windowStub;

const { renderTimelinePanel } = await import('./timeline.js');

const freeStart = 300 * GIB + MIB;
const disk = {
  name: 'nvme0n1',
  path: '/dev/nvme0n1',
  size_bytes: 1000 * GIB,
  serial: 'S1',
  model: 'Synthetic SSD',
  pttype: 'gpt',
  removable: false,
  mountpoints: [],
  children: [
    {
      name: 'nvme0n1p1', path: '/dev/nvme0n1p1',
      start_bytes: MIB, size_bytes: 200 * GIB,
      fstype: 'ntfs', mountpoints: [], holders: [],
    },
    {
      name: 'nvme0n1p2', path: '/dev/nvme0n1p2',
      start_bytes: 200 * GIB + MIB, size_bytes: 100 * GIB,
      fstype: 'ext4', mountpoints: [], holders: [],
    },
  ],
  free_extents: [{ start_bytes: freeStart, size_bytes: 700 * GIB - MIB }],
};

const state = {
  partitionMode: 'manual',
  layoutPreset: 'swap',
  swapGiB: 8,
  homeGiB: 0,
  manualSwapGiB: 8,
  manualHome: false,
  manualHomeGiB: 0,
  manualRootGiB: 0,
  selectedExtent: { start_bytes: freeStart, size_bytes: 700 * GIB - MIB },
  timelineZoom: 1,
  exactOpen: false,
  pendingExistingPath: '',
  existingRootPath: '',
};

const t = (key, fallback, params) => {
  let text = fallback ?? key;
  for (const [name, value] of Object.entries(params || {})) {
    text = text.replaceAll(`{${name}}`, String(value));
  }
  return text;
};
const formatBytes = (bytes) => `${Math.round(bytes / GIB)} GiB`;

let selectedGap = null;
let commits = 0;
let lastModel = null;

const panel = renderTimelinePanel({
  disk,
  state,
  model: null,
  t,
  formatBytes,
  onSelectGap: (gap) => { selectedGap = gap; },
  onSelectPartition: () => {},
  onPreset: () => {},
  onRemovePartition: () => {},
  onCommit: (_nextState, model) => { commits += 1; lastModel = model; },
  onRequestRender: () => {},
});

function walk(node, out = []) {
  out.push(node);
  for (const child of node.children || []) walk(child, out);
  return out;
}
function byClass(name) {
  return walk(panel).filter((node) => String(node.className).split(/\s+/).includes(name));
}

assert.ok(panel.className.includes('installer-timeline'));
assert.equal(byClass('installer-track--now').length, 1);
assert.equal(byClass('installer-track--after').length, 1);
assert.equal(byClass('installer-clip--planned').length >= 2, true);

const gaps = byClass('installer-clip--free').filter((node) => node.tagName === 'BUTTON');
assert.equal(gaps.length, 1);
gaps[0].dispatch('click');
assert.equal(selectedGap.start_bytes, freeStart);

const handles = byClass('installer-handle');
assert.equal(handles.length, 2);
const swapHandle = handles.find((node) => node.dataset.boundary === 'swap-root');
assert.ok(swapHandle);

const swapBefore = state.manualSwapGiB;
swapHandle.dispatch('keydown', { key: 'ArrowRight' });
assert.equal(state.manualSwapGiB, swapBefore + 1);
assert.equal(commits, 1);
assert.ok(lastModel.sizes.root >= 20 * GIB);

const beforeDrag = state.manualSwapGiB;
swapHandle.dispatch('pointerdown', {
  button: 0,
  pointerId: 1,
  pointerType: 'mouse',
  clientX: 100,
});
windowStub.dispatch('pointermove', { pointerId: 1, clientX: 110 });
windowStub.dispatch('pointerup', { pointerId: 1, clientX: 110 });
assert.equal(state.manualSwapGiB, beforeDrag + 10);
assert.ok(commits >= 2);

const beforeCancel = state.manualSwapGiB;
swapHandle.dispatch('pointerdown', {
  button: 0,
  pointerId: 2,
  pointerType: 'mouse',
  clientX: 100,
});
windowStub.dispatch('pointermove', { pointerId: 2, clientX: 130 });
windowStub.dispatch('keydown', { key: 'Escape' });
windowStub.dispatch('pointerup', { pointerId: 2, clientX: 130 });
assert.equal(state.manualSwapGiB, beforeCancel);

const placement = placementForPlan(disk, state);
assert.equal(placement.start_bytes, freeStart);
assert.equal(placement.size_bytes, 700 * GIB - MIB);

console.log('installer timeline smoke: all checks passed');

/* ── Wizard integration: mount the real plugin in a VM context ── */

function integrationElement(tag) {
  const el = {
    tagName: String(tag).toUpperCase(),
    children: [],
    attrs: {},
    listeners: {},
    style: {},
    dataset: {},
    className: '',
    textContentValue: '',
    valueValue: '',
    checked: false,
    disabled: false,
    type: '',
    title: '',
    placeholder: '',
    autocomplete: '',
    id: '',
    tabIndex: 0,
    open: false,
    dir: '',
    href: '',
    rel: '',
    scrollTop: 0,
    scrollLeft: 0,
    clientWidth: 400,
    clientHeight: 300,
    append(...kids) {
      for (const kid of kids) {
        if (kid != null) this.children.push(kid);
      }
    },
    appendChild(kid) { this.children.push(kid); return kid; },
    remove() {},
    setAttribute(name, value) { this.attrs[name] = String(value); },
    getAttribute(name) { return this.attrs[name]; },
    removeAttribute(name) { delete this.attrs[name]; },
    addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); },
    removeEventListener(type, fn) { this.listeners[type] = (this.listeners[type] || []).filter((f) => f !== fn); },
    dispatch(type, extra = {}) {
      const event = { type, target: this, currentTarget: this, preventDefault() {}, stopPropagation() {}, ...extra };
      for (const fn of [...(this.listeners[type] || [])]) fn(event);
      return event;
    },
    querySelector(selector) { return descendants(this).find((node) => matches(node, selector)) || null; },
    querySelectorAll(selector) { return descendants(this).filter((node) => matches(node, selector)); },
    getBoundingClientRect() { return { left: 0, top: 0, width: 1000, height: 44, right: 1000, bottom: 44 }; },
    getClientRects() { return [{}]; },
    scrollIntoView() {},
    focus() {},
    blur() {},
    setPointerCapture() {},
    releasePointerCapture() {},
  };
  Object.defineProperty(el, 'textContent', {
    get() { return el.textContentValue; },
    set(value) { el.textContentValue = String(value ?? ''); el.children = []; },
  });
  Object.defineProperty(el, 'value', {
    get() { return el.valueValue; },
    set(value) { el.valueValue = String(value ?? ''); },
  });
  el.classList = {
    add(...names) {
      const set = new Set(String(el.className).split(/\s+/).filter(Boolean));
      for (const name of names) set.add(name);
      el.className = [...set].join(' ');
    },
    remove(...names) {
      const set = new Set(String(el.className).split(/\s+/).filter(Boolean));
      for (const name of names) set.delete(name);
      el.className = [...set].join(' ');
    },
    toggle(name, force) {
      const set = new Set(String(el.className).split(/\s+/).filter(Boolean));
      const on = force === undefined ? !set.has(name) : !!force;
      if (on) set.add(name); else set.delete(name);
      el.className = [...set].join(' ');
      return on;
    },
    contains(name) { return String(el.className).split(/\s+/).includes(name); },
  };
  return el;
}

function descendants(root, out = []) {
  for (const child of root.children || []) {
    out.push(child);
    descendants(child, out);
  }
  return out;
}

function matches(node, selector) {
  return String(selector).split(',').some((raw) => {
    const part = raw.trim();
    if (!part) return false;
    const attr = part.match(/^\[([\w-]+)(?:="([^"]*)")?\]$/);
    if (attr) return attr[2] === undefined ? node.attrs[attr[1]] !== undefined : node.attrs[attr[1]] === attr[2];
    const tag = part.match(/^([a-zA-Z]+)/);
    if (tag && node.tagName !== tag[1].toUpperCase()) return false;
    const classes = [...part.matchAll(/\.([\w-]+)/g)].map((match) => match[1]);
    return classes.every((name) => node.classList.contains(name));
  });
}

async function runWizardIntegration() {
  const body = integrationElement('body');
  const head = integrationElement('head');
  const documentStub = {
    body,
    head,
    documentElement: integrationElement('html'),
    activeElement: null,
    createElement: integrationElement,
    createElementNS: (_namespace, tag) => integrationElement(tag),
    getElementById: () => null,
    querySelector: () => null,
    querySelectorAll: () => [],
    addEventListener() {},
    removeEventListener() {},
    dispatchEvent() {},
  };
  const windowStub = {
    document: documentStub,
    listeners: {},
    addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); },
    removeEventListener(type, fn) { this.listeners[type] = (this.listeners[type] || []).filter((f) => f !== fn); },
    setInterval: () => 0,
    clearInterval: () => {},
    setTimeout,
    clearTimeout,
    matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }),
  };

  const calls = [];
  const health = {
    version: 'test',
    protocol: 4,
    dry_run: true,
    root: false,
    socket: '',
    hardware: { vendor: 'Test', product: 'Synthetic machine', board: '', is_t2: false, firmware: 'uefi', secure_boot: false, gpus: [], live_medium: null },
  };
  const options = {
    locales: ['it_IT.UTF-8', 'en_US.UTF-8'],
    keyboards: ['it', 'us'],
    timezones: ['Europe/Rome', 'UTC'],
    default_locale: 'it_IT.UTF-8',
    default_keyboard: 'it',
    default_timezone: 'Europe/Rome',
  };
  const probe = {
    live_medium: null,
    hardware: health.hardware,
    disks: [{
      name: 'nvme0n1', path: '/dev/nvme0n1', kind: 'disk',
      size_bytes: 1000 * GIB, start_bytes: 0, model: 'Synthetic SSD', serial: 'TEST-SERIAL',
      tran: 'nvme', removable: false, read_only: false, is_loop: false, is_rom: false,
      mountpoints: [], fstype: '', pttype: 'gpt', partuuid: '', uuid: '', label: '', fsver: '',
      parttypename: '', partflags: [], holders: [],
      free_extents: [{ start_bytes: freeStart, size_bytes: 700 * GIB - MIB }],
      children: [
        { name: 'nvme0n1p1', path: '/dev/nvme0n1p1', kind: 'part', size_bytes: 200 * GIB,
          start_bytes: MIB, fstype: 'ntfs', mountpoints: [], holders: [], partuuid: 'p1', uuid: 'u1', partflags: [], children: [] },
        { name: 'nvme0n1p2', path: '/dev/nvme0n1p2', kind: 'part', size_bytes: 100 * GIB,
          start_bytes: 200 * GIB + MIB, fstype: 'ext4', mountpoints: [], holders: [], partuuid: 'p2', uuid: 'u2', partflags: [], children: [] },
      ],
    }],
  };

  const plans = [];
  const apiFetch = async (url, init = {}) => {
    calls.push(url);
    if (String(url).includes('/api/installer/health')) return { data: health };
    if (String(url).includes('/api/installer/options')) return { data: options };
    if (String(url).includes('/api/installer/probe')) return { data: probe };
    if (String(url).includes('/api/installer/plan')) {
      plans.push(JSON.parse(init.body));
      return { data: { ok: true, errors: [], warnings: [], hardware: health.hardware } };
    }
    throw new Error(`unexpected installer fetch: ${url}`);
  };
  const translate = (key, params, fallback) => {
    let text = fallback ?? key;
    for (const [name, value] of Object.entries(params || {})) text = text.replaceAll(`{${name}}`, String(value));
    return text;
  };
  const uiIcon = (name) => {
    const node = integrationElement('span');
    node.className = 'ui-icon';
    node.setAttribute('data-icon', name);
    return node;
  };
  const uiButton = ({ label = '', icon, onClick, disabled } = {}) => {
    const node = integrationElement('button');
    node.type = 'button';
    node.className = 'ui-btn';
    if (label) node.textContent = label;
    if (icon) node.appendChild(uiIcon(icon));
    if (disabled) node.disabled = true;
    if (onClick) node.addEventListener('click', onClick);
    return node;
  };
  const uiControlButton = () => uiButton();
  const uiInput = ({ type = 'text', value = '', placeholder = '', autocomplete = '', onInput } = {}) => {
    const node = integrationElement('input');
    node.type = type;
    node.value = value ?? '';
    node.placeholder = placeholder || '';
    node.autocomplete = autocomplete || '';
    if (onInput) node.addEventListener('input', () => onInput(node.value));
    return node;
  };
  const uiSelect = ({ options: items = [], value, onChange } = {}) => {
    const wrap = integrationElement('div');
    const select = integrationElement('select');
    for (const item of items) {
      const option = integrationElement('option');
      option.value = String(item.value);
      option.textContent = item.label;
      select.appendChild(option);
    }
    select.value = value ?? items[0]?.value ?? '';
    if (onChange) select.addEventListener('change', () => onChange(select.value));
    wrap.select = select;
    wrap.appendChild(select);
    return wrap;
  };
  const uiField = ({ label, hint, control } = {}) => {
    const wrap = integrationElement('div');
    wrap.className = 'ui-field';
    wrap.appendChild(integrationElement('label')).textContent = label || '';
    if (hint) wrap.appendChild(integrationElement('span')).textContent = hint;
    wrap.appendChild(control);
    return wrap;
  };
  const uiAvatar = () => {
    const node = integrationElement('div');
    node.set = () => {};
    return node;
  };
  const uiBadge = (text) => {
    const node = integrationElement('span');
    node.className = 'ui-badge';
    node.textContent = text;
    return node;
  };
  const uiKeyValue = (rows) => {
    const node = integrationElement('div');
    for (const row of rows || []) node.appendChild(integrationElement('span')).textContent = row.join(': ');
    return node;
  };
  const uiEmpty = ({ title = '', body: text = '', action } = {}) => {
    const node = integrationElement('div');
    node.appendChild(integrationElement('p')).textContent = title;
    node.appendChild(integrationElement('p')).textContent = text;
    if (action) node.appendChild(action);
    return node;
  };

  const context = vm.createContext({
    console,
    document: documentStub,
    window: windowStub,
    navigator: { language: 'it-IT' },
    requestAnimationFrame: (fn) => { fn(); return 1; },
    cancelAnimationFrame: () => {},
    queueMicrotask,
    setTimeout,
    clearTimeout,
    setInterval: () => 0,
    clearInterval: () => {},
    avatar: uiAvatar,
    badge: uiBadge,
    button: uiButton,
    controlButton: uiControlButton,
    emptyState: uiEmpty,
    field: uiField,
    icon: uiIcon,
    input: uiInput,
    keyValue: uiKeyValue,
    select: uiSelect,
    spinner: () => integrationElement('span'),
    toast: (text) => { calls.push(`toast:${text}`); },
    apiFetch,
    t: translate,
  });

  const stripCore = (source) => source.replace(
    /^import\s+[\s\S]*?\s+from\s+['"]\.\.\/\.\.[^'"]*['"];\s*/gm,
    '',
  );
  const modelModule = new vm.SourceTextModule(
    readFileSync(join(here, 'timeline-model.js'), 'utf8'),
    { context, identifier: 'timeline-model' },
  );
  const timelineModule = new vm.SourceTextModule(
    stripCore(readFileSync(join(here, 'timeline.js'), 'utf8')),
    { context, identifier: 'timeline' },
  );
  const pluginModule = new vm.SourceTextModule(
    stripCore(readFileSync(join(here, 'plugin.js'), 'utf8')),
    { context, identifier: 'installer-plugin' },
  );
  await pluginModule.link(async (specifier) => {
    if (specifier === './timeline-model.js') return modelModule;
    if (specifier === './timeline.js') return timelineModule;
    throw new Error(`unexpected import: ${specifier}`);
  });
  await pluginModule.evaluate();

  const mount = pluginModule.namespace.mountInstallerTile;
  assert.equal(typeof mount, 'function');
  const tile = mount();
  await new Promise((resolve) => setTimeout(resolve, 30));

  const steps = descendants(tile).filter((node) => node.classList.contains('installer-step-item'));
  assert.equal(steps.length, 5);
  steps[1].dispatch('click');

  const timeline = descendants(tile).filter((node) => node.classList.contains('installer-timeline'));
  assert.equal(timeline.length, 1);
  const planned = descendants(tile).filter((node) => node.classList.contains('installer-clip--planned'));
  assert.ok(planned.length >= 2);
  const gaps = descendants(tile).filter((node) => node.tagName === 'BUTTON'
    && node.classList.contains('installer-clip--free'));
  assert.ok(gaps.length >= 1);
  gaps[0].dispatch('click');
  const selectedGaps = descendants(tile).filter((node) => node.classList.contains('installer-clip--free')
    && node.classList.contains('is-selected'));
  assert.ok(selectedGaps.length >= 1);
  assert.ok(calls.every((url) => url.includes('/api/installer/health')
    || url.includes('/api/installer/options')
    || url.includes('/api/installer/probe')), `unexpected fetch: ${calls.join(', ')}`);

  const stepsAfterGap = descendants(tile).filter((node) => node.classList.contains('installer-step-item'));
  stepsAfterGap[2].dispatch('click');
  const setInput = (id, value) => {
    const input = descendants(tile).find((node) => node.id === id);
    assert.ok(input, `missing input ${id}`);
    input.value = value;
    input.dispatch('input');
  };
  setInput('installer-fullName', 'Ada Lovelace');
  setInput('installer-username', 'ada');
  setInput('installer-hostname', 'ada-peakd');
  setInput('installer-password', 'correct-horse');
  setInput('installer-password2', 'correct-horse');

  const stepsBeforeReview = descendants(tile).filter((node) => node.classList.contains('installer-step-item'));
  stepsBeforeReview[4].dispatch('click');
  await new Promise((resolve) => setTimeout(resolve, 50));

  assert.equal(plans.length, 1);
  const plan = plans[0];
  assert.equal(plan.mode, 'manual');
  assert.equal(plan.layout.free_extent.start_bytes, freeStart);
  assert.equal(plan.layout.free_extent.size_bytes, 700 * GIB - MIB);
  assert.equal(plan.layout.preserve.length, 2);
  assert.equal(plan.target.path, '/dev/nvme0n1');
  assert.ok(!calls.some((url) => url.includes('/api/installer/apply')));

  const stepsForModes = descendants(tile).filter((node) => node.classList.contains('installer-step-item'));
  stepsForModes[1].dispatch('click');
  const intents = descendants(tile).filter((node) => node.classList.contains('installer-intent'));
  assert.equal(intents.length, 3);
  intents[1].dispatch('click');
  const erasePlanned = descendants(tile).filter((node) => node.classList.contains('installer-clip--planned'));
  assert.ok(erasePlanned.length >= 2);

  const intentsAgain = descendants(tile).filter((node) => node.classList.contains('installer-intent'));
  intentsAgain[2].dispatch('click');
  const candidate = descendants(tile).find((node) => node.tagName === 'BUTTON'
    && node.classList.contains('installer-clip--kept'));
  assert.ok(candidate, 'the ext4 partition is selectable');
  candidate.dispatch('click');
  const confirmBlock = descendants(tile).find((node) => node.classList.contains('installer-existing-confirm'));
  assert.ok(confirmBlock, 'formatting an existing partition needs confirmation');
  const confirmButton = descendants(confirmBlock).find((node) => node.tagName === 'BUTTON'
    && String(node.textContent).includes('Format this partition'));
  assert.ok(confirmButton);
  confirmButton.dispatch('click');
  assert.equal(descendants(tile).filter((node) => node.classList.contains('installer-existing-confirm')).length, 0);
  console.log('installer wizard integration: all checks passed');
}

await runWizardIntegration();

/**
 * @@TITLE@@ — the plugin's window.
 *
 * The contract with the desktop (docs/plugins/authoring.md § Window surface):
 * the default export has `mount()` (returns a `section.tile` carrying
 * `data-plugin`), `unmount()`, `getElement()` and `wireEvents()`.
 *
 * Rules that keep a window working everywhere:
 * - import core modules by relative path (`../../ui/index.js`, `../../js/…`);
 * - build controls with the shared UI factories and icon names from the
 *   catalogue, never hand-drawn markup;
 * - every visible string goes through `t()` with a `plugin.@@NAME@@.` key that
 *   is in `web/i18n/en.json`, and its English as the fallback;
 * - no inline scripts or `eval`: the app's Content-Security-Policy refuses them.
 */
import { button, emptyState, input, hairlineList, hairlineRow, toast } from '../../ui/index.js';
import { apiFetch } from '../../js/api.js';
import { t } from '../../js/i18n.js';

const PLUGIN = '@@NAME@@';
const API = '/api/@@NAME@@/items';

let tileEl = null;
let bodyEl = null;
let fieldEl = null;
let wired = false;

function ensureStylesheet() {
  const id = '@@NAME@@-plugin-css';
  if (document.getElementById(id)) return;
  const link = document.createElement('link');
  link.id = id;
  link.rel = 'stylesheet';
  link.href = '/plugins/@@NAME@@/plugin.css';
  document.head.appendChild(link);
}

function render(items) {
  if (!bodyEl) return;
  bodyEl.textContent = '';
  if (!items.length) {
    bodyEl.appendChild(emptyState({
      title: t('plugin.@@NAME@@.empty', null, 'Nothing here yet'),
      body: t('plugin.@@NAME@@.emptyBody', null, 'Add an item, or ask the assistant to.'),
    }));
    return;
  }
  const list = hairlineList();
  for (const item of items) list.appendChild(hairlineRow({ title: item.text, meta: item.created_at }));
  bodyEl.appendChild(list);
}

async function load() {
  try {
    const res = await apiFetch(API);
    render(res?.data?.items || []);
  } catch {
    toast(t('plugin.@@NAME@@.loadFailed', null, 'Could not load the items'), { type: 'error' });
  }
}

async function add() {
  const text = (fieldEl?.value || '').trim();
  if (!text) return;
  try {
    await apiFetch(API, { method: 'POST', body: JSON.stringify({ text }) });
    fieldEl.value = '';
    await load();
  } catch {
    toast(t('plugin.@@NAME@@.addFailed', null, 'Could not add the item'), { type: 'error' });
  }
}

function mount() {
  if (tileEl) return tileEl;
  ensureStylesheet();

  tileEl = document.createElement('section');
  tileEl.className = 'tile @@NAME@@-tile';
  tileEl.dataset.plugin = PLUGIN;

  // The window's own top bar: the desktop merges it with the window controls.
  const bar = document.createElement('div');
  bar.className = '@@NAME@@-bar';
  bar.dataset.windowBar = '';
  const placeholder = t('plugin.@@NAME@@.placeholder', null, 'New item…');
  fieldEl = input({ placeholder, label: placeholder, maxlength: 500 });
  fieldEl.addEventListener('keydown', (e) => { if (e.key === 'Enter') void add(); });
  bar.append(fieldEl, button({
    icon: 'ui/plus',
    label: t('plugin.@@NAME@@.add', null, 'Add'),
    variant: 'primary',
    size: 'sm',
    onClick: () => void add(),
  }));

  bodyEl = document.createElement('div');
  bodyEl.className = '@@NAME@@-body';

  tileEl.append(bar, bodyEl);
  void load();
  return tileEl;
}

function unmount() {
  tileEl?.remove();
  tileEl = null;
  bodyEl = null;
  fieldEl = null;
}

/** The assistant used one of this plugin's tools: show the new state. */
function onAgentActions(e) {
  const mine = (e.detail || []).some((a) => String(a?.action || '').startsWith('@@IDENT@@_'));
  if (mine && tileEl) void load();
}

function wireEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
}

export default {
  name: PLUGIN,
  mount,
  unmount,
  getElement: () => tileEl,
  wireEvents,
};

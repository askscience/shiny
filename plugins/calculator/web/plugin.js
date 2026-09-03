/**
 * calculator.js — the Calculator plugin's window (basic + scientific).
 *
 * A flat calculator: a display (expression + result), a keypad, and a
 * collapsible history panel. `=` sends the expression to POST
 * /api/calculator/eval — the same Rust evaluator the AI tool uses — so the
 * window and the agent always agree. Results land in the shared
 * `calculator_history` log.
 *
 * AI wiring: `calculator_*` tool outcomes arrive via `agent:actions`; the
 * window shows the last `calculator_eval` result and refreshes its history.
 */

import { button, emptyState, toast } from '/ui/index.js';
import { setIcon } from '/ui/index.js';
import { apiFetch } from '/js/api.js';

export const CALCULATOR_PLUGIN = 'calculator';

let tileEl = null;
let exprEl = null;
let resultEl = null;
let keysEl = null;
let sciEl = null;
let historyEl = null;
let historyToggleBtn = null;

let expression = '';
let lastResult = null; // number|null — "Ans" for chained calculations
let sciMode = false;
let historyOpen = false;

/* ── API ────────────────────────────────────────────────────── */

async function apiEval(expression) {
  const res = await apiFetch('/api/calculator/eval', {
    method: 'POST',
    body: JSON.stringify({ expression }),
  });
  return res?.data ?? null;
}

async function apiHistory() {
  const res = await apiFetch('/api/calculator/history?limit=50');
  return res?.data ?? { history: [] };
}

async function apiClearHistory() {
  await apiFetch('/api/calculator/history', { method: 'DELETE' });
}

/* ── Display / expression state ─────────────────────────────── */

function renderDisplay() {
  if (!exprEl || !resultEl) return;
  exprEl.textContent = expression || '\u200b';
  resultEl.textContent = lastResult == null ? '' : String(lastResult);
}

function append(text) {
  expression += text;
  renderDisplay();
}

function backspace() {
  expression = expression.slice(0, -1);
  renderDisplay();
}

function clearAll() {
  expression = '';
  lastResult = null;
  renderDisplay();
}

/**
 * After "=" the display shows a result. If the user then presses an operator,
 * continue from the answer (like a pocket calculator); if they press a digit,
 * start a fresh expression.
 */
function continueFromResult(text) {
  if (lastResult != null && expression === '') {
    if (/^[+\-*/%^]/.test(text)) expression = String(lastResult);
    else lastResult = null;
  }
}

async function equals() {
  const src = expression.trim();
  if (!src) return;
  try {
    const r = await apiEval(src);
    expression = '';
    lastResult = r?.result_text ?? null;
    renderDisplay();
    await refreshHistory();
  } catch (e) {
    resultEl.textContent = 'Error';
    toast(e.message || 'Could not evaluate', { type: 'error' });
  }
}

/* ── Keypad ─────────────────────────────────────────────────── */

const OPERATOR_LABELS = {
  '+': 'add', '-': 'sub', '×': 'mul', '÷': 'div',
  '^': 'pow', '%': 'mod', '√': 'sqrt',
};

function keyBtn(label, action, cls = '') {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = `calculator-key ${cls}`.trim();
  b.textContent = label;
  if (label.length > 1 && !OPERATOR_LABELS[label]) b.classList.add('is-fn');
  b.addEventListener('click', action);
  return b;
}

function buildKeypad() {
  keysEl.textContent = '';

  const rows = [
    [
      keyBtn('C', clearAll, 'is-danger'),
      keyBtn('⌫', backspace),
      keyBtn('(', () => append('(')),
      keyBtn(')', () => append(')')),
      keyBtn('÷', () => { continueFromResult('/'); append('/'); }),
    ],
    [
      keyBtn('7', () => { continueFromResult('7'); append('7'); }),
      keyBtn('8', () => append('8')),
      keyBtn('9', () => append('9')),
      keyBtn('×', () => { continueFromResult('*'); append('*'); }),
      keyBtn('%', () => { continueFromResult('%'); append('%'); }),
    ],
    [
      keyBtn('4', () => append('4')),
      keyBtn('5', () => append('5')),
      keyBtn('6', () => append('6')),
      keyBtn('−', () => { continueFromResult('-'); append('-'); }),
      keyBtn('^', () => { continueFromResult('^'); append('^'); }),
    ],
    [
      keyBtn('1', () => append('1')),
      keyBtn('2', () => append('2')),
      keyBtn('3', () => append('3')),
      keyBtn('+', () => { continueFromResult('+'); append('+'); }),
      keyBtn('π', () => append('pi')),
    ],
    [
      keyBtn('±', () => { if (expression === '' && lastResult != null) { expression = String(-Number(lastResult)); lastResult = null; } else append('-'); renderDisplay(); }),
      keyBtn('0', () => append('0')),
      keyBtn('.', () => append('.')),
      keyBtn('=', equals, 'is-equals'),
      keyBtn('e', () => append('e')),
    ],
  ];
  for (const row of rows) {
    const r = document.createElement('div');
    r.className = 'calculator-key-row';
    row.forEach((b) => r.appendChild(b));
    keysEl.appendChild(r);
  }
}

function buildSciPad() {
  sciEl.textContent = '';
  const fns = [
    ['sin', () => append('sin(')],
    ['cos', () => append('cos(')],
    ['tan', () => append('tan(')],
    ['√', () => append('sqrt(')],
    ['asin', () => append('asin(')],
    ['acos', () => append('acos(')],
    ['atan', () => append('atan(')],
    ['x²', () => append('^2')],
    ['ln', () => append('ln(')],
    ['log', () => append('log(')],
    ['log₂', () => append('log2(')],
    ['exp', () => append('exp(')],
    ['deg', () => append('deg(')],
    ['rad', () => append('rad(')],
    ['!', () => append('!')],
    ['abs', () => append('abs(')],
    ['floor', () => append('floor(')],
    ['ceil', () => append('ceil(')],
    ['round', () => append('round(')],
    ['fact', () => append('fact(')],
  ];
  const row = document.createElement('div');
  row.className = 'calculator-key-row';
  fns.forEach(([label, action]) => row.appendChild(keyBtn(label, action)));
  sciEl.appendChild(row);
}

function toggleSci() {
  sciMode = !sciMode;
  sciEl.classList.toggle('hidden', !sciMode);
  historyToggleBtn?.classList.toggle('is-active', sciMode);
}

/* ── History panel ──────────────────────────────────────────── */

async function refreshHistory() {
  if (!historyEl) return;
  try {
    const { history } = await apiHistory();
    historyEl.textContent = '';
    if (!history || !history.length) {
      historyEl.appendChild(emptyState({ title: 'No history', body: 'Calculations you run here or ask the AI for appear below.' }));
      return;
    }
    for (const item of history) {
      const row = document.createElement('button');
      row.type = 'button';
      row.className = 'calculator-history-item';
      row.title = 'Reuse this result';
      const expr = document.createElement('span');
      expr.className = 'calculator-history-expr';
      expr.textContent = item.expression;
      const res = document.createElement('span');
      res.className = 'calculator-history-res';
      res.textContent = `= ${item.result}`;
      row.append(expr, res);
      row.addEventListener('click', () => {
        lastResult = item.result;
        expression = '';
        renderDisplay();
        closeHistory();
      });
      historyEl.appendChild(row);
    }
  } catch (e) {
    toast(e.message || 'Could not load history', { type: 'error' });
  }
}

async function clearHistory() {
  try {
    await apiClearHistory();
    await refreshHistory();
    toast('History cleared', { type: 'info' });
  } catch (e) {
    toast(e.message || 'Could not clear history', { type: 'error' });
  }
}

function toggleHistory() {
  historyOpen = !historyOpen;
  historyEl?.classList.toggle('hidden', !historyOpen);
  historyToggleBtn?.classList.toggle('is-active', historyOpen);
  if (historyOpen) void refreshHistory();
}

function closeHistory() {
  historyOpen = false;
  historyEl?.classList.add('hidden');
  historyToggleBtn?.classList.remove('is-active');
}

/* ── Tile lifecycle ─────────────────────────────────────────── */

/** Create the Calculator tile element (the plugin's window container). */
export function mountCalculatorTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile calculator-tile';
  tileEl.dataset.plugin = CALCULATOR_PLUGIN;

  /* Top bar: title + sci/history/clear controls */
  const bar = document.createElement('div');
  bar.className = 'calculator-bar';
  const title = document.createElement('div');
  title.className = 'calculator-title';
  title.textContent = 'Calculator';
  const spacer = document.createElement('div');
  spacer.className = 'calculator-bar-spacer';

  historyToggleBtn = button({ icon: 'ui/list', label: 'History', variant: 'ghost', size: 'sm', onClick: toggleHistory });
  historyToggleBtn.classList.add('ui-btn--icon', 'calculator-bar-btn');
  const sciBtn = button({ icon: 'ui/puzzle', label: 'Scientific', variant: 'ghost', size: 'sm', onClick: toggleSci });
  sciBtn.classList.add('ui-btn--icon', 'calculator-bar-btn');
  const clearBtn = button({ icon: 'ui/trash', label: 'Clear history', variant: 'ghost', size: 'sm', onClick: () => void clearHistory() });
  clearBtn.classList.add('ui-btn--icon', 'calculator-bar-btn');
  bar.append(title, spacer, historyToggleBtn, sciBtn, clearBtn);
  tileEl.appendChild(bar);

  /* Display */
  const display = document.createElement('div');
  display.className = 'calculator-display';
  exprEl = document.createElement('div');
  exprEl.className = 'calculator-display-expr';
  resultEl = document.createElement('div');
  resultEl.className = 'calculator-display-result';
  display.append(exprEl, resultEl);
  tileEl.appendChild(display);

  /* Scientific pad (collapsible) */
  sciEl = document.createElement('div');
  sciEl.className = 'calculator-sci hidden';
  tileEl.appendChild(sciEl);
  buildSciPad();

  /* Keypad */
  keysEl = document.createElement('div');
  keysEl.className = 'calculator-keys';
  tileEl.appendChild(keysEl);
  buildKeypad();

  /* History (collapsible, below the keypad) */
  historyEl = document.createElement('div');
  historyEl.className = 'calculator-history hidden';
  tileEl.appendChild(historyEl);

  renderDisplay();
  return tileEl;
}

/** Deactivated: drop the window. */
export function unmountCalculatorTile() {
  tileEl?.remove();
  tileEl = null;
  exprEl = null;
  resultEl = null;
  keysEl = null;
  sciEl = null;
  historyEl = null;
  historyToggleBtn = null;
}

/** The tile element (or null when the Calculator window is not mounted). */
export function getCalculatorTileElement() {
  return tileEl;
}

/* ── AI wiring ──────────────────────────────────────────────── */

function onAgentActions(e) {
  const actions = e.detail || [];
  const calcActions = actions.filter((a) => /^calculator_/.test(a?.action || ''));
  if (!calcActions.length) return;

  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: CALCULATOR_PLUGIN } }));

  const evaled = calcActions.find((a) => a.action === 'calculator_eval' && a.result === 'ok');
  if (evaled) {
    expression = '';
    lastResult = evaled.data?.result_text ?? null;
    renderDisplay();
  }
  if (historyOpen) void refreshHistory();
}

let wired = false;
export function wireCalculatorEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
}

export default {
  name: 'calculator',
  icon: 'ui/calculator',
  mount: mountCalculatorTile,
  unmount: unmountCalculatorTile,
  getElement: getCalculatorTileElement,
  wireEvents: wireCalculatorEvents,
};

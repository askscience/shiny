import {
  toolbar, toolButton, controlButton, emptyState, toast, setTileGlow, glowGradient,
  icon, eyebrow, hairlineList, listReveal, rollNumber,
} from '../../ui/index.js';

/**
 * calculator.js — the Calculator plugin's window (basic + scientific).
 *
 * An editorial calculator: a display (a small mono expression line over a
 * large display-font result that rolls up when set), a keypad of flat grey
 * pills with `=` as the ink pill, and a collapsible hairline history. `=` sends the expression to POST
 * /api/calculator/eval — the same Rust evaluator the AI tool uses — so the
 * window and the agent always agree. Results land in the shared
 * `calculator_history` log.
 *
 * AI wiring: `calculator_*` tool outcomes arrive via `agent:actions`; the
 * window shows the last `calculator_eval` result and refreshes its history.
 */

import { apiFetch } from '../../js/api.js';
import { t } from '../../js/i18n.js';

export const CALCULATOR_PLUGIN = 'calculator';

let tileEl = null;
let exprEl = null;
let resultEl = null;
let keysEl = null;
let sciEl = null;
let historyEl = null;
let historyToggleBtn = null;
let sciToggleBtn = null;

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

/**
 * Ambient glow mirrors the calculator's state (Tier 1), from theme tokens
 * only: the error colour on error, the accent pair otherwise — a result moves
 * the blobs (their position follows the result's magnitude).
 */
function updateGlow({ error = false } = {}) {
  if (error) {
    setTileGlow(tileEl, glowGradient('var(--error)', 'var(--accent-2)'));
    return;
  }
  const n = Number(lastResult);
  if (lastResult != null && Number.isFinite(n)) {
    const m = Math.abs(n) % 100;
    setTileGlow(tileEl, glowGradient('var(--accent)', 'var(--accent-2)', {
      x: Math.round(m), y: 0, x2: Math.round(100 - m), y2: 100,
    }));
    return;
  }
  setTileGlow(tileEl, glowGradient('var(--accent)', 'var(--accent-2)'));
}

/** Repaint the display. `roll` animates a freshly set result (rollNumber). */
function renderDisplay({ roll = false } = {}) {
  if (!exprEl || !resultEl) return;
  exprEl.textContent = expression || '\u200b';
  const text = lastResult == null ? '' : String(lastResult);
  // An empty display shows a muted 0, like a pocket calculator at rest.
  resultEl.textContent = text || '0';
  resultEl.classList.toggle('is-empty', !text);
  resultEl.classList.remove('is-error');
  resultEl.classList.toggle('is-long', text.length > 12);
  if (roll && text) rollNumber(resultEl);
  updateGlow();
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
    renderDisplay({ roll: true });
    // Keep the evaluated expression visible above the result until the next
    // key press (the expression state itself is already cleared).
    exprEl.textContent = src;
    await refreshHistory();
  } catch (e) {
    resultEl.textContent = t('plugin.calculator.display.error', null, 'Error');
    resultEl.classList.remove('is-empty');
    resultEl.classList.add('is-error');
    // Sticky until the next successful renderDisplay() repaints the glow.
    updateGlow({ error: true });
    toast(e.message || t('plugin.calculator.toast.evaluateFailed', null, 'Could not evaluate'), { type: 'error' });
  }
}

/* ── Keypad ─────────────────────────────────────────────────── */

const OPERATOR_LABELS = {
  '+': 'add', '-': 'sub', '×': 'mul', '÷': 'div',
  '^': 'pow', '%': 'mod', '√': 'sqrt',
};

/** One keypad key. `iconName` swaps the label for a catalog icon (the label
 *  then becomes the accessible name). */
function keyBtn(label, action, cls = '', iconName = '') {
  const b = controlButton();
  b.type = 'button';
  b.className = `calculator-key ${cls}`.trim();
  if (iconName) {
    b.appendChild(icon(iconName, { size: 18 }));
    b.setAttribute('aria-label', label);
    b.title = label;
  } else {
    b.textContent = label;
    if (label.length > 1 && !OPERATOR_LABELS[label]) b.classList.add('is-fn');
  }
  if (OPERATOR_LABELS[label]) b.classList.add('is-op');
  b.addEventListener('click', action);
  return b;
}

function buildKeypad() {
  keysEl.textContent = '';

  const rows = [
    [
      keyBtn('C', clearAll, 'is-clear'),
      keyBtn(t('plugin.calculator.key.backspace', null, 'Backspace'), backspace, '', 'ui/arrow-left'),
      keyBtn('(', () => append('(')),
      keyBtn(')', () => append(')')),
      keyBtn('÷', () => { continueFromResult('/'); append('/'); }),
    ],
    [
      keyBtn('7', () => { continueFromResult('7'); append('7'); }),
      keyBtn('8', () => { continueFromResult('8'); append('8'); }),
      keyBtn('9', () => { continueFromResult('9'); append('9'); }),
      keyBtn('×', () => { continueFromResult('*'); append('*'); }),
      keyBtn('%', () => { continueFromResult('%'); append('%'); }),
    ],
    [
      keyBtn('4', () => { continueFromResult('4'); append('4'); }),
      keyBtn('5', () => { continueFromResult('5'); append('5'); }),
      keyBtn('6', () => { continueFromResult('6'); append('6'); }),
      keyBtn('−', () => { continueFromResult('-'); append('-'); }),
      keyBtn('^', () => { continueFromResult('^'); append('^'); }),
    ],
    [
      keyBtn('1', () => { continueFromResult('1'); append('1'); }),
      keyBtn('2', () => { continueFromResult('2'); append('2'); }),
      keyBtn('3', () => { continueFromResult('3'); append('3'); }),
      keyBtn('+', () => { continueFromResult('+'); append('+'); }),
      keyBtn('π', () => append('pi')),
    ],
    [
      keyBtn('±', () => { if (expression === '' && lastResult != null) { expression = String(-Number(lastResult)); lastResult = null; } else append('-'); renderDisplay(); }),
      keyBtn('0', () => { continueFromResult('0'); append('0'); }),
      keyBtn('.', () => { continueFromResult('.'); append('.'); }),
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
  sciToggleBtn?.classList.toggle('is-active', sciMode);
  sciToggleBtn?.setAttribute('aria-pressed', String(sciMode));
}

/* ── History panel ──────────────────────────────────────────── */

/** Reload the history list. `reveal` plays listReveal on the rows — only
 *  when the panel opens, not on every refresh after `=`. */
async function refreshHistory({ reveal = false } = {}) {
  if (!historyEl) return;
  try {
    const { history } = await apiHistory();
    historyEl.textContent = '';
    const head = document.createElement('div');
    head.className = 'calculator-history-head';
    head.append(
      eyebrow(t('plugin.calculator.toolbar.history', null, 'History')),
      eyebrow(String(history?.length || 0)),
    );
    historyEl.appendChild(head);
    if (!history || !history.length) {
      historyEl.appendChild(emptyState({
        title: t('plugin.calculator.empty.noHistory', null, 'No history'),
        body: t('plugin.calculator.empty.noHistoryBody', null, 'Calculations you run here or ask the AI for appear below.'),
      }));
      return;
    }
    const list = hairlineList('calculator-history-list');
    for (const item of history) {
      const row = controlButton();
      row.type = 'button';
      row.className = 'calculator-history-item';
      row.setAttribute('role', 'listitem');
      row.title = t('plugin.calculator.history.reuse', null, 'Reuse this result');
      const expr = document.createElement('span');
      expr.className = 'calculator-history-expr';
      expr.textContent = item.expression;
      const res = document.createElement('span');
      res.className = 'calculator-history-res';
      res.textContent = String(item.result);
      row.append(expr, res);
      row.addEventListener('click', () => {
        lastResult = item.result;
        expression = '';
        renderDisplay({ roll: true });
        closeHistory();
      });
      list.appendChild(row);
    }
    historyEl.appendChild(list);
    if (reveal) void listReveal(list.children);
  } catch (e) {
    toast(e.message || t('plugin.calculator.toast.historyFailed', null, 'Could not load history'), { type: 'error' });
  }
}

async function clearHistory() {
  try {
    await apiClearHistory();
    await refreshHistory();
    toast(t('plugin.calculator.toast.historyCleared', null, 'History cleared'), { type: 'info' });
  } catch (e) {
    toast(e.message || t('plugin.calculator.toast.clearFailed', null, 'Could not clear history'), { type: 'error' });
  }
}

function toggleHistory() {
  historyOpen = !historyOpen;
  historyEl?.classList.toggle('hidden', !historyOpen);
  historyToggleBtn?.classList.toggle('is-active', historyOpen);
  historyToggleBtn?.setAttribute('aria-pressed', String(historyOpen));
  if (historyOpen) void refreshHistory({ reveal: true });
}

function closeHistory() {
  historyOpen = false;
  historyEl?.classList.add('hidden');
  historyToggleBtn?.classList.remove('is-active');
  historyToggleBtn?.setAttribute('aria-pressed', 'false');
}

/* ── Tile lifecycle ─────────────────────────────────────────── */

/** Create the Calculator tile element (the plugin's window container). */
export function mountCalculatorTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile calculator-tile';
  tileEl.dataset.plugin = CALCULATOR_PLUGIN;

  /* Top bar: sci/history/clear controls (the window title bar already names
     the plugin, so no title label here). */
  const bar = toolbar([], { className: 'calculator-bar' });
  bar.dataset.windowBar = '';
  const spacer = document.createElement('div');
  spacer.className = 'calculator-bar-spacer';

  const barBtn = (iconName, label, onClick) => toolButton(iconName, label, onClick, { className: 'calculator-bar-btn' });
  historyToggleBtn = barBtn('ui/list', t('plugin.calculator.toolbar.history', null, 'History'), toggleHistory);
  sciToggleBtn = barBtn('ui/scientific', t('plugin.calculator.toolbar.scientific', null, 'Scientific'), toggleSci);
  const clearBtn = barBtn('ui/trash', t('plugin.calculator.toolbar.clearHistory', null, 'Clear history'), () => void clearHistory());
  historyToggleBtn.setAttribute('aria-pressed', 'false');
  sciToggleBtn.setAttribute('aria-pressed', 'false');
  bar.append(spacer, historyToggleBtn, sciToggleBtn, clearBtn);
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
  sciToggleBtn = null;
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
    renderDisplay({ roll: true });
    if (evaled.data?.expression) exprEl.textContent = evaled.data.expression;
  }
  if (historyOpen) void refreshHistory();
}

let wired = false;
export function wireCalculatorEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
}

/** Entries core splices into this window's right-click menu (PLUGINS.md §19).
 *  Core supplies the surrounding separators + window management. */
export function calculatorContextMenu() {
  const hasExpr = !!expression.trim();
  return [
    { type: 'item', label: t('plugin.calculator.menu.equals', null, 'Equals'), icon: 'ui/check', disabled: !hasExpr, onClick: () => void equals() },
    { type: 'item', label: t('plugin.calculator.menu.clearAll', null, 'Clear all'), icon: 'ui/close', onClick: clearAll },
    { type: 'separator' },
    { type: 'item', label: t('plugin.calculator.toolbar.scientific', null, 'Scientific'), icon: 'ui/scientific', checked: sciMode, onClick: toggleSci },
    { type: 'item', label: t('plugin.calculator.toolbar.history', null, 'History'), icon: 'ui/list', checked: historyOpen, onClick: toggleHistory },
    { type: 'item', label: t('plugin.calculator.toolbar.clearHistory', null, 'Clear history'), icon: 'ui/trash', danger: true, onClick: () => void clearHistory() },
  ];
}

export default {
  name: 'calculator',
  icon: 'ui/calculator',
  mount: mountCalculatorTile,
  unmount: unmountCalculatorTile,
  getElement: getCalculatorTileElement,
  wireEvents: wireCalculatorEvents,
  contextMenu: calculatorContextMenu,
};

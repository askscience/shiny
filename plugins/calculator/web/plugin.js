import {
  toolbar, toolButton, controlButton, emptyState, toast, setTileGlow, glowGradient,
  icon, eyebrow, hairlineList, listReveal, rollNumber, paneStack,
} from '../../ui/index.js';

/**
 * calculator.js — the Calculator plugin's window (basic + scientific).
 *
 * An editorial calculator: a display (a small mono expression line over a
 * large display-font result that rolls up when set), a keypad of flat grey
 * pills with `=` as the ink pill, and a collapsible hairline history. The
 * body is a pane stack (ui paneStack): the pad (display, scientific row,
 * keypad) is the floor and the history is a pane that slides in over it in
 * a narrow window or on a phone; a wide window keeps the history inline,
 * under the keypad or beside it. `=` sends the expression to POST
 * /api/calculator/eval — the same Rust evaluator the AI tool uses — so the
 * window and the agent always agree. Results land in the shared
 * `calculator_history` log.
 *
 * AI wiring: `calculator_*` tool outcomes arrive via `agent:actions`; the
 * window shows the last `calculator_eval` result and refreshes its history.
 */

import { apiFetch } from '../../js/api.js';
import { t } from '../../js/i18n.js';
import { prettyExpr, closeParens, toggleSign, negateText, errorKind } from './expr.js';

export const CALCULATOR_PLUGIN = 'calculator';

let tileEl = null;
let exprEl = null;
let resultEl = null;
let keysEl = null;
let sciEl = null;
let historyEl = null;
let historyBodyEl = null;
let historyCountEl = null;
let historyToggleBtn = null;
let sciToggleBtn = null;
let angleBtn = null;
let nav = null; // paneStack() handle: pad (floor) → history

let expression = '';
let lastResult = null; // number|null — "Ans" for chained calculations
let sciMode = false;
let historyOpen = false;
// Trigonometry in degrees (DEG) or radians (RAD); remembered on this device.
const ANGLE_KEY = 'calculator.angle';
let angleMode = (() => {
  try { return localStorage.getItem(ANGLE_KEY) === 'rad' ? 'rad' : 'deg'; } catch { return 'deg'; }
})();
// The live result of what is being typed, shown muted until `=`.
let previewText = '';
let previewSeq = 0;
let previewTimer = null;

/* ── API ────────────────────────────────────────────────────── */

async function apiEval(expression, { preview = false } = {}) {
  const res = await apiFetch('/api/calculator/eval', {
    method: 'POST',
    body: JSON.stringify({ expression, angle: angleMode, preview }),
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
  exprEl.textContent = prettyExpr(expression) || '\u200b';
  const text = lastResult == null ? '' : String(lastResult);
  // While typing, the result line previews the answer, muted; an empty
  // display shows a muted 0, like a pocket calculator at rest.
  const preview = !text && expression ? previewText : '';
  const shown = text || preview || '0';
  resultEl.textContent = shown;
  resultEl.classList.toggle('is-empty', !text && !preview);
  resultEl.classList.toggle('is-preview', !!preview);
  resultEl.classList.remove('is-error');
  resultEl.classList.toggle('is-long', shown.length > 12);
  if (roll && text) rollNumber(resultEl);
  updateGlow();
}

/** Evaluate what is being typed, shortly after the last key, for the muted
 *  preview. Nothing is written to history; an incomplete expression keeps
 *  the last preview that worked. */
function schedulePreview() {
  clearTimeout(previewTimer);
  const seq = ++previewSeq;
  const src = expression.trim();
  if (!src) {
    previewText = '';
    return;
  }
  previewTimer = setTimeout(async () => {
    let text = null;
    try {
      const r = await apiEval(closeParens(src), { preview: true });
      text = r?.result_text ?? null;
    } catch { /* incomplete — keep the last preview */ }
    if (seq !== previewSeq || text == null) return;
    previewText = text;
    if (lastResult == null) renderDisplay();
  }, 90);
}

function setExpression(next) {
  expression = next;
  schedulePreview();
  renderDisplay();
}

function append(text) {
  setExpression(expression + text);
}

/** Type a key: continue from or replace the last answer, then append. */
function input(text) {
  continueFromResult(text);
  append(text);
}

// A function name goes with its bracket, `pi` as one symbol.
const TRAILING_SYMBOL = /([a-z]+[0-9]*\(|pi)$/;

function backspace() {
  const m = expression.match(TRAILING_SYMBOL);
  setExpression(m ? expression.slice(0, m.index) : expression.slice(0, -1));
}

function clearAll() {
  lastResult = null;
  previewText = '';
  setExpression('');
}

/** ± — flip the last answer, or the number being typed. */
function plusMinus() {
  if (expression === '' && lastResult != null) {
    const flipped = negateText(lastResult);
    lastResult = null;
    setExpression(flipped);
    return;
  }
  setExpression(toggleSign(expression));
}

/**
 * After "=" the display shows a result. If the user then presses an operator,
 * continue from the answer (like a pocket calculator); if they press a digit,
 * start a fresh expression.
 */
function continueFromResult(text) {
  if (lastResult != null && expression === '') {
    // The answer becomes the start of the new expression, previewed below.
    if (/^[+\-*/%^!]/.test(text)) expression = String(lastResult);
    lastResult = null;
  }
}

async function equals() {
  // A bracket left open is closed, as the user meant it.
  const src = closeParens(expression.trim());
  if (!src) return;
  try {
    const r = await apiEval(src);
    clearTimeout(previewTimer);
    previewSeq += 1;
    previewText = '';
    expression = '';
    lastResult = r?.result_text ?? null;
    renderDisplay({ roll: true });
    // Keep the evaluated expression visible above the result until the next
    // key press (the expression state itself is already cleared).
    exprEl.textContent = prettyExpr(src);
    await refreshHistory();
  } catch (e) {
    resultEl.textContent = t('plugin.calculator.display.error', null, 'Error');
    resultEl.classList.remove('is-empty');
    resultEl.classList.add('is-error');
    // Sticky until the next successful renderDisplay() repaints the glow.
    updateGlow({ error: true });
    toast(errorMessage(e?.message), { type: 'error' });
  }
}

const ERROR_TEXT = {
  divideByZero: "You can't divide by zero",
  incomplete: 'The expression is not finished',
  domain: 'Not defined for this value',
  tooLarge: 'The result is too large',
  tooLong: 'The expression is too long',
  invalid: 'Check the expression',
};

/** An evaluator error, said in the interface language. */
function errorMessage(message) {
  const kind = errorKind(message);
  return t(`plugin.calculator.error.${kind}`, null, ERROR_TEXT[kind]);
}

/** DEG ⇄ RAD for the trigonometric functions. */
function toggleAngle() {
  angleMode = angleMode === 'deg' ? 'rad' : 'deg';
  try { localStorage.setItem(ANGLE_KEY, angleMode); } catch { /* private mode */ }
  paintAngle();
  schedulePreview();
}

function paintAngle() {
  if (!angleBtn) return;
  const deg = angleMode === 'deg';
  angleBtn.textContent = deg ? 'DEG' : 'RAD';
  const label = deg
    ? t('plugin.calculator.toolbar.angleDeg', null, 'Angles in degrees — switch to radians')
    : t('plugin.calculator.toolbar.angleRad', null, 'Angles in radians — switch to degrees');
  angleBtn.title = label;
  angleBtn.setAttribute('aria-label', label);
}

/* ── Keyboard ───────────────────────────────────────────────── */

const KEY_INPUT = {
  '+': '+', '-': '-', '*': '*', x: '*', X: '*', '/': '/', ':': '/',
  '^': '^', '%': '%', '(': '(', ')': ')', '!': '!', '.': '.', ',': '.',
};

/** Typing into the Calculator while it is the window in front. */
function onKeydown(e) {
  if (!tileEl?.isConnected || !tileEl.classList.contains('tile--focused')) return;
  if (e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return;
  const target = e.target;
  if (target instanceof HTMLElement
    && (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName))) return;
  // Keys meant for something else open in front (a menu, a dialog).
  if (target instanceof HTMLElement && target !== document.body && !tileEl.contains(target)) return;

  const k = e.key;
  let handled = true;
  if (/^[0-9]$/.test(k)) input(k);
  else if (k in KEY_INPUT) {
    const text = KEY_INPUT[k];
    if (text === ')') append(text);
    else input(text);
  } else if (k === 'Enter' || k === '=') void equals();
  else if (k === 'Backspace') backspace();
  else if (k === 'Delete' || (k === 'Escape' && !historyOpen)) clearAll();
  else handled = false;
  if (handled) e.preventDefault();
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
      keyBtn('(', () => input('(')),
      keyBtn(')', () => append(')')),
      keyBtn('÷', () => input('/')),
    ],
    [
      keyBtn('7', () => input('7')),
      keyBtn('8', () => input('8')),
      keyBtn('9', () => input('9')),
      keyBtn('×', () => input('*')),
      keyBtn('%', () => input('%')),
    ],
    [
      keyBtn('4', () => input('4')),
      keyBtn('5', () => input('5')),
      keyBtn('6', () => input('6')),
      keyBtn('−', () => input('-')),
      keyBtn('^', () => input('^')),
    ],
    [
      keyBtn('1', () => input('1')),
      keyBtn('2', () => input('2')),
      keyBtn('3', () => input('3')),
      keyBtn('+', () => input('+')),
      keyBtn('π', () => input('pi')),
    ],
    [
      keyBtn('±', plusMinus),
      keyBtn('0', () => input('0')),
      keyBtn('.', () => input('.')),
      keyBtn('=', equals, 'is-equals'),
      keyBtn('e', () => input('e')),
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
    ['sin', () => input('sin(')],
    ['cos', () => input('cos(')],
    ['tan', () => input('tan(')],
    ['√', () => input('sqrt(')],
    ['asin', () => input('asin(')],
    ['acos', () => input('acos(')],
    ['atan', () => input('atan(')],
    ['x²', () => input('^2')],
    ['ln', () => input('ln(')],
    ['log', () => input('log(')],
    ['log₂', () => input('log2(')],
    ['exp', () => input('exp(')],
    ['10ˣ', () => input('10^')],
    ['∛', () => input('cbrt(')],
    ['!', () => input('!')],
    ['abs', () => input('abs(')],
    ['floor', () => input('floor(')],
    ['ceil', () => input('ceil(')],
    ['round', () => input('round(')],
    ['fact', () => input('fact(')],
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
  if (!historyBodyEl) return;
  try {
    const { history } = await apiHistory();
    historyBodyEl.textContent = '';
    if (historyCountEl) historyCountEl.textContent = String(history?.length || 0);
    if (!history || !history.length) {
      historyBodyEl.appendChild(emptyState({
        title: t('plugin.calculator.empty.noHistory', null, 'No history'),
        body: t('plugin.calculator.empty.noHistoryBody', null, 'Calculations you run here or ask the AI for appear here.'),
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
      expr.textContent = prettyExpr(item.expression);
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
    historyBodyEl.appendChild(list);
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

/** Paint the history's open state: inline (`hidden` off/on) while the window
 *  is wide, a pane the stack slides in while narrow — the stack owns its
 *  visibility then, so `hidden` must be off. */
function paintHistory() {
  const stacked = !!nav?.isStacked();
  historyEl?.classList.toggle('hidden', !stacked && !historyOpen);
  historyToggleBtn?.classList.toggle('is-active', historyOpen);
  historyToggleBtn?.setAttribute('aria-pressed', String(historyOpen));
}

function openHistory() {
  if (historyOpen) return;
  historyOpen = true;
  paintHistory();
  nav?.show('history');
  void refreshHistory({ reveal: true });
}

function closeHistory() {
  if (!historyOpen) return;
  historyOpen = false;
  paintHistory();
  // While stacked the pane is on the stack: pop it (a no-op otherwise).
  if (nav?.isOpen('history')) nav.back();
}

function toggleHistory() {
  if (historyOpen) closeHistory();
  else openHistory();
}

/** The stack changed its top or its layout (Back, Escape, a resize). */
function onPaneChange({ stacked }) {
  if (!nav) return; // the stack's first measure, before the handle is kept
  if (stacked) historyOpen = nav.isOpen('history');
  paintHistory();
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

  const barBtn = (iconName, label, onClick, className = '') => toolButton(iconName, label, onClick, { className: `calculator-bar-btn ${className}`.trim() });
  historyToggleBtn = barBtn('ui/history', t('plugin.calculator.toolbar.history', null, 'History'), toggleHistory, 'calculator-history-toggle');
  sciToggleBtn = barBtn('ui/scientific', t('plugin.calculator.toolbar.scientific', null, 'Scientific'), toggleSci);
  historyToggleBtn.setAttribute('aria-pressed', 'false');
  sciToggleBtn.setAttribute('aria-pressed', 'false');
  // DEG / RAD: a word, not an icon, so the unit in use is always in view.
  angleBtn = controlButton();
  angleBtn.type = 'button';
  angleBtn.className = 'calculator-bar-btn calculator-angle';
  angleBtn.addEventListener('click', toggleAngle);
  paintAngle();
  bar.append(spacer, angleBtn, historyToggleBtn, sciToggleBtn);
  tileEl.appendChild(bar);

  /* Body: the pane stack's root — the pad is the floor, the history a pane. */
  const body = document.createElement('div');
  body.className = 'calculator-body';
  tileEl.appendChild(body);

  const pad = document.createElement('div');
  pad.className = 'calculator-pad';
  body.appendChild(pad);

  /* Display */
  const display = document.createElement('div');
  display.className = 'calculator-display';
  exprEl = document.createElement('div');
  exprEl.className = 'calculator-display-expr';
  resultEl = document.createElement('div');
  resultEl.className = 'calculator-display-result';
  display.append(exprEl, resultEl);
  pad.appendChild(display);

  /* Scientific pad (collapsible) */
  sciEl = document.createElement('div');
  sciEl.className = 'calculator-sci hidden';
  pad.appendChild(sciEl);
  buildSciPad();

  /* Keypad */
  keysEl = document.createElement('div');
  keysEl.className = 'calculator-keys';
  pad.appendChild(keysEl);
  buildKeypad();

  /* History: inline under (or beside) the keypad while wide, a pane over
     the pad while stacked. Its head is the pane's bar (the stack prepends
     Back to it): title, count, and the clear action. */
  historyEl = document.createElement('div');
  historyEl.className = 'calculator-history hidden';
  const historyHead = document.createElement('div');
  historyHead.className = 'calculator-history-head';
  historyHead.dataset.paneBar = '';
  historyCountEl = eyebrow('0');
  const historySpacer = document.createElement('div');
  historySpacer.className = 'calculator-bar-spacer';
  historyHead.append(
    eyebrow(t('plugin.calculator.toolbar.history', null, 'History')),
    historyCountEl,
    historySpacer,
    toolButton('ui/trash', t('plugin.calculator.toolbar.clearHistory', null, 'Clear history'), () => void clearHistory(), { className: 'calculator-bar-btn calculator-history-clear' }),
  );
  historyBodyEl = document.createElement('div');
  historyBodyEl.className = 'calculator-history-body';
  historyEl.append(historyHead, historyBodyEl);
  body.appendChild(historyEl);

  nav = paneStack(body, {
    panes: [
      { id: 'pad', el: pad },
      { id: 'history', el: historyEl },
    ],
    onChange: onPaneChange,
  });
  paintHistory();

  renderDisplay();
  document.addEventListener('keydown', onKeydown);
  return tileEl;
}

/** Deactivated: drop the window. */
export function unmountCalculatorTile() {
  document.removeEventListener('keydown', onKeydown);
  clearTimeout(previewTimer);
  nav?.destroy();
  nav = null;
  tileEl?.remove();
  tileEl = null;
  exprEl = null;
  resultEl = null;
  keysEl = null;
  sciEl = null;
  historyEl = null;
  historyBodyEl = null;
  historyCountEl = null;
  historyToggleBtn = null;
  sciToggleBtn = null;
  angleBtn = null;
  historyOpen = false;
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
    if (evaled.data?.expression) exprEl.textContent = prettyExpr(evaled.data.expression);
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
    { type: 'item', label: t('plugin.calculator.toolbar.history', null, 'History'), icon: 'ui/history', checked: historyOpen, onClick: toggleHistory },
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

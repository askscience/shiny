/**
 * keyboard.js — the Keyboard plugin's frontend.
 *
 * A virtual multi-language keyboard bar at the bottom of the screen. It acts
 * like a normal keyboard: whatever editable element (chat input, Word editor,
 * settings fields…) gains focus becomes its input target. On touch devices
 * the native OS keyboard is suppressed while the plugin is active.
 *
 * The plugin itself contributes nothing to the agent (no skills/tools) —
 * this module only exists when `isPluginActive('keyboard')`.
 */
import { isPluginActive, refreshActivePlugins } from './activePlugins.js';
import { getTraveler } from './api.js';
import { setIcon } from '../ui/index.js';

export const KEYBOARD_PLUGIN = 'keyboard';

const TOUCH = window.matchMedia('(pointer: coarse)').matches;

/* ── Layouts ────────────────────────────────────────────────── */

const LAYOUTS = [
  {
    code: 'en', label: 'EN', dir: 'ltr',
    rows: [
      ['q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p'],
      ['a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l'],
      ['z', 'x', 'c', 'v', 'b', 'n', 'm'],
    ],
  },
  {
    code: 'it', label: 'IT', dir: 'ltr',
    rows: [
      ['q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p'],
      ['a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l', 'è'],
      ['z', 'x', 'c', 'v', 'b', 'n', 'm', 'ù'],
    ],
  },
  {
    code: 'es', label: 'ES', dir: 'ltr',
    rows: [
      ['q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p'],
      ['a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l', 'ñ'],
      ['z', 'x', 'c', 'v', 'b', 'n', 'm'],
    ],
  },
  {
    code: 'fr', label: 'FR', dir: 'ltr',
    rows: [
      ['a', 'z', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p'],
      ['q', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l', 'm'],
      ['w', 'x', 'c', 'v', 'b', 'n', 'è'],
    ],
  },
  {
    code: 'de', label: 'DE', dir: 'ltr',
    rows: [
      ['q', 'w', 'e', 'r', 't', 'z', 'u', 'i', 'o', 'p', 'ü'],
      ['a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l', 'ö', 'ä'],
      ['y', 'x', 'c', 'v', 'b', 'n', 'm'],
    ],
  },
  {
    code: 'ru', label: 'RU', dir: 'ltr',
    rows: [
      ['й', 'ц', 'у', 'к', 'е', 'н', 'г', 'ш', 'щ', 'з', 'х'],
      ['ф', 'ы', 'в', 'а', 'п', 'р', 'о', 'л', 'д', 'ж', 'э'],
      ['я', 'ч', 'с', 'м', 'и', 'т', 'ь', 'б', 'ю'],
    ],
    shiftRows: [
      ['Й', 'Ц', 'У', 'К', 'Е', 'Н', 'Г', 'Ш', 'Щ', 'З', 'Х'],
      ['Ф', 'Ы', 'В', 'А', 'П', 'Р', 'О', 'Л', 'Д', 'Ж', 'Э'],
      ['Я', 'Ч', 'С', 'М', 'И', 'Т', 'Ь', 'Б', 'Ю'],
    ],
  },
  {
    code: 'el', label: 'ΕΛ', dir: 'ltr',
    rows: [
      ['ς', 'ε', 'ρ', 'τ', 'υ', 'θ', 'ι', 'ο', 'π'],
      ['α', 'σ', 'δ', 'φ', 'γ', 'η', 'ξ', 'κ', 'λ'],
      ['ζ', 'χ', 'ψ', 'ω', 'β', 'ν', 'μ'],
    ],
    shiftRows: [
      ['Σ', 'Ε', 'Ρ', 'Τ', 'Υ', 'Θ', 'Ι', 'Ο', 'Π'],
      ['Α', 'Σ', 'Δ', 'Φ', 'Γ', 'Η', 'Ξ', 'Κ', 'Λ'],
      ['Ζ', 'Χ', 'Ψ', 'Ω', 'Β', 'Ν', 'Μ'],
    ],
  },
  {
    code: 'ar', label: 'ع', dir: 'rtl', noShift: true,
    rows: [
      ['ض', 'ص', 'ث', 'ق', 'ف', 'غ', 'ع', 'ه', 'خ', 'ح', 'ج', 'د'],
      ['ش', 'س', 'ي', 'ب', 'ل', 'ا', 'ت', 'ن', 'م', 'ك', 'ط'],
      ['ئ', 'ء', 'ؤ', 'ر', 'لا', 'ى', 'ة', 'و', 'ز', 'ظ'],
    ],
  },
];

/* Symbols page (123) — independent of the active language. */
const SYMBOL_ROWS = [
  ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0'],
  ['-', '/', ':', ';', '(', ')', '$', '&', '@', '"'],
  ['#', '=', '+', '.', ',', '?', '!', "'"],
];

/* ── State ──────────────────────────────────────────────────── */

let bar = null;          // #keyboard-bar
let rowsEl = null;
let hudBtn = null;       // toggle in the top HUD bar
let active = false;      // plugin activated
let visible = false;     // keyboard open on screen
let pinned = false;      // manual toggle keeps it open
let target = null;       // focused editable element
let activeRange = null;  // caret range (contenteditable)
let layoutIdx = 0;
let shift = false;
let symbols = false;
let suppressFocusOut = false;
const markedInputs = new Set(); // inputs we made readonly/inputmode-none

function langKey() {
  const id = getTraveler()?.id || 'default';
  return `keyboard.lang.${id}`;
}

function layout() {
  return LAYOUTS[layoutIdx];
}

function isEditable(el) {
  if (!el || el.nodeType !== 1) return false;
  if (el instanceof HTMLTextAreaElement) return true;
  if (el instanceof HTMLInputElement) {
    const t = el.type;
    return ['text', 'password', 'email', 'search', 'number', 'tel', 'url'].includes(t);
  }
  return el.isContentEditable === true;
}

function dispatchInput() {
  if (!target) return;
  try {
    target.dispatchEvent(new InputEvent('input', {
      bubbles: true,
      inputType: 'insertText',
    }));
  } catch (_) {
    target.dispatchEvent(new Event('input', { bubbles: true }));
  }
}

/* ── Edits ──────────────────────────────────────────────────── */

function insertChar(ch) {
  if (!target) return;
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLInputElement) {
    const start = target.selectionStart ?? target.value.length;
    const end = target.selectionEnd ?? start;
    target.value = target.value.slice(0, start) + ch + target.value.slice(end);
    const caret = start + ch.length;
    try { target.setSelectionRange(caret, caret); } catch (_) {}
    dispatchInput();
    return;
  }
  if (target.isContentEditable) {
    if (TOUCH) insertCharTouch(ch);
    else document.execCommand('insertText', false, ch);
  }
}

function backspace() {
  if (!target) return;
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLInputElement) {
    const start = target.selectionStart ?? target.value.length;
    const end = target.selectionEnd ?? start;
    const from = start === end ? Math.max(0, start - 1) : start;
    if (start === 0 && end === 0) return;
    target.value = target.value.slice(0, from) + target.value.slice(end);
    try { target.setSelectionRange(from, from); } catch (_) {}
    dispatchInput();
    return;
  }
  if (target.isContentEditable) {
    if (TOUCH) backspaceTouch();
    else document.execCommand('delete', false);
  }
}

function enter() {
  if (!target) return;
  const ev = new KeyboardEvent('keydown', {
    key: 'Enter',
    keyCode: 13,
    bubbles: true,
    cancelable: true,
    shiftKey: shift,
  });
  target.dispatchEvent(ev);
  if (target instanceof HTMLTextAreaElement && shift) {
    insertChar('\n');
  } else if (target.isContentEditable) {
    if (TOUCH) insertParagraphTouch();
    else document.execCommand(shift ? 'insertLineBreak' : 'insertParagraph', false);
  }
  if (shift) setShift(false);
}

/* Touch contenteditable: keep the editor blurred (no OS keyboard) and edit
 * the saved Range directly. */

function insertCharTouch(ch) {
  const range = activeRange;
  if (!range) return;
  const node = document.createTextNode(ch);
  range.deleteContents();
  range.insertNode(node);
  range.setStartAfter(node);
  range.collapse(true);
  activeRange = range.cloneRange();
  dispatchInput();
}

function backspaceTouch() {
  const range = activeRange;
  if (!range) return;
  if (!range.collapsed) {
    range.deleteContents();
  } else {
    const c = range.startContainer;
    const o = range.startOffset;
    if (c.nodeType === 3 && o > 0) {
      c.deleteData(o - 1, 1);
      range.setStart(c, o - 1);
      range.collapse(true);
    } else if (c.nodeType === 1 && o > 0) {
      const prev = c.childNodes[o - 1];
      if (prev) {
        if (prev.nodeType === 3) {
          prev.deleteData(prev.data.length - 1, 1);
          if (!prev.data.length) prev.remove();
        } else {
          prev.remove();
        }
        range.setStart(c, o - 1);
        range.collapse(true);
      }
    }
  }
  activeRange = range.cloneRange();
  dispatchInput();
}

function isBlockNode(n) {
  return /^(P|DIV|H1|H2|H3|LI|UL|OL|BLOCKQUOTE)$/.test(n.tagName || '');
}

function insertParagraphTouch() {
  const range = activeRange;
  if (!range) return;
  range.deleteContents();
  const p = document.createElement('p');
  p.appendChild(document.createElement('br'));
  let node = range.startContainer;
  if (node.nodeType === 3) node = node.parentElement;
  let block = null;
  while (node && node !== target && node.nodeType === 1) {
    if (isBlockNode(node)) { block = node; break; }
    node = node.parentElement;
  }
  if (block) {
    block.after(p);
  } else {
    range.insertNode(p);
  }
  range.setStart(p, 0);
  range.collapse(true);
  activeRange = range.cloneRange();
  dispatchInput();
}

/* ── Keys ───────────────────────────────────────────────────── */

function setShift(on) {
  shift = on;
  render();
}

function pressKey(k) {
  switch (k) {
    case 'shift':
      setShift(!shift);
      return;
    case 'lang':
      layoutIdx = (layoutIdx + 1) % LAYOUTS.length;
      localStorage.setItem(langKey(), LAYOUTS[layoutIdx].code);
      render();
      return;
    case 'sym':
      symbols = !symbols;
      render();
      return;
    case 'backspace':
      backspace();
      return;
    case 'enter':
      enter();
      return;
    case 'space':
      insertChar(' ');
      if (shift) setShift(false);
      return;
    case 'close':
      pinned = false;
      close();
      return;
    default:
      if (!k) return;
      insertChar(k);
      if (shift) setShift(false);
  }
}

/* ── Render ─────────────────────────────────────────────────── */

function keyButton(k, label, cls = '') {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = 'keyboard-key';
  if (cls) b.className += ` ${cls}`;
  b.dataset.k = k;
  b.textContent = label;
  b.addEventListener('pointerdown', (e) => {
    e.preventDefault();
    pressKey(k);
  });
  return b;
}

function render() {
  if (!rowsEl) return;
  rowsEl.textContent = '';
  const l = layout();

  if (symbols) {
    for (const row of SYMBOL_ROWS) {
      const r = document.createElement('div');
      r.className = 'keyboard-row';
      for (const ch of row) r.appendChild(keyButton(ch, shift ? ch.toUpperCase() : ch));
      rowsEl.appendChild(r);
    }
  } else {
    const rows = shift && l.shiftRows ? l.shiftRows : l.rows;
    for (const row of rows) {
      const r = document.createElement('div');
      r.className = 'keyboard-row';
      if (l.dir === 'rtl') r.setAttribute('dir', 'rtl');
      for (const ch of row) {
        const label = shift ? ch.toUpperCase() : ch;
        r.appendChild(keyButton(ch, label));
      }
      rowsEl.appendChild(r);
    }
  }

  /* Bottom row: language, symbols toggle, space, dot, enter. */
  const bot = document.createElement('div');
  bot.className = 'keyboard-row';
  bot.appendChild(keyButton('lang', l.label, 'keyboard-key--fn'));
  bot.appendChild(keyButton('sym', symbols ? 'ABC' : '123', 'keyboard-key--fn'));

  if (!symbols && !l.noShift) {
    const sh = keyButton('shift', '⇧', 'keyboard-key--fn');
    if (shift) sh.classList.add('is-active');
    bot.appendChild(sh);
  }

  const back = keyButton('backspace', '⌫', 'keyboard-key--fn');
  bot.appendChild(back);

  bot.appendChild(keyButton('space', ' ', 'keyboard-key--space'));
  if (!symbols) bot.appendChild(keyButton('.', '.'));
  bot.appendChild(keyButton('enter', '↵', 'keyboard-key--accent keyboard-key--fn'));
  bot.appendChild(keyButton('close', '⌄', 'keyboard-key--fn'));
  rowsEl.appendChild(bot);
}

/* ── Open / close ───────────────────────────────────────────── */

function open() {
  if (!bar) return;
  visible = true;
  bar.classList.add('visible');
  document.body.classList.add('keyboard-open');
  hudBtn?.setAttribute('aria-pressed', 'true');
}

function close() {
  if (!bar) return;
  visible = false;
  pinned = false;
  bar.classList.remove('visible');
  document.body.classList.remove('keyboard-open');
  hudBtn?.setAttribute('aria-pressed', 'false');
}

/* ── Target binding ─────────────────────────────────────────── */

function captureCaret() {
  if (!target) return;
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLInputElement) {
    return; // live selectionStart/End are authoritative
  }
  if (target.isContentEditable) {
    const sel = window.getSelection();
    if (sel && sel.rangeCount > 0) activeRange = sel.getRangeAt(0).cloneRange();
  }
}

function bindTarget(el) {
  if (target === el) return;
  target = el;
  captureCaret();
  // Touch contenteditable: blur to keep the OS keyboard away; the saved
  // Range drives the edits.
  if (TOUCH && el.isContentEditable) {
    suppressFocusOut = true;
    el.blur();
    suppressFocusOut = false;
  }
  open();
}

function unbind() {
  target = null;
  activeRange = null;
}

/* ── Wiring ─────────────────────────────────────────────────── */

let wired = false;

function wireEvents() {
  if (wired) return;
  wired = true;

  document.addEventListener('focusin', (e) => {
    if (!active) return;
    const t = e.target;
    if (!t || t.nodeType !== 1 || t.closest('#keyboard-bar')) return;
    if (isEditable(t)) bindTarget(t);
  });

  document.addEventListener('focusout', (e) => {
    if (!target || e.target !== target || suppressFocusOut) return;
    if (pinned) return;
    const next = e.relatedTarget;
    if (next && isEditable(next) && !next.closest('#keyboard-bar')) return;
    unbind();
    close();
  });

  // Keep the saved contenteditable range fresh while the caret moves.
  document.addEventListener('selectionchange', () => {
    if (!target?.isContentEditable) return;
    if (document.activeElement !== target) return;
    const sel = window.getSelection();
    if (sel && sel.rangeCount > 0) activeRange = sel.getRangeAt(0).cloneRange();
  });

  document.addEventListener('pointerdown', (e) => {
    if (!visible || pinned) return;
    if (e.target?.closest?.('#keyboard-bar, #hud-keyboard-btn')) return;
    if (isEditable(e.target)) return;
    unbind();
    close();
  });

  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && visible) {
      unbind();
      close();
    }
  });
}

/* ── HUD toggle + plugin lifecycle ──────────────────────────── */

function createHudButton() {
  if (hudBtn) return;
  hudBtn = document.createElement('button');
  hudBtn.type = 'button';
  hudBtn.id = 'hud-keyboard-btn';
  hudBtn.className = 'icon-btn';
  hudBtn.title = 'Keyboard';
  hudBtn.setAttribute('aria-label', 'Keyboard');
  hudBtn.setAttribute('aria-pressed', 'false');
  void setIcon(hudBtn, 'ui/keyboard', { size: 18 });
  hudBtn.addEventListener('click', () => {
    if (visible) close();
    else { pinned = true; open(); }
  });
  const hudTop = document.getElementById('hud-top');
  const settingsBtn = document.getElementById('settings-btn');
  if (hudTop) hudTop.insertBefore(hudBtn, settingsBtn || hudTop.firstChild);
}

function markInput(el) {
  if (!(el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement)) return;
  if (el.closest('#keyboard-bar')) return;
  el.setAttribute('inputmode', 'none');
  if (TOUCH) {
    el.readOnly = true;
    markedInputs.add(el);
  }
}

function unmarkAll() {
  for (const el of markedInputs) {
    el.removeAttribute('inputmode');
    el.readOnly = false;
  }
  markedInputs.clear();
}

let inputObserver = null;

function mount() {
  if (bar) return;
  bar = document.createElement('div');
  bar.id = 'keyboard-bar';
  bar.setAttribute('role', 'group');
  bar.setAttribute('aria-label', 'Virtual keyboard');
  rowsEl = document.createElement('div');
  rowsEl.className = 'keyboard-rows';
  bar.appendChild(rowsEl);
  document.body.appendChild(bar);
  createHudButton();

  document.querySelectorAll('input, textarea').forEach(markInput);
  inputObserver = new MutationObserver((muts) => {
    for (const m of muts) {
      m.addedNodes.forEach((n) => {
        if (n.nodeType !== 1) return;
        if (n.matches?.('input, textarea')) markInput(n);
        n.querySelectorAll?.('input, textarea').forEach(markInput);
      });
    }
  });
  inputObserver.observe(document.body, { childList: true, subtree: true });

  render();
}

function unmount() {
  pinned = false;
  unbind();
  close();
  inputObserver?.disconnect();
  inputObserver = null;
  unmarkAll();
  bar?.remove();
  bar = null;
  rowsEl = null;
  hudBtn?.remove();
  hudBtn = null;
}

/* ── Public API ─────────────────────────────────────────────── */

let initialized = false;

export function initKeyboard() {
  if (initialized) return;
  initialized = true;

  const saved = localStorage.getItem(langKey());
  const idx = LAYOUTS.findIndex((l) => l.code === saved);
  layoutIdx = idx >= 0 ? idx : 0;

  wireEvents();
  window.addEventListener('plugins:changed', () => { void refreshKeyboard(); });
}

export async function refreshKeyboard() {
  const activeSet = await refreshActivePlugins();
  const nowActive = activeSet.has(KEYBOARD_PLUGIN);
  if (nowActive === active) return;
  active = nowActive;
  if (active) mount();
  else unmount();
}

export function isKeyboardActive() {
  return active;
}

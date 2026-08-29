/**
 * calc.js — the Calc plugin's window (simple spreadsheet).
 *
 * Flat editorial surface: a hairline toolbar (sheet menu + title), a formula
 * bar (selected cell ref + raw value), and an A1-style cell grid. Only
 * non-empty cells are stored server-side as a JSON map ("A1" -> "value");
 * the window autosaves the whole map (PUT /api/spreadsheets/:id).
 *
 * Formulas: a value starting with "=" is evaluated live in the window —
 * cell refs (A1), ranges (A1:B3), SUM/AVERAGE/MIN/MAX/COUNT, + - * / ^ and
 * parentheses. The AI stores formulas as text; the window does the math.
 *
 * AI wiring: `calc_*` tool outcomes arrive via the `agent:actions` event —
 * the window refreshes its sheet list and opens what the AI touched.
 */

import {
  icon, button, emptyState, toast,
} from '../ui/index.js';
import { setIcon } from '../ui/index.js';
import { apiFetch } from './api.js';

export const CALC_PLUGIN = 'calc';

const DEFAULT_ROWS = 100;
const DEFAULT_COLS = 26;

let tileEl = null;
let sheetMenuBtn = null;
let titleInput = null;
let statusEl = null;
let saveDot = null;
let formulaRefEl = null;
let formulaInputEl = null;
let gridEl = null;
let editorInputEl = null;   // overlay input for in-cell editing

let sheets = [];
let current = null;         // { id, title, rows, cols, cells: Map, updated_at }
let dirty = false;
let saveTimer = null;
let saveSeq = 0;
let sel = { row: 1, col: 0 };  // 1-based row, 0-based col
let editing = false;

/* Popup (body-level, the tile clips overflow) */
let sheetMenuPopup = null;
let sheetMenuOpen = false;

/* ── Cell refs ───────────────────────────────────────────────── */

function colName(col) {
  let n = col + 1;
  let s = '';
  while (n > 0) {
    const rem = (n - 1) % 26;
    s = String.fromCharCode(65 + rem) + s;
    n = Math.floor((n - 1) / 26);
  }
  return s;
}

function cellRef(row, col) {
  return `${colName(col)}${row}`;
}

function parseRef(ref) {
  const m = /^([A-Z]{1,2})([1-9][0-9]*)$/.exec(String(ref || '').toUpperCase());
  if (!m) return null;
  let col = 0;
  for (const ch of m[1]) col = col * 26 + (ch.charCodeAt(0) - 64);
  return { row: Number(m[2]), col: col - 1 };
}

/* ── API ────────────────────────────────────────────────────── */

async function listSheets() {
  const res = await apiFetch('/api/spreadsheets');
  return res?.data || [];
}

async function fetchSheet(id) {
  const res = await apiFetch(`/api/spreadsheets/${encodeURIComponent(id)}`);
  return res?.data;
}

async function createSheet(title = 'Untitled') {
  const res = await apiFetch('/api/spreadsheets', {
    method: 'POST',
    body: JSON.stringify({ title }),
  });
  return res?.data;
}

async function saveSheet(id, title, cells) {
  const seq = ++saveSeq;
  await apiFetch(`/api/spreadsheets/${encodeURIComponent(id)}`, {
    method: 'PUT',
    body: JSON.stringify({ title, cells }),
  });
  return seq;
}

async function deleteSheet(id) {
  await apiFetch(`/api/spreadsheets/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

/* ── Formula evaluation ─────────────────────────────────────── */

const FN = {
  SUM: (vs) => vs.reduce((a, b) => a + b, 0),
  AVERAGE: (vs) => (vs.length ? vs.reduce((a, b) => a + b, 0) / vs.length : 0),
  MIN: (vs) => (vs.length ? Math.min(...vs) : 0),
  MAX: (vs) => (vs.length ? Math.max(...vs) : 0),
  COUNT: (vs) => vs.filter((v) => Number.isFinite(v)).length,
};

function tokenizeFormula(src) {
  const tokens = [];
  let i = 0;
  while (i < src.length) {
    const ch = src[i];
    if (/\s/.test(ch)) { i++; continue; }
    if (/[0-9.]/.test(ch)) {
      let j = i;
      while (j < src.length && /[0-9.]/.test(src[j])) j++;
      tokens.push({ type: 'num', value: parseFloat(src.slice(i, j)) });
      i = j;
      continue;
    }
    if (/[A-Z]/.test(ch)) {
      let j = i;
      while (j < src.length && /[A-Z0-9]/.test(src[j])) j++;
      tokens.push({ type: 'word', value: src.slice(i, j) });
      i = j;
      continue;
    }
    if ('+-*/^(),:'.includes(ch)) {
      tokens.push({ type: ch, value: ch });
      i++;
      continue;
    }
    tokens.push({ type: 'word', value: ch });
    i++;
  }
  return tokens;
}

/** Resolve a cell to a number for formulas (recursively, with cycle guard). */
function cellNumber(cells, ref, seen) {
  if (seen.has(ref)) return 0;
  const raw = cells.get(ref);
  if (raw == null || raw === '') return 0;
  if (String(raw).startsWith('=')) {
    seen.add(ref);
    const v = evalFormula(String(raw), cells, seen);
    seen.delete(ref);
    return Number.isFinite(v) ? v : 0;
  }
  const n = Number(String(raw).replace(/,/g, ''));
  return Number.isFinite(n) ? n : 0;
}

function rangeRefs(a, b, cells, seen) {
  const out = [];
  const r1 = Math.min(a.row, b.row);
  const r2 = Math.max(a.row, b.row);
  const c1 = Math.min(a.col, b.col);
  const c2 = Math.max(a.col, b.col);
  for (let r = r1; r <= r2; r++) {
    for (let c = c1; c <= c2; c++) {
      out.push(cellNumber(cells, cellRef(r, c), seen));
    }
  }
  return out;
}

/** Evaluate a formula string ("=SUM(A1:A3)+B1*2") to a number. */
export function evalFormula(src, cells, seen = new Set()) {
  const text = String(src).replace(/^=/, '').trim();
  if (!text) return 0;
  const tokens = tokenizeFormula(text);
  let pos = 0;

  function peek() { return tokens[pos]; }
  function next() { return tokens[pos++]; }

  function parseExpr() {
    let v = parseTerm();
    while (peek() && peek().type === '+') { next(); v += parseTerm(); }
    while (peek() && peek().type === '-') { next(); v -= parseTerm(); }
    return v;
  }

  function parseTerm() {
    let v = parseFactor();
    while (peek() && peek().type === '*') { next(); v *= parseFactor(); }
    while (peek() && peek().type === '/') {
      next();
      const d = parseFactor();
      v = d === 0 ? NaN : v / d;
    }
    return v;
  }

  function parseFactor() {
    let v = parseUnary();
    while (peek() && peek().type === '^') { next(); v = Math.pow(v, parseUnary()); }
    return v;
  }

  function parseUnary() {
    if (peek() && peek().type === '-') { next(); return -parseUnary(); }
    if (peek() && peek().type === '+') { next(); return parseUnary(); }
    return parseAtom();
  }

  /** Function-call arguments: ranges (A1:B3) expand to their cell values. */
  function parseArgs() {
    const out = [];
    while (peek() && peek().type !== ')') {
      if (peek().type === ',') { next(); continue; }
      // Lookahead: WORD : WORD → a range argument; expand to its values.
      if (peek().type === 'word') {
        const first = parseRef(peek().value);
        const colon = tokens[pos + 1];
        if (first && colon && colon.type === ':') {
          next(); // first word
          next(); // ':'
          const toTok = next();
          const second = toTok ? parseRef(toTok.value) : null;
          if (second) {
            out.push(...rangeRefs(first, second, cells, seen));
            continue;
          }
        }
      }
      out.push(parseExpr());
    }
    return out;
  }

  function parseAtom() {
    const t = next();
    if (!t) return 0;
    if (t.type === 'num') return t.value;
    if (t.type === '(') {
      const v = parseExpr();
      if (peek() && peek().type === ')') next();
      return v;
    }
    if (t.type === 'word') {
      // Function call: WORD(...) — SUM, AVERAGE, MIN, MAX, COUNT.
      if (peek() && peek().type === '(') {
        next();
        const fn = FN[t.value];
        const args = parseArgs();
        if (peek() && peek().type === ')') next();
        return fn ? fn(args) : NaN;
      }
      // Bare cell ref (A1) or range (A1:B3).
      const ref = parseRef(t.value);
      if (!ref) return NaN;
      const colon = peek();
      if (colon && colon.type === ':' && colon.value === ':') {
        next();
        const to = next();
        if (!to) return NaN;
        const ref2 = parseRef(to.value);
        if (!ref2) return NaN;
        return rangeRefs(ref, ref2, cells, seen)
          .reduce((a, b) => a + b, 0);
      }
      return cellNumber(cells, t.value, seen);
    }
    return NaN;
  }

  try {
    const v = parseExpr();
    return Number.isFinite(v) ? v : NaN;
  } catch (_) {
    return NaN;
  }
}

/** The display value for a cell: computed result for formulas. */
function displayValue(raw) {
  if (raw == null || raw === '') return '';
  const s = String(raw);
  if (s.startsWith('=')) {
    const v = evalFormula(s, current.cells);
    return Number.isFinite(v) ? String(v) : '#ERROR';
  }
  return s;
}

/* ── Status / persistence ───────────────────────────────────── */

function setStatus(mode) {
  if (!statusEl) return;
  const now = new Date();
  const t = now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  if (mode === 'saving') {
    statusEl.textContent = 'Saving…';
    saveDot?.classList.add('is-active');
  } else if (mode === 'dirty') {
    statusEl.textContent = 'Unsaved changes';
    saveDot?.classList.add('is-active');
  } else {
    const n = current?.cells.size || 0;
    statusEl.textContent = `Saved ${t} · ${n} ${n === 1 ? 'cell' : 'cells'}`;
    saveDot?.classList.remove('is-active');
  }
}

function markDirty() {
  dirty = true;
  setStatus('dirty');
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => void persist(), 1200);
}

async function persist() {
  if (!current || !dirty) return;
  dirty = false;
  setStatus('saving');
  try {
    const cells = Object.fromEntries(current.cells);
    await saveSheet(current.id, titleInput.value.trim(), cells);
    current.title = titleInput.value.trim();
    current.updated_at = new Date().toISOString();
    setStatus('saved');
  } catch (e) {
    dirty = true;
    setStatus('dirty');
    toast(e.message || 'Save failed', { type: 'error' });
  }
}

function setCellValue(ref, value) {
  const v = String(value ?? '').trim();
  if (v === '') current.cells.delete(ref);
  else current.cells.set(ref, v);
  markDirty();
  renderGrid();
}

/* ── Open / create / delete ─────────────────────────────────── */

async function openSheet(sheet) {
  window.clearTimeout(saveTimer);
  dirty = false;
  saveSeq++;
  try {
    const full = sheet.cells != null ? sheet : await fetchSheet(sheet.id);
    current = {
      id: full.id,
      title: full.title,
      rows: full.rows || DEFAULT_ROWS,
      cols: full.cols || DEFAULT_COLS,
      cells: new Map(Object.entries(full.cells || {})),
      updated_at: full.updated_at,
    };
    titleInput.value = full.title;
    sel = { row: 1, col: 0 };
    renderGrid();
    setStatus('saved');
  } catch (e) {
    toast(e.message || 'Could not open spreadsheet', { type: 'error' });
  }
}

async function openNewest() {
  const list = await listSheets();
  sheets = list;
  if (list.length) await openSheet(list[0]);
}

async function newSheet() {
  try {
    const created = await createSheet();
    await refreshSheets();
    await openSheet(created);
  } catch (e) {
    toast(e.message || 'Could not create spreadsheet', { type: 'error' });
  }
}

async function removeCurrent() {
  if (!current) return;
  if (!window.confirm(`Delete "${current.title}"?`)) return;
  try {
    await deleteSheet(current.id);
    current = null;
    await refreshSheets();
    await openNewest();
  } catch (e) {
    toast(e.message || 'Could not delete spreadsheet', { type: 'error' });
  }
}

async function refreshSheets() {
  try {
    sheets = await listSheets();
  } catch (_) { /* keep last list */ }
}

/* ── Sheet menu ─────────────────────────────────────────────── */

function ensureSheetMenu() {
  if (sheetMenuPopup) return;
  sheetMenuPopup = document.createElement('div');
  sheetMenuPopup.className = 'calc-sheet-menu hidden';
  sheetMenuPopup.setAttribute('role', 'menu');
  document.body.appendChild(sheetMenuPopup);
}

function closeSheetMenu() {
  if (!sheetMenuOpen) return;
  sheetMenuOpen = false;
  sheetMenuPopup?.classList.add('hidden');
  if (sheetMenuBtn) sheetMenuBtn.setAttribute('aria-expanded', 'false');
  document.removeEventListener('pointerdown', onSheetMenuOutside, true);
  document.removeEventListener('keydown', onSheetMenuKey, true);
}

function onSheetMenuOutside(e) {
  if (sheetMenuPopup && !sheetMenuPopup.contains(e.target)
    && sheetMenuBtn && !sheetMenuBtn.contains(e.target)) {
    closeSheetMenu();
  }
}

function onSheetMenuKey(e) {
  if (e.key === 'Escape') closeSheetMenu();
}

function renderSheetMenuItems() {
  if (!sheetMenuPopup) return;
  sheetMenuPopup.innerHTML = '';
  if (!sheets.length) {
    const empty = document.createElement('div');
    empty.className = 'calc-sheet-menu-empty';
    empty.textContent = 'No spreadsheets yet';
    sheetMenuPopup.appendChild(empty);
  } else {
    sheets.forEach((sheet) => {
      const item = document.createElement('button');
      item.type = 'button';
      item.className = 'calc-sheet-menu-item';
      if (sheet.id === current?.id) item.classList.add('is-active');
      item.setAttribute('role', 'menuitem');

      const title = document.createElement('span');
      title.className = 'calc-sheet-menu-title';
      title.textContent = sheet.title;
      const time = document.createElement('span');
      time.className = 'calc-sheet-menu-time';
      time.textContent = formatWhen(sheet.updated_at);
      const check = document.createElement('span');
      check.className = 'calc-sheet-menu-check';
      item.append(title, time, check);
      void setIcon(check, 'ui/check', { size: 13 });

      item.addEventListener('click', () => {
        closeSheetMenu();
        if (sheet.id !== current?.id) void openSheet(sheet);
      });
      sheetMenuPopup.appendChild(item);
    });
  }

  const foot = document.createElement('div');
  foot.className = 'calc-sheet-menu-foot';
  const footItem = (iconName, label, danger, onClick) => {
    const item = document.createElement('button');
    item.type = 'button';
    item.className = 'calc-sheet-menu-item';
    if (danger) item.classList.add('calc-sheet-menu-item--danger');
    item.setAttribute('role', 'menuitem');
    const ic = document.createElement('span');
    ic.className = 'calc-sheet-menu-foot-icon';
    item.appendChild(ic);
    void setIcon(ic, iconName, { size: 14 });
    const labelEl = document.createElement('span');
    labelEl.className = 'calc-sheet-menu-title';
    labelEl.textContent = label;
    item.appendChild(labelEl);
    item.addEventListener('click', () => {
      closeSheetMenu();
      onClick();
    });
    foot.appendChild(item);
  };
  footItem('ui/plus', 'New spreadsheet', false, () => void newSheet());
  footItem('ui/upload', 'Import .ods', false, pickOdsFile);
  footItem('ui/upload', 'Import CSV', false, pickCsvFile);
  footItem('ui/save', 'Export .ods', false, () => void exportOds());
  footItem('ui/doc', 'Export CSV', false, () => void exportCsv());
  footItem('ui/trash', 'Delete spreadsheet', true, () => void removeCurrent());
  sheetMenuPopup.appendChild(foot);
}

function formatWhen(iso) {
  if (!iso) return '';
  try {
    return new Date(iso).toLocaleDateString([], { month: 'short', day: 'numeric' });
  } catch (_) {
    return '';
  }
}

function openSheetMenu() {
  ensureSheetMenu();
  renderSheetMenuItems();
  sheetMenuPopup.classList.remove('hidden');
  const r = sheetMenuBtn.getBoundingClientRect();
  const left = Math.max(12, Math.min(r.left, window.innerWidth - 280 - 12));
  sheetMenuPopup.style.left = `${left}px`;
  sheetMenuPopup.style.top = `${r.bottom + 8}px`;
  sheetMenuOpen = true;
  sheetMenuBtn.setAttribute('aria-expanded', 'true');
  document.addEventListener('pointerdown', onSheetMenuOutside, true);
  document.addEventListener('keydown', onSheetMenuKey, true);
}

function toggleSheetMenu() {
  if (sheetMenuOpen) closeSheetMenu();
  else openSheetMenu();
}

/* ── Import / export ────────────────────────────────────────── */

function pickOdsFile() {
  const input = document.createElement('input');
  input.type = 'file';
  input.accept = '.ods,application/vnd.oasis.opendocument.spreadsheet';
  input.addEventListener('change', () => {
    const file = input.files?.[0];
    if (file) void importOds(file);
  });
  input.click();
}

async function importOds(file) {
  const form = new FormData();
  form.append('file', file);
  try {
    await apiFetch('/api/spreadsheets/import', { method: 'POST', body: form });
    toast(`Imported ${file.name}`, { type: 'info' });
    await refreshSheets();
    await openNewest();
  } catch (e) {
    toast(e.message || 'Import failed — is this a valid .ods file?', { type: 'error' });
  }
}

async function exportOds() {
  if (!current) return;
  try {
    const blob = await apiFetch(
      `/api/spreadsheets/${encodeURIComponent(current.id)}/export`,
      { responseType: 'blob' },
    );
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${current.title || 'spreadsheet'}.ods`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  } catch (e) {
    toast(e.message || 'Export failed', { type: 'error' });
  }
}

function csvEscape(v) {
  const s = String(v ?? '');
  return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

async function exportCsv() {
  if (!current) return;
  const rows = [];
  const byRow = new Map();
  for (const [ref, value] of current.cells) {
    const p = parseRef(ref);
    if (!p) continue;
    if (!byRow.has(p.row)) byRow.set(p.row, new Map());
    byRow.get(p.row).set(p.col, value);
  }
  const maxRow = Math.max(1, ...byRow.keys());
  const maxCol = Math.max(0, ...[...byRow.values()].flatMap((m) => [...m.keys()]));
  for (let r = 1; r <= maxRow; r++) {
    const line = [];
    for (let c = 0; c <= maxCol; c++) {
      line.push(csvEscape(byRow.get(r)?.get(c) ?? ''));
    }
    rows.push(line.join(','));
  }
  const blob = new Blob([rows.join('\n')], { type: 'text/csv' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = `${current.title || 'spreadsheet'}.csv`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

function pickCsvFile() {
  const input = document.createElement('input');
  input.type = 'file';
  input.accept = '.csv,text/csv';
  input.addEventListener('change', () => {
    const file = input.files?.[0];
    if (file) void importCsv(file);
  });
  input.click();
}

function parseCsv(text) {
  const rows = [];
  let row = [];
  let field = '';
  let inQuotes = false;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (inQuotes) {
      if (ch === '"') {
        if (text[i + 1] === '"') { field += '"'; i++; }
        else inQuotes = false;
      } else field += ch;
    } else if (ch === '"') {
      inQuotes = true;
    } else if (ch === ',') {
      row.push(field); field = '';
    } else if (ch === '\n' || ch === '\r') {
      if (ch === '\r' && text[i + 1] === '\n') i++;
      row.push(field); field = '';
      rows.push(row); row = [];
    } else {
      field += ch;
    }
  }
  row.push(field);
  rows.push(row);
  return rows;
}

async function importCsv(file) {
  try {
    const text = await file.text();
    const rows = parseCsv(text);
    const cells = {};
    rows.forEach((line, r) => {
      line.forEach((value, c) => {
        const v = String(value ?? '').trim();
        if (!v) return;
        cells[cellRef(r + 1, c)] = v;
      });
    });
    const created = await createSheet(file.name.replace(/\.csv$/i, '') || 'Imported CSV');
    await saveSheet(created.id, created.title, cells);
    await refreshSheets();
    await openSheet({ ...created, cells });
    toast(`Imported ${file.name}`, { type: 'info' });
  } catch (e) {
    toast(e.message || 'Import failed — is this a valid CSV file?', { type: 'error' });
  }
}

/* ── Grid ───────────────────────────────────────────────────── */

function renderGrid() {
  if (!gridEl || !current) return;

  const selRef = cellRef(sel.row, sel.col);

  // Keep editing state: the editor input is repositioned after re-render.
  const wasEditing = editing;
  const editValue = wasEditing && editorInputEl ? editorInputEl.value : null;

  gridEl.innerHTML = '';

  const corner = document.createElement('div');
  corner.className = 'calc-cell calc-cell--corner';
  corner.textContent = '';
  gridEl.appendChild(corner);

  const totalCols = Math.min(current.cols, 52);
  const totalRows = Math.min(current.rows, 200);
  for (let c = 0; c < totalCols; c++) {
    const head = document.createElement('div');
    head.className = 'calc-cell calc-cell--head';
    head.textContent = colName(c);
    gridEl.appendChild(head);
  }

  for (let r = 1; r <= totalRows; r++) {
    const rowHead = document.createElement('div');
    rowHead.className = 'calc-cell calc-cell--rowhead';
    rowHead.textContent = String(r);
    gridEl.appendChild(rowHead);

    for (let c = 0; c < totalCols; c++) {
      const ref = cellRef(r, c);
      const cell = document.createElement('div');
      cell.className = 'calc-cell calc-cell--data';
      cell.dataset.ref = ref;
      cell.dataset.row = String(r);
      cell.dataset.col = String(c);
      cell.textContent = displayValue(current.cells.get(ref));
      cell.title = ref;
      if (r === sel.row && c === sel.col) cell.classList.add('is-selected');
      if (String(current.cells.get(ref) ?? '').startsWith('=')) {
        cell.classList.add('is-formula');
      }
      cell.addEventListener('click', () => {
        sel = { row: r, col: c };
        editing = false;
        hideEditor();
        renderGrid();
      });
      cell.addEventListener('dblclick', () => {
        sel = { row: r, col: c };
        renderGrid();
        startEditing();
      });
      gridEl.appendChild(cell);
    }
  }

  // Re-attach the overlay editor if we were editing.
  if (wasEditing) {
    editing = true;
    showEditor(editValue);
  }
}

/** Overlay input positioned over the selected cell. */
function ensureEditor() {
  if (editorInputEl) return;
  editorInputEl = document.createElement('input');
  editorInputEl.className = 'calc-cell-editor';
  editorInputEl.autocomplete = 'off';
  editorInputEl.addEventListener('keydown', (e) => {
    e.stopPropagation();
    if (e.key === 'Enter') { e.preventDefault(); commitEdit(true); }
    else if (e.key === 'Tab') { e.preventDefault(); commitEdit(true); }
    else if (e.key === 'Escape') { e.preventDefault(); cancelEdit(); }
  });
  editorInputEl.addEventListener('blur', () => {
    if (editing) commitEdit(false);
  });
  gridEl.appendChild(editorInputEl);
}

function cellElFor(row, col) {
  return gridEl?.querySelector(`.calc-cell--data[data-row="${row}"][data-col="${col}"]`);
}

function showEditor(prefill = null) {
  ensureEditor();
  const cell = cellElFor(sel.row, sel.col);
  if (!cell) return;
  editing = true;
  editorInputEl.value = prefill != null
    ? prefill
    : (current.cells.get(cellRef(sel.row, sel.col)) ?? '');
  editorInputEl.style.display = '';
  const r = cell.getBoundingClientRect();
  const g = gridEl.getBoundingClientRect();
  editorInputEl.style.left = `${r.left - g.left}px`;
  editorInputEl.style.top = `${r.top - g.top}px`;
  editorInputEl.style.width = `${r.width}px`;
  editorInputEl.style.height = `${r.height}px`;
  editorInputEl.focus();
  editorInputEl.select();
}

function hideEditor() {
  editing = false;
  if (editorInputEl) editorInputEl.style.display = 'none';
}

function commitEdit(move) {
  if (!editing || !current) return;
  const value = editorInputEl?.value ?? '';
  setCellValue(cellRef(sel.row, sel.col), value);
  hideEditor();
  if (move) {
    sel = { row: sel.row + 1, col: sel.col };
    if (sel.row > current.rows) sel.row = current.rows;
    renderGrid();
    gridEl.querySelector(`.calc-cell--data[data-row="${sel.row}"][data-col="${sel.col}"]`)
      ?.scrollIntoView({ block: 'nearest' });
  } else {
    renderGrid();
  }
}

function cancelEdit() {
  hideEditor();
  renderGrid();
}

/* ── Formula bar ────────────────────────────────────────────── */

function syncFormulaBar() {
  if (!formulaRefEl || !formulaInputEl || !current) return;
  const ref = cellRef(sel.row, sel.col);
  formulaRefEl.textContent = ref;
  if (document.activeElement !== formulaInputEl) {
    formulaInputEl.value = current.cells.get(ref) ?? '';
  }
}

/* ── Tile lifecycle ─────────────────────────────────────────── */

/** Create the Calc tile element (the plugin's window container). */
export function mountCalcTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile calc-tile';
  tileEl.dataset.plugin = CALC_PLUGIN;

  /* Top bar: sheet menu + title + save indicator */
  const bar = document.createElement('div');
  bar.className = 'calc-bar';

  sheetMenuBtn = document.createElement('button');
  sheetMenuBtn.type = 'button';
  sheetMenuBtn.className = 'calc-sheet-btn';
  sheetMenuBtn.setAttribute('aria-haspopup', 'menu');
  sheetMenuBtn.setAttribute('aria-expanded', 'false');
  sheetMenuBtn.title = 'Spreadsheets';
  const sheetIcon = document.createElement('span');
  sheetIcon.className = 'calc-sheet-btn-icon';
  sheetMenuBtn.appendChild(sheetIcon);
  void setIcon(sheetIcon, 'ui/calc', { size: 15 });
  const chevron = document.createElement('span');
  chevron.className = 'calc-sheet-btn-chevron';
  sheetMenuBtn.appendChild(chevron);
  void setIcon(chevron, 'ui/chevron-down', { size: 12 });
  sheetMenuBtn.addEventListener('click', toggleSheetMenu);

  titleInput = document.createElement('input');
  titleInput.className = 'calc-title';
  titleInput.type = 'text';
  titleInput.placeholder = 'Untitled';
  titleInput.maxLength = 120;
  titleInput.autocomplete = 'off';
  titleInput.addEventListener('change', () => {
    if (current) void persist();
  });

  saveDot = document.createElement('span');
  saveDot.className = 'calc-save-dot';
  saveDot.setAttribute('aria-hidden', 'true');

  bar.append(sheetMenuBtn, titleInput, saveDot);
  tileEl.appendChild(bar);

  /* Toolbar — square flat buttons, always visible (like the Word window). */
  const tools = document.createElement('div');
  tools.className = 'calc-tools';
  const toolBtn = (iconName, label, onClick, danger) => {
    const btn = button({ icon: iconName, variant: 'ghost', onClick });
    btn.classList.add('ui-btn--icon', 'calc-tool');
    if (danger) btn.classList.add('calc-tool--danger');
    btn.title = label;
    btn.setAttribute('aria-label', label);
    return btn;
  };
  tools.append(
    toolBtn('ui/plus', 'New spreadsheet', () => void newSheet()),
    toolBtn('ui/upload', 'Import .ods', pickOdsFile),
    toolBtn('ui/save', 'Export .ods', () => void exportOds()),
    toolBtn('ui/doc', 'Export CSV', () => void exportCsv()),
    toolBtn('ui/trash', 'Delete spreadsheet', () => void removeCurrent(), true),
  );
  tileEl.appendChild(tools);

  /* Formula bar */
  const formulaBar = document.createElement('div');
  formulaBar.className = 'calc-formula-bar';
  formulaRefEl = document.createElement('span');
  formulaRefEl.className = 'calc-formula-ref';
  formulaRefEl.textContent = 'A1';
  formulaInputEl = document.createElement('input');
  formulaInputEl.className = 'calc-formula-input';
  formulaInputEl.type = 'text';
  formulaInputEl.placeholder = 'Value or formula (=SUM(A1:A5))';
  formulaInputEl.autocomplete = 'off';
  formulaInputEl.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      commitFormula();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      syncFormulaBar();
      formulaInputEl.blur();
    }
  });
  formulaInputEl.addEventListener('blur', commitFormula);
  formulaBar.append(formulaRefEl, formulaInputEl);
  tileEl.appendChild(formulaBar);

  function commitFormula() {
    if (!current || document.activeElement === formulaInputEl && editing) return;
    const ref = cellRef(sel.row, sel.col);
    const value = formulaInputEl.value;
    if ((current.cells.get(ref) ?? '') !== value) {
      setCellValue(ref, value);
    }
    syncFormulaBar();
  }

  /* Grid */
  gridEl = document.createElement('div');
  gridEl.className = 'calc-grid';
  gridEl.addEventListener('keydown', (e) => {
    if (editing) return;
    if (e.key === 'ArrowUp') { e.preventDefault(); moveSel(-1, 0); }
    else if (e.key === 'ArrowDown') { e.preventDefault(); moveSel(1, 0); }
    else if (e.key === 'ArrowLeft') { e.preventDefault(); moveSel(0, -1); }
    else if (e.key === 'ArrowRight') { e.preventDefault(); moveSel(0, 1); }
    else if (e.key === 'Enter') { e.preventDefault(); startEditing(); }
    else if (e.key === 'Tab') { e.preventDefault(); moveSel(0, 1); }
    else if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
      e.preventDefault();
      startEditing(e.key);
    }
  });
  tileEl.appendChild(gridEl);

  /* Status line */
  const status = document.createElement('div');
  status.className = 'calc-status';
  statusEl = document.createElement('span');
  status.appendChild(statusEl);
  tileEl.appendChild(status);

  function moveSel(dr, dc) {
    sel = {
      row: Math.max(1, Math.min(current?.rows || DEFAULT_ROWS, sel.row + dr)),
      col: Math.max(0, Math.min((current?.cols || DEFAULT_COLS) - 1, sel.col + dc)),
    };
    renderGrid();
    syncFormulaBar();
  }

  function startEditing(prefill = null) {
    renderGrid();
    showEditor(prefill);
  }

  void openNewest();
  return tileEl;
}

/** Deactivated mid-edit: flush, drop the window. */
export function unmountCalcTile() {
  if (current && dirty) void persist();
  hideEditor();
  tileEl?.remove();
  tileEl = null;
  gridEl = null;
  editorInputEl = null;
  sheetMenuBtn = null;
  titleInput = null;
  statusEl = null;
  saveDot = null;
  formulaRefEl = null;
  formulaInputEl = null;
}

/** The tile element (or null when the Calc window is not mounted). */
export function getCalcTileElement() {
  return tileEl;
}

/* ── AI wiring ──────────────────────────────────────────────── */

function onAgentActions(e) {
  const actions = e.detail || [];
  const calcActions = actions.filter((a) => /^calc_/.test(a?.action || ''));
  if (!calcActions.length) return;

  // Always surface the Calc window when the AI touches spreadsheets.
  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: CALC_PLUGIN } }));

  const created = calcActions.some((a) => a.action === 'calc_create' && a.result === 'ok');
  const wrote = calcActions.some((a) => a.action === 'calc_write' && a.result === 'ok');
  const deleted = calcActions.some((a) => a.action === 'calc_delete' && a.result === 'ok');
  const read = calcActions.some((a) => a.action === 'calc_read' && a.result === 'ok');

  // The action payload carries the touched sheet_id when compact.
  const touchedId = calcActions
    .map((a) => a.data?.sheet_id)
    .find((id) => !!id);

  if (created) {
    void refreshSheets().then(() => window.setTimeout(() => {
      if (touchedId) {
        const found = sheets.find((s) => s.id === touchedId);
        if (found) void openSheet(found);
      } else {
        void openNewest();
      }
    }, 250));
  } else if (wrote || read || deleted) {
    void refreshSheets().then(() => {
      if (touchedId && current?.id !== touchedId) {
        const found = sheets.find((s) => s.id === touchedId);
        if (found) void openSheet(found);
      } else if (wrote && current) {
        const full = sheets.find((s) => s.id === current.id);
        if (full) void openSheet(full);
      } else if (deleted) {
        void openNewest();
      }
    });
  }
}

let wired = false;
export function wireCalcEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
  window.addEventListener('beforeunload', (e) => {
    if (current && dirty) {
      e.preventDefault();
      e.returnValue = '';
    }
  });
}

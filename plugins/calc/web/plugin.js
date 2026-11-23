import {
  menuSurface, menuItem, confirmDialog, toolButton, menuButton, toolbar, input as uiInput,
  placePopover, toast, setTileGlow, glowFromDrawable, button, emptyState, listReveal,
  setIcon, iconButton,
} from '../../ui/index.js';

/**
 * calc.js — the Calc plugin's window (simple spreadsheet).
 *
 * Editorial surface: a slim bar (sheet menu, the sheet title, a mono save
 * status, one save tool and a "New" pill — import/export/delete live in the
 * sheet menu's footer), a formula bar of hairline-underlined fields (mono
 * cell ref + raw value), and an A1-style grid with mono headers and a
 * neutral ink selection. With no sheets the window shows an empty state. Only
 * non-empty cells are stored server-side as a JSON map ("A1" -> "value");
 * the window autosaves the whole map (PUT /api/spreadsheets/:id).
 *
 * Formulas: a value starting with "=" is computed by the plugin's Rust
 * engine (POST /api/spreadsheets/evaluate) — the same one the AI tools and
 * the .ods export use. The window sends the whole sheet after each change and
 * paints what comes back. Values are stored canonically (`2.50`, `=ROUND(A1,1)`)
 * and typed and shown in the user's language (`2,50`, `=ROUND(A1;1)`), see
 * sheet.js.
 *
 * Editing: a range selection (drag, Shift+click, Shift+arrows), Delete to
 * clear it, Ctrl+Z / Ctrl+Y, Ctrl+C / X / V (formulas move their relative
 * refs; text from other apps pastes as a block), F2 to edit, column widths
 * dragged on the column header (kept with the sheet).
 *
 * AI wiring: `calc_*` tool outcomes arrive via the `agent:actions` event —
 * the window refreshes its sheet list and opens what the AI touched.
 */

import { apiFetch } from '../../js/api.js';
import { t, tp, getLocale } from '../../js/i18n.js';
import { formatMenuDate, appendMenuFooter } from '../../js/officeMenu.js';
import { saveOrDownload, onOpenFromFiles, fileFromHome, pickFiles } from '../../js/files.js';
import {
  separators, toStored, toEditable, cellText, formatNumber, colName, shiftFormula,
  toTsv, parseTsv, parseDelimited, csvDelimiter, ERROR_KEYS,
} from './sheet.js';

export const CALC_PLUGIN = 'calc';

const DEFAULT_ROWS = 100;
const DEFAULT_COLS = 26;
const DEFAULT_COL_WIDTH = 110;
const MIN_COL_WIDTH = 40;
const MAX_COL_WIDTH = 600;
const UNDO_LIMIT = 200;

let tileEl = null;
let sheetMenuBtn = null;
let titleInput = null;
let statusEl = null;
let saveStateEl = null;     // mono "SAVED 14:02" / "UNSAVED" / "SAVING…" in the bar
let bodyEl = null;          // formula bar + grid + status (hidden with no sheets)
let emptyEl = null;
let formulaRefEl = null;
let formulaInputEl = null;
let gridWrapEl = null;      // the scroll container around the grid (pinch surface)
let gridEl = null;
let editorInputEl = null;   // overlay input for in-cell editing

/* Grid zoom: a CSS `zoom` on the grid, from a pinch, Ctrl+wheel or the
   bar's zoom tools. */
const ZOOM_MIN = 0.5;
const ZOOM_MAX = 2;
const ZOOM_STEP = 1.2;
let zoom = 1;
let pinchedAt = 0;          // when a pinch last started (its taps are not edits)

let sheets = [];
let current = null;         // { id, title, rows, cols, cells: Map, updated_at }
let dirty = false;
let saveTimer = null;
let saveSeq = 0;
let sel = { row: 1, col: 0 };  // the active cell: 1-based row, 0-based col
let anchor = { row: 1, col: 0 }; // the other corner of the selected range
let editing = false;
let editMode = 'enter';       // 'enter': typed over the cell, arrows move on;
                              // 'edit': F2 / double click, arrows move the caret
let values = new Map();       // ref → { t, v } from the engine
let evalSeq = 0;
let colWidths = {};           // column index → width in px (default otherwise)
let undoStack = [];           // batches of { ref, before, after }
let redoStack = [];
let clip = null;              // the last copy: { text, rows: [[raw]], origin: {row, col} }
let dragSelecting = false;

/* Popup (body-level, the tile clips overflow) */
let sheetMenuPopup = null;
let sheetMenuOpen = false;

/* ── Cell refs ───────────────────────────────────────────────── */

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

async function createSheet(title = t('plugin.calc.sheet.untitled', null, 'Untitled')) {
  const res = await apiFetch('/api/spreadsheets', {
    method: 'POST',
    body: JSON.stringify({ title }),
  });
  return res?.data;
}

async function saveSheet(id, title, cells, widths = undefined) {
  const seq = ++saveSeq;
  await apiFetch(`/api/spreadsheets/${encodeURIComponent(id)}`, {
    method: 'PUT',
    body: JSON.stringify(widths ? { title, cells, col_widths: widths } : { title, cells }),
  });
  return seq;
}

async function evaluateCells(cells) {
  const res = await apiFetch('/api/spreadsheets/evaluate', {
    method: 'POST',
    body: JSON.stringify({ cells }),
  });
  return res?.data?.values || {};
}

async function deleteSheet(id) {
  await apiFetch(`/api/spreadsheets/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

/* ── Values ─────────────────────────────────────────────────── */

const seps = () => separators(getLocale());
const bools = () => [
  t('plugin.calc.value.true', null, 'TRUE'),
  t('plugin.calc.value.false', null, 'FALSE'),
];

/** What a cell shows, from its stored text and the engine's value. */
function displayValue(ref) {
  const raw = current?.cells.get(ref);
  return cellText(raw, values.get(ref), getLocale(), seps(), bools());
}

/** The explanation of an error code, in the interface language. */
function errorExplanation(code) {
  const key = ERROR_KEYS[code];
  if (!key) return '';
  const fallback = {
    div0: 'Division by zero',
    value: 'A value of the wrong kind — text where a number is needed',
    name: 'Unknown function or name',
    ref: 'The reference points outside the sheet',
    num: 'The number is out of range',
    na: 'No value available',
    circ: 'The formula refers to itself, directly or through other cells',
    parse: 'The formula cannot be read — check brackets and separators',
  }[key];
  return t(`plugin.calc.error.${key}`, null, fallback);
}

/**
 * Ask the engine for every value of the sheet as it is now (saved or not),
 * then repaint. Only the latest answer is painted.
 */
async function recompute() {
  if (!current) return;
  const seq = ++evalSeq;
  const sheet = current;
  try {
    const next = await evaluateCells(Object.fromEntries(sheet.cells));
    if (seq !== evalSeq || sheet !== current) return;
    values = new Map(Object.entries(next));
    paintAllCells();
    paintStatusLine();
    updateGlow();
  } catch (e) {
    if (seq === evalSeq) toast(e.message || t('plugin.calc.toast.computeFailed', null, 'Could not compute the sheet'), { type: 'error' });
  }
}

/* ── Ambient glow ───────────────────────────────────────────── */

/**
 * A sheet has no image, so the glow is a small heatmap of its numeric
 * cells: value magnitude → hue/lightness, cell position → pixel.
 */
function updateGlow() {
  if (!current) { setTileGlow(tileEl, null); return; }

  const W = 64;
  const H = 32;
  const SCALE = 2; // a 2×2 block per cell so the map survives the blur
  const canvas = document.createElement('canvas');
  canvas.width = W;
  canvas.height = H;
  const ctx = canvas.getContext('2d');
  if (!ctx) { setTileGlow(tileEl, null); return; }

  // Theme tokens, resolved to concrete colours for the canvas: the window
  // colour as the base (transparent reads as black), the accent pair for the
  // values (low → --accent-2, high → --accent).
  const css = getComputedStyle(tileEl);
  const token = (name, fallback) => css.getPropertyValue(name).trim() || fallback;
  const base = token('--bg-elevated', token('--bg', 'black'));
  const low = token('--accent-2', token('--accent', 'gray'));
  const high = token('--accent', low);
  ctx.fillStyle = base;
  ctx.fillRect(0, 0, W, H);

  const points = [];
  for (const [ref, val] of values) {
    if (val?.t !== 'n' || !Number.isFinite(val.v)) continue;
    const pos = parseRef(ref);
    if (!pos) continue;
    points.push({ row: pos.row, col: pos.col, v: val.v });
  }
  if (!points.length) { setTileGlow(tileEl, null); return; }

  const magnitudes = points.map((p) => p.v);
  const min = Math.min(...magnitudes);
  const max = Math.max(...magnitudes);
  const span = max - min || 1;
  for (const p of points) {
    const t = (p.v - min) / span;
    ctx.globalAlpha = 1;
    ctx.fillStyle = low;
    ctx.fillRect(p.col % W, p.row % H, SCALE, SCALE);
    ctx.globalAlpha = t;
    ctx.fillStyle = high;
    ctx.fillRect(p.col % W, p.row % H, SCALE, SCALE);
  }
  ctx.globalAlpha = 1;

  setTileGlow(tileEl, glowFromDrawable(canvas));
}

/* ── Status / persistence ───────────────────────────────────── */

/** Save state in the bar as short mono capitals ("SAVED 14:02", "UNSAVED",
 *  "SAVING…"); the mono status line under the grid counts the cells. */
function setStatus(mode) {
  const now = new Date();
  // The interface language decides 14:02 or 2:02 PM, not the browser's.
  let time;
  try { time = now.toLocaleTimeString(getLocale(), { hour: '2-digit', minute: '2-digit' }); } catch { time = now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }); }
  let short;
  if (mode === 'saving') short = t('plugin.calc.status.saving', null, 'Saving…');
  else if (mode === 'dirty') short = t('plugin.calc.status.unsaved', null, 'Unsaved');
  else short = t('plugin.calc.status.savedAt', { time }, 'Saved {time}');
  if (saveStateEl) {
    saveStateEl.textContent = short;
    saveStateEl.dataset.state = mode;
  }
  paintStatusLine();
}

/**
 * The line under the grid: the sum, average and count of a selected range;
 * what an error in the selected cell means; otherwise how many cells are
 * filled.
 */
function paintStatusLine() {
  if (!statusEl) return;
  const r = selRange();
  statusEl.parentElement?.classList.remove('is-message');
  if (current && (r.r1 !== r.r2 || r.c1 !== r.c2)) {
    const nums = [];
    for (let row = r.r1; row <= r.r2; row++) {
      for (let col = r.c1; col <= r.c2; col++) {
        const v = values.get(cellRef(row, col));
        if (v?.t === 'n' && Number.isFinite(v.v)) nums.push(v.v);
      }
    }
    const sum = nums.reduce((a, b) => a + b, 0);
    const loc = getLocale();
    statusEl.textContent = nums.length
      ? t('plugin.calc.status.range', {
        sum: formatNumber(sum, loc),
        average: formatNumber(Math.round((sum / nums.length) * 1e4) / 1e4, loc),
        count: formatNumber(nums.length, loc),
      }, 'Sum {sum} · Average {average} · Count {count}')
      : tp('plugin.calc.status.cells', current?.cells.size || 0);
    return;
  }
  const v = current ? values.get(cellRef(sel.row, sel.col)) : null;
  statusEl.parentElement?.classList.toggle('is-message', v?.t === 'e');
  if (v?.t === 'e') {
    statusEl.textContent = `${v.v} — ${errorExplanation(v.v)}`;
    return;
  }
  statusEl.textContent = tp('plugin.calc.status.cells', current?.cells.size || 0);
}

/** No sheet open (none exist, or the last was deleted): show the empty state
 *  instead of a stale grid. */
function syncEmpty() {
  const empty = !current;
  tileEl?.classList.toggle('is-empty', empty);
  bodyEl?.classList.toggle('hidden', empty);
  emptyEl?.classList.toggle('hidden', !empty);
  if (titleInput) {
    titleInput.disabled = empty;
    if (empty) titleInput.value = '';
  }
  if (saveStateEl) saveStateEl.hidden = empty;
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
    await saveSheet(current.id, titleInput.value.trim(), cells, colWidths);
    current.title = titleInput.value.trim();
    current.updated_at = new Date().toISOString();
    setStatus('saved');
    updateGlow();
  } catch (e) {
    dirty = true;
    setStatus('dirty');
    toast(e.message || t('plugin.calc.toast.saveFailed', null, 'Save failed'), { type: 'error' });
  }
}

/**
 * Change cells: `changes` is `[{ ref, value }]` with values already in
 * stored form. One call is one step of undo. The changed cells repaint at
 * once (in place: the DOM stays, so a click that committed an edit still
 * lands where it was aimed); formulas that depend on them follow when the
 * engine answers.
 */
function setCells(changes, { record = true } = {}) {
  if (!current || !changes.length) return;
  const batch = [];
  for (const { ref, value } of changes) {
    const before = current.cells.get(ref) ?? '';
    const after = String(value ?? '').trim();
    if (before === after) continue;
    if (after === '') current.cells.delete(ref);
    else current.cells.set(ref, after);
    // Until the engine answers, a formula shows its old value, a plain
    // value shows itself.
    if (after.startsWith('=')) {
      if (!before.startsWith('=')) values.delete(ref);
    } else {
      values.delete(ref);
    }
    batch.push({ ref, before, after });
  }
  if (!batch.length) return;
  if (record) {
    undoStack.push(batch);
    if (undoStack.length > UNDO_LIMIT) undoStack.shift();
    redoStack = [];
  }
  markDirty();
  for (const { ref } of batch) {
    const p = parseRef(ref);
    if (p) paintCellAt(p.row, p.col);
  }
  paintSpillRows(batch);
  syncFormulaBar();
  void recompute();
}

function setCellValue(ref, value) {
  setCells([{ ref, value }]);
}

/** Undo (`dir = -1`) or redo (`dir = 1`) one step, selecting what changed. */
function stepHistory(dir) {
  const from = dir < 0 ? undoStack : redoStack;
  const to = dir < 0 ? redoStack : undoStack;
  const batch = from.pop();
  if (!batch || !current) return;
  to.push(batch);
  setCells(batch.map((c) => ({ ref: c.ref, value: dir < 0 ? c.before : c.after })), { record: false });
  const cells = batch.map((c) => parseRef(c.ref)).filter(Boolean);
  if (cells.length) {
    const rows = cells.map((p) => p.row);
    const cols = cells.map((p) => p.col);
    anchor = { row: Math.min(...rows), col: Math.min(...cols) };
    selectCell(Math.max(...rows), Math.max(...cols), { extend: true, scroll: true });
  }
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
    values = new Map(Object.entries(full.values || {}));
    colWidths = { ...(full.col_widths || {}) };
    undoStack = [];
    redoStack = [];
    titleInput.value = full.title;
    sel = { row: 1, col: 0 };
    anchor = { ...sel };
    syncEmpty();
    renderGrid();
    setStatus('saved');
    updateGlow();
    if (!full.values) void recompute();
  } catch (e) {
    toast(e.message || t('plugin.calc.toast.openFailed', null, 'Could not open spreadsheet'), { type: 'error' });
  }
}

async function openNewest() {
  const list = await listSheets();
  sheets = list;
  if (list.length) await openSheet(list[0]);
  else syncEmpty();
}

async function newSheet() {
  try {
    const created = await createSheet();
    await refreshSheets();
    await openSheet(created);
  } catch (e) {
    toast(e.message || t('plugin.calc.toast.createFailed', null, 'Could not create spreadsheet'), { type: 'error' });
  }
}

async function removeCurrent() {
  if (!current) return;
  const target = current;
  if (!await confirmDialog(t('plugin.calc.dialog.delete', { title: current.title }, 'Delete "{title}"?'))) return;
  if (current !== target || !tileEl) return;
  try {
    await deleteSheet(current.id);
    current = null;
    await refreshSheets();
    await openNewest();
    updateGlow();
  } catch (e) {
    toast(e.message || t('plugin.calc.toast.deleteFailed', null, 'Could not delete spreadsheet'), { type: 'error' });
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
  sheetMenuPopup = menuSurface({ className: 'calc-sheet-menu', onEscape: closeSheetMenu });
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
    empty.textContent = t('plugin.calc.menu.noSheets', null, 'No spreadsheets yet');
    sheetMenuPopup.appendChild(empty);
  } else {
    sheets.forEach((sheet) => {
      const item = menuItem({ className: 'calc-sheet-menu-item' });
      if (sheet.id === current?.id) item.classList.add('is-active');
      item.setAttribute('role', 'menuitem');

      const title = document.createElement('span');
      title.className = 'calc-sheet-menu-title';
      title.textContent = sheet.title;
      const time = document.createElement('span');
      time.className = 'calc-sheet-menu-time';
      time.textContent = formatMenuDate(sheet.updated_at);
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
  const listItems = [...sheetMenuPopup.querySelectorAll('.calc-sheet-menu-item, .calc-sheet-menu-empty')];

  appendMenuFooter(sheetMenuPopup, {
    prefix: 'calc-sheet',
    close: closeSheetMenu,
    items: [
      { icon: 'ui/plus', label: t('plugin.calc.toolbar.newSheet', null, 'New spreadsheet'), onClick: () => void newSheet() },
      { icon: 'ui/upload', label: t('plugin.calc.toolbar.importOds', null, 'Import .ods'), onClick: pickOdsFile },
      { icon: 'ui/upload', label: t('plugin.calc.toolbar.importCsv', null, 'Import CSV'), onClick: pickCsvFile },
      { icon: 'ui/save', label: t('plugin.calc.toolbar.saveToDocuments', null, 'Save to Documents'), onClick: () => void exportOds() },
      { icon: 'ui/trash', label: t('plugin.calc.toolbar.deleteSheet', null, 'Delete spreadsheet'), danger: true, onClick: () => void removeCurrent() },
    ],
  });
  return listItems;
}

function openSheetMenu() {
  ensureSheetMenu();
  const items = renderSheetMenuItems();
  sheetMenuPopup.classList.remove('hidden');
  void listReveal(items);
  placePopover(sheetMenuPopup, sheetMenuBtn);
  sheetMenuPopup.focusFirst();
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

async function pickOdsFile() {
  const [file] = await pickFiles({ accept: '.ods,application/vnd.oasis.opendocument.spreadsheet' });
  if (file) void importOds(file);
}

async function importOds(file) {
  const form = new FormData();
  form.append('file', file);
  try {
    await apiFetch('/api/spreadsheets/import', { method: 'POST', body: form });
    toast(t('plugin.calc.toast.imported', { name: file.name }, 'Imported {name}'), { type: 'info' });
    await refreshSheets();
    await openNewest();
  } catch (e) {
    toast(e.message || t('plugin.calc.toast.importOdsFailed', null, 'Import failed — is this a valid .ods file?'), { type: 'error' });
  }
}

async function exportOds() {
  if (!current) return;
  try {
    const blob = await apiFetch(
      `/api/spreadsheets/${encodeURIComponent(current.id)}/export`,
      { responseType: 'blob' },
    );
    await saveOrDownload(blob, { name: `${current.title || 'spreadsheet'}.ods`, dir: 'Documents', app: 'Calc' });
  } catch (e) {
    toast(e.message || t('plugin.calc.toast.saveFailed', null, 'Save failed'), { type: 'error' });
  }
}

async function pickCsvFile() {
  const [file] = await pickFiles({ accept: '.csv,text/csv' });
  if (file) void importCsv(file);
}

async function importCsv(file) {
  try {
    const text = await file.text();
    // A `;` file comes from a decimal-comma spreadsheet: read its numbers so.
    const delimiter = csvDelimiter(text);
    const rows = parseDelimited(text, delimiter);
    const csvSeps = { decimal: delimiter === ';' ? ',' : '.' };
    const cells = {};
    rows.forEach((line, r) => {
      line.forEach((value, c) => {
        const v = toStored(String(value ?? ''), csvSeps);
        if (!v) return;
        cells[cellRef(r + 1, c)] = v;
      });
    });
    const created = await createSheet(file.name.replace(/\.csv$/i, '') || t('plugin.calc.sheet.importedCsv', null, 'Imported CSV'));
    await saveSheet(created.id, created.title, cells);
    await refreshSheets();
    await openSheet({ ...created, cells });
    toast(t('plugin.calc.toast.imported', { name: file.name }, 'Imported {name}'), { type: 'info' });
  } catch (e) {
    toast(e.message || t('plugin.calc.toast.importCsvFailed', null, 'Import failed — is this a valid CSV file?'), { type: 'error' });
  }
}

/* ── Grid ───────────────────────────────────────────────────── */

/** The data cells of the grid as it was last built:
 *  `{ host, rows, cols, cells, at }` — `at[row][col]` is a cell's element. */
let builtGrid = null;

const widthOf = (col) => {
  const w = Number(colWidths[col]);
  return Number.isFinite(w) && w > 0 ? w : DEFAULT_COL_WIDTH;
};

function gridTemplate(totalCols) {
  const cols = [];
  for (let c = 0; c < totalCols; c++) cols.push(`${widthOf(c)}px`);
  return `44px ${cols.join(' ')}`;
}

/** The selected range, corners ordered. */
function selRange() {
  return {
    r1: Math.min(sel.row, anchor.row),
    r2: Math.max(sel.row, anchor.row),
    c1: Math.min(sel.col, anchor.col),
    c2: Math.max(sel.col, anchor.col),
  };
}

/**
 * Paint one cell: its text and what kind of value it holds (a number sits
 * right, an error centred, text left). Text with empty cells to its right
 * runs on over them (`is-spill`), as in a spreadsheet, up to the next
 * filled cell; otherwise it ends in an ellipsis.
 */
function paintCell(el, ref, r, c) {
  const raw = current.cells.get(ref);
  const val = values.get(ref);
  const text = displayValue(ref);
  const formula = String(raw ?? '').startsWith('=');
  const kind = !raw ? '' : val?.t === 'e' ? 'error'
    : val?.t === 'n' || val?.t === 'b' ? 'number'
      : formula && !val ? 'number' : 'text';
  el.classList.toggle('is-formula', formula);
  el.classList.toggle('is-number', kind === 'number');
  el.classList.toggle('is-error', kind === 'error');
  el.title = kind === 'error' ? `${text} — ${errorExplanation(text)}` : ref;
  let spill = 0;
  if (kind === 'text' && text) {
    const totalCols = builtGrid?.cols || 0;
    for (let k = c + 1; k < totalCols && k <= c + 12; k++) {
      if (current.cells.has(cellRef(r, k))) break;
      spill += widthOf(k);
    }
  }
  if (spill > 0) {
    el.classList.add('is-spill');
    el.style.setProperty('--calc-spill', `${widthOf(c) + spill - 16}px`);
    const span = el.firstElementChild?.classList.contains('calc-cell-text') ? el.firstElementChild : null;
    if (span) {
      if (span.textContent !== text) span.textContent = text;
    } else {
      el.textContent = '';
      const inner = document.createElement('span');
      inner.className = 'calc-cell-text';
      inner.textContent = text;
      el.appendChild(inner);
    }
  } else {
    el.classList.remove('is-spill');
    el.style.removeProperty('--calc-spill');
    if (el.firstElementChild || el.textContent !== text) el.textContent = text;
  }
}

function paintCellAt(row, col) {
  const el = cellElFor(row, col);
  if (el) paintCell(el, cellRef(row, col), row, col);
}

/** Text to the left of a changed cell may now spill further, or less. */
function paintSpillRows(batch) {
  const rows = new Set();
  for (const { ref } of batch) {
    const p = parseRef(ref);
    if (p) rows.add(p.row);
  }
  for (const row of rows) {
    for (let c = 0; c < (builtGrid?.cols || 0); c++) {
      if (current.cells.has(cellRef(row, c))) paintCellAt(row, c);
    }
  }
}

function paintAllCells() {
  if (!builtGrid || !current) return;
  for (const { el, ref, r, c } of builtGrid.cells) paintCell(el, ref, r, c);
}

/** Mark the active cell and the selected range. */
function paintSelection() {
  if (!builtGrid) return;
  gridEl.querySelectorAll('.calc-cell--data.is-selected, .calc-cell--data.is-in-range')
    .forEach((el) => el.classList.remove('is-selected', 'is-in-range'));
  const r = selRange();
  if (r.r1 !== r.r2 || r.c1 !== r.c2) {
    for (let row = r.r1; row <= r.r2; row++) {
      for (let col = r.c1; col <= r.c2; col++) cellElFor(row, col)?.classList.add('is-in-range');
    }
  }
  cellElFor(sel.row, sel.col)?.classList.add('is-selected');
  gridEl.querySelectorAll('.calc-cell--head').forEach((h) => {
    const c = Number(h.dataset.col);
    h.classList.toggle('is-active', c >= r.c1 && c <= r.c2);
  });
  gridEl.querySelectorAll('.calc-cell--rowhead').forEach((h) => {
    const row = Number(h.dataset.row);
    h.classList.toggle('is-active', row >= r.r1 && row <= r.r2);
  });
}

/**
 * Bring an already-built grid of the same size up to date instead of
 * rebuilding it: only the cells whose text or state changed are touched.
 * A rebuild creates one element and three listeners per cell (up to 200×52).
 * False when the grid has to be built.
 */
function refreshGrid(totalRows, totalCols) {
  const built = builtGrid;
  if (!built || built.host !== gridEl || built.rows !== totalRows || built.cols !== totalCols) return false;
  // An open editor is positioned over its cell and is dropped by a rebuild:
  // keep that path as it was.
  if (editing || !built.cells.length || !built.cells[0].el.isConnected) return false;
  gridEl.style.gridTemplateColumns = gridTemplate(totalCols);
  for (const { el, ref, r, c } of built.cells) {
    paintCell(el, ref, r, c);
    el.__editOnTap = false;
  }
  paintSelection();
  return true;
}

/** The data cell under a screen point (a drag across the grid). */
function cellAtPoint(x, y) {
  const el = document.elementFromPoint(x, y)?.closest?.('.calc-cell--data');
  return el && gridEl?.contains(el) ? el : null;
}

function onDragMove(e) {
  if (!dragSelecting) return;
  const el = cellAtPoint(e.clientX, e.clientY);
  if (!el) return;
  const row = Number(el.dataset.row);
  const col = Number(el.dataset.col);
  if (row !== sel.row || col !== sel.col) selectCell(row, col, { extend: true, focus: false });
}

function onDragEnd() {
  if (!dragSelecting) return;
  dragSelecting = false;
  document.removeEventListener('pointermove', onDragMove);
  document.removeEventListener('pointerup', onDragEnd);
  cellElFor(sel.row, sel.col)?.focus({ preventScroll: true });
}

/** Drag a column header's right edge to resize it; double-click resets it. */
function wireColumnResize(handle, col) {
  handle.addEventListener('pointerdown', (e) => {
    e.preventDefault();
    e.stopPropagation();
    const startX = e.clientX;
    const startW = widthOf(col);
    handle.setPointerCapture?.(e.pointerId);
    const move = (ev) => {
      const w = Math.round(Math.min(MAX_COL_WIDTH, Math.max(MIN_COL_WIDTH, startW + (ev.clientX - startX) / zoom)));
      colWidths[col] = w;
      gridEl.style.gridTemplateColumns = gridTemplate(builtGrid?.cols || DEFAULT_COLS);
      if (editing) placeEditor();
    };
    const up = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', up);
      handle.removeEventListener('pointercancel', up);
      if (widthOf(col) === DEFAULT_COL_WIDTH) delete colWidths[col];
      paintAllCells();
      markDirty();
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', up);
    handle.addEventListener('pointercancel', up);
  });
  handle.addEventListener('dblclick', (e) => {
    e.stopPropagation();
    delete colWidths[col];
    gridEl.style.gridTemplateColumns = gridTemplate(builtGrid?.cols || DEFAULT_COLS);
    paintAllCells();
    markDirty();
  });
}

function renderGrid() {
  if (!gridEl || !current) return;

  const totalCols = Math.max(1, Math.min(current.cols, 52));
  const totalRows = Math.min(current.rows, 200);
  if (refreshGrid(totalRows, totalCols)) return;

  gridEl.innerHTML = '';
  const cells = [];
  const at = [];
  builtGrid = { host: gridEl, rows: totalRows, cols: totalCols, cells, at };

  // Size the grid columns to the actual sheet width (and the widths the user
  // dragged). If this stayed fixed while totalCols varies, CSS auto-flow would
  // wrap the row headers/data cells into the wrong columns.
  gridEl.style.gridTemplateColumns = gridTemplate(totalCols);

  const corner = document.createElement('div');
  corner.className = 'calc-cell calc-cell--corner';
  corner.textContent = '';
  gridEl.appendChild(corner);

  const resizeLabel = t('plugin.calc.grid.resizeColumn', null, 'Drag to resize the column, double-click to reset it');
  for (let c = 0; c < totalCols; c++) {
    const head = document.createElement('div');
    head.className = 'calc-cell calc-cell--head';
    head.dataset.col = String(c);
    head.textContent = colName(c);
    const handle = document.createElement('div');
    handle.className = 'calc-col-resize';
    handle.title = resizeLabel;
    handle.setAttribute('aria-hidden', 'true');
    wireColumnResize(handle, c);
    head.appendChild(handle);
    gridEl.appendChild(head);
  }

  for (let r = 1; r <= totalRows; r++) {
    const rowHead = document.createElement('div');
    rowHead.className = 'calc-cell calc-cell--rowhead';
    rowHead.dataset.row = String(r);
    rowHead.textContent = String(r);
    gridEl.appendChild(rowHead);
    at[r] = [];

    for (let c = 0; c < totalCols; c++) {
      const ref = cellRef(r, c);
      const cell = document.createElement('div');
      cell.className = 'calc-cell calc-cell--data';
      cell.dataset.ref = ref;
      cell.dataset.row = String(r);
      cell.dataset.col = String(c);
      cell.tabIndex = 0;
      // Select on pointerdown (before any blur-triggered work) so a click
      // always lands on the cell under the pointer. A finger has no double
      // click: a second tap on the selected cell opens the editor (a long
      // press is swallowed by the context menu and never reaches `click`).
      // A mouse drags out a range; Shift+click extends it.
      cell.addEventListener('pointerdown', (e) => {
        if (e.button > 0) {
          // A right-click inside the range keeps it (the menu acts on it).
          const rg = selRange();
          if (r >= rg.r1 && r <= rg.r2 && c >= rg.c1 && c <= rg.c2) return;
        }
        const again = e.pointerType !== 'mouse' && e.isPrimary
          && sel.row === r && sel.col === c && !editing;
        selectCell(r, c, { extend: e.shiftKey });
        cell.__editOnTap = again;
        if (e.pointerType === 'mouse' && e.button === 0 && !e.shiftKey) {
          dragSelecting = true;
          document.addEventListener('pointermove', onDragMove);
          document.addEventListener('pointerup', onDragEnd);
        }
      });
      cell.addEventListener('click', () => {
        if (!cell.__editOnTap) return;
        cell.__editOnTap = false;
        if (Date.now() - pinchedAt < 600) return;
        showEditor(null, 'edit');
      });
      cell.addEventListener('dblclick', () => {
        // Two quick taps already opened the editor through `click`.
        if (editing && sel.row === r && sel.col === c) return;
        selectCell(r, c);
        showEditor(null, 'edit');
      });
      gridEl.appendChild(cell);
      at[r][c] = cell;
      cells.push({ el: cell, ref, r, c });
    }
  }
  for (const { el, ref, r, c } of cells) paintCell(el, ref, r, c);
  paintSelection();
}

/** Overlay input positioned over the selected cell. */
function ensureEditor() {
  // The grid re-render (innerHTML = '') detaches the editor — recreate it.
  if (editorInputEl && editorInputEl.isConnected) return;
  editorInputEl = uiInput();
  editorInputEl.classList.add('calc-cell-editor');
  editorInputEl.autocomplete = 'off';
  editorInputEl.addEventListener('keydown', (e) => {
    e.stopPropagation();
    const arrow = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] }[e.key];
    if (e.key === 'Enter') { e.preventDefault(); commitEdit(e.shiftKey ? [-1, 0] : [1, 0]); }
    else if (e.key === 'Tab') { e.preventDefault(); commitEdit(e.shiftKey ? [0, -1] : [0, 1]); }
    else if (e.key === 'Escape') { e.preventDefault(); cancelEdit(); }
    // Typed over a cell, an arrow confirms and moves on; while editing
    // (F2, double click) left and right move the caret.
    else if (arrow && (editMode === 'enter' || arrow[0] !== 0) && !e.shiftKey) {
      e.preventDefault();
      commitEdit(arrow);
    }
  });
  editorInputEl.addEventListener('blur', () => {
    if (!editing) return;
    const value = editorInputEl.value;
    const ref = cellRef(sel.row, sel.col);
    hideEditor();
    // Commit in place (no rebuild) so a click-away lands on the right cell.
    setCellValue(ref, toStored(value, seps()));
  });
  gridEl.appendChild(editorInputEl);
}

function cellElFor(row, col) {
  return builtGrid?.host === gridEl ? builtGrid.at[row]?.[col] || null : null;
}

/**
 * Move the active cell without rebuilding the grid. `extend` keeps the
 * range's other corner where it is (Shift); otherwise the range collapses.
 */
function selectCell(row, col, { focus = true, scroll = false, extend = false } = {}) {
  const maxRow = builtGrid?.rows || current?.rows || DEFAULT_ROWS;
  const maxCol = (builtGrid?.cols || current?.cols || DEFAULT_COLS) - 1;
  sel = { row: Math.max(1, Math.min(maxRow, row)), col: Math.max(0, Math.min(maxCol, col)) };
  if (!extend) anchor = { ...sel };
  paintSelection();
  syncFormulaBar();
  paintStatusLine();
  const cell = cellElFor(sel.row, sel.col);
  if (scroll) cell?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  if (focus) cell?.focus({ preventScroll: !scroll });
}

/**
 * Open the in-cell editor. A key typed on a cell starts it with that key
 * (`enter` mode); Enter, F2 or a double click edit what is there (`edit`).
 */
function showEditor(prefill = null, mode = prefill != null ? 'enter' : 'edit') {
  ensureEditor();
  const cell = cellElFor(sel.row, sel.col);
  if (!cell) return;
  editing = true;
  editMode = mode;
  editorInputEl.value = prefill != null
    ? prefill
    : toEditable(current.cells.get(cellRef(sel.row, sel.col)) ?? '', seps());
  // The stylesheet keeps the overlay at `display: none` between edits, so
  // showing it needs a value of its own, not a cleared inline style.
  editorInputEl.style.display = 'block';
  placeEditor(cell);
  editorInputEl.focus();
  // Typing on a cell starts the edit with that key: keep it and put the caret
  // after it, or the next key would replace it. Enter/F2 put the caret at the
  // end of the old value, ready to change it.
  const end = editorInputEl.value.length;
  editorInputEl.setSelectionRange(end, end);
}

/** Lay the overlay over a cell. The grid is zoomed, and the overlay lives
 *  inside it, so screen distances convert back to the grid's own pixels. */
function placeEditor(cell = cellElFor(sel.row, sel.col)) {
  if (!cell || !editorInputEl) return;
  const r = cell.getBoundingClientRect();
  const g = gridEl.getBoundingClientRect();
  editorInputEl.style.left = `${(r.left - g.left) / zoom}px`;
  editorInputEl.style.top = `${(r.top - g.top) / zoom}px`;
  editorInputEl.style.width = `${r.width / zoom}px`;
  editorInputEl.style.height = `${r.height / zoom}px`;
}

/* ── Zoom ───────────────────────────────────────────────────── */

/** Set the grid zoom, keeping the content under (x, y) — a screen point,
 *  the wrapper's centre by default — where it is. */
function setZoom(value, focal = null) {
  const next = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(value * 100) / 100));
  if (!gridEl || next === zoom) return;
  const wrap = gridWrapEl;
  const box = wrap?.getBoundingClientRect();
  const fx = focal ? focal.x - box.left : (box?.width || 0) / 2;
  const fy = focal ? focal.y - box.top : (box?.height || 0) / 2;
  const cx = (wrap ? wrap.scrollLeft + fx : 0) / zoom;
  const cy = (wrap ? wrap.scrollTop + fy : 0) / zoom;
  zoom = next;
  gridEl.style.setProperty('--calc-zoom', String(zoom));
  if (wrap) {
    wrap.scrollLeft = cx * zoom - fx;
    wrap.scrollTop = cy * zoom - fy;
  }
  if (editing) placeEditor();
}

function zoomBy(factor) {
  setZoom(zoom * factor);
}

/** Pinch (two pointers), Ctrl+wheel (a trackpad pinch) and WebKit's gesture
 *  events on the grid's scroll container. One finger still pans (the
 *  wrapper's `touch-action`); two fingers are kept from the browser's own
 *  scroll so their pointers survive. */
function wirePinch(wrap) {
  const pts = new Map();
  let startDist = 0;
  let startZoom = 1;
  const dist = () => {
    const [a, b] = [...pts.values()];
    return Math.hypot(a.x - b.x, a.y - b.y);
  };
  const mid = () => {
    const [a, b] = [...pts.values()];
    return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
  };
  wrap.addEventListener('pointerdown', (e) => {
    if (e.pointerType === 'mouse') return;
    pts.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pts.size === 2) {
      startDist = dist();
      startZoom = zoom;
      pinchedAt = Date.now();
    }
  });
  wrap.addEventListener('pointermove', (e) => {
    if (!pts.has(e.pointerId)) return;
    pts.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pts.size === 2 && startDist > 0) {
      setZoom(startZoom * (dist() / startDist), mid());
    }
  });
  const end = (e) => {
    pts.delete(e.pointerId);
    if (pts.size < 2) startDist = 0;
  };
  wrap.addEventListener('pointerup', end);
  wrap.addEventListener('pointercancel', end);
  wrap.addEventListener('touchmove', (e) => {
    if (e.touches.length >= 2) e.preventDefault();
  }, { passive: false });
  wrap.addEventListener('wheel', (e) => {
    if (!e.ctrlKey) return;
    e.preventDefault();
    setZoom(zoom * (e.deltaY < 0 ? 1.1 : 1 / 1.1), { x: e.clientX, y: e.clientY });
  }, { passive: false });
  wrap.addEventListener('gesturestart', (e) => {
    e.preventDefault();
    startZoom = zoom;
    pinchedAt = Date.now();
  });
  wrap.addEventListener('gesturechange', (e) => {
    e.preventDefault();
    setZoom(startZoom * (e.scale || 1));
  });
}

function hideEditor() {
  editing = false;
  if (editorInputEl) editorInputEl.style.display = 'none';
}

/** Confirm the edit, then move by `[dRow, dCol]` (down after Enter). */
function commitEdit(move = [1, 0]) {
  if (!editing || !current) return;
  const value = editorInputEl?.value ?? '';
  const editedRef = cellRef(sel.row, sel.col);
  hideEditor();
  setCellValue(editedRef, toStored(value, seps()));
  selectCell(sel.row + move[0], sel.col + move[1], { scroll: true });
}

function cancelEdit() {
  hideEditor();
  cellElFor(sel.row, sel.col)?.focus();
}

/* ── Formula bar ────────────────────────────────────────────── */

function syncFormulaBar() {
  if (!formulaRefEl || !formulaInputEl || !current) return;
  const ref = cellRef(sel.row, sel.col);
  const r = selRange();
  formulaRefEl.textContent = r.r1 !== r.r2 || r.c1 !== r.c2
    ? `${cellRef(r.r1, r.c1)}:${cellRef(r.r2, r.c2)}`
    : ref;
  if (document.activeElement !== formulaInputEl) {
    formulaInputEl.value = toEditable(current.cells.get(ref) ?? '', seps());
  }
}

/* ── Clear, copy, cut, paste ────────────────────────────────── */

/** Empty every cell of the selected range, as one step of undo. */
function clearSelection() {
  if (!current) return;
  const r = selRange();
  const changes = [];
  for (let row = r.r1; row <= r.r2; row++) {
    for (let col = r.c1; col <= r.c2; col++) {
      const ref = cellRef(row, col);
      if (current.cells.has(ref)) changes.push({ ref, value: '' });
    }
  }
  setCells(changes);
}

/** The keyboard is on the grid (not in a field, not editing a cell). */
function gridHasFocus() {
  return !!(gridEl && current && !editing && gridEl.contains(document.activeElement)
    && document.activeElement !== editorInputEl);
}

/** The selected range as text for other apps (what the cells show), and
 *  as stored text for a paste back into a sheet. */
function copySelection() {
  const r = selRange();
  const shown = [];
  const raw = [];
  for (let row = r.r1; row <= r.r2; row++) {
    const s = [];
    const w = [];
    for (let col = r.c1; col <= r.c2; col++) {
      const ref = cellRef(row, col);
      s.push(displayValue(ref));
      w.push(current.cells.get(ref) ?? '');
    }
    shown.push(s);
    raw.push(w);
  }
  const text = toTsv(shown);
  clip = { text, rows: raw, origin: { row: r.r1, col: r.c1 } };
  return text;
}

/**
 * Paste at the selection. Cells copied here keep their formulas, with
 * relative refs moved to the new place; one copied cell fills a selected
 * range. Text from elsewhere is read as rows and columns (tabs, lines), each
 * piece typed in as the user would.
 */
function pasteText(text) {
  if (!current) return;
  const r = selRange();
  const changes = [];
  const internal = clip && clip.text === text.replace(/\r\n/g, '\n').replace(/\n$/, '');
  if (internal) {
    const rows = clip.rows;
    const single = rows.length === 1 && rows[0].length === 1;
    const height = single ? r.r2 - r.r1 + 1 : rows.length;
    const width = single ? r.c2 - r.c1 + 1 : rows[0].length;
    for (let i = 0; i < height; i++) {
      for (let j = 0; j < width; j++) {
        const row = r.r1 + i;
        const col = r.c1 + j;
        if (row > builtGrid.rows || col >= builtGrid.cols) continue;
        const src = single ? rows[0][0] : rows[i][j];
        const from = single ? clip.origin : { row: clip.origin.row + i, col: clip.origin.col + j };
        changes.push({ ref: cellRef(row, col), value: shiftFormula(src, row - from.row, col - from.col) });
      }
    }
    setCells(changes);
    anchor = { row: r.r1, col: r.c1 };
    selectCell(r.r1 + height - 1, r.c1 + width - 1, { extend: true });
    return;
  }
  const rows = parseTsv(text);
  if (!rows.length) return;
  const sp = seps();
  rows.forEach((line, i) => line.forEach((value, j) => {
    const row = r.r1 + i;
    const col = r.c1 + j;
    if (row > builtGrid.rows || col >= builtGrid.cols) return;
    changes.push({ ref: cellRef(row, col), value: toStored(value, sp) });
  }));
  setCells(changes);
  const width = Math.max(...rows.map((l) => l.length));
  anchor = { row: r.r1, col: r.c1 };
  selectCell(r.r1 + rows.length - 1, r.c1 + width - 1, { extend: true });
}

function onCopy(e) {
  if (!gridHasFocus()) return;
  e.preventDefault();
  e.clipboardData?.setData('text/plain', copySelection());
}

function onCut(e) {
  if (!gridHasFocus()) return;
  e.preventDefault();
  e.clipboardData?.setData('text/plain', copySelection());
  clearSelection();
}

function onPaste(e) {
  if (!gridHasFocus()) return;
  const text = e.clipboardData?.getData('text/plain');
  if (text == null) return;
  e.preventDefault();
  pasteText(text);
}

/** Paste from the context menu: the system clipboard when the browser lets
 *  us read it, else the last copy made here. */
async function pasteFromMenu() {
  let text = null;
  try { text = await navigator.clipboard?.readText(); } catch { text = null; }
  if (text == null && clip) text = clip.text;
  if (text == null) {
    toast(t('plugin.calc.toast.pasteKeys', null, 'Use Ctrl+V to paste'), { type: 'info' });
    return;
  }
  cellElFor(sel.row, sel.col)?.focus();
  pasteText(text);
}

/* ── Tile lifecycle ─────────────────────────────────────────── */

/** Create the Calc tile element (the plugin's window container). */
export function mountCalcTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile calc-tile';
  tileEl.dataset.plugin = CALC_PLUGIN;

  /* Top bar: sheet menu + title + save status, one save tool and a "New"
     pill. Import / export / delete live in the sheet menu's footer. */
  const bar = toolbar([], { className: 'calc-bar' });
  bar.dataset.windowBar = '';

  sheetMenuBtn = menuButton({ icon: 'ui/calc', label: t('plugin.calc.bar.spreadsheets', null, 'Spreadsheets'), onClick: toggleSheetMenu, className: 'calc-sheet-btn' });

  titleInput = uiInput();
  titleInput.classList.add('calc-title');
  titleInput.type = 'text';
  titleInput.placeholder = t('plugin.calc.sheet.untitled', null, 'Untitled');
  titleInput.maxLength = 120;
  titleInput.autocomplete = 'off';
  titleInput.addEventListener('change', () => {
    if (current) void persist();
  });

  saveStateEl = document.createElement('span');
  saveStateEl.className = 'calc-save-state';
  saveStateEl.setAttribute('role', 'status');

  const saveBtn = toolButton('ui/save', t('plugin.calc.toolbar.saveNow', null, 'Save now'), () => void persist(), { className: 'calc-tool' });
  const newLabel = t('plugin.calc.toolbar.newSheet', null, 'New spreadsheet');
  const newBtn = button({ icon: 'ui/plus', label: t('plugin.calc.bar.new', null, 'New'), variant: 'ghost', size: 'sm', className: 'calc-new-btn', onClick: () => void newSheet() });
  newBtn.title = newLabel;
  newBtn.setAttribute('aria-label', newLabel);
  // Grid zoom for the mouse (a finger pinches); they fold into the bar's
  // "More" menu first, being last.
  const zoomOutBtn = toolButton('ui/zoom-out', t('plugin.calc.toolbar.zoomOut', null, 'Zoom out'), () => zoomBy(1 / ZOOM_STEP), { className: 'calc-tool calc-zoom-out' });
  const zoomInBtn = toolButton('ui/zoom-in', t('plugin.calc.toolbar.zoomIn', null, 'Zoom in'), () => zoomBy(ZOOM_STEP), { className: 'calc-tool calc-zoom-in' });

  bar.append(sheetMenuBtn, titleInput, saveStateEl, saveBtn, newBtn, zoomOutBtn, zoomInBtn);
  tileEl.appendChild(bar);

  /* Body (formula bar, grid, status) — swapped for the empty state when
     there is no sheet. */
  bodyEl = document.createElement('div');
  bodyEl.className = 'calc-body';
  tileEl.appendChild(bodyEl);

  /* Three icon tiles, each named by a caption under it: the caption is plain
     text, so a phone keeps reading what each does while the button is an
     icon. */
  const emptyActions = document.createElement('div');
  emptyActions.className = 'calc-empty-actions';
  const emptyTile = (icon, label, onClick, variant = 'ghost') => {
    const tile = document.createElement('div');
    tile.className = 'calc-empty-tile';
    const btn = iconButton({ icon, label, onClick, variant, iconSize: 20 });
    const caption = document.createElement('span');
    caption.className = 'calc-empty-caption';
    caption.textContent = label;
    caption.addEventListener('click', () => btn.click());
    tile.append(btn, caption);
    return tile;
  };
  emptyActions.append(
    emptyTile('ui/plus', newLabel, () => void newSheet(), 'primary'),
    emptyTile('ui/upload', t('plugin.calc.toolbar.importCsv', null, 'Import CSV'), pickCsvFile),
    emptyTile('ui/upload', t('plugin.calc.toolbar.importOds', null, 'Import .ods'), pickOdsFile),
  );
  emptyEl = emptyState({
    icon: 'ui/calc',
    title: t('plugin.calc.menu.noSheets', null, 'No spreadsheets yet'),
    body: t('plugin.calc.empty.body', null, 'Start a blank sheet, import a CSV or .ods file, or ask the AI to build one.'),
    action: emptyActions,
  });
  emptyEl.classList.add('calc-empty', 'hidden');
  tileEl.appendChild(emptyEl);

  /* Formula bar */
  const formulaBar = toolbar([], { className: 'calc-formula-bar' });
  formulaRefEl = document.createElement('span');
  formulaRefEl.className = 'calc-formula-ref';
  formulaRefEl.textContent = 'A1';
  formulaInputEl = uiInput();
  formulaInputEl.classList.add('calc-formula-input');
  formulaInputEl.type = 'text';
  formulaInputEl.placeholder = t('plugin.calc.formula.placeholder', null, 'Value or formula (=SUM(A1:A5))');
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
  bodyEl.appendChild(formulaBar);

  function commitFormula() {
    if (!current || document.activeElement === formulaInputEl && editing) return;
    const ref = cellRef(sel.row, sel.col);
    const value = toStored(formulaInputEl.value, seps());
    // In place (no grid rebuild): a rebuild inside the blur handler would
    // detach the node under the pointer and send the click to the wrong cell.
    if ((current.cells.get(ref) ?? '') !== value) setCellValue(ref, value);
    syncFormulaBar();
  }

  /* Grid, inside its scroll container (the pinch surface). */
  gridWrapEl = document.createElement('div');
  gridWrapEl.className = 'calc-grid-wrap';
  gridEl = document.createElement('div');
  gridEl.className = 'calc-grid';
  gridEl.style.setProperty('--calc-zoom', String(zoom));
  gridEl.addEventListener('keydown', (e) => {
    if (editing) return;
    const ctrl = e.ctrlKey || e.metaKey;
    const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    const arrow = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] }[e.key];
    if (arrow) {
      e.preventDefault();
      // Shift stretches the range from its anchor.
      selectCell(sel.row + arrow[0], sel.col + arrow[1], { scroll: true, extend: e.shiftKey });
    } else if (e.key === 'Enter') { e.preventDefault(); showEditor(null, 'edit'); }
    else if (e.key === 'F2') { e.preventDefault(); showEditor(null, 'edit'); }
    else if (e.key === 'Tab') { e.preventDefault(); selectCell(sel.row, sel.col + (e.shiftKey ? -1 : 1), { scroll: true }); }
    else if (e.key === 'Delete' || e.key === 'Backspace') { e.preventDefault(); clearSelection(); }
    else if (e.key === 'Escape') { e.preventDefault(); selectCell(sel.row, sel.col); }
    else if (ctrl && key === 'z' && !e.shiftKey) { e.preventDefault(); stepHistory(-1); }
    else if (ctrl && (key === 'y' || (key === 'z' && e.shiftKey))) { e.preventDefault(); stepHistory(1); }
    else if (ctrl && key === 'a') {
      e.preventDefault();
      anchor = { row: 1, col: 0 };
      selectCell(builtGrid?.rows || 1, (builtGrid?.cols || 1) - 1, { extend: true, focus: false });
    } else if (e.key.length === 1 && !ctrl && !e.altKey) {
      e.preventDefault();
      showEditor(e.key, 'enter');
    }
  });
  gridWrapEl.appendChild(gridEl);
  wirePinch(gridWrapEl);
  bodyEl.appendChild(gridWrapEl);

  /* Status line */
  const status = document.createElement('div');
  status.className = 'calc-status';
  statusEl = document.createElement('span');
  status.appendChild(statusEl);
  bodyEl.appendChild(status);

  document.addEventListener('copy', onCopy);
  document.addEventListener('cut', onCut);
  document.addEventListener('paste', onPaste);

  void openNewest();
  return tileEl;
}

/** Deactivated mid-edit: flush, drop the window. */
export function unmountCalcTile() {
  if (current && dirty) void persist();
  hideEditor();
  document.removeEventListener('copy', onCopy);
  document.removeEventListener('cut', onCut);
  document.removeEventListener('paste', onPaste);
  onDragEnd();
  tileEl?.remove();
  tileEl = null;
  gridEl = null;
  builtGrid = null;
  gridWrapEl = null;
  editorInputEl = null;
  sheetMenuBtn = null;
  titleInput = null;
  statusEl = null;
  saveStateEl = null;
  bodyEl = null;
  emptyEl = null;
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
  const cleared = calcActions.some((a) => a.action === 'calc_clear' && a.result === 'ok');
  const deleted = calcActions.some((a) => a.action === 'calc_delete' && a.result === 'ok');
  const read = calcActions.some((a) => a.action === 'calc_read' && a.result === 'ok');

  // The action payload carries the touched sheet_id when compact.
  const touchedId = calcActions
    .map((a) => a.data?.sheet_id)
    .find((id) => !!id);

  if (created) {
    void refreshSheets().then(() => window.setTimeout(() => {
      if (dirty) return; // never clobber the user's unsaved edits
      if (touchedId) {
        const found = sheets.find((s) => s.id === touchedId);
        if (found) void openSheet(found);
      } else {
        void openNewest();
      }
    }, 250));
  } else if (wrote || read || cleared || deleted) {
    void (async () => {
      // Flush any pending local edits first so they aren't lost, then reload
      // so the AI's write is always visible (never skip the reload silently).
      if (dirty) await persist();
      await refreshSheets();
      if (touchedId && current?.id !== touchedId) {
        const found = sheets.find((s) => s.id === touchedId);
        if (found) void openSheet(found);
      } else if ((wrote || cleared) && current) {
        const full = sheets.find((s) => s.id === current.id);
        if (full) void openSheet(full);
      } else if (deleted) {
        void openNewest();
      }
    })();
  }
}

async function importFromFiles(path, name) {
  try {
    if (!getCalcTileElement()) mountCalcTile();
    const file = await fileFromHome(path, name, 'application/vnd.oasis.opendocument.spreadsheet');
    await importOds(file);
  } catch (e) {
    toast(e.message || t('plugin.calc.toast.openFileFailed', null, 'Could not open file'), { type: 'error' });
  }
}

let wired = false;
export function wireCalcEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
  onOpenFromFiles(CALC_PLUGIN, (d) => importFromFiles(d.path, d.name));
  window.addEventListener('beforeunload', (e) => {
    if (current && dirty) {
      e.preventDefault();
      e.returnValue = '';
    }
  });
}

/** Entries core splices into this window's right-click menu (PLUGINS.md §19).
 *  Core supplies the surrounding separators + window management. */
export function calcContextMenu(ctx) {
  const hasSheet = !!current;
  // Opened on a cell (right-click, or a long press on a touchscreen): that
  // cell's own entries come first.
  const cellEl = hasSheet ? ctx?.target?.closest?.('.calc-cell--data') : null;
  const cellItems = [];
  if (cellEl) {
    const row = Number(cellEl.dataset.row);
    const col = Number(cellEl.dataset.col);
    const rg = selRange();
    const inRange = row >= rg.r1 && row <= rg.r2 && col >= rg.c1 && col <= rg.c2;
    if (!inRange) selectCell(row, col);
    const r = selRange();
    let filled = false;
    for (let rr = r.r1; rr <= r.r2 && !filled; rr++) {
      for (let cc = r.c1; cc <= r.c2; cc++) if (current.cells.has(cellRef(rr, cc))) { filled = true; break; }
    }
    // Copy and cut go through the browser's own copy event, so the system
    // clipboard gets the text as Ctrl+C would.
    const viaKeys = (cmd) => () => {
      cellElFor(sel.row, sel.col)?.focus();
      try { document.execCommand(cmd); } catch { /* the keys still work */ }
    };
    cellItems.push(
      { type: 'item', label: t('plugin.calc.cell.edit', null, 'Edit cell'), icon: 'ui/pencil', onClick: () => { selectCell(row, col); showEditor(null, 'edit'); } },
      { type: 'item', label: t('plugin.calc.cell.copy', null, 'Copy'), icon: 'ui/copy', onClick: viaKeys('copy') },
      { type: 'item', label: t('plugin.calc.cell.cut', null, 'Cut'), icon: 'ui/scissors', disabled: !filled, onClick: viaKeys('cut') },
      { type: 'item', label: t('plugin.calc.cell.paste', null, 'Paste'), icon: 'ui/clipboard', onClick: () => void pasteFromMenu() },
      { type: 'item', label: t('plugin.calc.cell.clear', null, 'Clear cell'), icon: 'ui/eraser', disabled: !filled, onClick: clearSelection },
      { type: 'separator' },
      { type: 'item', label: t('plugin.calc.cell.undo', null, 'Undo'), icon: 'ui/undo', disabled: !undoStack.length, onClick: () => stepHistory(-1) },
      { type: 'item', label: t('plugin.calc.cell.redo', null, 'Redo'), icon: 'ui/redo', disabled: !redoStack.length, onClick: () => stepHistory(1) },
      { type: 'separator' },
    );
  }
  return [
    ...cellItems,
    { type: 'item', label: t('plugin.calc.toolbar.newSheet', null, 'New spreadsheet'), icon: 'ui/plus', onClick: () => void newSheet() },
    { type: 'item', label: t('plugin.calc.toolbar.saveNow', null, 'Save now'), icon: 'ui/save', disabled: !hasSheet || !dirty, onClick: () => void persist() },
    { type: 'item', label: t('plugin.calc.toolbar.saveToDocuments', null, 'Save to Documents'), icon: 'ui/save', disabled: !hasSheet, onClick: () => void exportOds() },
    { type: 'separator' },
    { type: 'item', label: t('plugin.calc.toolbar.deleteSheet', null, 'Delete spreadsheet'), icon: 'ui/trash', danger: true, disabled: !hasSheet, onClick: () => void removeCurrent() },
  ];
}

export default {
  name: 'calc',
  icon: 'ui/calc',
  mount: mountCalcTile,
  unmount: unmountCalcTile,
  getElement: getCalcTileElement,
  wireEvents: wireCalcEvents,
  contextMenu: calcContextMenu,
};

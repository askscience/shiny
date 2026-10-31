/**
 * The PDF window's document editor: write on A4 paper, export an exact PDF.
 *
 * The document is HTML (kept to `sanitize.js`'s vocabulary) styled by
 * `doc-themes.js`. The paper has the printer's page width and margins, so
 * line breaks match; dashed guides show where pages end. Export hands the
 * same HTML and stylesheet to the shell (peakd prints it with Chromium:
 * `peakd:print:` → `window.__peakdPrinted`), then uploads the PDF. Without the
 * shell (a plain browser) export falls back to the browser's print dialog.
 */

import { apiFetch } from '../../js/api.js';
import { t } from '../../js/i18n.js';
import { pickFiles } from '../../js/files.js';
import { controlButton, icon, toast, promptDialog, select as uiSelect } from '../../ui/index.js';
import { DOC_CSS, PAGE, THEMES, printHtml, themeStyle } from './doc-themes.js';
import { sanitizeHtml } from './sanitize.js';

const enc = (id) => encodeURIComponent(id);
const THEME_MARK = /^<!--pdfdoc theme=(\w+)( numbers=1)?-->/;

/** Split stored source into theme + body. */
export function parseSource(source) {
  const m = THEME_MARK.exec(source || '');
  return { theme: m && THEMES[m[1]] ? m[1] : 'modern', numbers: !!m?.[2], body: (source || '').replace(THEME_MARK, '') };
}

function joinSource(theme, body, numbers) {
  return `<!--pdfdoc theme=${theme}${numbers ? ' numbers=1' : ''}-->${body}`;
}

let docStyleInstalled = false;
function installDocStyle() {
  if (docStyleInstalled) return;
  docStyleInstalled = true;
  const style = document.createElement('style');
  style.dataset.owner = 'pdf-editor';
  style.textContent = DOC_CSS;
  document.head.appendChild(style);
}

/* ── Shell printing ──────────────────────────────────────────── */

const pending = new Map();
function onPrinted(result) {
  const p = pending.get(result?.id);
  if (!p) return;
  pending.delete(result.id);
  clearTimeout(p.timer);
  if (result.ok) {
    const bin = atob(result.data);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    p.resolve(bytes);
  } else {
    p.reject(new Error(result.error || 'print failed'));
  }
}

/** Is the PEAK'D shell's print bridge there? */
export function canShellPrint() {
  return typeof window !== 'undefined' && !!window.ipc && typeof window.ipc.postMessage === 'function' && !!window.__peakdCaps?.print;
}

/** Print HTML to PDF through the shell; resolves with the bytes. */
function shellPrint(html) {
  window.__peakdPrinted = onPrinted;
  return new Promise((resolve, reject) => {
    const id = `p${Date.now()}${Math.random().toString(36).slice(2, 8)}`;
    // An older shell ignores the message: give up and let the caller fall back.
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(Object.assign(new Error('no-shell-print'), { code: 'no-shell-print' }));
    }, 45000);
    pending.set(id, { resolve, reject, timer });
    window.ipc.postMessage(`peakd:print:${JSON.stringify({ id, html, options: { size: PAGE.size, margins: PAGE.margins } })}`);
  });
}

/* ── Images ──────────────────────────────────────────────────── */

/** Read an image file, scaled to at most 1600 px wide, as a data URL. */
function imageDataUrl(file) {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => {
      const scale = Math.min(1, 1600 / img.naturalWidth);
      const c = document.createElement('canvas');
      c.width = Math.round(img.naturalWidth * scale);
      c.height = Math.round(img.naturalHeight * scale);
      c.getContext('2d').drawImage(img, 0, 0, c.width, c.height);
      URL.revokeObjectURL(url);
      // Photos as JPEG, everything with transparency as PNG.
      const png = /png|gif|webp/i.test(file.type);
      resolve(c.toDataURL(png ? 'image/png' : 'image/jpeg', 0.86));
    };
    img.onerror = () => { URL.revokeObjectURL(url); reject(new Error('not an image')); };
    img.src = url;
  });
}

/* ── The editor ──────────────────────────────────────────────── */

/**
 * Build an editor for document `doc` ({ pdf_id, title, editor_html }).
 * Returns { toolbar, surface, exportPdf(), saveNow(), setTitle(), undo(),
 * redo(), destroy(), get dirty, get exporting }.
 */
export function createEditor(doc, { onState } = {}) {
  installDocStyle();
  const parsed = parseSource(doc.editor_html || '');
  let theme = parsed.theme;
  let pageNumbers = parsed.numbers;
  let title = doc.title || '';
  let saveTimer = null;
  let saving = false;
  let dirty = false;
  let exporting = false;
  let lastSaved = null;

  /* Surface: paper on the bare window. */
  const surface = document.createElement('div');
  surface.className = 'pdfed-surface';
  const paper = document.createElement('div');
  paper.className = 'pdfed-paper';
  paper.style.width = `${PAGE.width}mm`;
  paper.style.minHeight = `${PAGE.height}mm`;
  paper.style.padding = PAGE.margins.map((m) => `${m}mm`).join(' ');
  const body = document.createElement('div');
  body.className = 'pdfdoc pdfed-body';
  body.setAttribute('contenteditable', 'true');
  body.setAttribute('spellcheck', 'true');
  body.setAttribute('role', 'textbox');
  body.setAttribute('aria-multiline', 'true');
  body.setAttribute('aria-label', t('plugin.pdf.editor.aria', null, 'Document'));
  body.setAttribute('style', themeStyle(theme));
  body.innerHTML = sanitizeHtml(parsed.body) || '<h1><br></h1><p><br></p>';
  const guides = document.createElement('div');
  guides.className = 'pdfed-guides';
  guides.setAttribute('aria-hidden', 'true');
  paper.append(body, guides);
  surface.appendChild(paper);

  const state = () => onState?.({ dirty, saving, exporting, lastSaved, stale: doc.pdf_stale !== false });

  /* Page guides: where the printer will start a new page. */
  const mm = 96 / 25.4;
  const pageContent = (PAGE.height - PAGE.margins[0] - PAGE.margins[2]) * mm;
  function drawGuides() {
    guides.innerHTML = '';
    const top0 = body.offsetTop;
    let pageStart = 0;
    let page = 1;
    const marks = [];
    const total = body.offsetHeight;
    const breaks = [...body.querySelectorAll('.page-break')].map((el) => el.offsetTop - top0 + el.offsetHeight);
    let bi = 0;
    while (pageStart < total) {
      const nextBreak = breaks[bi];
      const natural = pageStart + pageContent;
      if (nextBreak != null && nextBreak <= natural) {
        marks.push(nextBreak);
        pageStart = nextBreak;
        bi++;
      } else if (natural < total) {
        marks.push(natural);
        pageStart = natural;
      } else break;
      page++;
      if (page > 500) break;
    }
    marks.forEach((y, i) => {
      const g = document.createElement('div');
      g.className = 'pdfed-guide';
      g.style.top = `${top0 + y}px`;
      const label = document.createElement('span');
      label.textContent = t('plugin.pdf.editor.pageN', { page: i + 2 }, `Page ${i + 2}`);
      g.appendChild(label);
      guides.appendChild(g);
    });
    // The paper grows by whole pages, like the PDF will.
    const pages = marks.length + 1;
    paper.style.minHeight = `${PAGE.height * pages}mm`;
  }
  const ro = new ResizeObserver(() => drawGuides());
  ro.observe(body);

  /* Saving. */
  function currentHtml() {
    return sanitizeHtml(body.innerHTML);
  }

  async function saveNow() {
    clearTimeout(saveTimer);
    if (!dirty || saving) return;
    saving = true;
    dirty = false;
    state();
    try {
      await apiFetch(`/api/pdfs/${enc(doc.pdf_id)}/source`, {
        method: 'PUT',
        body: JSON.stringify({ format: 'html', title: title || t('plugin.pdf.toolbar.titlePlaceholder', null, 'Untitled'), content: joinSource(theme, currentHtml(), pageNumbers) }),
      });
      doc.pdf_stale = true;
      lastSaved = new Date();
    } catch (e) {
      dirty = true;
      toast(e.message || t('plugin.pdf.toast.saveFailed', null, 'Save failed'), { type: 'error' });
    } finally {
      saving = false;
      state();
    }
  }

  function changed() {
    dirty = true;
    state();
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => void saveNow(), 1200);
  }

  body.addEventListener('input', changed);

  /* Paste: keep structure, drop foreign styling. */
  body.addEventListener('paste', (e) => {
    const html = e.clipboardData?.getData('text/html');
    const text = e.clipboardData?.getData('text/plain');
    const files = [...(e.clipboardData?.files || [])].filter((f) => f.type.startsWith('image/'));
    e.preventDefault();
    if (files.length) {
      for (const f of files) void imageDataUrl(f).then((src) => document.execCommand('insertHTML', false, `<img src="${src}" alt="">`));
      return;
    }
    if (html) document.execCommand('insertHTML', false, sanitizeHtml(html));
    else if (text) document.execCommand('insertText', false, text);
  });

  body.addEventListener('keydown', (e) => {
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key.toLowerCase() === 's') { e.preventDefault(); void saveNow(); }
    if (mod && e.key === 'Enter') { e.preventDefault(); insertPageBreak(); }
  });

  /* Commands. */
  const exec = (cmd, value = null) => {
    body.focus();
    document.execCommand('styleWithCSS', false, cmd === 'foreColor' || cmd === 'hiliteColor');
    document.execCommand(cmd, false, value);
    changed();
    syncToolbar();
  };

  function currentBlock() {
    const sel = window.getSelection();
    let n = sel?.anchorNode;
    if (!n || !body.contains(n)) return null;
    while (n && n.parentNode !== body) n = n.parentNode;
    return n?.nodeType === Node.ELEMENT_NODE ? n : null;
  }

  function setBlock(kind) {
    body.focus();
    if (kind === 'eyebrow') {
      document.execCommand('formatBlock', false, 'p');
      currentBlock()?.classList.add('eyebrow');
    } else {
      document.execCommand('formatBlock', false, kind);
      const b = currentBlock();
      b?.classList.remove('eyebrow');
      if (b && !b.className) b.removeAttribute('class');
    }
    changed();
    syncToolbar();
  }

  function insertPageBreak() {
    exec('insertHTML', '<div class="page-break" contenteditable="false"></div><p><br></p>');
  }

  function insertTable() {
    const head = `<tr><th>${t('plugin.pdf.editor.column', null, 'Column')}</th><th>${t('plugin.pdf.editor.column', null, 'Column')}</th><th>${t('plugin.pdf.editor.column', null, 'Column')}</th></tr>`;
    const row = '<tr><td><br></td><td><br></td><td><br></td></tr>';
    exec('insertHTML', `<table><thead>${head}</thead><tbody>${row}${row}</tbody></table><p><br></p>`);
  }

  function tableCell() {
    const n = window.getSelection()?.anchorNode;
    const el = n?.nodeType === Node.ELEMENT_NODE ? n : n?.parentElement;
    const cell = el?.closest?.('td, th');
    return cell && body.contains(cell) ? cell : null;
  }

  function tableEdit(op) {
    const cell = tableCell();
    if (!cell) return;
    const row = cell.parentElement;
    const table = cell.closest('table');
    const col = [...row.children].indexOf(cell);
    if (op === 'row+') {
      const r = document.createElement('tr');
      for (let i = 0; i < row.children.length; i++) r.innerHTML += '<td><br></td>';
      (row.parentElement.tagName === 'THEAD' ? table.tBodies[0] || table.createTBody() : row.parentElement).insertBefore(r, row.parentElement.tagName === 'THEAD' ? null : row.nextSibling);
    } else if (op === 'col+') {
      for (const r of table.rows) {
        const c = document.createElement(r.parentElement.tagName === 'THEAD' ? 'th' : 'td');
        c.innerHTML = '<br>';
        r.insertBefore(c, r.children[col + 1] || null);
      }
    } else if (op === 'row-') {
      if (table.rows.length > 1) row.remove(); else table.remove();
    } else if (op === 'col-') {
      if (row.children.length > 1) for (const r of table.rows) r.children[col]?.remove();
      else table.remove();
    }
    changed();
    syncToolbar();
  }

  async function insertLink() {
    const url = await promptDialog(t('plugin.pdf.dialog.linkUrl', null, 'Link address:'), 'https://');
    if (!url || url.trim() === 'https://') return;
    const u = /^(https?:|mailto:)/i.test(url.trim()) ? url.trim() : `https://${url.trim()}`;
    if (window.getSelection()?.isCollapsed) exec('insertHTML', `<a href="${u.replace(/"/g, '&quot;')}">${u.replace(/</g, '&lt;')}</a>`);
    else exec('createLink', u);
  }

  async function insertImage() {
    const sel = window.getSelection();
    const range = sel?.rangeCount ? sel.getRangeAt(0).cloneRange() : null;
    const files = await pickFiles({ accept: 'image/png,image/jpeg,image/gif,image/webp' });
    if (!files?.length) return;
    body.focus();
    if (range) { sel.removeAllRanges(); sel.addRange(range); }
    for (const f of files) {
      try {
        const src = await imageDataUrl(f);
        exec('insertHTML', `<img src="${src}" alt="">`);
      } catch (_) {
        toast(t('plugin.pdf.toast.imageFailed', null, 'Could not read the image'), { type: 'error' });
      }
    }
  }

  /* Toolbar. */
  const toolbar = document.createElement('div');
  toolbar.className = 'pdfed-toolbar';
  toolbar.setAttribute('role', 'toolbar');
  toolbar.setAttribute('aria-label', t('plugin.pdf.editor.formatting', null, 'Formatting'));
  // Toolbar clicks must not steal the selection from the page.
  toolbar.addEventListener('mousedown', (e) => {
    if (!e.target.closest('select, input')) e.preventDefault();
  });

  const btn = (iconName, label, onClick, { text } = {}) => {
    const b = controlButton();
    b.type = 'button';
    b.className = 'pdfed-btn';
    b.title = label;
    b.setAttribute('aria-label', label);
    if (iconName) b.appendChild(icon(iconName, { size: 15 }));
    else b.textContent = text;
    b.addEventListener('click', onClick);
    toolbar.appendChild(b);
    return b;
  };
  const sep = () => {
    const s = document.createElement('span');
    s.className = 'pdfed-sep';
    toolbar.appendChild(s);
  };

  const BLOCKS = [
    ['p', t('plugin.pdf.editor.paragraph', null, 'Text')],
    ['h1', t('plugin.pdf.editor.h1', null, 'Title')],
    ['h2', t('plugin.pdf.editor.h2', null, 'Heading')],
    ['h3', t('plugin.pdf.editor.h3', null, 'Subheading')],
    ['eyebrow', t('plugin.pdf.editor.eyebrow', null, 'Overline')],
    ['blockquote', t('plugin.pdf.editor.quote', null, 'Quote')],
    ['pre', t('plugin.pdf.editor.code', null, 'Code')],
  ];
  const blockSel = uiSelect({ options: BLOCKS.map(([value, label]) => ({ value, label })) });
  blockSel.classList.add('pdfed-block');
  blockSel.select.setAttribute('aria-label', t('plugin.pdf.editor.style', null, 'Paragraph style'));
  blockSel.select.addEventListener('change', () => setBlock(blockSel.select.value));
  toolbar.appendChild(blockSel);
  sep();
  const fmt = {
    bold: btn('ui/bold', t('plugin.pdf.edit.bold', null, 'Bold'), () => exec('bold')),
    italic: btn('ui/italic', t('plugin.pdf.edit.italic', null, 'Italic'), () => exec('italic')),
    underline: btn('ui/underline', t('plugin.pdf.editor.underline', null, 'Underline'), () => exec('underline')),
    strike: btn(null, t('plugin.pdf.editor.strike', null, 'Strikethrough'), () => exec('strikeThrough'), { text: 'S̶' }),
  };
  const color = document.createElement('input');
  color.type = 'color';
  color.className = 'pdfed-color';
  color.value = '#2f5bd3';
  color.title = t('plugin.pdf.edit.textColour', null, 'Text colour');
  color.addEventListener('change', () => exec('foreColor', color.value));
  toolbar.appendChild(color);
  btn('ui/brush', t('plugin.pdf.editor.mark', null, 'Highlight'), () => exec('hiliteColor', '#ffe58a'));
  sep();
  btn(null, t('plugin.pdf.editor.alignLeft', null, 'Align left'), () => exec('justifyLeft'), { text: '⟸' });
  btn(null, t('plugin.pdf.editor.alignCenter', null, 'Centre'), () => exec('justifyCenter'), { text: '⇔' });
  btn(null, t('plugin.pdf.editor.alignRight', null, 'Align right'), () => exec('justifyRight'), { text: '⟹' });
  sep();
  btn('ui/list', t('plugin.pdf.editor.bullets', null, 'Bulleted list'), () => exec('insertUnorderedList'));
  btn(null, t('plugin.pdf.editor.numbers', null, 'Numbered list'), () => exec('insertOrderedList'), { text: '1.' });
  sep();
  btn('ui/link', t('plugin.pdf.editor.link', null, 'Link'), () => void insertLink());
  btn('ui/image', t('plugin.pdf.editor.image', null, 'Image'), () => void insertImage());
  btn('ui/grid', t('plugin.pdf.editor.table', null, 'Table'), insertTable);
  btn(null, t('plugin.pdf.editor.rule', null, 'Line'), () => exec('insertHorizontalRule'), { text: '—' });
  btn('ui/doc', t('plugin.pdf.editor.pageBreak', null, 'Page break (Ctrl+Enter)'), insertPageBreak);
  const tableTools = document.createElement('span');
  tableTools.className = 'pdfed-table-tools';
  tableTools.hidden = true;
  for (const [op, label, txt] of [
    ['row+', t('plugin.pdf.editor.rowAdd', null, 'Add row'), '+▭'],
    ['col+', t('plugin.pdf.editor.colAdd', null, 'Add column'), '+▯'],
    ['row-', t('plugin.pdf.editor.rowDel', null, 'Delete row'), '−▭'],
    ['col-', t('plugin.pdf.editor.colDel', null, 'Delete column'), '−▯'],
  ]) {
    const b = btn(null, label, () => tableEdit(op), { text: txt });
    tableTools.appendChild(b);
  }
  toolbar.appendChild(tableTools);
  btn(null, t('plugin.pdf.editor.clear', null, 'Clear formatting'), () => { exec('removeFormat'); }, { text: 'T̸' });

  const spacer = document.createElement('span');
  spacer.className = 'pdfed-spacer';
  toolbar.appendChild(spacer);
  const numbersBtn = btn(null, t('plugin.pdf.editor.pageNumbers', null, 'Page numbers on the PDF'), () => {
    pageNumbers = !pageNumbers;
    numbersBtn.classList.toggle('is-on', pageNumbers);
    numbersBtn.setAttribute('aria-pressed', String(pageNumbers));
    changed();
  }, { text: '1/n' });
  numbersBtn.classList.toggle('is-on', pageNumbers);
  numbersBtn.setAttribute('aria-pressed', String(pageNumbers));
  const themeSel = uiSelect({ options: Object.entries(THEMES).map(([value, th]) => ({ value, label: t(`plugin.pdf.theme.${value}`, null, th.label) })) });
  themeSel.classList.add('pdfed-theme');
  themeSel.select.value = theme;
  themeSel.select.setAttribute('aria-label', t('plugin.pdf.editor.theme', null, 'Look'));
  themeSel.title = t('plugin.pdf.editor.theme', null, 'Look');
  themeSel.select.addEventListener('change', () => {
    theme = themeSel.select.value;
    body.setAttribute('style', themeStyle(theme));
    changed();
  });
  toolbar.appendChild(themeSel);

  function syncToolbar() {
    const b = currentBlock();
    let kind = b ? b.tagName.toLowerCase() : 'p';
    if (b?.classList.contains('eyebrow')) kind = 'eyebrow';
    if (!BLOCKS.some(([v]) => v === kind)) kind = 'p';
    blockSel.select.value = kind;
    for (const [k, cmd] of [['bold', 'bold'], ['italic', 'italic'], ['underline', 'underline'], ['strike', 'strikeThrough']]) {
      let on = false;
      try { on = document.queryCommandState(cmd); } catch (_) { /* unsupported */ }
      fmt[k].classList.toggle('is-on', on);
    }
    tableTools.hidden = !tableCell();
  }
  const onSel = () => { if (body.contains(window.getSelection()?.anchorNode)) syncToolbar(); };
  document.addEventListener('selectionchange', onSel);

  /* Export. */
  async function exportPdf() {
    if (exporting) return null;
    exporting = true;
    state();
    try {
      await saveNow();
      const html = printHtml(currentHtml(), { theme, title });
      // Without the shell's print engine the server lays the document out
      // itself: always a PDF, though the layout is its own, not the editor's.
      const fallback = async () => {
        const res = await apiFetch(`/api/pdfs/${enc(doc.pdf_id)}/render-html`, { method: 'POST', body: '{}' });
        doc.pdf_stale = false;
        toast(t('plugin.pdf.toast.approxExport', null, 'PDF made without the shell print engine: the layout may differ a little'), { type: 'info' });
        return res?.data;
      };
      if (!canShellPrint()) return await fallback();
      let bytes;
      try {
        bytes = await shellPrint(html);
      } catch (e) {
        if (e.code === 'no-shell-print') return await fallback();
        throw e;
      }
      const form = new FormData();
      form.append('file', new Blob([bytes], { type: 'application/pdf' }), `${title || 'document'}.pdf`);
      const res = await apiFetch(`/api/pdfs/${enc(doc.pdf_id)}/replace-file`, { method: 'POST', body: form });
      doc.pdf_stale = false;
      return res?.data;
    } finally {
      exporting = false;
      state();
    }
  }

  setTimeout(drawGuides, 0);
  state();

  return {
    toolbar,
    surface,
    exportPdf,
    saveNow,
    focus: () => body.focus(),
    setTitle(v) { if (v !== title) { title = v; changed(); } },
    undo: () => exec('undo'),
    redo: () => exec('redo'),
    get dirty() { return dirty || saving; },
    get pageNumbers() { return pageNumbers; },
    get exporting() { return exporting; },
    destroy() {
      clearTimeout(saveTimer);
      if (dirty) void saveNow();
      ro.disconnect();
      document.removeEventListener('selectionchange', onSel);
      toolbar.remove();
      surface.remove();
    },
  };
}

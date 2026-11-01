/**
 * page.js — the Word window's page model: paper, margins, theme typography.
 *
 * The values mirror `src/engine.rs` (`Theme`, `compose`) so the page you
 * edit is set like the exported .docx/PDF: same faces (the fonts WordCraft
 * bundles, served by the plugin at /api/word-fonts/…, same origin), sizes in
 * points, colours, spacing and margins in millimetres. Line height is the
 * face's own line gap (hhea, 1.326 for Source Sans 3, 1.371 for Source
 * Serif 4) times the theme's line spacing, which is how the layout engine
 * measures a line. The exact pages — cover, contents, header, footer — are
 * the server's preview (Preview view); this is the live approximation you
 * type into.
 */

export const MM = 96 / 25.4;           // CSS px per millimetre at 100 %
export const PT = 96 / 72;             // CSS px per point at 100 %
export const DEFAULT_MARGIN = 25;      // mm (engine: DEFAULT_MARGIN_MM)
export const HEADER_FROM_EDGE = 12.7;  // mm (engine: 36 pt)

export const PAPERS = {
  a4: { w: 210, h: 297, label: 'A4' },
  letter: { w: 215.9, h: 279.4, label: 'Letter' },
};

const SANS = "'Source Sans 3'";
const SERIF = "'Source Serif 4'";
const LINE = { sans: 1.326, serif: 1.371 };

/** Engine themes (`theme()` in engine.rs) plus the built-in "plain" styles. */
export const THEMES = {
  plain: {
    body: `'Aptos', ${SANS}, system-ui, sans-serif`, head: `'Aptos Display', ${SANS}, system-ui, sans-serif`,
    face: 'sans', size: 12, lineSpacing: 1.15, after: 8,
    heads: [20, 16, 14], headWeight: 400, headBefore: [18, 8, 8], headAfter: [4, 4, 4],
    title: 28, titleWeight: 400, titleAlign: 'left',
    accent: '#0f4761', ink: '#000000', muted: '#595959', h3Accent: true,
    rule: false, quote: 'plain', subtitle: 14,
  },
  classic: {
    body: `${SERIF}, Georgia, serif`, head: `${SERIF}, Georgia, serif`,
    face: 'serif', size: 11, lineSpacing: 1.2, after: 6.6,
    heads: [18, 14, 12], headWeight: 700, headBefore: [20, 14, 14], headAfter: [8, 4, 4],
    title: 30, titleWeight: 700, titleAlign: 'center',
    accent: '#1f3864', ink: '#1a1a1a', muted: '#595959',
    rule: true, quote: 'bar', subtitleItalic: true,
  },
  modern: {
    body: `${SANS}, system-ui, sans-serif`, head: `${SANS}, system-ui, sans-serif`,
    face: 'sans', size: 11, lineSpacing: 1.25, after: 6.6,
    heads: [20, 15, 12.5], headWeight: 700, headBefore: [20, 14, 14], headAfter: [8, 4, 4],
    title: 34, titleWeight: 700, titleAlign: 'left',
    accent: '#1d5fbf', ink: '#1f2328', muted: '#5b636e',
    rule: false, quote: 'bar',
  },
  minimal: {
    body: `${SANS}, system-ui, sans-serif`, head: `${SANS}, system-ui, sans-serif`,
    face: 'sans', size: 10.5, lineSpacing: 1.3, after: 6.3,
    heads: [16, 13, 11], headWeight: 700, headBefore: [20, 14, 14], headAfter: [8, 4, 4],
    title: 26, titleWeight: 700, titleAlign: 'left',
    accent: '#111111', ink: '#222222', muted: '#6b7280',
    rule: false, quote: 'bar',
  },
};

export const TEMPLATE_IDS = ['plain', 'classic', 'modern', 'minimal'];

/** Accent presets offered by the Design dialog ('' = the theme's own). */
export const ACCENTS = ['', '#1d5fbf', '#1f3864', '#0f766e', '#2f7d32', '#8b1e3f', '#b45309', '#374151'];

/** `c` mixed with white, `t` = share of `c` (engine `tint`). */
export function tint(hex, t) {
  const n = parseInt(String(hex).replace('#', ''), 16);
  if (!Number.isFinite(n)) return hex;
  const mix = (v) => Math.round(255 - (255 - v) * t);
  const r = mix((n >> 16) & 255), g = mix((n >> 8) & 255), b = mix(n & 255);
  return `rgb(${r}, ${g}, ${b})`;
}

export function themeOf(design) {
  return THEMES[(design?.template || '').toLowerCase()] || THEMES.plain;
}

export function isThemed(design) {
  return !!THEMES[(design?.template || '').toLowerCase()] && (design.template || '').toLowerCase() !== 'plain';
}

/** Paper size and margins in millimetres. */
export function geometry(design = {}) {
  const paper = PAPERS[(design.paper || 'a4').toLowerCase()] || PAPERS.a4;
  const m = (v) => (Number(v) > 0 ? Number(v) : DEFAULT_MARGIN);
  return {
    w: paper.w,
    h: paper.h,
    paper: paper.label,
    top: m(design.margin_top),
    right: m(design.margin_right),
    bottom: m(design.margin_bottom),
    left: m(design.margin_left),
  };
}

/** CSS custom properties for `.word-sheets` (see `.word-sheet` in tiles.css). */
export function themeVars(design = {}) {
  const th = themeOf(design);
  const accent = /^#[0-9a-f]{6}$/i.test(design.accent || '') ? design.accent : th.accent;
  const lh = LINE[th.face] * th.lineSpacing;
  const g = geometry(design);
  return {
    '--doc-body': th.body,
    '--doc-head': th.head,
    '--doc-size': `${th.size}pt`,
    '--doc-lh': String(lh),
    '--doc-after': `${th.after}pt`,
    '--doc-ink': th.ink,
    '--doc-muted': th.muted,
    '--doc-accent': accent,
    '--doc-h1': `${th.heads[0]}pt`,
    '--doc-h2': `${th.heads[1]}pt`,
    '--doc-h3': `${th.heads[2]}pt`,
    '--doc-h1-before': `${th.headBefore[0]}pt`,
    '--doc-h2-before': `${th.headBefore[1]}pt`,
    '--doc-h3-before': `${th.headBefore[2]}pt`,
    '--doc-h1-after': `${th.headAfter[0]}pt`,
    '--doc-h2-after': `${th.headAfter[1]}pt`,
    '--doc-h3-after': `${th.headAfter[2]}pt`,
    '--doc-head-weight': String(th.headWeight),
    '--doc-h3-color': th.h3Accent ? accent : th.ink,
    '--doc-h1-rule': th.rule ? `0.75pt solid ${tint(accent, 0.35)}` : '0',
    '--doc-title': `${th.title}pt`,
    '--doc-title-weight': String(th.titleWeight),
    '--doc-title-align': th.titleAlign,
    '--doc-subtitle': `${th.subtitle || th.size + 3}pt`,
    '--doc-title-color': isThemed(design) ? accent : th.ink,
    '--doc-subtitle-style': th.subtitleItalic ? 'italic' : 'normal',
    '--doc-quote-bar': th.quote === 'bar' ? `2.25pt solid ${accent}` : '0',
    '--doc-quote-style': th.quote === 'bar' ? 'normal' : 'italic',
    '--doc-table-head': isThemed(design) ? accent : 'transparent',
    '--doc-table-head-ink': isThemed(design) ? '#ffffff' : th.ink,
    '--doc-table-band': isThemed(design) ? tint(accent, 0.08) : 'transparent',
    '--doc-table-line': isThemed(design) ? tint(accent, 0.3) : '#000000',
    '--doc-rule-soft': tint(accent, 0.5),
    '--page-w': `${g.w}mm`,
    '--page-h': `${g.h}mm`,
    '--m-top': `${g.top}mm`,
    '--m-right': `${g.right}mm`,
    '--m-bottom': `${g.bottom}mm`,
    '--m-left': `${g.left}mm`,
    '--hf-edge': `${HEADER_FROM_EDGE}mm`,
  };
}

export function applyVars(el, vars) {
  for (const [k, v] of Object.entries(vars)) el.style.setProperty(k, v);
}

/** Contents heading, localised like the engine's `contents_label`. */
const CONTENTS = {
  bg: 'Съдържание', cs: 'Obsah', sk: 'Obsah', da: 'Indhold', de: 'Inhalt', el: 'Περιεχόμενα',
  es: 'Índice', pt: 'Índice', it: 'Indice', et: 'Sisukord', fi: 'Sisällys', fr: 'Sommaire',
  ga: 'Clár ábhair', hr: 'Sadržaj', hu: 'Tartalom', lt: 'Turinys', lv: 'Saturs', mt: 'Werrej',
  nl: 'Inhoud', pl: 'Spis treści', ro: 'Cuprins', ru: 'Содержание', sl: 'Kazalo', sv: 'Innehåll',
  uk: 'Зміст',
};
export function contentsLabel(lang) {
  return CONTENTS[String(lang || '').split(/[-_]/)[0].toLowerCase()] || 'Contents';
}

/* ── Fonts ──────────────────────────────────────────────────── */

const FONT_FILES = [
  ['SourceSans3-Regular.ttf', 'Source Sans 3', 400, 'normal'],
  ['SourceSans3-Semibold.ttf', 'Source Sans 3', 600, 'normal'],
  ['SourceSans3-Bold.ttf', 'Source Sans 3', 700, 'normal'],
  ['SourceSans3-It.ttf', 'Source Sans 3', 400, 'italic'],
  ['SourceSerif4-Regular.ttf', 'Source Serif 4', 400, 'normal'],
  ['SourceSerif4-It.ttf', 'Source Serif 4', 400, 'italic'],
  ['SourceSerif4-Semibold.ttf', 'Source Serif 4', 600, 'normal'],
  ['SourceSerif4-Bold.ttf', 'Source Serif 4', 700, 'normal'],
  ['SourceSerif4-BoldIt.ttf', 'Source Serif 4', 700, 'italic'],
];

let fontsInstalled = false;
/**
 * Declare the document faces (the ones the PDF embeds). Same-origin plugin
 * route, loaded only when a page uses them — no remote font requests.
 */
export function installDocumentFonts() {
  if (fontsInstalled) return;
  fontsInstalled = true;
  const style = document.createElement('style');
  style.dataset.owner = 'word-page';
  style.textContent = FONT_FILES.map(([file, family, weight, styleName]) =>
    `@font-face{font-family:'${family}';src:url('/api/word-fonts/${file}') format('truetype');font-weight:${weight};font-style:${styleName};font-display:swap}`,
  ).join('\n');
  document.head.appendChild(style);
}

/** Families a document can use without host fonts. */
export const DOCUMENT_FAMILIES = ['Source Sans 3', 'Source Serif 4'];

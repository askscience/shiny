/**
 * page.js — the Word window's page: paper and margins, the theme's look.
 *
 * The look — faces, sizes, spacing, colours — is computed by the plugin from
 * the very styles the export uses (`page_vars` in src/engine.rs) and arrives
 * with the document as `page.vars`; the window only adds the paper and the
 * margins, which it moves live while a ruler is dragged. The exact pages —
 * cover, contents, header, footer — are the server's preview (Preview view);
 * this is the live page you type into.
 */

export const MM = 96 / 25.4;           // CSS px per millimetre at 100 %
export const PT = 96 / 72;             // CSS px per point at 100 %
export const DEFAULT_MARGIN = 25;      // mm (engine: DEFAULT_MARGIN_MM)
export const HEADER_FROM_EDGE = 12.7;  // mm (engine: 36 pt)

export const PAPERS = {
  a4: { w: 210, h: 297, label: 'A4' },
  letter: { w: 215.9, h: 279.4, label: 'Letter' },
};

export const TEMPLATE_IDS = ['plain', 'classic', 'modern', 'minimal'];

/** Accent presets offered by the Design dialog ('' = the theme's own). */
export const ACCENTS = ['', '#1d5fbf', '#1f3864', '#0f766e', '#2f7d32', '#8b1e3f', '#b45309', '#374151'];

/**
 * A margin as the Design dialog saves it: 0 means "the default" (25 mm) and is
 * what an untouched field holds, so it stays 0, as does an explicit 25;
 * anything else is kept within the engine's 5–80 mm.
 */
export function savedMargin(value) {
  const v = Number(value) || 0;
  return v === 0 || v === DEFAULT_MARGIN ? 0 : Math.min(80, Math.max(5, v));
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

/**
 * CSS custom properties for `.word-sheets` (see `.word-sheet` in tiles.css):
 * the plugin's `page.vars` for the look, plus paper and margins from the
 * design as the window holds it.
 */
export function pageVars(page, design = {}) {
  const g = geometry(design);
  return {
    ...(page?.vars || {}),
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

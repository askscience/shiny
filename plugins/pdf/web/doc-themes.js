/**
 * Document themes, templates and the print HTML for the PDF window's editor.
 *
 * One stylesheet serves both the editor and the printed PDF: the editor shows
 * the document inside `.pdfdoc` with exactly these rules, and export wraps the
 * same body in a standalone page with the same rules, so what you write is
 * what the PDF shows. Sizes are in points and millimetres, never in screen
 * pixels, for the same reason.
 *
 * The fonts (Inter, Inter Display, Instrument Serif, P052) are installed
 * system fonts; Chromium embeds the subsets it uses into the PDF.
 */

/** Page setup shared by the editor and the printer (millimetres). */
export const PAGE = {
  size: 'A4',
  width: 210,
  height: 297,
  margins: [18, 20, 20, 20], // top, right, bottom, left
};

export const THEMES = {
  modern: {
    label: 'Modern',
    vars: {
      '--doc-body': "'Inter', 'Helvetica Neue', Arial, sans-serif",
      '--doc-head': "'Inter Display', 'Inter', sans-serif",
      '--doc-head-weight': '650',
      '--doc-h1': '26pt',
      '--doc-h1-spacing': '-0.02em',
      '--doc-size': '10.5pt',
      '--doc-ink': '#16191d',
      '--doc-muted': '#5b6470',
      '--doc-accent': '#2f5bd3',
      '--doc-rule': '#d9dde3',
      '--doc-fill': '#f3f5f8',
    },
  },
  editorial: {
    label: 'Editorial',
    vars: {
      '--doc-body': "'Inter', 'Helvetica Neue', Arial, sans-serif",
      '--doc-head': "'Instrument Serif', 'P052', Georgia, serif",
      '--doc-head-weight': '400',
      '--doc-h1': '36pt',
      '--doc-h1-spacing': '-0.01em',
      '--doc-size': '10.5pt',
      '--doc-ink': '#1b1a17',
      '--doc-muted': '#6a645a',
      '--doc-accent': '#a2552a',
      '--doc-rule': '#e2dccf',
      '--doc-fill': '#f6f1e7',
    },
  },
  classic: {
    label: 'Classic',
    vars: {
      '--doc-body': "'P052', 'Palatino Linotype', 'Book Antiqua', Georgia, serif",
      '--doc-head': "'P052', 'Palatino Linotype', Georgia, serif",
      '--doc-head-weight': '700',
      '--doc-h1': '24pt',
      '--doc-h1-spacing': '0',
      '--doc-size': '11pt',
      '--doc-ink': '#141414',
      '--doc-muted': '#555555',
      '--doc-accent': '#1d3b6a',
      '--doc-rule': '#cfcfcf',
      '--doc-fill': '#f2f2f2',
    },
  },
};

/** The document's own typography: used verbatim in the editor and in print. */
export const DOC_CSS = `
.pdfdoc {
  font-family: var(--doc-body);
  font-size: var(--doc-size);
  line-height: 1.55;
  color: var(--doc-ink);
  -webkit-print-color-adjust: exact;
  print-color-adjust: exact;
  font-kerning: normal;
  font-variant-ligatures: common-ligatures;
  overflow-wrap: break-word;
}
.pdfdoc > :first-child { margin-top: 0; }
.pdfdoc p { margin: 0 0 7pt; orphans: 2; widows: 2; }
.pdfdoc h1, .pdfdoc h2, .pdfdoc h3 {
  font-family: var(--doc-head);
  font-weight: var(--doc-head-weight);
  color: var(--doc-ink);
  line-height: 1.15;
  break-after: avoid;
  page-break-after: avoid;
}
.pdfdoc h1 { font-size: var(--doc-h1); letter-spacing: var(--doc-h1-spacing); margin: 0 0 10pt; }
.pdfdoc h2 { font-size: 15pt; margin: 16pt 0 6pt; }
.pdfdoc h3 { font-size: 12pt; margin: 12pt 0 4pt; }
.pdfdoc .eyebrow {
  font-family: var(--doc-body);
  font-size: 7.5pt;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: var(--doc-accent);
  margin: 0 0 4pt;
}
.pdfdoc a { color: var(--doc-accent); text-decoration: underline; text-underline-offset: 1.5pt; }
.pdfdoc ul, .pdfdoc ol { margin: 0 0 8pt; padding-left: 16pt; }
.pdfdoc li { margin: 0 0 2pt; }
.pdfdoc li::marker { color: var(--doc-muted); }
.pdfdoc blockquote {
  margin: 10pt 0;
  padding: 2pt 0 2pt 11pt;
  border-left: 2.5pt solid var(--doc-accent);
  color: var(--doc-muted);
  font-style: italic;
}
.pdfdoc pre, .pdfdoc code { font-family: 'JetBrains Mono', 'DejaVu Sans Mono', monospace; font-size: 0.86em; }
.pdfdoc pre { background: var(--doc-fill); padding: 7pt 9pt; border-radius: 3pt; white-space: pre-wrap; margin: 0 0 8pt; }
.pdfdoc hr { border: 0; border-top: 0.75pt solid var(--doc-rule); margin: 12pt 0; }
.pdfdoc table { width: 100%; border-collapse: collapse; margin: 6pt 0 10pt; break-inside: auto; }
.pdfdoc tr { break-inside: avoid; page-break-inside: avoid; }
.pdfdoc thead { display: table-header-group; }
.pdfdoc th {
  text-align: left;
  font-weight: 600;
  border-bottom: 1.2pt solid var(--doc-ink);
  padding: 4pt 6pt 4pt 0;
}
.pdfdoc td { border-bottom: 0.5pt solid var(--doc-rule); padding: 4pt 6pt 4pt 0; vertical-align: top; }
.pdfdoc img { max-width: 100%; height: auto; border-radius: 2pt; }
.pdfdoc mark { background: #ffe58a; color: inherit; padding: 0 1pt; }
.pdfdoc .page-break { break-after: page; page-break-after: always; height: 0; }
.pdfdoc [style*="text-align: center"] { text-align: center; }
`;

/** CSS custom properties for a theme, as a `style` attribute value. */
export function themeStyle(theme) {
  const t = THEMES[theme] || THEMES.modern;
  return Object.entries(t.vars).map(([k, v]) => `${k}: ${v}`).join('; ');
}

/** The standalone page the shell prints: same rules, same body. */
export function printHtml(bodyHtml, { theme = 'modern', title = '' } = {}) {
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  return `<!doctype html>
<html><head><meta charset="utf-8"><title>${esc(title)}</title>
<style>
@page { size: ${PAGE.size}; margin: 0; }
html, body { margin: 0; padding: 0; background: #fff; }
${DOC_CSS}
</style></head>
<body><div class="pdfdoc" style="${themeStyle(theme)}">${bodyHtml}</div></body></html>`;
}

/** Templates: a starting document per kind (HTML in the theme's vocabulary). */
export function templateHtml(id, t) {
  const today = new Date().toLocaleDateString(undefined, { day: 'numeric', month: 'long', year: 'numeric' });
  const tr = (key, fallback) => t(`plugin.pdf.tpl.${key}`, null, fallback);
  switch (id) {
    case 'letter':
      return `<p class="eyebrow">${tr('letterEyebrow', 'Letter')}</p>
<h1>${tr('letterTitle', 'Subject of the letter')}</h1>
<p>${today}</p>
<p>${tr('letterGreeting', 'Dear …,')}</p>
<p>${tr('letterBody', 'Write the letter here.')}</p>
<p>${tr('letterClosing', 'Kind regards,')}<br>${tr('letterName', 'Name Surname')}</p>`;
    case 'invoice':
      return `<p class="eyebrow">${tr('invoiceEyebrow', 'Invoice')}</p>
<h1>${tr('invoiceTitle', 'Invoice no. 1')}</h1>
<p><strong>${tr('invoiceFrom', 'From')}:</strong> ${tr('invoiceFromValue', 'Your name, address')}<br>
<strong>${tr('invoiceTo', 'To')}:</strong> ${tr('invoiceToValue', 'Customer, address')}<br>
<strong>${tr('invoiceDate', 'Date')}:</strong> ${today}</p>
<table><thead><tr><th>${tr('invoiceItem', 'Item')}</th><th>${tr('invoiceQty', 'Qty')}</th><th>${tr('invoicePrice', 'Price')}</th></tr></thead>
<tbody><tr><td>${tr('invoiceService', 'Service')}</td><td>1</td><td>100,00 €</td></tr>
<tr><td><strong>${tr('invoiceTotal', 'Total')}</strong></td><td></td><td><strong>100,00 €</strong></td></tr></tbody></table>
<p>${tr('invoiceTerms', 'Payment within 30 days.')}</p>`;
    case 'report':
      return `<p class="eyebrow">${tr('reportEyebrow', 'Report')}</p>
<h1>${tr('reportTitle', 'Report title')}</h1>
<p>${tr('reportLead', 'Two or three sentences on what this is about.')}</p>
<h2>${tr('reportDetails', 'Details')}</h2>
<ul><li>${tr('reportPoint1', 'First point')}</li><li>${tr('reportPoint2', 'Second point')}</li></ul>
<h2>${tr('reportConclusions', 'Conclusions')}</h2>
<p>${tr('reportEnd', 'What follows from this.')}</p>`;
    case 'notes':
      return `<h1>${tr('notesTitle', 'Notes')}</h1><p>${today}</p><ul><li><br></li></ul>`;
    default:
      return '<h1><br></h1><p><br></p>';
  }
}

export const TEMPLATES = ['blank', 'letter', 'invoice', 'report', 'notes'];

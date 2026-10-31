/**
 * Keep a document's HTML to the vocabulary the editor and the printer
 * understand: structure, emphasis, lists, tables, links and images — no
 * scripts, no event handlers, no foreign styling. Used on paste, on load and
 * before every save.
 */

const ALLOWED = new Set([
  'P', 'BR', 'H1', 'H2', 'H3', 'STRONG', 'B', 'EM', 'I', 'U', 'S', 'STRIKE', 'DEL', 'SUB', 'SUP', 'MARK', 'SPAN',
  'UL', 'OL', 'LI', 'BLOCKQUOTE', 'PRE', 'CODE', 'HR', 'A', 'IMG', 'TABLE', 'THEAD', 'TBODY', 'TR', 'TH', 'TD', 'DIV', 'FONT',
]);
const DROP_WITH_CONTENT = new Set(['SCRIPT', 'STYLE', 'IFRAME', 'OBJECT', 'EMBED', 'NOSCRIPT', 'TEMPLATE', 'SVG', 'MATH', 'HEAD', 'TITLE', 'META', 'LINK']);
const RENAME = { B: 'STRONG', I: 'EM', STRIKE: 'S', DEL: 'S', H4: 'H3', H5: 'H3', H6: 'H3' };

/** Inline styles worth keeping (what the toolbar itself produces). */
const STYLE_KEEP = ['color', 'background-color', 'text-align'];

function safeUrl(url, { image = false } = {}) {
  const u = String(url || '').trim();
  if (image && /^data:image\/(png|jpe?g|gif|webp);base64,/i.test(u)) return u;
  if (/^(https?:|mailto:)/i.test(u)) return u;
  return null;
}

function cleanStyle(style) {
  const out = [];
  for (const decl of String(style || '').split(';')) {
    const [prop, ...rest] = decl.split(':');
    const name = (prop || '').trim().toLowerCase();
    const value = rest.join(':').trim();
    if (!STYLE_KEEP.includes(name) || !value) continue;
    if (/url\(|expression|javascript:/i.test(value)) continue;
    if (name === 'text-align' && !/^(left|center|right|justify)$/.test(value)) continue;
    out.push(`${name}: ${value}`);
  }
  return out.join('; ');
}

function cleanNode(node, doc) {
  for (let child of [...node.childNodes]) {
    if (child.nodeType === Node.COMMENT_NODE) { child.remove(); continue; }
    if (child.nodeType !== Node.ELEMENT_NODE) continue;
    let tag = child.tagName.toUpperCase();
    if (DROP_WITH_CONTENT.has(tag)) { child.remove(); continue; }
    if (RENAME[tag]) {
      const repl = doc.createElement(RENAME[tag]);
      while (child.firstChild) repl.appendChild(child.firstChild);
      for (const a of [...child.attributes]) repl.setAttribute(a.name, a.value);
      child.replaceWith(repl);
      child = repl;
      tag = RENAME[tag];
    }
    if (!ALLOWED.has(tag)) {
      // Unknown wrapper: keep its content, drop the element.
      cleanNode(child, doc);
      child.replaceWith(...child.childNodes);
      continue;
    }
    if (tag === 'FONT') {
      const span = doc.createElement('span');
      const color = child.getAttribute('color');
      if (color) span.style.color = color;
      while (child.firstChild) span.appendChild(child.firstChild);
      child.replaceWith(span);
      cleanNode(span, doc);
      continue;
    }
    for (const attr of [...child.attributes]) {
      const name = attr.name.toLowerCase();
      const keep =
        (name === 'href' && tag === 'A') ||
        (name === 'src' && tag === 'IMG') ||
        (name === 'alt' && tag === 'IMG') ||
        ((name === 'colspan' || name === 'rowspan') && (tag === 'TD' || tag === 'TH')) ||
        name === 'style' ||
        (name === 'class' && /^(eyebrow|page-break)$/.test(attr.value)) ||
        (name === 'contenteditable' && child.classList.contains('page-break'));
      if (!keep) { child.removeAttribute(attr.name); continue; }
      if (name === 'href') {
        const u = safeUrl(attr.value);
        if (u) child.setAttribute('href', u); else child.removeAttribute('href');
      } else if (name === 'src') {
        const u = safeUrl(attr.value, { image: true });
        if (u) child.setAttribute('src', u); else child.remove();
      } else if (name === 'style') {
        const s = cleanStyle(attr.value);
        if (s) child.setAttribute('style', s); else child.removeAttribute('style');
      }
    }
    if (tag === 'A') {
      child.setAttribute('rel', 'noopener');
    }
    cleanNode(child, doc);
  }
}

/** Sanitize an HTML fragment string; returns safe HTML. */
export function sanitizeHtml(html) {
  const doc = new DOMParser().parseFromString(`<body>${html || ''}</body>`, 'text/html');
  cleanNode(doc.body, doc);
  return doc.body.innerHTML;
}

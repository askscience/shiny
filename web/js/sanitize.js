// Allowlist HTML sanitizer for rendered markdown.
//
// Assistant replies are attacker-influenceable: the agent reads web pages,
// search results and documents, so a prompt-injected model can emit raw HTML.
// `marked` passes raw HTML through by design, and its output is assigned to
// innerHTML — so every rendered fragment goes through here first.
//
// This is a DOM walk with a tag/attribute allowlist, not a regex: the browser
// parser resolves malformed-markup tricks, and anything not explicitly allowed
// is dropped. `<script>`, event handlers, `javascript:` URLs and `<style>` are
// therefore inert even when the model repeats them verbatim.

const ALLOWED_TAGS = new Set([
  'p', 'br', 'hr', 'strong', 'b', 'em', 'i', 'u', 's', 'del', 'ins', 'mark',
  'code', 'pre', 'blockquote', 'ul', 'ol', 'li', 'h1', 'h2', 'h3', 'h4', 'h5',
  'h6', 'a', 'img', 'table', 'thead', 'tbody', 'tfoot', 'tr', 'th', 'td',
  'sub', 'sup', 'kbd', 'dl', 'dt', 'dd', 'figure', 'figcaption',
]);

const ALLOWED_ATTRS = {
  a: ['href', 'title'],
  img: ['src', 'alt', 'title', 'width', 'height'],
  th: ['colspan', 'rowspan'],
  td: ['colspan', 'rowspan'],
  ol: ['start'],
  code: ['class'],
  pre: ['class'],
};

// Content of these is never shown: dropped whole rather than converted to text.
const DROP_TAGS = new Set([
  'script', 'style', 'template', 'iframe', 'object', 'embed', 'svg', 'math',
  'form', 'input', 'button', 'textarea', 'select', 'option', 'link', 'meta',
  'base', 'title',
]);

// Only same-origin/relative links, http(s), and mailto. Everything else
// (`javascript:`, `data:`, `vbscript:`, …) is stripped.
const SAFE_URL = /^(https?:|mailto:|#|\/)/i;

export function sanitizeHtml(html) {
  const doc = new DOMParser().parseFromString(String(html ?? ''), 'text/html');
  sanitizeChildren(doc.body);
  return doc.body.innerHTML;
}

function sanitizeChildren(node) {
  // Snapshot: replacing a node mutates the live child list.
  for (const child of Array.from(node.children)) {
    const tag = child.tagName.toLowerCase();
    if (DROP_TAGS.has(tag)) {
      child.remove();
      continue;
    }
    const allowedAttrs = ALLOWED_ATTRS[tag];
    if (!ALLOWED_TAGS.has(tag)) {
      // Keep the element's text, drop the element and its markup entirely.
      child.replaceWith(child.ownerDocument.createTextNode(child.textContent || ''));
      continue;
    }
    for (const attr of Array.from(child.attributes)) {
      const name = attr.name.toLowerCase();
      if (!allowedAttrs || !allowedAttrs.includes(name)) {
        child.removeAttribute(attr.name);
        continue;
      }
      if ((name === 'href' || name === 'src') && !SAFE_URL.test(attr.value.trim())) {
        child.removeAttribute(attr.name);
      }
    }
    if (tag === 'img' && !child.hasAttribute('src')) {
      child.remove();
      continue;
    }
    if (tag === 'a') {
      // Never navigate the privileged shell frame away from the app.
      child.setAttribute('rel', 'noopener noreferrer');
      child.setAttribute('target', '_blank');
    }
    sanitizeChildren(child);
  }
}

export default sanitizeHtml;

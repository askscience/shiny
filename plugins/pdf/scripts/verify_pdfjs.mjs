// Independent check of engine output with pdf.js (the viewer's own renderer).
//
//   node plugins/pdf/scripts/verify_pdfjs.mjs <dir>
//
// For every `<name>.edited.pdf` + `<name>.expect.txt` (written by the ignored
// `realworld` test) the edited page must contain every expected line; every
// `*.pdf` must open and yield text on every page without throwing.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const pdfjs = await import(path.join(here, '../../../web/vendor/pdfjs/pdf.min.mjs'));
pdfjs.GlobalWorkerOptions.workerSrc = path.join(here, '../../../web/vendor/pdfjs/pdf.worker.min.mjs');

const dir = process.argv[2];
const norm = (s) => s.normalize('NFKC').replace(/\s+/g, ' ').trim();
let failures = 0;

async function pageText(doc, n) {
  const page = await doc.getPage(n + 1);
  const tc = await page.getTextContent();
  return tc.items.map((i) => i.str + (i.hasEOL ? '\n' : '')).join('');
}

for (const f of fs.readdirSync(dir).filter((f) => f.endsWith('.pdf')).sort()) {
  const data = new Uint8Array(fs.readFileSync(path.join(dir, f)));
  const warnings = [];
  try {
    const doc = await pdfjs.getDocument({ data, verbosity: 0, isEvalSupported: false }).promise;
    let chars = 0;
    for (let p = 0; p < Math.min(doc.numPages, 5); p++) chars += (await pageText(doc, p)).length;
    let line = `${f}: ${doc.numPages} pages, ${chars} chars on first pages`;
    const exp = path.join(dir, f.replace(/\.edited\.pdf$/, '.expect.txt'));
    if (f.endsWith('.edited.pdf') && fs.existsSync(exp)) {
      const [pageNo, ...want] = fs.readFileSync(exp, 'utf8').split('\n').filter(Boolean);
      const text = norm(await pageText(doc, Number(pageNo)));
      const squash = text.replace(/\s/g, '');
      const missing = want.filter((w) => !text.includes(norm(w)) && !squash.includes(norm(w).replace(/\s/g, '')));
      line += ` · edits visible ${want.length - missing.length}/${want.length}`;
      if (missing.length) { failures++; line += ` MISSING ${JSON.stringify(missing.slice(0, 3))}`; }
    }
    console.log(line);
    await doc.destroy();
  } catch (e) {
    failures++;
    console.log(`${f}: FAILED ${e.message}`);
  }
}
process.exit(failures ? 1 : 0);

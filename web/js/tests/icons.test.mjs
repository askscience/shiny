/**
 * Icon pipeline test — pins the curation contract for every icon set.
 *
 * Icons come from three layers (active set → theme override → base) generated
 * from GPL-3.0 KDE sources by `scripts/kde-icons/convert.py`. This test guards
 * the things that silently break inlined SVGs and the set plumbing:
 *   - every generated icon carries a viewBox (or it cannot scale),
 *   - no `<style>` or `ColorScheme` leaks into a shipped icon,
 *   - icons outside a `tint` palette must be `currentColor`,
 *   - every palette colour must actually occur in its artwork (the accent
 *     tint keys off the exact hex strings),
 *   - index.json matches what is on disk,
 *   - every icon name a plugin references exists in the base catalog,
 *   - the plugin → icon map stays valid and distinct where it must be.
 *
 * Run:  node web/js/tests/icons.test.mjs
 */
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..', '..', '..');
const baseRoot = join(root, 'web', 'ui', 'icons');
const setsRoot = join(root, 'web', 'ui', 'iconsets');

let failures = 0;
const check = (name, condition, detail = '') => {
  console.log(`  ${condition ? 'ok  ' : 'FAIL'} ${name}${detail ? ` — ${detail}` : ''}`);
  if (!condition) failures += 1;
};

function walk(dir) {
  const out = [];
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) out.push(...walk(p));
    else if (e.name.endsWith('.svg')) out.push(p);
  }
  return out;
}

function relative(dir, file) {
  return file.slice(dir.length + 1).replaceAll('\\', '/').replace(/\.svg$/, '');
}

/** Shared checks for one generated set (base or /ui/iconsets/<id>). */
function validateSet(label, dir) {
  console.log(`icons — ${label}`);
  check(`${label} generated`, existsSync(dir), dir);
  if (!existsSync(dir)) return;
  const files = walk(dir);
  check(`${label} has icons`, files.length > 0, `${files.length} files`);
  const indexFile = join(dir, 'index.json');
  check(`${label} ships index.json`, existsSync(indexFile));
  const index = existsSync(indexFile)
    ? JSON.parse(readFileSync(indexFile, 'utf8'))
    : { names: {}, tint: {} };
  const listed = new Set(Object.values(index.names || {}).flat());
  const tint = index.tint || {};

  let missingViewBox = 0;
  let leakingStyle = 0;
  let notCurrentColor = 0;
  let paletteMismatch = 0;
  for (const f of files) {
    const name = relative(dir, f);
    const text = readFileSync(f, 'utf8');
    if (!text.includes('viewBox')) missingViewBox += 1;
    if (text.includes('<style') || text.includes('ColorScheme')) leakingStyle += 1;
    const palette = tint[name];
    if (palette) {
      const lower = text.toLowerCase();
      for (const [hex] of palette) if (!lower.includes(hex)) paletteMismatch += 1;
    } else if (!text.includes('currentColor')) {
      notCurrentColor += 1;
    }
  }
  check(`${label}: all icons have a viewBox`, missingViewBox === 0, `${missingViewBox} without`);
  check(`${label}: no leaked <style>/ColorScheme`, leakingStyle === 0, `${leakingStyle} leaking`);
  check(`${label}: symbolic icons use currentColor`, notCurrentColor === 0, `${notCurrentColor} not`);
  check(`${label}: tint palettes match their artwork`, paletteMismatch === 0, `${paletteMismatch} hex misses`);

  const onDisk = new Set(walk(dir).map((f) => relative(dir, f)));
  const missing = [...listed].filter((n) => !onDisk.has(n));
  const unlisted = [...onDisk].filter((n) => !listed.has(n));
  check(`${label}: index.json lists every icon`, missing.length === 0, missing.slice(0, 5).join(', '));
  check(`${label}: no icon is missing from index.json`, unlisted.length === 0, unlisted.slice(0, 5).join(', '));
  if (tint['ui/folder']) {
    check(`${label}: folder palette has >= 2 colours`, tint['ui/folder'].length >= 2);
  }
}

validateSet('base /ui/icons', baseRoot);
validateSet('set infinity', join(setsRoot, 'infinity'));
validateSet('set infinity-dark', join(setsRoot, 'infinity-dark'));

// ── The shared catalog must carry every name a plugin references ──────────
console.log('icons — plugin name coverage');
const baseIndex = JSON.parse(readFileSync(join(baseRoot, 'index.json'), 'utf8'));
const baseNames = new Set(Object.values(baseIndex.names).flat());
const referenced = new Set();
for (const dir of readdirSync(join(root, 'plugins'), { withFileTypes: true })) {
  if (!dir.isDirectory()) continue;
  const web = join(root, 'plugins', dir.name, 'web');
  if (!existsSync(web)) continue;
  for (const f of readdirSync(web)) {
    if (!f.endsWith('.js')) continue;
    const text = readFileSync(join(web, f), 'utf8');
    for (const m of text.matchAll(/'(ui|apps|artifacts|insights)\/[a-z0-9-]+'/g)) {
      referenced.add(m[0].slice(1, -1));
    }
  }
}
check('plugins reference some icons', referenced.size > 20, `${referenced.size} names`);
// Folder variants may be alias-resolved (see ALIASES in icon.js) when a set
// does not ship them; everything else must exist in the base catalog.
const aliasBlock = readFileSync(join(root, 'web', 'ui', 'icon.js'), 'utf8')
  .match(/const ALIASES\s*=\s*\{([^}]*)\}/s)?.[1] || '';
const aliased = new Set([...aliasBlock.matchAll(/'([^']+)'\s*:/g)].map((m) => m[1]));
const unresolved = [...referenced].filter((n) => !baseNames.has(n) && !aliased.has(n));
check('every plugin icon resolves in the base catalog', unresolved.length === 0, unresolved.join(', '));
const variantRefs = [...referenced].filter((n) => [
  'ui/folder-desktop', 'ui/folder-documents', 'ui/folder-downloads',
  'ui/folder-music', 'ui/folder-pictures', 'ui/folder-public',
  'ui/folder-templates', 'ui/folder-videos'].includes(n));
check('every referenced folder variant has an alias',
  variantRefs.every((n) => aliased.has(n)), variantRefs.filter((n) => !aliased.has(n)).join(', '));

// ── Plugin→icon map ────────────────────────────────────────────────────────
console.log('icons — plugin map');
const pluginIconSrc = readFileSync(join(root, 'web', 'js', 'pluginIcon.js'), 'utf8');
const mapBlock = pluginIconSrc.match(/PLUGIN_ICONS\s*=\s*\{([^}]*)\}/s);
check('PLUGIN_ICONS map present', !!mapBlock);
if (mapBlock) {
  const pairs = [...mapBlock[1].matchAll(/(\w+)\s*:\s*'([^']+)'/g)];
  check('map has entries', pairs.length >= 17, `${pairs.length} entries`);
  for (const [, plugin, target] of pairs) {
    check(`${plugin} → ${target}.svg exists`,
      existsSync(join(baseRoot, `${target}.svg`)));
  }
  const targets = new Map(pairs.map(([, p, t]) => [p, t]));
  for (const [a, b] of [['calc', 'calculator'], ['radio', 'studio']]) {
    check(`${a} and ${b} use different icons`,
      targets.has(a) && targets.has(b) && targets.get(a) !== targets.get(b),
      `${targets.get(a)} vs ${targets.get(b)}`);
  }
}

// ── Files folder variants ──────────────────────────────────────────────────
console.log('icons — folder variants');
const FOLDERS = ['folder-documents', 'folder-downloads', 'folder-desktop',
  'folder-music', 'folder-pictures', 'folder-public', 'folder-templates',
  'folder-videos'];
for (const set of ['infinity', 'infinity-dark']) {
  const index = JSON.parse(readFileSync(join(setsRoot, set, 'index.json'), 'utf8'));
  const names = new Set(Object.values(index.names).flat());
  for (const folder of FOLDERS) {
    check(`${set} ships ui/${folder}`, names.has(`ui/${folder}`));
  }
}
const filesSrc = readFileSync(join(root, 'plugins', 'files', 'web', 'plugin.js'), 'utf8');
check('Files maps special folders', filesSrc.includes('folderIconFor')
  && filesSrc.includes('ui/folder-documents') && filesSrc.includes('ui/folder-music'));

// ── Runtime contract ───────────────────────────────────────────────────────
console.log('icons — runtime');
const iconSrc = readFileSync(join(root, 'web', 'ui', 'icon.js'), 'utf8');
check('icon.js resolves through the active set', iconSrc.includes('getActiveIconset'));
check('icon.js has a generic tint', iconSrc.includes('export function tintSvg'));
check('icon.js repaints live icons', iconSrc.includes('export function refreshIcons'));
check('icon.js caches per set+theme', iconSrc.includes('${set}:${theme}:${name}'));
const loader = readFileSync(join(root, 'web', 'ui', 'iconset-loader.js'), 'utf8');
check('iconset-loader defaults to infinity', loader.includes("DEFAULT_CHOICE = 'infinity'"));
check('iconset-loader defaults to native colours',
  loader.includes("localStorage.getItem(TINT_KEY) === '1'"));
check('iconset-loader resolves light/dark', loader.includes("light: 'infinity'")
  && loader.includes("dark: 'infinity-dark'"));

// ── Docs ───────────────────────────────────────────────────────────────────
console.log('icons — docs');
check('web/ui/icons/README.md exists', existsSync(join(baseRoot, 'README.md')));
check('web/ui/icons/INDEX.md exists', existsSync(join(baseRoot, 'INDEX.md')));
check('docs mention the Infinity set',
  readFileSync(join(root, 'docs', 'core', 'themes-icons.md'), 'utf8').includes('Infinity'));

console.log(`\n${failures ? `${failures} failure(s)` : 'all ok'}`);
process.exit(failures ? 1 : 0);

/**
 * KDE icon set test — pins the curation contract.
 *
 * The icons under `web/ui/icons/` are generated from the (GPL-3.0)
 * Slot-Plasma-Themes set by `scripts/kde-icons/convert.py`. This test guards
 * the things that silently break inlined SVGs:
 *   - every generated icon must carry a viewBox (or it cannot scale),
 *   - symbolic icons must be `currentColor` (or they ignore the theme),
 *   - no `<style>` or `ColorScheme` may leak into a shipped icon,
 *   - the coloured folder must keep its own fills.
 * It also pins the plugin → icon map so a plugin can never lose its glyph.
 *
 * Run:  node web/js/tests/kdeIcons.test.mjs
 */
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..', '..', '..');
const iconsRoot = join(root, 'web', 'ui', 'icons');

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

console.log('kde icons — generated set');

const files = existsSync(iconsRoot) ? walk(iconsRoot) : [];
check('iconset generated', files.length > 0, `${files.length} icons under web/ui/icons`);

let missingViewBox = 0;
let leakingStyle = 0;
let notCurrentColor = 0;
for (const f of files) {
  const text = readFileSync(f, 'utf8');
  if (!text.includes('viewBox')) missingViewBox += 1;
  if (text.includes('<style') || text.includes('ColorScheme')) leakingStyle += 1;
  const keepColor = f.endsWith(join('ui', 'folder.svg'));
  if (!keepColor && !text.includes('currentColor')) notCurrentColor += 1;
}
check('all icons have a viewBox', missingViewBox === 0, `${missingViewBox} without`);
check('no leaked <style>/ColorScheme', leakingStyle === 0, `${leakingStyle} leaking`);
check('symbolic icons use currentColor', notCurrentColor === 0, `${notCurrentColor} not`);

// The coloured folder: shared copy (dark artwork) keeps original fills, and
// the two light themes each carry their own coloured folder override.
const sharedFolder = join(iconsRoot, 'ui', 'folder.svg');
const sharedFolderText = readFileSync(sharedFolder, 'utf8');
check('shared folder keeps colour', /fill:#[0-9a-fA-F]{6}/.test(sharedFolderText));
check('shared folder is not currentColor-only',
  !/^[^>]*fill="currentColor"[^>]*>\s*<path[^>]*fill="currentColor"/.test(sharedFolderText));
for (const theme of ['light', 'neumorphic-light']) {
  const p = join(root, 'web', 'themes', theme, 'icons', 'ui', 'folder.svg');
  check(`${theme} ships a folder override`, existsSync(p));
  if (existsSync(p)) {
    const t = readFileSync(p, 'utf8');
    check(`${theme} folder keeps colour`, /fill:#[0-9a-fA-F]{6}/.test(t));
  }
}

// Files plugin icons the plugin references must exist in the shared set.
console.log('kde icons — files plugin names');
for (const name of ['folder', 'folder-open', 'folder-plus', 'file', 'doc', 'image',
  'video', 'music', 'archive', 'forward', 'home', 'trash', 'monitor', 'download',
  'search', 'list', 'grid', 'close', 'chevron-left', 'chevron-right', 'chevron-up']) {
  check(`ui/${name}.svg exists`, existsSync(join(iconsRoot, 'ui', `${name}.svg`)));
}

// `ui/power` is used by the top bar, the power menu header and Settings →
// System; it must resolve from the shared set (no stale theme override).
console.log('kde icons — power');
check('ui/power.svg exists', existsSync(join(iconsRoot, 'ui', 'power.svg')));
for (const theme of ['noir', 'neumorphic', 'pitch', 'light', 'neumorphic-light']) {
  check(`${theme} has no stale power override`,
    !existsSync(join(root, 'web', 'themes', theme, 'icons', 'ui', 'power.svg')));
}

// Plugin icon map: read it from source (the module imports the DOM, so we pin
// the map textually) and assert every mapped target exists.
console.log('kde icons — plugin map');
const pluginIconSrc = readFileSync(join(root, 'web', 'js', 'pluginIcon.js'), 'utf8');
const mapBlock = pluginIconSrc.match(/PLUGIN_ICONS\s*=\s*\{([^}]*)\}/s);
check('PLUGIN_ICONS map present', !!mapBlock);
if (mapBlock) {
  const pairs = [...mapBlock[1].matchAll(/(\w+)\s*:\s*'([^']+)'/g)];
  check('map has entries', pairs.length >= 17, `${pairs.length} entries`);
  for (const [, plugin, target] of pairs) {
    check(`${plugin} → ${target}.svg exists`,
      existsSync(join(iconsRoot, `${target}.svg`)));
  }
  // Plugins that are easy to conflate must map to *different* icons.
  const targets = new Map(pairs.map(([, p, t]) => [p, t]));
  const mustDiffer = [
    ['calc', 'calculator'],
    ['radio', 'studio'],
  ];
  for (const [a, b] of mustDiffer) {
    check(`${a} and ${b} use different icons`,
      targets.has(a) && targets.has(b) && targets.get(a) !== targets.get(b),
      `${targets.get(a)} vs ${targets.get(b)}`);
  }
}

// Docs that plugin authors (and the AI) read must exist and stay in sync.
console.log('kde icons — docs');
check('web/ui/icons/README.md exists',
  existsSync(join(iconsRoot, 'README.md')));
const index = join(iconsRoot, 'INDEX.md');
check('web/ui/icons/INDEX.md exists', existsSync(index));
if (existsSync(index)) {
  const idx = readFileSync(index, 'utf8');
  // Every mapped destination must be listed in the catalog.
  const pluginIconSrc0 = readFileSync(join(root, 'web', 'js', 'pluginIcon.js'), 'utf8');
  const map0 = pluginIconSrc0.match(/PLUGIN_ICONS\s*=\s*\{([^}]*)\}/s);
  if (map0) {
    for (const [, , target] of map0[1].matchAll(/(\w+)\s*:\s*'([^']+)'/g)) {
      check(`INDEX.md lists ${target}`, idx.includes(`${target}.svg`));
    }
  }
  check('INDEX.md mentions the coloured folder', idx.includes('coloured folder'));
}

// The coloured folder follows the accent: its fixed KDE blues are remapped to
// an accent-derived ramp at load time (web/ui/icon.js `colorizeFolder`).
console.log('kde icons — folder accent colouring');
{
  const iconSrc = readFileSync(join(root, 'web', 'ui', 'icon.js'), 'utf8');
  check('colorizeFolder present', iconSrc.includes('colorizeFolder'));
  check('icon.js listens for appearance:change',
    iconSrc.includes("addEventListener('appearance:change'"));
  // Every blue in the folder artwork must be one the colorizer remaps.
  const blues = [...new Set(sharedFolderText.match(/#[0-9a-fA-F]{6}/g) || [])].map((s) => s.toLowerCase());
  const known = new Set(['#3a435f', '#2c5ba0', '#4077cb', '#4b7fcd', '#5294e2', '#739bd9']);
  for (const b of blues) {
    check(`folder blue ${b} is remapped`, known.has(b));
  }
  // hexToRgb returns an [r,g,b] array — the colorizer must not destructure it
  // as an object (that silently produced NaN colours).
  check('colorizer does not destructure hexToRgb as an object',
    !/const\s*\{\s*r\s*,\s*g\s*,\s*b\s*\}\s*=\s*hexToRgb/.test(iconSrc));
  // The light folder override must use the same blues so it recolors too.
  const lightFolder = readFileSync(
    join(root, 'web', 'themes', 'light', 'icons', 'ui', 'folder.svg'), 'utf8');
  check('light folder shares the folder blues',
    blues.every((b) => lightFolder.toLowerCase().includes(b)));
}

console.log(`\n${failures ? `${failures} failure(s)` : 'all ok'}`);
process.exit(failures ? 1 : 0);

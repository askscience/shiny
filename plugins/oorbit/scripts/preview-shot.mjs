// Screenshot the Oorbit window served by preview-server.py, in a headless
// browser (run it under xvfb-run; never on the desktop display).
//
//   SHINY_PLAYWRIGHT_MODULE=/path/to/playwright/package.json \
//     xvfb-run -a node plugins/oorbit/scripts/preview-shot.mjs OUT_DIR [WIDTHxHEIGHT] [QUERY]
//
// Writes OUT_DIR/1-start.png once the game has started, then one shot per
// step of STEPS, a JSON list of [name, x, y, waitMs] clicks
// (e.g. STEPS='[["research",28,76]]'), and prints the page's console.
// QUERY is passed to the parent page (e.g. "?light=500&lang=en").
import { createRequire } from 'module';

const [out = '.', size = '1100x700', query = ''] = process.argv.slice(2);
const [width, height] = size.split('x').map(Number);
const require = createRequire(process.env.SHINY_PLAYWRIGHT_MODULE || import.meta.url);
const { chromium } = require('playwright');
const browser = await chromium.launch({ args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader'] });
const page = await browser.newPage({ viewport: { width, height } });
const logs = [];
page.on('console', (m) => logs.push(`[${m.type()}] ${m.text()}`));
page.on('pageerror', (e) => logs.push(`[pageerror] ${e.message}`));
await page.goto(`http://127.0.0.1:${process.env.PORT || 8791}/${query}`);
await page.waitForTimeout(Number(process.env.START_MS || 6000));
await page.screenshot({ path: `${out}/1-start.png` });
let i = 2;
for (const [name, x, y, wait] of JSON.parse(process.env.STEPS || '[]')) {
  await page.mouse.click(x, y);
  await page.waitForTimeout(wait || 1500);
  await page.screenshot({ path: `${out}/${i++}-${name}.png` });
}
const quiet = /^\[(debug|verbose)\]|Godot Engine|OpenGL API|Build configuration/;
console.log(logs.filter((l) => !quiet.test(l)).slice(0, 40).join('\n'));
await browser.close();

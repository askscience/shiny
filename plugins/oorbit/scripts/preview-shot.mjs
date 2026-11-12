// Screenshot the Oorbit window served by preview-server.py, in a headless
// browser (run it under xvfb-run; never on the desktop display).
//
//   SHINY_PLAYWRIGHT_MODULE=/path/to/playwright/package.json \
//     xvfb-run -a node plugins/oorbit/scripts/preview-shot.mjs OUT_DIR [WIDTHxHEIGHT] [light]
//
// Writes OUT_DIR/oorbit-1.png and oorbit-2.png a few seconds apart (the
// planet turns between them) and prints the page's console.
import { createRequire } from 'module';

const [out = '.', size = '960x600', mode = ''] = process.argv.slice(2);
const [width, height] = size.split('x').map(Number);
const require = createRequire(process.env.SHINY_PLAYWRIGHT_MODULE || import.meta.url);
const { chromium } = require('playwright');
const browser = await chromium.launch({ args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader'] });
const page = await browser.newPage({ viewport: { width, height } });
const logs = [];
page.on('console', (m) => logs.push(`[${m.type()}] ${m.text()}`));
page.on('pageerror', (e) => logs.push(`[pageerror] ${e.message}`));
await page.goto(`http://127.0.0.1:${process.env.PORT || 8791}/${mode === 'light' ? '?light=500' : ''}`);
await page.waitForTimeout(5000);
await page.screenshot({ path: `${out}/oorbit-1.png` });
await page.waitForTimeout(4000);
await page.screenshot({ path: `${out}/oorbit-2.png` });
console.log(logs.filter((l) => !/^\[(debug|verbose)\]/.test(l)).slice(0, 40).join('\n'));
await browser.close();

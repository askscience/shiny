// Record a short tour of the Oorbit window served by preview-server.py, in a
// headless browser (run it under xvfb-run; never on the desktop display):
// the whole planet turning, a slow descent to the home, a look around, and
// back up.
//
//   SHINY_PLAYWRIGHT_MODULE=/path/to/playwright/package.json \
//     xvfb-run -a node plugins/oorbit/scripts/preview-video.mjs OUT_DIR [WIDTHxHEIGHT] [QUERY]
//
// Writes OUT_DIR/tour.webm. CHROME_ARGS picks the renderer as in
// preview-shot.mjs ("--use-angle=vulkan --enable-gpu --ignore-gpu-blocklist
// --enable-features=Vulkan" records smoothly on a machine with a GPU).
import { createRequire } from 'module';
import { renameSync, writeFileSync } from 'fs';

const [out = '.', size = '1280x800', query = ''] = process.argv.slice(2);
const [width, height] = size.split('x').map(Number);
const require = createRequire(process.env.SHINY_PLAYWRIGHT_MODULE || import.meta.url);
const { chromium } = require('playwright');
const args = process.env.CHROME_ARGS ? process.env.CHROME_ARGS.split(' ') : ['--use-angle=swiftshader', '--enable-unsafe-swiftshader'];
const browser = await chromium.launch({ args });
const context = await browser.newContext({ viewport: { width, height }, recordVideo: { dir: out, size: { width, height } } });
const page = await context.newPage();
const opened = Date.now();
await page.goto(`http://127.0.0.1:${process.env.PORT || 8791}/${query}`);
await page.waitForTimeout(Number(process.env.START_MS || 16000));
const cx = width / 2;
const cy = height / 2 + 40;
const wheel = async (amount, steps, pause) => {
  await page.mouse.move(cx, cy);
  for (let k = 0; k < steps; k++) {
    await page.mouse.wheel(0, amount);
    await page.waitForTimeout(pause);
  }
};
const drag = async (dx, dy, steps) => {
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  for (let k = 1; k <= steps; k++) {
    await page.mouse.move(cx + (dx * k) / steps, cy + (dy * k) / steps);
    await page.waitForTimeout(40);
  }
  await page.mouse.up();
};
// Writes OUT_DIR/start.txt: the second the tour starts (the loading before
// it can be cut, e.g. ffmpeg -ss).
const started = Date.now();
await page.waitForTimeout(5000);            // the planet turns
await wheel(-50, 30, 260);                  // down to the ground
await page.waitForTimeout(5000);            // people, animals, smoke
await drag(-700, 0, 120);                   // a look along the land
await page.waitForTimeout(4000);
await drag(0, 400, 80);
await page.waitForTimeout(4000);
await wheel(50, 30, 200);                   // back up
await page.waitForTimeout(4000);
writeFileSync(`${out}/start.txt`, String((started - opened) / 1000));
const video = page.video();
await context.close();
renameSync(await video.path(), `${out}/tour.webm`);
await browser.close();

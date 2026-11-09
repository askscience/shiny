// Drive the Capture window in a private server under Xvfb.
import { createRequire } from 'node:module';
import { writeFileSync, readdirSync, existsSync } from 'node:fs';
const { chromium } = createRequire(process.env.SHINY_PLAYWRIGHT_MODULE || import.meta.url)('playwright');

const T = process.argv[2];
const OUT = (process.env.CAPTURE_E2E_OUT || '/tmp') + '/';
const BASE = 'http://127.0.0.1:18731';
const log = (...a) => console.log('[e2e]', ...a);
const fails = [];
const check = (name, ok, extra = '') => { log(ok ? 'ok  ' : 'FAIL', name, extra); if (!ok) fails.push(name); };

async function api(path, opts = {}, token) {
  const r = await fetch(BASE + path, { ...opts, headers: { 'content-type': 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}), ...(opts.headers || {}) } });
  const text = await r.text();
  try { return { status: r.status, body: JSON.parse(text) }; } catch { return { status: r.status, body: text }; }
}

const reg = await api('/api/auth/register', { method: 'POST', body: JSON.stringify({ username: 'tester', password: 'correct horse battery' }) });
const token = reg.body?.token || reg.body?.data?.token;
const traveler = reg.body?.traveler || reg.body?.data?.traveler;
check('register', !!token, JSON.stringify(reg.body).slice(0, 200));
const act = await api('/api/plugins/activate', { method: 'POST', body: JSON.stringify({ name: 'capture' }) }, token);
check('activate capture', act.status === 200, JSON.stringify(act.body).slice(0, 200));
const plugins = await api('/api/plugins', {}, token);
const cap = (plugins.body?.data || plugins.body || []).find?.((p) => p.name === 'capture');
check('plugin listed with hud', !!cap && cap.hud === true, JSON.stringify(cap || {}).slice(0, 300));

// REST smoke.
const dev = await api('/api/capture/devices', {}, token);
check('devices', dev.status === 200 && Array.isArray(dev.body?.data?.audio), JSON.stringify(dev.body).slice(0, 300));
const dest = await api('/api/capture/destinations', { method: 'POST', body: JSON.stringify({ platform: 'twitch', key: 'live_secret_123456' }) }, token);
check('destination saved, key masked', dest.body?.data?.key_hint === '••••3456' && !JSON.stringify(dest.body).includes('secret'), JSON.stringify(dest.body).slice(0, 300));
const bad = await api('/api/capture/destinations', { method: 'POST', body: JSON.stringify({ platform: 'kick', key: 'x' }) }, token);
check('kick needs a server', bad.status === 400);

const browser = await chromium.launch({ headless: false, args: ['--window-position=0,0', '--window-size=1920,1080', '--autoplay-policy=no-user-gesture-required'] });
const ctx = await browser.newContext({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
await ctx.addCookies([{ name: 'shiny_token', value: token, url: BASE }]);
await ctx.addInitScript(([tk, tr]) => {
  localStorage.setItem('token', tk);
  localStorage.setItem('traveler', JSON.stringify(tr));
  localStorage.setItem('onboarding.done', '1');
}, [token, traveler]);
const page = await ctx.newPage();
const errors = [];
page.on('pageerror', (e) => errors.push(String(e)));
page.on('response', (r) => { if (r.status() >= 400) errors.push(`${r.status()} ${r.url()}`); });
page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
await page.goto(BASE + '/', { waitUntil: 'domcontentloaded' });
await page.waitForTimeout(4000);
const opened = await page.evaluate(async () => { try { const m = await import('/js/tiles.js'); await m.activatePlugin('capture'); await new Promise((r) => setTimeout(r, 1500)); return String(await m.openPluginWindow('capture', { focus: true })); } catch (e) { return 'ERR ' + e; } });
log('openPluginWindow →', opened);
log('catalog', await page.evaluate(async () => { const r = await fetch('/api/plugins', { headers: { authorization: 'Bearer ' + localStorage.getItem('token') } }); const j = await r.json(); const c = (j.data || j).find((p) => p.name === 'capture'); const a = await (await fetch('/api/plugins/active', { headers: { authorization: 'Bearer ' + localStorage.getItem('token') } })).json(); return JSON.stringify({ surface: c?.surface, web_app: c?.web_app, active: a }); }));
await page.waitForSelector('section.capture-tile .capture-record', { timeout: 15000 }).catch(() => {});
await page.waitForTimeout(1500);
check('capture window mounted', await page.$('section.capture-tile .capture-record') !== null, errors.slice(0, 8).join(' | '));
if (!await page.$('section.capture-tile .capture-record')) { await page.screenshot({ path: OUT + 'record-view.png' }); console.log(errors.join('\n')); await browser.close(); process.exit(1); }
await page.screenshot({ path: OUT + 'record-view.png' });

// Start a recording of the whole screen (no camera in Xvfb, mic off).
await page.evaluate(() => {
  const s = document.querySelector('.capture-audio-row .ui-toggle');
  // Turn the mic and desktop sound off so the test needs no audio server.
  document.querySelectorAll('.capture-audio-row .ui-toggle').forEach((t) => { if (t.getAttribute('aria-checked') === 'true') t.click(); });
  void s;
});
await page.waitForTimeout(800);
await page.click('section.capture-tile .capture-rec');
await page.waitForTimeout(1500);
const st1 = await api('/api/capture/status', {}, token);
check('recording started', st1.body?.data?.active === true, JSON.stringify(st1.body?.data).slice(0, 300));
check('hud chip visible', await page.$('#hud-capture:not([hidden])') !== null);
await page.waitForTimeout(2500);
await page.screenshot({ path: OUT + 'recording.png' });
await api('/api/capture/marker', { method: 'POST', body: JSON.stringify({ label: 'test marker' }) }, token);
await page.click('section.capture-tile .capture-bar .ui-tool[title="Stop"]').catch(async () => { await api('/api/capture/stop', { method: 'POST', body: '{}' }, token); });
let st2;
for (let i = 0; i < 40; i++) {
  await page.waitForTimeout(500);
  st2 = await api('/api/capture/status', {}, token);
  if (st2.body?.data?.state === 'done' || st2.body?.data?.state === 'failed') break;
}
check('recording saved', st2.body?.data?.state === 'done' && !!st2.body?.data?.output_id, JSON.stringify(st2.body?.data).slice(0, 400));
const recs = await api('/api/capture/recordings', {}, token);
const rec = recs.body?.data?.[0];
check('library lists it with a marker', !!rec && rec.meta?.markers?.length === 1, JSON.stringify(rec || {}).slice(0, 300));

// Library and editor.
await page.click('section.capture-tile .capture-views [role="radio"]:nth-of-type(2)').catch(() => {});
await page.waitForTimeout(1500);
await page.screenshot({ path: OUT + 'library.png' });
const proj = await api('/api/capture/projects', { method: 'POST', body: JSON.stringify({ title: 'E2E', media: [rec.id] }) }, token);
const pid = proj.body?.data?.id;
check('project from the recording', !!pid && proj.body.data.clips.length >= 1, JSON.stringify(proj.body).slice(0, 300));
// An AI-style proposal with a graphic, then accept it.
const html = '<!doctype html><html><body><div id="t" style="font:900 70vh sans-serif;color:#ff2200;animation:in 1s ease-out both">Hello</div><style>@keyframes in{from{opacity:0;transform:translateY(20px)}}</style></body></html>';
const e1 = await api(`/api/capture/projects/${pid}`, { method: 'PUT', body: JSON.stringify({ ops: [{ op: 'add_overlay', html, at: 0.5, dur: 2, name: 'Title', rect: { x: 0.1, y: 0.1, w: 0.5, h: 0.2 } }, { op: 'add_caption', at: 0, dur: 2, text: 'Ciao' }] }) }, token);
check('ops applied', e1.status === 200 && e1.body?.data?.overlays?.length === 1, JSON.stringify(e1.body).slice(0, 300));
const ov = await fetch(`${BASE}/api/capture/overlay/${pid}/${e1.body.data.overlays[0].id}`, { headers: { authorization: `Bearer ${token}` } });
check('overlay served sandboxed', ov.status === 200 && (ov.headers.get('content-security-policy') || '').includes('sandbox allow-scripts'));
await page.evaluate((id) => window.dispatchEvent(new CustomEvent('capture:open-project', { detail: { id } })), pid);
await page.waitForSelector('.capture-ed-work:not([hidden]) .ui-tl-clip', { timeout: 10000 }).catch(() => {});
await page.waitForTimeout(2500);
check('editor shows clips', (await page.$$('.capture-ed-work .ui-tl-clip')).length >= 2);
await page.screenshot({ path: OUT + 'editor.png' });
// Export (source preset).
const ex = await api(`/api/capture/projects/${pid}/export`, { method: 'POST', body: JSON.stringify({ preset: '720p' }) }, token);
let job;
for (let i = 0; i < 120; i++) {
  await new Promise((r) => setTimeout(r, 1000));
  job = await api(`/api/capture/jobs/${ex.body?.data?.job}`, {}, token);
  if (['done', 'failed'].includes(job.body?.data?.state)) break;
}
check('export', job.body?.data?.state === 'done', JSON.stringify(job.body?.data).slice(0, 600));
writeFileSync(OUT + 'export.json', JSON.stringify(job.body?.data || {}, null, 2));

const real = errors.filter((e) => !/installer\/health|Failed to load resource/.test(e));
check('no page errors', real.length === 0, real.slice(0, 6).join(' | '));
await browser.close();
log(fails.length ? `${fails.length} failed: ${fails.join(', ')}` : 'all passed');
process.exit(fails.length ? 1 : 0);

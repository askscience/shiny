// LiteAura README tour: desktop, phone and wearable, recorded from one stage page.
// The app runs in an iframe (full screen, phone-shaped, watch-shaped); captions
// and title cards are HTML on the stage. Frames come from CDP screencast
// (JPEG q92) with timestamps; build.js turns them into the video, speeding up
// the spans marked `fast` (the model thinking).
const { chromium } = require('/home/eev/.npm/_npx/e41f203b7505f1fb/node_modules/playwright');
const fs = require('fs');
const path = require('path');

const BASE = 'http://127.0.0.1:18091/';
const token = fs.readFileSync('/tmp/liteaura-demo/token', 'utf8').trim();
const traveler = JSON.parse(fs.readFileSync('/tmp/liteaura-demo/traveler.json', 'utf8'));
const OUT = '/tmp/liteaura-rec/frames';
fs.rmSync(OUT, { recursive: true, force: true });
fs.mkdirSync(OUT, { recursive: true });

const STAGE = `<!doctype html><html><head><meta charset="utf-8"><style>
  html,body{margin:0;width:1920px;height:1080px;overflow:hidden;font-family:'Inter Display','Inter',system-ui,sans-serif;color:#fff}
  body{background:radial-gradient(1200px 800px at 20% 10%,#3b2f5c 0%,transparent 60%),radial-gradient(1000px 700px at 85% 90%,#7a3b2e 0%,transparent 55%),#121016}
  #device{position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);transition:none}
  #device.phone,#device.watch{left:36%}
  #device iframe{border:0;display:block;background:#000}
  .desktop iframe{width:1920px;height:1080px}
  .desktop{width:1920px;height:1080px}
  .phone{padding:14px;border-radius:64px;background:#0b0b0d;box-shadow:0 40px 120px rgba(0,0,0,.6),0 0 0 2px #2a2a30 inset}
  .phone iframe{width:390px;height:844px;border-radius:50px;zoom:1.12}
  .watch{padding:22px;border-radius:50%;background:linear-gradient(145deg,#2c2c33,#0b0b0d);box-shadow:0 40px 120px rgba(0,0,0,.65),0 0 0 3px #3a3a42 inset}
  .watch iframe{width:460px;height:460px;border-radius:50%;zoom:1.45}
  #caption{position:absolute;left:50%;bottom:64px;transform:translate(-50%,20px);padding:16px 30px;border-radius:999px;
    background:rgba(12,10,16,.72);backdrop-filter:blur(18px);font-size:30px;font-weight:500;letter-spacing:-.01em;
    opacity:0;transition:opacity .45s ease,transform .45s ease;white-space:nowrap;box-shadow:0 10px 40px rgba(0,0,0,.35)}
  #caption.on{opacity:1;transform:translate(-50%,0)}
  #caption.side{left:auto;right:150px;bottom:auto;top:50%;transform:translate(0,-50%);white-space:normal;width:560px;border-radius:28px;font-size:34px;line-height:1.25}
  #caption.side.on{transform:translate(0,-50%)}
  #caption,#badge,#title{pointer-events:none}
  #title{position:absolute;inset:0;display:grid;place-content:center;text-align:center;opacity:0;transition:opacity .6s ease;
    background:radial-gradient(900px 600px at 50% 40%,#3b2f5c 0%,#121016 70%)}
  #title.on{opacity:1}
  #title h1{margin:0;font-size:120px;font-weight:600;letter-spacing:-.04em}
  #title p{margin:18px 0 0;font-size:36px;opacity:.75;font-weight:400}
  #badge{position:absolute;top:36px;right:40px;padding:10px 18px;border-radius:999px;background:rgba(255,120,60,.92);font-size:22px;font-weight:600;opacity:0;transition:opacity .3s}
  #badge.on{opacity:1}
</style></head><body>
  <div id="device" class="desktop"></div>
  <div id="caption"></div>
  <div id="badge">AI at work · sped up</div>
  <div id="title"><div><h1 id="t1"></h1><p id="t2"></p></div></div>
</body></html>`;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  const browser = await chromium.launch({ args: ['--autoplay-policy=no-user-gesture-required'] });
  const context = await browser.newContext({
    viewport: { width: 1920, height: 1080 },
    deviceScaleFactor: 1,
    geolocation: { latitude: 38.7223, longitude: -9.1393 },
    permissions: ['geolocation'],
    locale: 'en-US',
    timezoneId: 'Europe/Lisbon',
  });
  await context.grantPermissions(['geolocation'], { origin: 'http://127.0.0.1:18091' });
  await context.addInitScript(([tk, tr]) => {
    if (location.port !== '18091') return;
    localStorage.setItem('token', tk);
    localStorage.setItem('traveler', JSON.stringify(tr));
    // The demo server runs without the speech sidecars: keep their notices out.
    const noisy = /whisper|vosk|voice model|microphone|speech/i;
    new MutationObserver(() => {
      document.querySelectorAll('.ui-toast').forEach((t) => { if (noisy.test(t.textContent)) t.remove(); });
    }).observe(document, { childList: true, subtree: true });
  }, [token, traveler]);
  // Recording only: the app forbids being framed (X-Frame-Options, CSP
  // frame-ancestors). Drop those two headers on its HTML documents so the
  // stage can show it inside a device frame. Everything else passes through.
  await context.route((url) => url.href === BASE, async (route) => {
    const res = await route.fetch();
    const headers = { ...res.headers() };
    delete headers['x-frame-options'];
    if (headers['content-security-policy']) {
      headers['content-security-policy'] = headers['content-security-policy'].replace(/;?\s*frame-ancestors[^;]*/i, '');
    }
    return route.fulfill({ response: res, headers });
  });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.log('stage error', e.message));
  await page.setContent(STAGE);

  // ── Recorder ──
  const frames = [];
  const marks = [];
  const cdp = await context.newCDPSession(page);
  let n = 0;
  cdp.on('Page.screencastFrame', async (f) => {
    const file = path.join(OUT, `f${String(n++).padStart(6, '0')}.jpg`);
    fs.writeFileSync(file, Buffer.from(f.data, 'base64'));
    frames.push([file, f.metadata.timestamp]);
    try { await cdp.send('Page.screencastFrameAck', { sessionId: f.sessionId }); } catch (_) {}
  });
  const now = () => Date.now() / 1000;
  const mark = (type) => { marks.push([type, now()]); };

  // ── Stage helpers ──
  const stage = (fn, arg) => page.evaluate(fn, arg);
  const caption = async (text, { side = false } = {}) => {
    await stage(([t, s]) => {
      const c = document.getElementById('caption');
      if (!t) { c.classList.remove('on'); return; }
      c.classList.remove('on');
      setTimeout(() => { c.textContent = t; c.classList.toggle('side', s); c.classList.add('on'); }, c.textContent ? 300 : 0);
    }, [text, side]);
  };
  const title = async (h, p) => stage(([a, b]) => {
    const el = document.getElementById('title');
    if (!a) { el.classList.remove('on'); return; }
    document.getElementById('t1').textContent = a;
    document.getElementById('t2').textContent = b || '';
    el.classList.add('on');
  }, [h, p]);
  const badge = (on) => stage((o) => document.getElementById('badge').classList.toggle('on', o), on);
  async function mountApp(kind) {
    await stage((k) => {
      const d = document.getElementById('device');
      d.className = k;
      d.innerHTML = '';
      const f = document.createElement('iframe');
      f.allow = 'geolocation; microphone; autoplay';
      f.src = 'http://127.0.0.1:18091/';
      d.appendChild(f);
    }, kind);
    await sleep(800);
    const handle = await page.$('#device iframe');
    const frame = await handle.contentFrame();
    await frame.waitForSelector(kind === 'watch' ? '#wearable' : '#app:not(.hidden)', { state: 'attached', timeout: 30000 }).catch(() => {});
    await sleep(3000);
    return frame;
  }
  const api = (body) => fetch(BASE + 'api/preferences', {
    method: 'PUT', headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' }, body: JSON.stringify(body),
  });
  const mod = (frame, src, fn, arg) => frame.evaluate(async ([s, f, a]) => {
    const m = await import(s);
    return (0, eval)(f)(m, a);
  }, [src, fn.toString(), arg]);

  async function askAI(frame, text) {
    // What a tap on the voice bar does (app.js typeOrb).
    await mod(frame, '/js/textInput.js', (m) => m.openTextInput());
    await sleep(900);
    await frame.type('.compose-dock-field', text, { delay: 32 });
    await sleep(500);
    await frame.press('.compose-dock-field', 'Enter');
    await sleep(2200);
    mark('fast'); await badge(true);
    // The turn starts a moment after the send; wait for it, then for its end.
    for (let i = 0; i < 40; i++) {
      if (await mod(frame, '/js/agent.js', (m) => m.isTurnActive())) break;
      await sleep(250);
    }
    const t0 = Date.now();
    // Wait for the turn to finish (the agent module is the app's own instance).
    while (Date.now() - t0 < 300000) {
      const busy = await mod(frame, '/js/agent.js', (m) => m.isTurnActive());
      if (!busy) break;
      await sleep(500);
    }
    await badge(false); mark('normal');
    await sleep(2500);
  }

  await api({ 'ui.wearable': 'false' });
  await cdp.send('Page.startScreencast', { format: 'jpeg', quality: 92, everyNthFrame: 1 });
  mark('start');

  // ── Intro ──
  await title('LiteAura', 'An assistant-first desktop — on a desktop, a phone and a watch');
  await sleep(2600);

  // ── 1. Desktop ──
  await title('Desktop', 'Ask in plain words. The assistant does the work.');
  let app = await mountApp('desktop');
  await mod(app, '/js/desktop.js', (m) => m.setLayout({ mode: 'master' }));
  await sleep(1200);
  await title(null);
  await sleep(1200);
  await caption('Ask for a document — it is written for you');
  await askAI(app, 'Write a short guide to Lisbon\'s historic trams in Word, with a few sections, and open it.');
  await caption('The document opens in Word, written and formatted');
  await sleep(3000);
  await caption('Then a spreadsheet, filled in');
  await askAI(app, 'Now make a small budget for a 3-day trip to Lisbon in Calc and open it.');
  await sleep(2500);
  await mod(app, '/js/textInput.js', (m) => m.closeTextInput(true));
  await mod(app, '/js/hudPlugins.js', async (m) => { await m.openPlugin('calendar'); await m.openPlugin('files'); });
  await sleep(1500);
  await caption('Tiling: windows snap into place');
  await mod(app, '/js/desktop.js', (m) => m.setLayout({ mode: 'master' }));
  await sleep(2600);
  await caption('Columns: side by side');
  await mod(app, '/js/desktop.js', (m) => m.setLayout({ mode: 'columns' }));
  await sleep(2600);
  await caption('Windows: free, overlapping, draggable');
  await mod(app, '/js/desktop.js', (m) => m.setLayout({ mode: 'windows' }));
  await sleep(2800);
  await caption('Every window at a glance');
  await app.evaluate(() => window.__shinyGesture('up'));
  await sleep(2600);
  await app.evaluate(() => window.__shinyGesture('down'));
  await sleep(1500);
  await mod(app, '/js/desktop.js', (m) => m.setLayout({ mode: 'master' }));
  await caption('Themes switch with their own motion');
  await sleep(800);
  await mod(app, '/ui/themeTransition.js', (m) => m.switchTheme('ink'));
  await sleep(2600);
  await mod(app, '/ui/themeTransition.js', (m) => m.switchTheme('editorial'));
  await sleep(2600);
  await mod(app, '/ui/themeTransition.js', (m) => m.switchTheme('dusk'));
  await sleep(1800);
  await caption(null);

  // ── 2. Phone ──
  await title('Phone', 'Every window reads like a phone app');
  await sleep(1200);
  app = await mountApp('phone');
  await sleep(600);
  await title(null);
  await caption('Home: every app, one tap away', { side: true });
  await sleep(2600);
  await caption('The document the assistant wrote', { side: true });
  await mod(app, '/js/hudPlugins.js', (m) => m.openPlugin('word'));
  await sleep(3600);
  await app.evaluate(() => history.back());
  await sleep(1500);
  await caption('The budget, in Calc', { side: true });
  await mod(app, '/js/hudPlugins.js', (m) => m.openPlugin('calc'));
  await sleep(3200);
  await caption('Switch apps like on any phone', { side: true });
  await mod(app, '/js/overview.js', (m) => m.openOverview());
  await sleep(2600);
  await mod(app, '/js/overview.js', (m) => m.closeOverview());
  await sleep(1200);
  await caption('Same themes, same motion', { side: true });
  await mod(app, '/ui/themeTransition.js', (m) => m.switchTheme('ink'));
  await sleep(2800);
  await mod(app, '/ui/themeTransition.js', (m) => m.switchTheme('dusk'));
  await sleep(1200);
  await caption(null);

  // ── 3. Wearable ──
  await title('Wearable', 'A watch face that is always listening');
  await api({ 'ui.wearable': 'true' });
  await sleep(1200);
  app = await mountApp('watch');
  await sleep(600);
  await title(null);
  await caption('Clock, date and weather over the assistant', { side: true });
  await sleep(2800);
  // A spoken request, played as a demo: listening, a tool, the spoken answer.
  const voice = (state) => app.evaluate((s) => { document.documentElement.dataset.voice = s; window.dispatchEvent(new CustomEvent('voice:state', { detail: { state: s } })); }, state);
  const talk = (ms) => app.evaluate((d) => new Promise((res) => {
    let i = 0; const t0 = performance.now();
    const id = setInterval(() => {
      const p = Math.random() < 0.2 ? 0.05 : 0.35 + 0.65 * Math.abs(Math.sin(i++ / 2.3)) * Math.random();
      window.dispatchEvent(new CustomEvent('voice:level', { detail: { level: p, pan: Math.sin(i / 9) * 0.6 } }));
      if (performance.now() - t0 > d) { clearInterval(id); res(); }
    }, 70);
  }), ms);
  const step = (message, action) => app.evaluate(([m, a]) => window.dispatchEvent(new CustomEvent('agent:step', { detail: m ? { message: m, action: a } : {} })), [message, action]);
  async function exchange(effect, said, tool) {
    await mod(app, '/js/preferences.js', (m, e) => m.setImmersive({ orb_effect: e }), effect);
    await caption(said, { side: true });
    await voice('listening'); await talk(1700);
    await voice('processing'); await step('Working', tool); await sleep(1800);
    await step(null); await voice('speaking'); await talk(1700);
    await voice('idle'); await sleep(500);
  }
  await exchange('glyph', '“Hey Alex, add lunch with Sara tomorrow at one.”  — Glyph', 'calendar_create');
  await exchange('eq', '“What\'s the weather this weekend?”  — Equaliser', 'web_search');
  await exchange('ripple', '“Read me my last email.”  — Ripple', 'mail_read');
  await exchange('firefly', '“Save that as a note.”  — Fireflies', 'doc_create');
  await caption('No windows: plugins work in the background', { side: true });
  await sleep(2200);
  await caption('One button: wallpaper, AI model, theme, animation', { side: true });
  await app.evaluate(() => document.querySelector('.wearable-settings').click());
  await sleep(1800);
  // Wallpaper, then the animation row stepping through its choices.
  await app.evaluate(() => document.querySelector('.wearable-swatch:nth-child(3)')?.click());
  await sleep(1200);
  for (let i = 0; i < 2; i++) {
    await app.evaluate(() => document.querySelectorAll('.wearable-row')[2]?.click());
    await sleep(900);
  }
  await app.evaluate(() => document.querySelectorAll('.wearable-row')[1]?.click());
  await sleep(2200);
  await app.evaluate(() => document.querySelector('.wearable-panel-actions .ui-btn')?.click());
  await sleep(1500);
  await voice('speaking'); await talk(2000); await voice('idle');
  await caption(null);

  // ── End ──
  await title('LiteAura', 'Desktop · Phone · Wearable');
  await sleep(3500);

  await cdp.send('Page.stopScreencast');
  mark('end');
  await sleep(500);
  fs.writeFileSync('/tmp/liteaura-rec/timeline.json', JSON.stringify({ frames, marks }));
  console.log('frames', frames.length, 'marks', JSON.stringify(marks));
  // Leave the demo account on the desktop for next time.
  await api({ 'ui.wearable': 'false' });
  await browser.close();
})().catch((e) => { console.error(e); process.exit(1); });

/**
 * The login screen's look: a session publishes its account's stored look
 * (theme, accent, gradient, motion) once and on every change, never the look
 * the sign-in stage happens to be painting; the login screen picks the most
 * recently published one.
 */
import './helpers/i18n.mjs';
import assert from 'node:assert/strict';

const listeners = {};
window.addEventListener = (type, fn) => { (listeners[type] ||= []).push(fn); };
const fire = (type) => { for (const fn of listeners[type] || []) fn({ type }); };

const sent = [];
globalThis.fetch = async (url, options = {}) => {
  if (String(url).includes('/api/greeter-look')) {
    sent.push(JSON.parse(options.body));
    return new Response(JSON.stringify({ success: true, published: true }), {
      status: 200, headers: { 'Content-Type': 'application/json' },
    });
  }
  return new Response('{}', { status: 404, headers: { 'Content-Type': 'application/json' } });
};

const { setAuth } = await import('../api.js');
const { currentLook, initGreeterLook } = await import('../greeterLook.js');
const { latestLook } = await import('../gate.js');
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

assert.equal(currentLook(), null, 'no one signed in, nothing to publish');

setAuth(null, { id: 'u1' });
localStorage.setItem('ui.theme.name', 'ink');
localStorage.setItem('ui.accent.u1', '#4f7cff');
localStorage.setItem('ui.gradient.u1', JSON.stringify({ id: 'aurora', angle: 135, stops: ['#112233', '#445566'] }));
localStorage.setItem('desktop.surface.u1', JSON.stringify({ motion: 'fluid' }));
// Another profile's choice on the same device is not this account's look.
localStorage.setItem('ui.accent.u2', '#ff0000');

assert.deepEqual(currentLook(), {
  theme: 'ink',
  accent: '#4f7cff',
  gradient: { id: 'aurora', angle: 135, stops: ['#112233', '#445566'] },
  motion: 'fluid',
});

initGreeterLook();
await wait(900);
assert.equal(sent.length, 1, 'published once the session starts');
assert.equal(sent[0].accent, '#4f7cff');

fire('appearance:change');
await wait(900);
assert.equal(sent.length, 1, 'an unchanged look is not sent again');

localStorage.setItem('desktop.surface.u1', JSON.stringify({ motion: 'depth' }));
fire('desktop:surface');
fire('theme:change');
await wait(900);
assert.equal(sent.length, 2, 'a burst of changes is one publish');
assert.equal(sent[1].motion, 'depth');

assert.equal(latestLook([]), null);
assert.deepEqual(
  latestLook([
    { look: { theme: 'dusk', updated: 10 } },
    { look: null },
    { look: { theme: 'ink', updated: 20 } },
  ]),
  { theme: 'ink', updated: 20 },
  'the login screen opens in the look published last',
);

console.log('greeter look checks passed');

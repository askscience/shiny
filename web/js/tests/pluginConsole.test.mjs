/**
 * pluginConsole.js: which errors belong to a plugin's window, and what is
 * sent for them.
 *
 * Run:  node web/js/tests/pluginConsole.test.mjs
 */
import './helpers/i18n.mjs';

let failures = 0;
function check(name, condition, detail = '') {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

const sent = [];
const listeners = {};
Object.defineProperty(globalThis, 'window', {
  configurable: true,
  writable: true,
  value: { ...globalThis.window, addEventListener: (type, fn) => { listeners[type] = fn; }, dispatchEvent: () => true },
});
Object.defineProperty(globalThis, 'fetch', {
  configurable: true,
  writable: true,
  value: async (url, init) => {
    sent.push({ url: String(url), body: JSON.parse(init?.body || '{}') });
    return { ok: true, status: 200, json: async () => ({ success: true }), text: async () => '{"success":true}', headers: { get: () => 'application/json' } };
  },
});

const { pluginOf, reportPluginError, initPluginConsole } = await import('../pluginConsole.js');
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

check('a module URL names its plugin and file',
  JSON.stringify(pluginOf('http://localhost:8080/plugins/notes/plugin.js?v=abc')) === '{"plugin":"notes","source":"plugin.js"}');
check('a file in a subfolder keeps its path', pluginOf('/plugins/my-notes/lib/list.js')?.source === 'lib/list.js');
check('a stack trace is searched too',
  pluginOf('TypeError: x\n    at render (http://h/plugins/tasks/plugin.js:41:7)')?.plugin === 'tasks');
check('a core module is nobody\'s plugin', pluginOf('http://localhost:8080/js/tiles.js') === null && pluginOf('') === null);

initPluginConsole();
listeners.error({ filename: 'http://h/plugins/notes/plugin.js', message: 'items is not defined', lineno: 41 });
listeners.error({ filename: 'http://h/js/app.js', message: 'core trouble', lineno: 3 });
listeners.unhandledrejection({ reason: { message: 'load failed', stack: 'Error: load failed\n at http://h/plugins/tasks/plugin.js:9:1' } });
listeners.unhandledrejection({ reason: 'plain text' });
await flush();
check('an error in a plugin file is reported with its place',
  sent.length === 2 && sent[0].url.endsWith('/api/plugins/dev/console')
    && sent[0].body.plugin === 'notes' && sent[0].body.message === 'items is not defined'
    && sent[0].body.source === 'plugin.js' && sent[0].body.line === 41, JSON.stringify(sent));
check('a rejected promise from a plugin is reported', sent[1]?.body.plugin === 'tasks' && sent[1]?.body.message === 'load failed');

reportPluginError('notes', 'x'.repeat(5000), 'plugin.js', 'not a number');
await flush();
check('a report is bounded', sent[2]?.body.message.length === 600 && sent[2]?.body.line === 0);
reportPluginError('', 'no plugin');
await flush();
check('nothing is sent without a plugin', sent.length === 3);

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall pluginConsole checks passed');

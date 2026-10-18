import './helpers/i18n.mjs';
import assert from 'node:assert/strict';
import { extractAfterWake } from '../voice/wake.js';
import { floatTo16BitPCM, acquireMicrophone } from '../voice/capture.js';
assert.equal(extractAfterWake('hey peak d open files', "Peak'd"), 'open files');
assert.equal(extractAfterWake('hello unrelated sentence', "Peak'd"), null);
assert.equal(extractAfterWake('hey shiny', 'Shiny'), '');
assert.deepEqual([...floatTo16BitPCM(new Float32Array([-2, -1, 0, 1, 2]))], [-32768, -32768, 0, 32767, 32767]);

// Import preferences without applying a desktop surface; playback gets a
// small event/audio harness rather than a live microphone or real provider.
delete globalThis.document;
window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
window.addEventListener = () => {};
window.removeEventListener = () => {};
const { createVoicePlayback } = await import('../voice/playback.js');
const classes = new Set();
globalThis.document = { body: { classList: { add: (x) => classes.add(x), remove: (x) => classes.delete(x) } } };
window.addEventListener = () => {};
window.removeEventListener = () => {};
globalThis.CustomEvent = class { constructor(type) { this.type = type; } };
let created = 0;
let revoked = 0;
URL.createObjectURL = () => { created++; return `blob:${created}`; };
URL.revokeObjectURL = () => revoked++;
let audio;
globalThis.Audio = class {
  constructor() { audio = this; }
  play() { return Promise.resolve(); }
  pause() {}
};
let finishRequest;
let signal;
const playback = createVoicePlayback({ request: (_path, options) => {
  signal = options.signal;
  return new Promise((resolve) => { finishRequest = resolve; });
} });
const pending = playback.speak('cancel while fetching');
playback.stopSpeaking();
assert.equal(signal.aborted, true);
finishRequest(new Blob(['audio']));
await pending;
assert.equal(created, 0, 'cancelled synthesis never starts audio');

const playing = playback.speak('play then stop');
finishRequest(new Blob(['audio']));
await new Promise((resolve) => setImmediate(resolve));
assert.equal(classes.has('orb-speaking'), true);
assert.equal(playback.stopSpeaking(), true);
await playing;
assert.equal(revoked, 1, 'stopped playback releases its object URL');

const failure = playback.speak('fail to play');
finishRequest(new Blob(['audio']));
await new Promise((resolve) => setImmediate(resolve));
const originalWarn = console.warn;
console.warn = () => {};
audio.onerror();
await failure;
console.warn = originalWarn;
assert.equal(created, revoked, 'failed playback also releases its object URL');
assert.equal(classes.has('orb-speaking'), false);

let releaseMicrophone;
let tracksStopped = 0;
navigator.mediaDevices = { getUserMedia: () => new Promise((resolve) => { releaseMicrophone = resolve; }) };
const monitoring = playback.startBargeInMonitor();
playback.stopBargeInMonitor();
releaseMicrophone({ getTracks: () => [{ stop: () => tracksStopped++ }] });
await monitoring;
assert.equal(tracksStopped, 1, 'a microphone granted after cancellation is released');
assert.equal(playback.isBargeInMonitoring(), false);
console.log('Voice lifecycle checks passed');

let grant;
let generation = 1;
let staleTracksStopped = 0;
const oldGrant = acquireMicrophone({ getUserMedia: () => new Promise((resolve) => { grant = resolve; }) }, {}, () => generation === 1);
generation++;
const newer = { getTracks: () => [] };
assert.equal(await acquireMicrophone({ getUserMedia: async () => newer }, {}, () => generation === 2), newer);
grant({ getTracks: () => [{ stop: () => staleTracksStopped++ }] });
assert.equal(await oldGrant, null);
assert.equal(staleTracksStopped, 1, 'a stale permission grant is released without replacing the new stream');

// A rejection from an old permission prompt must not stop a new monitor.
let rejectOld;
let grantNew;
let requested = 0;
navigator.mediaDevices.getUserMedia = () => new Promise((resolve, reject) => {
  if (requested++ === 0) rejectOld = reject;
  else grantNew = resolve;
});
window.AudioContext = class {
  constructor() { this.destination = {}; }
  createMediaStreamSource() { return { connect() {}, disconnect() {} }; }
  createScriptProcessor() { return { connect() {}, disconnect() {} }; }
  close() {}
};
const staleMonitor = playback.startBargeInMonitor();
playback.stopBargeInMonitor();
const freshMonitor = playback.startBargeInMonitor();
let freshStopped = 0;
grantNew({ getTracks: () => [{ stop: () => freshStopped++ }] });
await freshMonitor;
rejectOld(new Error('old prompt denied'));
await staleMonitor;
assert.equal(playback.isBargeInMonitoring(), true);
assert.equal(freshStopped, 0);
playback.stopBargeInMonitor();
assert.equal(freshStopped, 1);

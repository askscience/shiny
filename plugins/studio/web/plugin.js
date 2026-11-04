import {
  confirmDialog, slider as uiSlider, toolbar, dial, element as h, input as uiInput,
  selectControl, controlButton, toast, icon, setIcon, button, searchBar, setTileGlow,
  glowGradient, toolButton, segmented, listReveal, decodeText, eyebrow,
} from '../../ui/index.js';

/**
 * studio.js — the Studio plugin's window (DAW engine + Bitwig-style shell).
 *
 * The window is catalog-driven: on mount it fetches /api/studio/catalog and
 * rebuilds its instrument/effect/MIDI-FX/master tables from the engine's own
 * descriptions, so new parameters and modules appear here automatically.
 *
 * Bitwig Studio-style layout:
 *   • Header — the Browser menu (left), project title, transport (stop / play /
 *     loop / metronome) + position + BPM, view toggles, WAV export, save, status.
 *   • Body — Arranger (timeline: ruler, track lanes, clips, playhead) and
 *     Clip Launcher (tracks × scenes grid of looping clip slots) side by
 *     side; each is toggled from the header.
 *   • Detail panel — selection-driven bottom panel with three pages:
 *     Editor (step grid / piano roll / drum pads), Devices (instrument →
 *     FX → output chain + macros + master FX), Mixer (channel strips).
 *   • Footer — context hints + live parameter readout.
 *   • Browser — floating pop-up (search + Instruments / Effects / Presets /
 *     Patterns / Arrangements) opened from the header.
 *
 * All audio renders through the plugin's REST API (the Rust DSP engine); playback
 * is WebAudio. Launcher clips loop and launch quantized to the next bar.
 */

import { apiFetch } from '../../js/api.js';
import { saveOrDownload } from '../../js/files.js';
import { t, tp } from '../../js/i18n.js';

export const STUDIO_PLUGIN = 'studio';

const KINDS = ['kick', 'snare', 'hat', 'clap', 'tom', 'perc', 'bass', 'pluck', 'lead', 'pad', 'sub', 'organ', 'ep', 'bell', 'strings', 'brass', 'synthme', 'grid', 'drumkit'];
const KIND_LABELS = { kick: 'Kick', snare: 'Snare', hat: 'Hat', clap: 'Clap', tom: 'Tom', perc: 'Perc', rim: 'Rim', cowbell: 'Cowbell', shaker: 'Shaker', crash: 'Crash', ride: 'Ride', bass: 'Bass', pluck: 'Pluck', lead: 'Lead', pad: 'Pad', sub: 'Sub', organ: 'Organ', ep: 'E-Piano', bell: 'Bell', strings: 'Strings', brass: 'Brass', synthme: 'SynthMe', fm: 'FM', grid: 'WaveMe', drumkit: 'Drum Machine' };
const KIND_INITIALS = { kick: 'K', snare: 'S', hat: 'H', clap: 'C', tom: 'T', perc: 'P', rim: 'R', cowbell: 'CB', shaker: 'Sh', crash: 'Cr', ride: 'Rd', bass: 'B', pluck: 'P', lead: 'L', pad: 'P', sub: 'S', organ: 'O', ep: 'EP', bell: 'B', strings: 'St', brass: 'Br', synthme: 'SY', fm: 'FM', grid: 'WM', drumkit: 'DM' };
const TUNINGS = [['edo12', '12-TET'], ['edo19', '19-TET'], ['ji7', 'Just 7']];
const SWING_OPTS = [['0', 'plugin.studio.swing.straight'], ['0.34', 'plugin.studio.swing.light'], ['0.67', 'plugin.studio.swing.medium'], ['1', 'plugin.studio.swing.triplet']];
const WAVES = [['sine', 'plugin.studio.wave.sine'], ['triangle', 'plugin.studio.wave.triangle'], ['saw', 'plugin.studio.wave.saw'], ['square', 'plugin.studio.wave.square'], ['pulse', 'plugin.studio.wave.pulse']];
const TRACK_COLORS = ['#ff5d5d', '#ffb454', '#ffe156', '#8dff9e', '#57d9ff', '#7aa2ff', '#c792ff', '#ff8fd8'];
const MELODIC = new Set(['bass', 'pluck', 'lead', 'pad', 'sub', 'organ', 'ep', 'bell', 'strings', 'brass', 'synthme', 'grid']);

// Mirrors `voices::default_level` in the Rust engine — clap 0.5, sub 0.7,
// drumkit 0.7 (the old JS table disagreed, so the UI prefilled levels the
// renderer never used).
const DEFAULT_LEVEL = { kick: 0.9, snare: 0.75, hat: 0.45, clap: 0.5, tom: 0.7, perc: 0.4, bass: 0.7, pluck: 0.5, lead: 0.55, pad: 0.5, sub: 0.7, organ: 0.5, ep: 0.5, bell: 0.5, strings: 0.6, brass: 0.6, synthme: 0.6, grid: 0.6, drumkit: 0.7 };
const DEFAULT_PAN = { hat: 0.35, lead: -0.25, snare: 0.05, kick: -0.05, bass: 0, pluck: 0, organ: 0.1, ep: -0.1, strings: 0.15, brass: 0.1 };

let PPB = 56;          // px per beat (arranger zoom, 4…240)
const PPB_MIN = 4;
const PPB_MAX = 240;
let viewExtraBeats = 0;  // extra room added as the user scrolls right
let arrScrollEl = null;
const HEAD_W = 168;    // track header column width
const RULER_H = 26;    // ruler row height
const TRACK_H = 52;    // track lane height
const AUTO_H = 34;     // automation lane height

/* Synth parameter catalog — fallback mirror of the Rust DSP catalog (the live
 * `/api/studio/catalog` is preferred when available). */
const SYNTH = {
  kick: [
    { key: 'pitch', label: 'Pitch', min: 20, max: 200, step: 1, def: 50 },
    { key: 'decay', label: 'Decay', min: 2, max: 30, step: 0.1, def: 8 },
    { key: 'sweep', label: 'Sweep', min: 5, max: 80, step: 0.5, def: 30 },
  ],
  snare: [
    { key: 'tone', label: 'Tone', min: 80, max: 400, step: 1, def: 200 },
    { key: 'body', label: 'Body', min: 5, max: 60, step: 0.1, def: 25 },
    { key: 'noise', label: 'Noise', min: 5, max: 40, step: 0.1, def: 15 },
  ],
  hat: [
    { key: 'decay', label: 'Decay', min: 10, max: 100, step: 0.5, def: 40 },
  ],
  clap: [
    { key: 'tone', label: 'Tone', min: 80, max: 400, step: 1, def: 180 },
    { key: 'body', label: 'Body', min: 5, max: 60, step: 0.1, def: 10 },
    { key: 'noise', label: 'Noise', min: 5, max: 40, step: 0.1, def: 35 },
  ],
  tom: [
    { key: 'pitch', label: 'Pitch', min: 20, max: 400, step: 1, def: 150 },
    { key: 'decay', label: 'Decay', min: 2, max: 40, step: 0.1, def: 20 },
    { key: 'sweep', label: 'Sweep', min: 5, max: 100, step: 0.5, def: 50 },
  ],
  perc: [
    { key: 'decay', label: 'Decay', min: 10, max: 200, step: 0.5, def: 60 },
  ],
  bass: [
    { key: 'detune', label: 'Detune', min: -24, max: 24, step: 0.1, def: 0 },
    { key: 'cutoff', label: 'Cutoff', min: 20, max: 20000, step: 10, def: 700 },
    { key: 'resonance', label: 'Res', min: 0.1, max: 20, step: 0.1, def: 0.9 },
    { key: 'attack', label: 'Attack', min: 0.001, max: 5, step: 0.005, def: 0.004 },
    { key: 'decay', label: 'Decay', min: 0.001, max: 5, step: 0.01, def: 0.12 },
    { key: 'sustain', label: 'Sustain', min: 0, max: 1, step: 0.05, def: 0.5 },
    { key: 'release', label: 'Release', min: 0.001, max: 5, step: 0.01, def: 0.12 },
  ],
  pluck: [
    { key: 'detune', label: 'Detune', min: -24, max: 24, step: 0.1, def: 0.1 },
    { key: 'osc_mix', label: 'Osc Mix', min: 0, max: 1, step: 0.05, def: 0.5 },
    { key: 'cutoff', label: 'Cutoff', min: 20, max: 20000, step: 10, def: 2000 },
    { key: 'resonance', label: 'Res', min: 0.1, max: 20, step: 0.1, def: 1.5 },
    { key: 'attack', label: 'Attack', min: 0.001, max: 5, step: 0.005, def: 0.005 },
    { key: 'decay', label: 'Decay', min: 0.001, max: 5, step: 0.01, def: 0.2 },
    { key: 'sustain', label: 'Sustain', min: 0, max: 1, step: 0.05, def: 0.6 },
    { key: 'release', label: 'Release', min: 0.001, max: 5, step: 0.01, def: 0.3 },
  ],
  lead: [
    { key: 'detune', label: 'Detune', min: -24, max: 24, step: 0.1, def: 0.1 },
    { key: 'osc_mix', label: 'Osc Mix', min: 0, max: 1, step: 0.05, def: 0.52 },
    { key: 'wt_mix', label: 'WT Mix', min: 0, max: 1, step: 0.05, def: 0.88 },
    { key: 'wt_shape', label: 'WT Shape', min: 0, max: 3, step: 0.05, def: 1.4 },
    { key: 'cutoff', label: 'Cutoff', min: 20, max: 20000, step: 10, def: 2800 },
    { key: 'resonance', label: 'Res', min: 0.1, max: 20, step: 0.1, def: 1.65 },
    { key: 'lfo_rate', label: 'LFO Rate', min: 0.01, max: 50, step: 0.01, def: 0.28 },
    { key: 'lfo_depth', label: 'LFO Depth', min: 0, max: 2000, step: 1, def: 520 },
    { key: 'attack', label: 'Attack', min: 0.001, max: 5, step: 0.005, def: 0.004 },
    { key: 'decay', label: 'Decay', min: 0.001, max: 5, step: 0.01, def: 0.18 },
    { key: 'sustain', label: 'Sustain', min: 0, max: 1, step: 0.05, def: 0.55 },
    { key: 'release', label: 'Release', min: 0.001, max: 5, step: 0.01, def: 0.22 },
  ],
  pad: [
    { key: 'detune', label: 'Detune', min: -24, max: 24, step: 0.1, def: 0.4 },
    { key: 'cutoff', label: 'Cutoff', min: 20, max: 20000, step: 10, def: 1200 },
    { key: 'resonance', label: 'Res', min: 0.1, max: 20, step: 0.1, def: 0.8 },
    { key: 'attack', label: 'Attack', min: 0.001, max: 5, step: 0.005, def: 0.4 },
    { key: 'decay', label: 'Decay', min: 0.001, max: 5, step: 0.01, def: 0.5 },
    { key: 'sustain', label: 'Sustain', min: 0, max: 1, step: 0.05, def: 0.8 },
    { key: 'release', label: 'Release', min: 0.001, max: 5, step: 0.01, def: 0.9 },
  ],
  sub: [
    { key: 'drive', label: 'Drive', min: 0.25, max: 24, step: 0.05, def: 3 },
    { key: 'attack', label: 'Attack', min: 0.001, max: 5, step: 0.005, def: 0.002 },
    { key: 'decay', label: 'Decay', min: 0.001, max: 5, step: 0.01, def: 0.3 },
    { key: 'sustain', label: 'Sustain', min: 0, max: 1, step: 0.05, def: 0.7 },
    { key: 'release', label: 'Release', min: 0.001, max: 5, step: 0.01, def: 0.2 },
  ],
  organ: [
    { key: 'detune', label: 'Detune', min: -24, max: 24, step: 0.1, def: 12 },
    { key: 'osc_mix', label: 'Drawbar', min: 0, max: 1, step: 0.05, def: 0.4 },
    { key: 'attack', label: 'Attack', min: 0.001, max: 5, step: 0.005, def: 0.01 },
    { key: 'release', label: 'Release', min: 0.001, max: 5, step: 0.01, def: 0.15 },
  ],
  ep: [
    { key: 'detune', label: 'Detune', min: -24, max: 24, step: 0.1, def: 12 },
    { key: 'osc_mix', label: 'Tine Mix', min: 0, max: 1, step: 0.05, def: 0.7 },
    { key: 'cutoff', label: 'Cutoff', min: 20, max: 20000, step: 10, def: 3000 },
    { key: 'resonance', label: 'Res', min: 0.1, max: 20, step: 0.1, def: 1 },
    { key: 'decay', label: 'Decay', min: 0.001, max: 5, step: 0.01, def: 0.4 },
    { key: 'release', label: 'Release', min: 0.001, max: 5, step: 0.01, def: 0.3 },
  ],
  bell: [
    { key: 'detune', label: 'Detune', min: -24, max: 24, step: 0.1, def: 7 },
    { key: 'osc_mix', label: 'Partial', min: 0, max: 1, step: 0.05, def: 0.5 },
    { key: 'decay', label: 'Decay', min: 0.001, max: 5, step: 0.01, def: 1.2 },
    { key: 'release', label: 'Release', min: 0.001, max: 5, step: 0.01, def: 1.5 },
  ],
  strings: [
    { key: 'detune', label: 'Detune', min: -24, max: 24, step: 0.1, def: 0.12 },
    { key: 'cutoff', label: 'Cutoff', min: 20, max: 20000, step: 10, def: 1800 },
    { key: 'attack', label: 'Attack', min: 0.001, max: 5, step: 0.005, def: 0.8 },
    { key: 'sustain', label: 'Sustain', min: 0, max: 1, step: 0.05, def: 0.8 },
    { key: 'release', label: 'Release', min: 0.001, max: 5, step: 0.01, def: 1.2 },
  ],
  brass: [
    { key: 'detune', label: 'Detune', min: -24, max: 24, step: 0.1, def: -0.06 },
    { key: 'cutoff', label: 'Cutoff', min: 20, max: 20000, step: 10, def: 1200 },
    { key: 'resonance', label: 'Res', min: 0.1, max: 20, step: 0.1, def: 2 },
    { key: 'attack', label: 'Attack', min: 0.001, max: 5, step: 0.005, def: 0.05 },
    { key: 'sustain', label: 'Sustain', min: 0, max: 1, step: 0.05, def: 0.7 },
    { key: 'release', label: 'Release', min: 0.001, max: 5, step: 0.01, def: 0.3 },
  ],
  synthme: [
    { key: 'o1w', label: 'Osc 1', min: 0, max: 3, step: 1, def: 2 },
    { key: 'o2w', label: 'Osc 2', min: 0, max: 3, step: 1, def: 3 },
    { key: 'detune', label: 'Detune', min: -24, max: 24, step: 0.1, def: 0.1 },
    { key: 'mix', label: 'Osc Mix', min: 0, max: 1, step: 0.05, def: 0.5 },
    { key: 'noise', label: 'Noise', min: 0, max: 1, step: 0.05, def: 0 },
    { key: 'ftype', label: 'Filter', min: 0, max: 2, step: 1, def: 0 },
    { key: 'cutoff', label: 'Cutoff', min: 20, max: 20000, step: 10, def: 2000 },
    { key: 'res', label: 'Res', min: 0.1, max: 20, step: 0.1, def: 1 },
    { key: 'drive', label: 'Drive', min: 0.25, max: 24, step: 0.05, def: 2 },
    { key: 'attack', label: 'Attack', min: 0.001, max: 5, step: 0.005, def: 0.005 },
    { key: 'decay', label: 'Decay', min: 0.001, max: 5, step: 0.01, def: 0.2 },
    { key: 'sustain', label: 'Sustain', min: 0, max: 1, step: 0.05, def: 0.6 },
    { key: 'release', label: 'Release', min: 0.001, max: 5, step: 0.01, def: 0.3 },
  ],
};

const FX = [
  { key: 'delay_mix', label: 'Delay Mix', min: 0, max: 1, step: 0.01, def: 0 },
  { key: 'delay_time', label: 'Delay Time', min: 1, max: 2000, step: 1, def: 250 },
  { key: 'feedback', label: 'Feedback', min: 0, max: 0.95, step: 0.01, def: 0.4 },
  { key: 'reverb_mix', label: 'Reverb Mix', min: 0, max: 1, step: 0.01, def: 0 },
  { key: 'reverb_size', label: 'Reverb Size', min: 0, max: 1, step: 0.01, def: 0.5 },
  { key: 'reverb_damp', label: 'Reverb Damp', min: 0, max: 1, step: 0.01, def: 0.5 },
];

const PARAM_GROUP = {
  detune: 'Oscillator', osc_mix: 'Oscillator', wt_mix: 'Oscillator', wt_shape: 'Oscillator',
  o1w: 'Oscillator', o2w: 'Oscillator', mix: 'Oscillator', noise: 'Oscillator',
  cutoff: 'Filter', resonance: 'Filter', lfo_rate: 'Filter', lfo_depth: 'Filter', ftype: 'Filter', res: 'Filter',
  attack: 'Envelope', decay: 'Envelope', sustain: 'Envelope', release: 'Envelope',
  drive: 'Drive',
  pitch: 'Drum', sweep: 'Drum', tone: 'Drum', body: 'Drum',
};
const GROUP_ORDER = ['Oscillator', 'Filter', 'Drive', 'Envelope', 'Drum'];

const KIND_DESC = {
  kick: 'plugin.studio.kindDesc.kick', snare: 'plugin.studio.kindDesc.snare', hat: 'plugin.studio.kindDesc.hat', clap: 'plugin.studio.kindDesc.clap', tom: 'plugin.studio.kindDesc.tom', perc: 'plugin.studio.kindDesc.perc',
  bass: 'plugin.studio.kindDesc.bass', pluck: 'plugin.studio.kindDesc.pluck', lead: 'plugin.studio.kindDesc.lead', pad: 'plugin.studio.kindDesc.pad', sub: 'plugin.studio.kindDesc.sub',
  organ: 'plugin.studio.kindDesc.organ', ep: 'plugin.studio.kindDesc.ep', bell: 'plugin.studio.kindDesc.bell', strings: 'plugin.studio.kindDesc.strings', brass: 'plugin.studio.kindDesc.brass',
  synthme: 'plugin.studio.kindDesc.synthme',
  grid: 'plugin.studio.kindDesc.grid',
  drumkit: 'plugin.studio.kindDesc.drumkit',
};

const EFFECTS = {
  distortion: { label: 'Distortion', params: [
    { key: 'mode', label: 'Mode', min: 0, max: 4, step: 1, def: 0 },
    { key: 'drive', label: 'Drive', min: 0.25, max: 24, step: 0.05, def: 2 },
    { key: 'mix', label: 'Mix', min: 0, max: 1, step: 0.01, def: 0.5 },
    { key: 'out', label: 'Out', min: 0.1, max: 3, step: 0.05, def: 1 },
  ] },
  filter: { label: 'Filter', params: [
    { key: 'type', label: 'Type', min: 0, max: 2, step: 1, def: 0 },
    { key: 'cutoff', label: 'Cutoff', min: 20, max: 20000, step: 10, def: 2000 },
    { key: 'resonance', label: 'Res', min: 0.1, max: 20, step: 0.1, def: 1 },
  ] },
  eq: { label: 'EQ', params: [
    { key: 'low_gain', label: 'Lo Gain', min: -24, max: 24, step: 0.5, def: 0 },
    { key: 'mid_freq', label: 'Mid Freq', min: 20, max: 20000, step: 10, def: 1000 },
    { key: 'mid_gain', label: 'Mid Gain', min: -24, max: 24, step: 0.5, def: 0 },
    { key: 'hi_gain', label: 'Hi Gain', min: -24, max: 24, step: 0.5, def: 0 },
  ] },
  compressor: { label: 'Compressor', params: [
    { key: 'threshold', label: 'Threshold', min: -60, max: 0, step: 0.5, def: -18 },
    { key: 'ratio', label: 'Ratio', min: 1, max: 20, step: 0.1, def: 4 },
    { key: 'attack', label: 'Attack', min: 0.1, max: 200, step: 0.5, def: 10 },
    { key: 'release', label: 'Release', min: 1, max: 2000, step: 1, def: 150 },
    { key: 'makeup', label: 'Makeup', min: 0, max: 30, step: 0.5, def: 0 },
  ] },
  delay: { label: 'Delay', params: [
    { key: 'time', label: 'Time', min: 1, max: 2000, step: 1, def: 250 },
    { key: 'feedback', label: 'Feedback', min: 0, max: 0.95, step: 0.01, def: 0.4 },
    { key: 'mix', label: 'Mix', min: 0, max: 1, step: 0.01, def: 0.3 },
  ] },
  reverb: { label: 'Reverb', params: [
    { key: 'size', label: 'Size', min: 0, max: 1, step: 0.01, def: 0.5 },
    { key: 'damping', label: 'Damping', min: 0, max: 1, step: 0.01, def: 0.5 },
    { key: 'mix', label: 'Mix', min: 0, max: 1, step: 0.01, def: 0.2 },
  ] },
};
const EFFECT_KINDS = Object.keys(EFFECTS);

/* MIDI (note-processing) effects applied before synthesis. */
const MIDI_FX = {
  transpose: { label: 'Transpose', params: [
    { key: 'steps', label: 'Steps', min: -24, max: 24, step: 1, def: 0 },
  ] },
  velocity: { label: 'Velocity', params: [
    { key: 'amount', label: 'Amount', min: 0, max: 1, step: 0.01, def: 1 },
  ] },
  gate: { label: 'Gate', params: [
    { key: 'amount', label: 'Amount', min: 0.1, max: 2, step: 0.01, def: 1 },
  ] },
  ratchet: { label: 'Ratchet', params: [
    { key: 'count', label: 'Count', min: 2, max: 8, step: 1, def: 2 },
  ] },
};
const MIDI_FX_KINDS = Object.keys(MIDI_FX);

const FACTORY_PRESETS = {
  kick: [
    { name: '808', params: { synth: { pitch: 45, decay: 12, sweep: 20 } } },
    { name: 'Punch', params: { synth: { pitch: 80, decay: 6, sweep: 40 } } },
    { name: 'Deep', params: { synth: { pitch: 35, decay: 20, sweep: 25 } } },
  ],
  snare: [
    { name: 'Crisp', params: { synth: { tone: 240, body: 20, noise: 18 } } },
    { name: 'Fat', params: { synth: { tone: 160, body: 45, noise: 10 } } },
  ],
  hat: [
    { name: 'Tight', params: { synth: { decay: 20 } } },
    { name: 'Open', params: { synth: { decay: 80 } } },
  ],
  clap: [
    { name: 'Clap', params: { synth: { tone: 180, body: 10, noise: 35 } } },
  ],
  tom: [
    { name: 'Tom', params: { synth: { pitch: 150, decay: 20, sweep: 50 } } },
  ],
  perc: [
    { name: 'Perc', params: { synth: { decay: 60 } } },
  ],
  bass: [
    { name: 'Deep Sub', params: { wave: 'sine', synth: { cutoff: 400, resonance: 0.5, decay: 0.2, sustain: 0.6 } } },
    { name: 'Acid Square', params: { wave: 'square', synth: { cutoff: 900, resonance: 4, decay: 0.1, sustain: 0.4 } } },
    { name: 'Triangle Pluck', params: { wave: 'triangle', synth: { cutoff: 1200, resonance: 1, attack: 0.002, decay: 0.15, sustain: 0.2, release: 0.1 } } },
  ],
  pluck: [
    { name: 'Soft Pluck', params: { synth: { cutoff: 1800, resonance: 1.2, attack: 0.003, decay: 0.2, sustain: 0.1, release: 0.3 } } },
    { name: 'Bright Pluck', params: { synth: { cutoff: 6000, resonance: 2, attack: 0.002, decay: 0.15, sustain: 0, release: 0.25 } } },
  ],
  lead: [
    { name: 'Warm Saw', params: { synth: { cutoff: 2200, resonance: 1.2, detune: 0.2, attack: 0.01, decay: 0.3, sustain: 0.6, release: 0.4 } } },
    { name: 'Plucky Keys', params: { synth: { cutoff: 5200, resonance: 1.5, wt_mix: 0.3, attack: 0.002, decay: 0.12, sustain: 0, release: 0.18 } } },
    { name: 'Big Lead', params: { synth: { cutoff: 3000, resonance: 3, lfo_depth: 800, lfo_rate: 0.5, sustain: 0.7 } } },
  ],
  pad: [
    { name: 'Warm Pad', params: { synth: { cutoff: 1200, resonance: 0.8, attack: 0.6, release: 1.2 } } },
    { name: 'Bright Pad', params: { synth: { cutoff: 3200, resonance: 1.5, detune: 0.4, attack: 0.2, release: 0.8 } } },
  ],
  sub: [
    { name: 'Clean Sub', params: { synth: { drive: 3, attack: 0.002, sustain: 0.7 } } },
    { name: 'Dirty Sub', params: { synth: { drive: 10, attack: 0.001, sustain: 0.6 } } },
  ],
  organ: [
    { name: 'Full Drawbar', params: { synth: { detune: 12, osc_mix: 0.55, attack: 0.01, release: 0.15 } } },
    { name: 'Soft', params: { synth: { detune: 12, osc_mix: 0.25, attack: 0.02, release: 0.2 } } },
    { name: 'Percussive', params: { synth: { detune: 19, osc_mix: 0.4, attack: 0.002, release: 0.1 } } },
  ],
  ep: [
    { name: 'Rhodes', params: { synth: { osc_mix: 0.7, cutoff: 3000, resonance: 1, decay: 0.4, release: 0.3 } } },
    { name: 'Bell Tine', params: { synth: { osc_mix: 0.5, cutoff: 5000, resonance: 1.5, decay: 0.6, release: 0.4 } } },
  ],
  bell: [
    { name: 'FM Bell', params: { synth: { detune: 7, osc_mix: 0.5, decay: 1.2, release: 1.5 } } },
    { name: 'Glass', params: { synth: { detune: 12, osc_mix: 0.4, decay: 0.8, release: 1.0 } } },
  ],
  strings: [
    { name: 'Warm Strings', params: { synth: { cutoff: 1800, attack: 0.8, sustain: 0.8, release: 1.2 } } },
    { name: 'Bright Ensemble', params: { synth: { detune: 0.4, cutoff: 4000, attack: 0.5, sustain: 0.8, release: 1.0 } } },
  ],
  brass: [
    { name: 'Stab', params: { synth: { cutoff: 1200, resonance: 2, attack: 0.02, sustain: 0.6, release: 0.2 } } },
    { name: 'Swell', params: { synth: { cutoff: 900, resonance: 1.5, attack: 0.3, sustain: 0.8, release: 0.4 } } },
  ],
};

/* ── dom helpers ────────────────────────────────────────────── */

function fmt(n) {
  if (!Number.isFinite(n)) return '0';
  if (Number.isInteger(n)) return String(n);
  return String(Math.round(n * 1000) / 1000);
}

/* ── euclidean / rhythm helpers (mirror the engine) ────── */

function euclid(hits, steps) {
  const p = new Array(steps).fill(false);
  if (steps <= 0 || hits <= 0) return p;
  if (hits >= steps) return p.fill(true);
  for (let i = 0; i < hits; i++) p[Math.floor((i * steps) / hits)] = true;
  return p;
}
function rotateRight(p, offset) {
  const n = p.length;
  if (!n) return [];
  const off = n - (offset % n);
  return [...p.slice(off), ...p.slice(0, off)];
}
function rhythmToCells(rhythm, steps) {
  const r = (rhythm || '').trim();
  const out = new Array(steps).fill(false);
  if (r.startsWith('e')) {
    const m = r.match(/^e(\d+)(?:,(\d+))?$/);
    if (m) return rotateRight(euclid(Math.min(parseInt(m[1], 10) || 0, steps), steps), parseInt(m[2] || '0', 10) || 0);
    return out;
  }
  const chars = r.split('');
  for (let i = 0; i < steps && i < chars.length; i++) out[i] = chars[i] === 'x';
  return out;
}
function cellsToRhythm(cells) {
  return cells.map((b) => (b ? 'x' : '.')).join('');
}
function trackColor(i) {
  return TRACK_COLORS[(i ?? 0) % TRACK_COLORS.length];
}
/* Tier 1 ambient glow: the arrangement's first few track colours (falls back
   to TRACK_COLORS[0] / TRACK_COLORS[4] when there are no tracks). */
function updateStudioGlow() {
  const tracks = arrangement?.tracks || [];
  const a = trackColor(tracks[0]?.color ?? 0);
  const b = trackColor(tracks[2]?.color ?? 4);
  setTileGlow(tileEl, glowGradient(a, b, { x: 18, y: 0, x2: 85, y2: 100 }));
}

const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
// degree 0 resolves to the reference pitch (A4 = 440 Hz = MIDI 69).
function degreeLabel(d, tuning) {
  if (tuning === 'edo12') {
    const midi = d + 69;
    const n = ((midi % 12) + 12) % 12;
    const oct = Math.floor(midi / 12) - 1;
    return `${NOTE_NAMES[n]}${oct}`;
  }
  return String(d);
}
function isBlack(d, tuning) {
  if (tuning !== 'edo12') return false;
  const n = ((d + 69) % 12 + 12) % 12;
  return [1, 3, 6, 8, 10].includes(n);
}
function noteAt(voice, step) {
  return (voice.notes || []).find((n) => n.step <= step && step < n.step + (n.length || 1)) || null;
}
function defaultPads() {
  const K = ['kick', 'snare', 'clap', 'hat', 'tom', 'perc', 'kick', 'snare', 'hat', 'clap', 'tom', 'perc', 'kick', 'hat', 'perc', 'tom'];
  const N = ['Kick 1', 'Snare', 'Clap', 'Hat', 'Tom 1', 'Perc', 'Kick 2', 'Snare 2', 'Hat 2', 'Clap 2', 'Tom 2', 'Perc 2', 'Kick 3', 'Hat 3', 'Perc 3', 'Tom 3'];
  return K.map((k, i) => ({ name: N[i], kind: k }));
}
function drumkitNoteAt(voice, pad, step) {
  return (voice.notes || []).find((n) => n.degree === pad && n.step === step) || null;
}

function voice(kind, rhythm, opts = {}) {
  return { kind, rhythm, degree: 0, octave: 0, wave: 'sine', notes: [], level: null, pan: null, accent: 0, synth: {}, fx: [], macros: [], pads: [], midi: [], grid: null, ...opts };
}
/* Subtle "produced" master bus for new patterns — a touch of plate reverb so
   fresh clips don't sound bone-dry. Users can dial it back in Devices. */
function defaultFx() {
  return { reverb_mix: 0.1, reverb_size: 0.55, reverb_damp: 0.6 };
}
function defaultKit() {
  return [
    voice('kick', 'e4,0'),
    voice('hat', 'e8,2'),
    voice('snare', 'e4,8'),
  ];
}
function kitPattern(steps = 16) {
  return { title: 'Kit', bpm: 120, steps, swing: 0, tuning: 'edo12', voices: defaultKit(), fx: defaultFx() };
}
function bassPattern(steps = 16) {
  return { title: 'Bass', bpm: 120, steps, swing: 0, tuning: 'edo12', voices: [voice('bass', 'x...x...x...x...', { octave: -2, wave: 'triangle' })], fx: defaultFx() };
}
function leadPattern(steps = 16) {
  return { title: 'Lead', bpm: 120, steps, swing: 0, tuning: 'edo12', voices: [voice('lead', 'x.x.x.x.x.x.x.x.', { degree: 4, octave: 0, wave: 'saw' })], fx: defaultFx() };
}
function padPattern(steps = 16) {
  return { title: 'Pad', bpm: 120, steps, swing: 0, tuning: 'edo12', voices: [voice('pad', 'x...x...x...x...', { degree: 0, octave: -1 })], fx: defaultFx() };
}
function simplePattern(kind, steps = 16) {
  if (kind === 'drumkit') {
    const notes = [0, 4, 8, 12].map((s) => ({ step: s, length: 1, degree: 0, octave: 0, velocity: null }));
    return { title: KIND_LABELS[kind], bpm: 120, steps, swing: 0, tuning: 'edo12', voices: [voice('drumkit', '', { pads: defaultPads(), notes })], fx: defaultFx() };
  }
  /* Octave 0 is the A4 octave (440 Hz): basses sit two below, pads and keys
     one below, leads/bells at it. */
  const oct = (kind === 'bass' || kind === 'sub') ? -2 : (['lead', 'bell', 'pluck', 'synthme', 'fm'].includes(kind) ? 0 : -1);
  return { title: KIND_LABELS[kind], bpm: 120, steps, swing: 0, tuning: 'edo12', voices: [voice(kind, 'x.x.x.x.x.x.x.x.', { degree: 0, octave: oct })], fx: defaultFx() };
}

function blankArrangement() {
  // Start completely empty — the AI (or the user) adds every track and clip.
  return {
    id: null,
    title: 'Untitled Arrangement',
    bpm: 120,
    length_beats: 32,
    master: 0.9,
    fx: { ceiling: -1 },
    master_fx: [],
    returns: { reverb: {}, delay: {}, reverb_level: null, delay_level: null },
    tracks: [],
    clips: [],
  };
}
function blankLauncher() {
  return {
    title: 'Untitled Session',
    bpm: 120,
    tracks: [
      { id: 'lt0', name: 'Drums', color: 0, mute: false, level: 0.85, pan: 0 },
      { id: 'lt1', name: 'Bass', color: 3, mute: false, level: 0.7, pan: -0.1 },
      { id: 'lt2', name: 'Lead', color: 4, mute: false, level: 0.55, pan: 0.2 },
      { id: 'lt3', name: 'Pad', color: 5, mute: false, level: 0.5, pan: 0.3 },
    ],
    scenes: 8,
    clips: {
      'lt0:0': { pattern: kitPattern(16), title: 'Kit' },
      'lt1:0': { pattern: bassPattern(16), title: 'Bass' },
      'lt2:0': { pattern: leadPattern(16), title: 'Lead' },
      'lt3:0': { pattern: padPattern(16), title: 'Pad' },
    },
  };
}

function normalizeVoice(v) {
  return {
    kind: KINDS.includes(v?.kind) ? v.kind : 'kick',
    rhythm: typeof v?.rhythm === 'string' ? v.rhythm : 'x...',
    degree: Number.isFinite(v?.degree) ? v.degree : 0,
    octave: Number.isFinite(v?.octave) ? v.octave : 0,
    wave: WAVES.some(([w]) => w === v?.wave) ? v.wave : 'sine',
    notes: Array.isArray(v?.notes) ? v.notes.map((n) => ({ step: n.step, length: n.length || 1, degree: n.degree, octave: n.octave, velocity: Number.isFinite(n?.velocity) ? clamp(n.velocity, 0.05, 1) : null })) : [],
    level: typeof v?.level === 'number' ? v.level : null,
    pan: typeof v?.pan === 'number' ? v.pan : null,
    accent: Number.isFinite(v?.accent) ? clamp(v.accent, 0, 0.6) : 0,
    synth: (v?.synth && typeof v.synth === 'object') ? { ...v.synth } : {},
    fx: Array.isArray(v?.fx) ? v.fx.map((f) => ({
      kind: EFFECT_KINDS.includes(f?.kind) ? f.kind : 'distortion',
      params: (f?.params && typeof f.params === 'object') ? { ...f.params } : {},
      bypass: !!f?.bypass,
    })) : [],
    macros: normalizeMacros(v?.macros),
    pads: Array.isArray(v?.pads) ? v.pads.map((p) => ({ name: p.name || 'Pad', kind: KINDS.includes(p.kind) ? p.kind : 'kick' })) : [],
    midi: normalizeMidiFx(v?.midi),
    soundfont: typeof v?.soundfont === 'string' ? v.soundfont : undefined,
    grid: (v?.grid && Array.isArray(v.grid.modules)) ? { modules: v.grid.modules.map((m) => ({ id: m.id, kind: m.kind, params: { ...(m.params || {}) } })), cables: Array.isArray(v.grid.cables) ? v.grid.cables.map((c) => ({ from: [c.from[0], c.from[1]], to: [c.to[0], c.to[1]] })) : [] } : null,
  };
}
function normalizeMidiFx(m) {
  return Array.isArray(m) ? m.map((x) => ({
    kind: MIDI_FX_KINDS.includes(x?.kind) ? x.kind : 'transpose',
    params: (x?.params && typeof x.params === 'object') ? { ...x.params } : {},
  })) : [];
}
/* Normalize a macro rack into { value, entries: [{path, amount, base}] }. */
function normalizeMacros(m) {
  const src = Array.isArray(m) ? m.slice(0, 8) : [];
  return src.map((e) => ({
    value: Number.isFinite(e?.value) ? e.value : 0.5,
    entries: Array.isArray(e?.entries)
      ? e.entries.map((x) => ({
          path: typeof x?.path === 'string' ? x.path : '',
          amount: Number.isFinite(x?.amount) ? x.amount : 1,
          base: Number.isFinite(x?.base) ? x.base : 0,
        }))
      : [],
  }));
}
function normalizePattern(p) {
  return {
    title: p?.title || 'Clip',
    bpm: p?.bpm ?? 120,
    steps: p?.steps ?? 16,
    swing: Number.isFinite(p?.swing) ? clamp(p.swing, 0, 1) : 0,
    tuning: p?.tuning || 'edo12',
    voices: (Array.isArray(p?.voices) && p.voices.length ? p.voices.map(normalizeVoice) : defaultKit()),
    fx: (p?.fx && typeof p.fx === 'object') ? { ...p.fx } : {},
  };
}
/* Serialize the pattern being edited (config contract with the backend). */
function serializePattern(p) {
  return {
    title: p.title || 'Untitled',
    bpm: p.bpm,
    steps: p.steps,
    swing: p.swing ?? 0,
    tuning: p.tuning,
    voices: p.voices.map((v) => ({
      kind: v.kind, rhythm: v.rhythm, degree: v.degree, octave: v.octave,
      wave: v.wave, notes: v.notes, level: v.level, pan: v.pan, accent: v.accent ?? 0, synth: v.synth,
      fx: v.fx.map((f) => ({ kind: f.kind, params: f.params, bypass: f.bypass })),
      macros: v.macros,
      pads: v.pads,
      midi: v.midi,
      grid: v.grid,
      ...(v.soundfont ? { soundfont: v.soundfont } : {}),
    })),
    fx: p.fx || {},
  };
}
function serializeEffects(list) {
  return (Array.isArray(list) ? list : []).map((f) => ({ kind: f.kind, params: { ...(f.params || {}) }, bypass: !!f.bypass }));
}
function serializeClip(c) {
  const out = { track: c.track, start: c.start, gain_db: c.gain_db || 0, loop: c.loop !== false };
  if (Number.isFinite(c.length_beats)) out.length_beats = c.length_beats;
  if (!c.audio && c.offset_beats > 0) out.offset_beats = c.offset_beats;
  if (c.audio) {
    out.audio = {
      src: c.audio.src, name: c.audio.name || '', offset: c.audio.offset || 0,
      fade_in: c.audio.fade_in || 0, fade_out: c.audio.fade_out || 0,
      ...(Number.isFinite(c.audio.bpm) ? { bpm: c.audio.bpm } : {}),
    };
    out.pattern = { steps: 16, voices: [] };
  } else {
    out.pattern = serializePattern(normalizePattern(c.pattern));
  }
  return out;
}
function arrangementPayload() {
  return {
    title: arrangement.title,
    bpm: arrangement.bpm,
    length_beats: arrangement.length_beats,
    master: arrangement.master ?? 0.9,
    fx: { ...(arrangement.fx || {}) },
    master_fx: serializeEffects(arrangement.master_fx),
    returns: {
      reverb: { ...(arrangement.returns?.reverb || {}) },
      delay: { ...(arrangement.returns?.delay || {}) },
      ...(Number.isFinite(arrangement.returns?.reverb_level) ? { reverb_level: arrangement.returns.reverb_level } : {}),
      ...(Number.isFinite(arrangement.returns?.delay_level) ? { delay_level: arrangement.returns.delay_level } : {}),
    },
    tracks: arrangement.tracks.map((t) => ({
      id: t.id, name: t.name, kind: t.kind || 'instrument', color: t.color, mute: !!t.mute, solo: soloTracks.has(t.id),
      level: t.level, pan: t.pan, fx: serializeEffects(t.fx),
      ...(t.sends ? { sends: { reverb: t.sends.reverb || 0, delay: t.sends.delay || 0 } } : {}),
      automation: t.automation || { lanes: [] },
    })),
    clips: arrangement.clips.map(serializeClip),
  };
}
/* Normalize an arrangement from the server (or the AI) into the window's
   model, keeping every field the engine understands. */
function normalizeArrangement(data) {
  const tracks = (data?.tracks || []).map((t, i) => ({
    id: t.id || `t${Date.now()}${i}`,
    name: t.name || `Track ${i + 1}`,
    kind: t.kind === 'audio' ? 'audio' : 'instrument',
    color: t.color ?? i,
    mute: !!t.mute,
    solo: !!t.solo,
    level: Number.isFinite(t.level) ? t.level : 0.8,
    pan: Number.isFinite(t.pan) ? t.pan : 0,
    fx: normalizeEffects(t.fx),
    sends: t.sends ? { reverb: t.sends.reverb || 0, delay: t.sends.delay || 0 } : null,
    automation: normalizeAutomation(t.automation),
  }));
  const clips = (data?.clips || []).map((c) => {
    const clip = {
      track: c.track,
      start: Number.isFinite(c.start) ? c.start : 0,
      length_beats: Number.isFinite(c.length_beats) ? c.length_beats : null,
      loop: c.loop !== false,
      gain_db: Number.isFinite(c.gain_db) ? c.gain_db : 0,
      offset_beats: Number.isFinite(c.offset_beats) ? Math.max(0, c.offset_beats) : 0,
      audio: null,
      pattern: null,
    };
    if (c.audio && typeof c.audio.src === 'string') {
      clip.audio = {
        src: c.audio.src, name: c.audio.name || c.audio.src.split('/').pop().replace(/\.wav$/i, ''),
        offset: c.audio.offset || 0, fade_in: c.audio.fade_in || 0, fade_out: c.audio.fade_out || 0,
        bpm: Number.isFinite(c.audio.bpm) ? c.audio.bpm : null,
      };
    } else {
      clip.pattern = normalizePattern(c.pattern || {});
    }
    return clip;
  });
  return {
    id: data?.id || null,
    title: data?.title || 'Untitled',
    bpm: data?.bpm ?? 120,
    length_beats: data?.length_beats ?? 32,
    master: data?.master ?? 0.9,
    fx: (data?.fx && typeof data.fx === 'object') ? { ...data.fx } : {},
    master_fx: normalizeEffects(data?.master_fx),
    returns: {
      reverb: { ...(data?.returns?.reverb || {}) },
      delay: { ...(data?.returns?.delay || {}) },
      reverb_level: Number.isFinite(data?.returns?.reverb_level) ? data.returns.reverb_level : null,
      delay_level: Number.isFinite(data?.returns?.delay_level) ? data.returns.delay_level : null,
    },
    tracks,
    clips,
  };
}
function normalizeEffects(list) {
  return Array.isArray(list) ? list.filter((f) => f && typeof f.kind === 'string').map((f) => ({
    kind: f.kind, params: (f.params && typeof f.params === 'object') ? { ...f.params } : {}, bypass: !!f.bypass,
  })) : [];
}
/* Render-only payload: solo/mute logic applied without persisting it. */
function arrangementRenderPayload() {
  const p = arrangementPayload();
  const anySolo = arrangement.tracks.some((t) => soloTracks.has(t.id));
  p.tracks = p.tracks.map((t) => ({ ...t, mute: anySolo ? (!soloTracks.has(t.id) || !!t.mute) : !!t.mute }));
  return p;
}
function trackAudible(tr) {
  const anySolo = launcher.tracks.some((t) => soloTracks.has(t.id)) || arrangement.tracks.some((t) => soloTracks.has(t.id));
  return anySolo ? (soloTracks.has(tr.id) && !tr.mute) : !tr.mute;
}

/* ── state ──────────────────────────────────────────────────── */

let tileEl = null;

let panels = { arranger: true, launcher: false, synthme: false, grid: false };
let detailPage = 'editor';       // 'editor' | 'devices' | 'mixer'
let detailOpen = true;

let arrangement = null;          // { id, title, bpm, length_beats, master, tracks:[], clips:[] }
let launcher = null;             // { title, bpm, tracks:[], scenes, clips:{} }
let arrangements = [];           // saved arrangements (DB)
let presets = [];                // saved user presets (DB)
let tracks = [];                 // saved patterns (DB)

/* Selection — the detail panel edits the selected clip's pattern live. */
let sel = null;                  // { area:'arr', clipIndex } | { area:'lch', trackId, scene }
let current = null;              // live reference to the selected clip's pattern
let selectedVoice = 0;
let selectedPad = 0;
let dirty = false;
let busy = false;

let soloTracks = new Set();      // frontend-only solo state (track ids)

/* audio */
let audioCtx = null;
let arrPlaying = false;
let arrPlayStart = 0;
let arrPlayOffset = 0;           // seconds into the buffer playback began at
let arrLoop = false;
let arrStartBeat = 0;            // ruler-clicked playback start (beats)
let rafId = 0;
let trackLoops = {};             // launcher trackId -> { source, slotKey, when, state }
let trackAudio = {};             // launcher trackId -> { gain, panner }
let clockRunning = false;        // launcher master clock
let clockT0 = 0;
let prevSource = null;           // editor preview
let prevPlaying = false;
let prevStart = 0;
let prevDur = 0;
let prevSig = '';                // pattern signature the editor preview was rendered from
let prevGain = null;              // crossfade gain of the editor preview
let auditionSource = null;       // looping SynthMe / WaveMe preview
let ctxClip = null;               // { index, beat } under the last right-click
let pendingAiTrackId = null;     // AI-created track to drop in once mounted
let lastConsumedAiTrackId = null; // de-dupe re-delivered agent:actions
let metroOn = false;
let metroTimer = 0;
let clockUiTimer = 0;
let beatDotEl = null;
let beatFlashTimer = 0;
let lastClockBeat = -1;
let autoKnobs = [];             // { path, trackId, min, max, dial, set } (automation-follow)

/* dom refs */
let barEl = null;
let timeEl = null;
let titleInput = null;
let bpmInput = null;
let statusEl = null;
let saveDotEl = null;   // saved/unsaved indicator (dot, not text)
let playBtn = null;
let loopBtn = null;
let metroBtn = null;
let viewSeg = null;
let bodyEl = null;
let arrWrapEl = null;
let arrGridEl = null;
let arrPlayheadEl = null;
let lchWrapEl = null;
let lchGridEl = null;
let detailEl = null;
let detailBodyEl = null;
let gridEl = null;
let pianoEl = null;
let editorPageEl = null;
let devicesEl = null;
let mixerEl = null;
let browserEl = null;
let browserBtn = null;          // the left-hand Browser menu button in the bar
let browserListEl = null;
let browserTab = 'patterns';
let browserMode = null;          // null | 'addTrack'
let synthmeWrapEl = null;       // body panel for the SynthMe builder
let synthmeDraft = null;        // { name, synth: {…}, fx: […] }
let gridWrapEl = null;          // body panel for The Grid
let gridCanvasEl = null;
let gridSvgEl = null;
let gridSel = null;             // selected module id
let gridDraft = null;           // { name, modules:[{id,kind,x,y,params}], cables:[] }
let footerHintEl = null;
let footerParamEl = null;
let pageSeg = null;
let collapseBtn = null;

/* ── api ────────────────────────────────────────────────────── */

async function api(path, options = {}) {
  try {
    const res = await apiFetch(path, options);
    if (res && res.success === false) throw new Error(res.error || 'Request failed');
    return res?.data;
  } catch (e) {
    const m = (options.method || 'GET').toUpperCase();
    const st = e.status ? ` (HTTP ${e.status})` : '';
    throw new Error(`${m} ${path}${st} — ${e.message || 'request failed'}`);
  }
}

/* ── engine catalog ──────────────────────────────────────────────
   The engine describes itself at /api/studio/catalog; the UI builds its
   parameter tables from that, so an instrument/effect/module the engine
   learns about shows up here automatically instead of drifting. */

let CATALOG = null;

function catalogParam(p) {
  return {
    key: p.key, label: p.label, min: p.min, max: p.max,
    step: p.step || 0.01, def: p.default,
    choices: Array.isArray(p.choices) ? p.choices : [],
    group: p.group || '', log: !!p.log,
  };
}

function masterFxLabel(key) {
  return key.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase());
}

function applyCatalog(cat) {
  if (!cat) return;
  CATALOG = cat;

  /* kinds */
  if (Array.isArray(cat.kinds) && cat.kinds.length) {
    KINDS.length = 0;
    MELODIC.clear();
    for (const k of cat.kinds) {
      KINDS.push(k.kind);
      if (!KIND_LABELS[k.kind]) KIND_LABELS[k.kind] = k.kind.charAt(0).toUpperCase() + k.kind.slice(1);
      if (!KIND_INITIALS[k.kind]) KIND_INITIALS[k.kind] = k.kind.slice(0, 2).toUpperCase();
      if (k.category === 'synth') MELODIC.add(k.kind);
      if (typeof k.default_level === 'number') DEFAULT_LEVEL[k.kind] = k.default_level;
      if (typeof k.default_pan === 'number') DEFAULT_PAN[k.kind] = k.default_pan;
      SYNTH[k.kind] = (k.params || []).map(catalogParam);
      for (const p of (k.params || [])) PARAM_GROUP[p.key] = p.group || 'Other';
    }
    // drumkit / grid have no parametric face of their own
    for (const k of KINDS) if (!SYNTH[k]) SYNTH[k] = [];
    MELODIC.add('synthme');
    MELODIC.add('fm');
  }

  /* effects */
  if (Array.isArray(cat.effects) && cat.effects.length) {
    EFFECT_KINDS.length = 0;
    for (const e of cat.effects) {
      EFFECT_KINDS.push(e.kind);
      EFFECTS[e.kind] = { label: e.label || e.kind, params: (e.params || []).map(catalogParam), defaults: e.defaults || {} };
    }
  }

  /* MIDI effects */
  if (Array.isArray(cat.midi_fx) && cat.midi_fx.length) {
    MIDI_FX_KINDS.length = 0;
    for (const m of cat.midi_fx) {
      MIDI_FX_KINDS.push(m.kind);
      MIDI_FX[m.kind] = { label: m.kind.charAt(0).toUpperCase() + m.kind.slice(1), params: (m.params || []).map(catalogParam) };
    }
  }

  /* master bus */
  if (Array.isArray(cat.master_fx) && cat.master_fx.length) {
    FX.length = 0;
    for (const m of cat.master_fx) {
      FX.push({
        key: m.key, label: masterFxLabel(m.key), min: m.min, max: m.max,
        step: (m.max - m.min) / 200, def: m.default, help: m.help || '',
      });
    }
  }

  /* tunings */
  if (Array.isArray(cat.tunings) && cat.tunings.length) {
    TUNINGS.length = 0;
    const nice = { edo12: '12-TET', edo19: '19-TET', edo24: '24-TET', edo31: '31-TET', ji7: 'Just 7', pyth: 'Pythagorean', harm: 'Harmonic' };
    for (const t of cat.tunings) TUNINGS.push([t, nice[t] || t]);
  }

  /* parameter group order: known groups first, then whatever is new */
  if (cat.kinds) {
    const seen = new Set(GROUP_ORDER);
    for (const k of cat.kinds) for (const p of (k.params || [])) if (p.group && !seen.has(p.group)) { seen.add(p.group); GROUP_ORDER.push(p.group); }
  }
}

async function loadCatalog() {
  try {
    applyCatalog(await api('/api/studio/catalog'));
    if (!tileEl) return;
    renderEditor();
    renderArranger();
    renderLauncher();
    renderDevices();
    renderMixer();
    renderBrowser();
  } catch (e) {
    console.warn('studio: catalog unavailable, using built-in defaults', e);
  }
}

/* ── audio plumbing ─────────────────────────────────────────── */

/* ── mix engine: server-rendered stems + a live Web Audio mixer ──────────
   The Rust engine renders every track to a cached pre-fader stem (clips +
   inserts) plus the shared reverb/delay returns (POST /api/studio/mix/stems).
   The window plays those stems through its own channel strips —
   input (stem scale) → fader → StereoPanner → meter → mute/solo gate → master
   — so faders, pan, mute, solo and return levels react instantly, and only a
   change to the music itself (clips, patterns, inserts, sends, tempo…) asks
   the server for new stems. The browser's GainNode + StereoPannerNode laws
   are exactly the ones the offline mixer uses, so monitoring matches the
   export (master inserts, bus drive/width and the true-peak limiter are
   approximated by the Web Audio master chain). */

const STEM_MEMORY_LIMIT = 400 * 1024 * 1024;
const RETURN_IDS = { reverb: 'ret:reverb', delay: 'ret:delay' };
let stemBuffers = new Map();     // stem key → AudioBuffer
let mixManifest = null;          // last /mix/stems manifest
let mixManifestSig = '';         // arrangement signature it was rendered for
let mixContentSig = '';          // content-only part of that signature
let mixInflight = null;          // { sig, promise }
let mixRefreshTimer = 0;
let mixdownBuffer = null;        // fallback: one mixed WAV when stems are too big
let channels = new Map();        // channel id → { input, fader, pan, meter, gate }
let masterNodes = null;          // { input, glue, limiter, meter }
let playSources = [];
let meterRaf = 0;
let meterViews = [];             // { node: () => AnalyserNode, fill, hold, clip, peak, holdT, clipT }
let playLoopEnd = 0;             // seconds
let playTotal = 0;               // seconds
let playWraps = 0;

function audioCtxOr() {
  if (!audioCtx) audioCtx = new (window.AudioContext || window.webkitAudioContext)();
  return audioCtx;
}
function fmtTime(sec) {
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  const ms = Math.floor((sec - Math.floor(sec)) * 1000);
  return `${m}:${String(s).padStart(2, '0')}.${String(ms).padStart(3, '0')}`;
}
function dbToGain(db) { return Math.pow(10, db / 20); }
function gainToDb(g) { return g > 0 ? 20 * Math.log10(g) : -Infinity; }
function fmtDb(g) {
  const db = gainToDb(g);
  if (!Number.isFinite(db) || db < -60) return '−∞';
  const r = Math.round(db * 10) / 10;
  return `${r > 0 ? '+' : r < 0 ? '−' : ''}${Math.abs(r).toFixed(1)}`;
}

/* Master chain: input (master fader × loudness trim) → glue → limiter →
   meter → speakers. Every audible source (stems, previews, launcher,
   metronome) goes through it. */
function masterOut() {
  const ctx = audioCtxOr();
  if (!masterNodes) {
    const input = ctx.createGain();
    const glue = ctx.createDynamicsCompressor();
    const limiter = ctx.createDynamicsCompressor();
    const meter = ctx.createAnalyser();
    meter.fftSize = 2048;
    input.connect(glue);
    glue.connect(limiter);
    limiter.connect(meter);
    meter.connect(ctx.destination);
    masterNodes = { input, glue, limiter, meter };
    updateMasterNodes();
  }
  return masterNodes.input;
}
function masterGainValue() {
  const fx = arrangement?.fx || {};
  const trim = mixManifest?.master?.trim_db || 0;
  return (arrangement?.master ?? 0.9) * (Number.isFinite(fx.master_gain) ? fx.master_gain : 1) * dbToGain(trim);
}
function updateMasterNodes() {
  if (!masterNodes || !audioCtx) return;
  const now = audioCtx.currentTime;
  const fx = arrangement?.fx || {};
  masterNodes.input.gain.setTargetAtTime(masterGainValue(), now, 0.01);
  const glue = clamp(Number(fx.glue) || 0, 0, 1);
  const g = masterNodes.glue;
  g.threshold.setValueAtTime(glue > 0 ? -18 : 0, now);
  g.ratio.setValueAtTime(1 + glue, now);
  g.knee.setValueAtTime(8, now);
  g.attack.setValueAtTime(0.03, now);
  g.release.setValueAtTime(0.2, now);
  const l = masterNodes.limiter;
  l.threshold.setValueAtTime(clamp(Number.isFinite(fx.ceiling) ? fx.ceiling : -1, -24, 0), now);
  l.ratio.setValueAtTime(20, now);
  l.knee.setValueAtTime(0, now);
  l.attack.setValueAtTime(0.001, now);
  l.release.setValueAtTime(0.08, now);
}

/* One mixer channel (track or return). */
function ensureChannel(id) {
  const ctx = audioCtxOr();
  let ch = channels.get(id);
  if (!ch) {
    const input = ctx.createGain();
    const fader = ctx.createGain();
    const pan = ctx.createStereoPanner();
    const meter = ctx.createAnalyser();
    meter.fftSize = 1024;
    const gate = ctx.createGain();
    input.connect(fader);
    fader.connect(pan);
    pan.connect(meter);
    meter.connect(gate);
    gate.connect(masterOut());
    ch = { input, fader, pan, meter, gate, automated: new Set() };
    channels.set(id, ch);
  }
  return ch;
}
function dropChannels(keep) {
  for (const [id, ch] of channels) {
    if (keep.has(id)) continue;
    try { ch.gate.disconnect(); } catch (_) { /* noop */ }
    channels.delete(id);
  }
}
function returnLevel(which) {
  const v = arrangement?.returns?.[`${which}_level`];
  return Number.isFinite(v) ? v : 1;
}
/* Push the model's fader/pan/mute/solo values into the live graph. */
function refreshLiveMix() {
  if (!audioCtx || !arrangement) return;
  const now = audioCtx.currentTime;
  for (const tr of arrangement.tracks) {
    const ch = channels.get(tr.id);
    if (!ch) continue;
    if (!ch.automated.has('level')) ch.fader.gain.setTargetAtTime(tr.level ?? 0.8, now, 0.008);
    if (!ch.automated.has('pan')) ch.pan.pan.setTargetAtTime(clamp(tr.pan ?? 0, -1, 1), now, 0.008);
    ch.gate.gain.setTargetAtTime(arrTrackAudible(tr) ? 1 : 0, now, 0.006);
  }
  for (const which of ['reverb', 'delay']) {
    const ch = channels.get(RETURN_IDS[which]);
    if (ch) ch.fader.gain.setTargetAtTime(returnLevel(which), now, 0.008);
  }
  updateMasterNodes();
}

/* The arrangement as the stems route wants it (solo state included). */
function stemsPayload() {
  const p = arrangementPayload();
  p.tracks = p.tracks.map((t) => ({ ...t, solo: soloTracks.has(t.id) }));
  return p;
}
/* Content signature: everything that changes the stems themselves. Fader,
   pan, mute and solo are live controls and stay out of it. */
function contentSignature(p) {
  return JSON.stringify({
    bpm: p.bpm, length: p.length_beats, fx: p.fx, master_fx: p.master_fx, returns: { reverb: p.returns.reverb, delay: p.returns.delay },
    tracks: p.tracks.map((t) => ({ id: t.id, fx: t.fx, sends: t.sends, automation: t.automation })),
    clips: p.clips,
  });
}
function invalidateMix() {
  mixManifestSig = '';
}

async function fetchStem(url) {
  const blob = await apiFetch(url, { responseType: 'blob' });
  return audioCtxOr().decodeAudioData(await blob.arrayBuffer());
}
async function mapLimit(items, n, fn) {
  const out = new Array(items.length);
  let i = 0;
  const worker = async () => { while (i < items.length) { const k = i++; out[k] = await fn(items[k]); } };
  await Promise.all(Array.from({ length: Math.min(n, items.length) }, worker));
  return out;
}

/* Render (or reuse) the mix for the current arrangement. */
/* At most ONE /mix/stems request is in flight. A request for a newer state
   while one runs is coalesced: it waits for the running one and then sends
   only the latest arrangement (however many edits happened meanwhile). */
let mixFollowup = null;          // promise of the coalesced next render
async function ensureMix({ allowStale = false } = {}) {
  const payload = stemsPayload();
  const sig = JSON.stringify(payload);
  if (sig === mixManifestSig && mixManifest) return mixManifest;
  if (allowStale && mixManifest && !mixdownBuffer && contentSignature(payload) === mixContentSig) {
    // Only live controls changed (fader/pan/mute/solo): play the stems we
    // have — no render; returns refresh with the next content change.
    return mixManifest;
  }
  if (mixInflight) {
    if (mixInflight.sig === sig) return mixInflight.promise;
    if (!mixFollowup) {
      mixFollowup = mixInflight.promise.catch(() => {}).then(() => {
        mixFollowup = null;
        return ensureMix();
      });
    }
    return mixFollowup;
  }
  const promise = (async () => {
    const manifest = await api('/api/studio/mix/stems', { method: 'POST', body: JSON.stringify(payload) });
    if (manifest?.superseded) throw Object.assign(new Error('superseded'), { superseded: true });
    const bytes = (manifest.frames || 0) * 2 * 4 * ((manifest.stems || []).filter((s) => !s.silent).length + 2);
    let mixdown = null;
    if (bytes > STEM_MEMORY_LIMIT) {
      // Too much audio to hold every stem: fall back to one mixed render
      // (mixer changes then apply on the next play).
      stemBuffers = new Map();
      const blob = await apiFetch('/api/studio/arrangement/render', { method: 'POST', body: JSON.stringify(arrangementRenderPayload()), responseType: 'blob' });
      mixdown = await audioCtxOr().decodeAudioData(await blob.arrayBuffer());
    } else {
      const want = [
        ...(manifest.stems || []).filter((s) => !s.silent),
        ...['reverb', 'delay'].map((w) => manifest.returns?.[w]).filter(Boolean),
      ];
      // Drop buffers this mix no longer references before fetching new ones
      // (playing sources keep their own reference until they stop).
      const keep = new Set(want.map((s) => s.key));
      for (const k of [...stemBuffers.keys()]) if (!keep.has(k)) stemBuffers.delete(k);
      mixdownBuffer = null;
      const missing = want.filter((s) => !stemBuffers.has(s.key));
      const decoded = await mapLimit(missing, 2, (s) => fetchStem(s.url));
      missing.forEach((s, i) => stemBuffers.set(s.key, decoded[i]));
    }
    return { manifest, mixdown, sig, content: contentSignature(payload) };
  })();
  mixInflight = { sig, promise };
  try {
    const r = await promise;
    mixManifest = r.manifest;
    mixdownBuffer = r.mixdown;
    mixManifestSig = r.sig;
    mixContentSig = r.content;
    updateMasterReadout();
    if (detailPage === 'mixer') renderMixerReadouts();
    return r.manifest;
  } catch (e) {
    // Another client's newer request won on the server: retry once with
    // the latest state, after anything already queued.
    if (e?.superseded) {
      mixInflight = null;
      return mixFollowup || ensureMix();
    }
    throw e;
  } finally {
    if (mixInflight?.promise === promise) mixInflight = null;
  }
}

/* Schedule an automation lane onto an AudioParam from `fromBeat`. */
function scheduleLane(param, lane, when, fromBeat, endBeat, spb, lo, hi) {
  const span = Math.max(0.01, endBeat - fromBeat);
  const n = Math.max(2, Math.min(4000, Math.ceil(span * spb * 60)));
  const curve = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    const beat = fromBeat + (span * i) / (n - 1);
    curve[i] = clamp(sampleEnv(lane.points, beat, lane.points[0].value), lo, hi);
  }
  param.cancelScheduledValues(0);
  param.setValueAtTime(curve[0], when);
  try { param.setValueCurveAtTime(curve, when, span * spb); } catch (_) { /* overlapping curve: keep the start value */ }
}
function scheduleAutomation(when, fromBeat) {
  const spb = 60 / arrangement.bpm;
  const end = arrangement.length_beats;
  for (const tr of arrangement.tracks) {
    const ch = channels.get(tr.id);
    if (!ch) continue;
    ch.automated.clear();
    const lv = autoLanes(tr).find((l) => l.param === 'track.level' && l.points.length);
    const pn = autoLanes(tr).find((l) => l.param === 'track.pan' && l.points.length);
    if (lv) { ch.automated.add('level'); scheduleLane(ch.fader.gain, lv, when, fromBeat, end, spb, 0, 4); }
    else { ch.fader.gain.cancelScheduledValues(0); ch.fader.gain.setValueAtTime(tr.level ?? 0.8, when); }
    if (pn) { ch.automated.add('pan'); scheduleLane(ch.pan.pan, pn, when, fromBeat, end, spb, -1, 1); }
    else { ch.pan.pan.cancelScheduledValues(0); ch.pan.pan.setValueAtTime(clamp(tr.pan ?? 0, -1, 1), when); }
  }
}

function clearArrPlayhead() {
  if (arrPlayheadEl) arrPlayheadEl.style.opacity = '0';
  if (timeEl) timeEl.textContent = fmtPosition(arrStartBeat, arrStartBeat * 60 / (arrangement?.bpm || 120));
}
function fmtPosition(beats, sec) {
  return `${Math.floor(beats / 4) + 1}.${Math.floor(beats % 4) + 1}.${Math.floor((beats % 1) * 4) + 1}  ${fmtTime(sec)}`;
}
/* Stop the playing sources — at once, or with a short fade at `at` (used
   by the hot swap so the old and new renders crossfade without a click). */
function stopSources(at = null, fade = 0.01) {
  const ctx = audioCtx;
  for (const p of playSources) {
    try {
      if (ctx && at != null) {
        p.gain.gain.setValueAtTime(1, at);
        p.gain.gain.linearRampToValueAtTime(0, at + fade);
        p.src.stop(at + fade + 0.01);
      } else {
        p.src.stop();
      }
    } catch (_) { /* noop */ }
  }
  playSources = [];
}
function stopPlayback() {
  if (rafId) { cancelAnimationFrame(rafId); rafId = 0; }
  stopSources();
  for (const ch of channels.values()) ch.automated.clear();
  refreshLiveMix();
  arrPlaying = false;
  stopAudition();
  metroStop();
  clearArrPlayhead();
  clearPrevPlayhead();
  stopMeters();
  updateTransport();
  updateStudioGlow();
}

/* Start every stem at `fromBeat` (at audio time `at`, fading in over
   `fade` seconds when hot-swapping). */
function startSources(manifest, fromBeat, at = null, fade = 0) {
  const ctx = audioCtxOr();
  const spb = 60 / arrangement.bpm;
  const when = at ?? ctx.currentTime + 0.06;
  const offset = Math.max(0, fromBeat * spb);
  playLoopEnd = arrangement.length_beats * spb;
  playTotal = Math.max(playLoopEnd, (manifest.frames || 0) / (manifest.sample_rate || 44100));
  const loop = arrLoop && offset < playLoopEnd;
  const begin = (buf, dest) => {
    const src = ctx.createBufferSource();
    src.buffer = buf;
    if (loop) { src.loop = true; src.loopStart = 0; src.loopEnd = Math.min(playLoopEnd, buf.duration); }
    const gain = ctx.createGain();
    if (fade > 0) { gain.gain.setValueAtTime(0, when); gain.gain.linearRampToValueAtTime(1, when + fade); }
    src.connect(gain);
    gain.connect(dest);
    src.start(when, Math.min(offset, Math.max(0, buf.duration - 0.001)));
    playSources.push({ src, gain });
  };
  const keep = new Set();
  if (mixdownBuffer) {
    begin(mixdownBuffer, masterOut());
  } else {
    for (const s of manifest.stems || []) {
      keep.add(s.track_id);
      const ch = ensureChannel(s.track_id);
      ch.input.gain.value = s.scale || 1;
      const buf = stemBuffers.get(s.key);
      if (buf && !s.silent) begin(buf, ch.input);
    }
    for (const which of ['reverb', 'delay']) {
      const r = manifest.returns?.[which];
      const id = RETURN_IDS[which];
      keep.add(id);
      const ch = ensureChannel(id);
      ch.input.gain.value = r?.scale || 1;
      ch.fader.gain.value = returnLevel(which);
      const buf = r && stemBuffers.get(r.key);
      if (buf) begin(buf, ch.input);
    }
  }
  dropChannels(keep);
  refreshLiveMix();
  if (!mixdownBuffer) scheduleAutomation(when, fromBeat);
  return { when, offset, loop };
}

async function renderArrangementAndPlay(fromBeat = arrStartBeat) {
  if (busy || !arrangement) return;
  if (!arrangement.tracks.length || !arrangement.clips.length) {
    toast(t('plugin.studio.toast.needTrackAndClip', null, 'Add a track and a clip to the Arranger first (or use the Launcher)'), { type: 'error' });
    return;
  }
  busy = true;
  setStatus('Rendering…');
  try {
    const ctx = audioCtxOr();
    if (ctx.state === 'suspended') await ctx.resume();
    const manifest = await ensureMix({ allowStale: true });
    stopSources();
    beginTransport(manifest, fromBeat);
    setStatus('Playing');
  } catch (e) {
    console.error('studio: arrangement render/play failed', e);
    toast(e.message || t('plugin.studio.toast.renderFailed', null, 'Render failed'), { type: 'error' });
    setStatus('error');
  } finally {
    busy = false;
    updateTransport();
  }
}
function beginTransport(manifest, fromBeat, { at = null, fade = 0, keepClock = false } = {}) {
  const { when, offset, loop } = startSources(manifest, fromBeat, at, fade);
  arrPlaying = true;
  arrPlayStart = when;
  arrPlayOffset = offset;
  playWraps = loop && playLoopEnd > 0 ? Math.floor(offset / playLoopEnd) : 0;
  if (!keepClock) {
    metroStop();
    metroStart(when, arrangement.bpm, fromBeat);
  }
  updateStudioGlow();
  startMeters();
  if (rafId) cancelAnimationFrame(rafId);
  const spb = 60 / arrangement.bpm;
  const tick = () => {
    if (!arrPlaying || !audioCtx) return;
    const raw = audioCtx.currentTime - arrPlayStart;
    let pos = arrPlayOffset + Math.max(0, raw);
    if (loop && pos >= playLoopEnd) {
      const wraps = Math.floor(pos / playLoopEnd);
      pos -= wraps * playLoopEnd;
      if (wraps !== playWraps) {
        playWraps = wraps;
        // Each pass replays the automation from the top.
        scheduleAutomation(audioCtx.currentTime, pos / spb);
        metroStop();
        metroStart(audioCtx.currentTime - pos, arrangement.bpm, 0);
      }
    } else if (!loop && pos >= playTotal) { stopPlayback(); return; }
    const beats = pos / spb;
    if (arrPlayheadEl) { arrPlayheadEl.style.opacity = '1'; arrPlayheadEl.style.transform = `translateX(${HEAD_W + beats * PPB}px)`; }
    if (timeEl) timeEl.textContent = fmtPosition(beats, pos);
    updateAutoKnobs(beats);
    rafId = requestAnimationFrame(tick);
  };
  rafId = requestAnimationFrame(tick);
}
/* Current playhead position in beats (while playing). */
function currentBeat(atTime = null) {
  if (!arrPlaying || !audioCtx) return arrStartBeat;
  let pos = arrPlayOffset + Math.max(0, (atTime ?? audioCtx.currentTime) - arrPlayStart);
  if (arrLoop && playLoopEnd > 0) pos %= playLoopEnd;
  return pos / (60 / arrangement.bpm);
}

/* Called after every model change: live controls update now; a change to
   the music re-renders (debounced) and, while playing, swaps the stems in at
   the current position. */
function scheduleMixRefresh() {
  refreshLiveMix();
  if (!arrangement) return;
  if (mixRefreshTimer) clearTimeout(mixRefreshTimer);
  mixRefreshTimer = setTimeout(() => {
    mixRefreshTimer = 0;
    void hotSwapArrangement();
    void hotSwapLoops();
  }, 320);
}
/* While the arrangement plays, re-render what changed (the server reuses
   every unchanged stem; the window downloads only new keys) and swap it in
   at the current position: same sample position, 10 ms crossfade. */
let hotSwapBusy = false;
let hotSwapAgain = false;
async function hotSwapArrangement() {
  if (!arrPlaying) return;
  if (hotSwapBusy) { hotSwapAgain = true; return; }
  const content = contentSignature(stemsPayload());
  if (content === mixContentSig) return;
  hotSwapBusy = true;
  try {
    const manifest = await ensureMix();
    if (!arrPlaying || !audioCtx) return;
    const at = audioCtx.currentTime + 0.04;
    const beat = currentBeat(at);
    stopSources(at, 0.01);
    beginTransport(manifest, beat, { at, fade: 0.01, keepClock: true });
    setStatus('Playing');
  } catch (e) {
    console.warn('studio: live re-render failed', e);
    setStatus('error');
  } finally {
    hotSwapBusy = false;
    if (hotSwapAgain) { hotSwapAgain = false; void hotSwapArrangement(); }
  }
}
/* Launcher loops and the editor preview: re-render a playing loop whose
   pattern changed and crossfade it in at the same phase of the loop. */
function loopSignature(cfg) { return JSON.stringify(cfg); }
async function hotSwapLoops() {
  const ctx = audioCtx;
  if (!ctx) return;
  const jobs = [];
  for (const [trackId, lp] of Object.entries(trackLoops)) {
    if (lp.state === 'stopping' || !lp.cfgOf) continue;
    const cfg = lp.cfgOf();
    if (!cfg || loopSignature(cfg) === lp.sig) continue;
    jobs.push((async () => {
      const sig = loopSignature(cfg);
      lp.sig = sig;
      const { src, decoded } = await previewPattern(cfg, { loop: true, bus: lp.bus });
      if (trackLoops[trackId] !== lp || lp.sig !== sig) { try { src.disconnect(); } catch (_) { /* noop */ } return; }
      crossfadeLoop(lp, src, decoded, cfg);
    })().catch((e) => console.warn('studio: loop hot swap failed', e)));
  }
  if (prevPlaying && prevSource && current) {
    const cfg = serializePattern(current);
    const sig = loopSignature(cfg);
    if (sig !== prevSig) {
      prevSig = sig;
      jobs.push((async () => {
        const { src, decoded } = await previewPattern(cfg, { loop: false });
        if (!prevPlaying || prevSig !== sig) return;
        const at = ctx.currentTime + 0.03;
        const pos = Math.max(0, at - prevStart);
        if (pos >= decoded.duration) return;
        const g = ctx.createGain();
        src.disconnect();
        src.connect(g);
        g.connect(masterOut());
        g.gain.setValueAtTime(0, at);
        g.gain.linearRampToValueAtTime(1, at + 0.01);
        try { prevGain?.gain.setValueAtTime(1, at); prevGain?.gain.linearRampToValueAtTime(0, at + 0.01); prevSource.onended = null; prevSource.stop(at + 0.02); } catch (_) { /* noop */ }
        src.onended = endPreview;
        src.start(at, pos);
        prevSource = src;
        prevGain = g;
        prevDur = decoded.duration;
      })().catch(() => {}));
    }
  }
  await Promise.all(jobs);
}
/* Swap a looping launcher source for `src` at the same loop phase. */
function crossfadeLoop(lp, src, decoded, cfg) {
  const ctx = audioCtx;
  const loopDur = (Math.max(4, cfg.steps || 16) / 4) * 60 / (cfg.bpm || 120);
  src.loopStart = 0;
  src.loopEnd = Math.min(loopDur, decoded.duration);
  const at = Math.max(ctx.currentTime + 0.03, lp.when);
  const phase = ((at - lp.when) % loopDur + loopDur) % loopDur;
  const g = ctx.createGain();
  src.disconnect();
  src.connect(g);
  g.connect(lp.bus);
  g.gain.setValueAtTime(0, at);
  g.gain.linearRampToValueAtTime(1, at + 0.01);
  try {
    lp.gain?.gain.setValueAtTime(1, at);
    lp.gain?.gain.linearRampToValueAtTime(0, at + 0.01);
    lp.source.stop(at + 0.02);
  } catch (_) { /* noop */ }
  src.start(at, phase);
  lp.source = src;
  lp.gain = g;
}

/* ── meters (run only while something plays) ── */

function meterNode(getNode, { vertical = true, compact = false } = {}) {
  const wrap = h('div', `studio-vu${vertical ? '' : ' studio-vu--h'}${compact ? ' studio-vu--compact' : ''}`);
  const fill = h('span', 'studio-vu-fill');
  const hold = h('span', 'studio-vu-hold');
  const clip = h('span', 'studio-vu-clip');
  clip.title = t('plugin.studio.mixer.clipLed', null, 'Clip — click to reset');
  clip.addEventListener('click', (e) => { e.stopPropagation(); clip.classList.remove('is-on'); });
  wrap.append(fill, hold, clip);
  meterViews.push({ getNode, fill, hold, clip, peak: 0, holdV: 0, holdT: 0, vertical });
  return wrap;
}
const METER_FLOOR_DB = -60;
function meterFrac(peak) {
  const db = gainToDb(peak);
  if (!Number.isFinite(db) || db <= METER_FLOOR_DB) return 0;
  return clamp((db - METER_FLOOR_DB) / (6 - METER_FLOOR_DB), 0, 1);
}
let meterBuf = null;
function paintMeters() {
  const now = performance.now();
  meterViews = meterViews.filter((m) => m.fill.isConnected);
  for (const m of meterViews) {
    const node = m.getNode();
    let peak = 0;
    if (node && arrPlaying + Object.keys(trackLoops).length) {
      if (!meterBuf || meterBuf.length !== node.fftSize) meterBuf = new Float32Array(node.fftSize);
      node.getFloatTimeDomainData(meterBuf);
      for (let i = 0; i < meterBuf.length; i++) { const a = Math.abs(meterBuf[i]); if (a > peak) peak = a; }
    }
    // Fast attack, ~20 dB/s release; 1.5 s peak hold; latching clip LED.
    m.peak = peak > m.peak ? peak : m.peak * 0.89;
    if (m.peak >= m.holdV || now - m.holdT > 1500) { m.holdV = m.peak; m.holdT = now; }
    const f = meterFrac(m.peak);
    m.fill.style.transform = m.vertical ? `scaleY(${f.toFixed(4)})` : `scaleX(${f.toFixed(4)})`;
    const hf = meterFrac(m.holdV);
    if (m.vertical) m.hold.style.bottom = `${(hf * 100).toFixed(2)}%`;
    else m.hold.style.left = `${(hf * 100).toFixed(2)}%`;
    m.hold.style.opacity = hf > 0.01 ? '1' : '0';
    if (peak >= 0.999) m.clip.classList.add('is-on');
  }
}
function startMeters() {
  if (meterRaf) return;
  const tick = () => {
    if (!arrPlaying && !Object.keys(trackLoops).length) { meterRaf = 0; paintMeters(); return; }
    if (!document.hidden) paintMeters();
    meterRaf = requestAnimationFrame(tick);
  };
  meterRaf = requestAnimationFrame(tick);
}
function stopMeters() {
  if (meterRaf) { cancelAnimationFrame(meterRaf); meterRaf = 0; }
  for (const m of meterViews) { m.peak = 0; m.holdV = 0; }
  paintMeters();
}

/* ── metronome (WebAudio lookahead scheduler) ───────────────── */

function metroClick(when, accent) {
  const ctx = audioCtxOr();
  const osc = ctx.createOscillator();
  const gain = ctx.createGain();
  osc.frequency.value = accent ? 1760 : 880;
  gain.gain.setValueAtTime(0.0001, when);
  gain.gain.exponentialRampToValueAtTime(0.25, when + 0.002);
  gain.gain.exponentialRampToValueAtTime(0.0001, when + 0.06);
  osc.connect(gain).connect(masterOut());
  osc.start(when);
  osc.stop(when + 0.08);
  flashBeat(accent);
}
function metroStart(startTime, bpm, startBeat = 0) {
  metroStop();
  if (!metroOn) return;
  const beat = 60 / bpm;
  let nextBeat = startTime;
  let beatIdx = Math.max(0, Math.ceil(startBeat - 1e-6));
  // Start on the first beat line at or after `startBeat`, never in the past.
  nextBeat += (beatIdx - startBeat) * beat;
  const now0 = audioCtxOr().currentTime;
  while (nextBeat < now0 - 0.005) { nextBeat += beat; beatIdx += 1; }
  metroTimer = setInterval(() => {
    if (!audioCtx) return;
    while (nextBeat < audioCtx.currentTime + 0.12) {
      metroClick(nextBeat, beatIdx % 4 === 0);
      nextBeat += beat;
      beatIdx += 1;
    }
  }, 30);
}
function metroStop() {
  if (metroTimer) { clearInterval(metroTimer); metroTimer = 0; }
}

/* ── editor preview playback ────────────────────────────────── */

function clearPrevPlayhead() {
  if (prevSource) { try { prevSource.stop(); } catch (_) { /* noop */ } prevSource = null; }
  prevPlaying = false;
  gridEl?.querySelectorAll('.studio-scene').forEach((el) => el.classList.remove('studio-scene--playing'));
  pianoEl?.querySelectorAll('.studio-pr-step').forEach((el) => el.classList.remove('studio-pr-step--playing'));
}
async function previewPattern(cfg, { loop = false, bus = null } = {}) {
  const blob = await apiFetch('/api/studio/preview', { method: 'POST', body: JSON.stringify(cfg), responseType: 'blob' });
  const buf = await blob.arrayBuffer();
  const ctx = audioCtxOr();
  if (ctx.state === 'suspended') await ctx.resume();
  const decoded = await ctx.decodeAudioData(buf);
  const src = ctx.createBufferSource();
  src.buffer = decoded;
  src.loop = loop;
  src.connect(bus || masterOut());
  return { src, decoded };
}
/* Looping audition for SynthMe / WaveMe — starts immediately and stops on re-call. */
function stopAudition() {
  try { auditionSource?.stop(); } catch (_) { /* noop */ }
  auditionSource = null;
}
async function previewAudition(cfg) {
  stopAudition();
  const { src } = await previewPattern(cfg, { loop: true });
  src.start();
  auditionSource = src;
  setStatus('Playing');
}
async function previewClipInEditor() {
  if (!current) return;
  if (prevPlaying) { endPreview(); return; }
  setStatus('Rendering…');
  try {
    const cfg0 = serializePattern(current);
    const { src, decoded } = await previewPattern(cfg0);
    clearPrevPlayhead();
    prevSig = loopSignature(cfg0);
    prevGain = null;
    prevSource = src;
    prevPlaying = true;
    prevStart = audioCtx.currentTime + 0.05;
    prevDur = decoded.duration;
    src.onended = endPreview;
    src.start(prevStart);
    renderEditorToolbar();
    setStatus('Playing');
    const tick = () => {
      if (!prevPlaying || !audioCtx) return;
      const elapsed = audioCtx.currentTime - prevStart;
      if (elapsed < 0) { requestAnimationFrame(tick); return; }
      if (elapsed >= prevDur) { endPreview(); return; }
      const step = Math.floor((elapsed / prevDur) * current.steps);
      gridEl?.querySelectorAll('.studio-scene').forEach((el) => el.classList.toggle('studio-scene--playing', Number(el.dataset.step) === step));
      pianoEl?.querySelectorAll('.studio-pr-step').forEach((el) => el.classList.toggle('studio-pr-step--playing', Number(el.textContent) - 1 === step));
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  } catch (e) {
    console.error('studio: clip preview failed', e);
    toast(e.message || t('plugin.studio.toast.previewFailed', null, 'Preview failed'), { type: 'error' });
    setStatus('error');
  }
}

function endPreview() {
  try { prevSource?.stop(); } catch (_) { /* noop */ }
  prevSource = null;
  prevPlaying = false;
  clearPrevPlayhead();
  renderEditorToolbar();
  setStatus(dirty ? 'dirty' : 'saved');
}
async function playSaved(id) {
  try {
    const blob = await apiFetch(`/api/studio/${id}/audio`, { responseType: 'blob' });
    const buf = await blob.arrayBuffer();
    const ctx = audioCtxOr();
    if (ctx.state === 'suspended') await ctx.resume();
    const decoded = await ctx.decodeAudioData(buf);
    const src = ctx.createBufferSource();
    src.buffer = decoded;
    src.connect(masterOut());
    src.start();
    updateStudioGlow();
  } catch (e) {
    toast(e.message || t('plugin.studio.toast.playFailed', null, 'Play failed'), { type: 'error' });
  }
}

/* ── launcher playback (bar-quantized clip launching) ───────── */

function barDur() { return (60 / launcher.bpm) * 4; }
function nextBarTime() {
  const ctx = audioCtxOr();
  if (!clockRunning) return ctx.currentTime + 0.06;
  const bar = barDur();
  const t = ctx.currentTime;
  return clockT0 + Math.ceil((t - clockT0) / bar) * bar;
}
function ensureTrackAudio(tr) {
  const ctx = audioCtxOr();
  let ta = trackAudio[tr.id];
  if (!ta) {
    const gain = ctx.createGain();
    const panner = ctx.createStereoPanner ? ctx.createStereoPanner() : null;
    const tap = ctx.createAnalyser();
    tap.fftSize = 1024;
    tap.smoothingTimeConstant = 0.8;
    if (panner) { panner.connect(gain); gain.connect(masterOut()); }
    else { gain.connect(masterOut()); }
    gain.connect(tap);            // per-track level tap (launcher meters)
    ta = { gain, panner, analyser: tap };
    trackAudio[tr.id] = ta;
  }
  ta.gain.gain.value = trackAudible(tr) ? (tr.level ?? 0.8) : 0;
  if (ta.panner) ta.panner.pan.value = tr.pan ?? 0;
  return ta;
}
async function launchClip(ti, si) {
  const tr = launcher.tracks[ti];
  if (!tr) return;
  const key = `${tr.id}:${si}`;
  const clip = launcher.clips[key];
  if (!clip) return;
  const existing = trackLoops[tr.id];
  if (existing && existing.slotKey === key && existing.state !== 'stopping') { stopLauncherTrack(tr.id, true); return; }
  stopLauncherTrack(tr.id, true);
  setStatus('Rendering…');
  try {
    const cfgOf = () => {
      const c = launcher.clips[key];
      return c ? { ...serializePattern(normalizePattern(c.pattern)), title: c.title || 'Clip', bpm: launcher.bpm } : null;
    };
    const cfg = cfgOf();
    const ta = ensureTrackAudio(tr);
    const bus = ta.panner || ta.gain;
    const { src: raw, decoded } = await previewPattern(cfg, { loop: true, bus });
    // Loop exactly one pattern pass (the render carries a release tail).
    raw.loopStart = 0;
    raw.loopEnd = Math.min(decoded.duration, (Math.max(4, cfg.steps || 16) / 4) * 60 / launcher.bpm);
    const lgain = audioCtxOr().createGain();
    raw.disconnect();
    raw.connect(lgain);
    lgain.connect(bus);
    const src = raw;
    const ctx = audioCtxOr();
    if (!clockRunning) {
      clockRunning = true;
      clockT0 = ctx.currentTime + 0.08;
      metroStart(clockT0, launcher.bpm);
      startClockUi();
    }
    const when = nextBarTime();
    src.start(when);
    trackLoops[tr.id] = { source: src, slotKey: key, when, state: 'queued', cfgOf, sig: loopSignature(cfg), bus, gain: lgain };
    startMeters();
    setStatus('Queued');
  } catch (e) {
    console.error('studio: launch clip failed', e);
    toast(e.message || t('plugin.studio.toast.launchFailed', null, 'Launch failed'), { type: 'error' });
    setStatus('error');
  }
  renderLauncher();
  updateTransport();
}
function stopLauncherTrack(trackId, quantized = true) {
  const p = trackLoops[trackId];
  if (!p) return;
  const ctx = audioCtxOr();
  if (quantized && clockRunning && p.state === 'playing') {
    try { p.source.stop(nextBarTime()); } catch (_) { /* noop */ }
    p.state = 'stopping';
    p.stopAt = nextBarTime();
  } else {
    try { p.source.stop(ctx.currentTime); } catch (_) { /* noop */ }
    delete trackLoops[trackId];
  }
  renderLauncher();
}
function stopAllLauncher() {
  const ctx = audioCtxOr();
  for (const id of Object.keys(trackLoops)) {
    try { trackLoops[id].source.stop(ctx.currentTime); } catch (_) { /* noop */ }
  }
  trackLoops = {};
  clockRunning = false;
  metroStop();
  renderLauncher();
  updateTransport();
}
function launchScene(si) {
  for (let t = 0; t < launcher.tracks.length; t++) {
    const tr = launcher.tracks[t];
    if (launcher.clips[`${tr.id}:${si}`]) void launchClip(t, si);
  }
}
/* Drives queued→playing transitions, clock display and launcher highlight. */
function startClockUi() {
  if (clockUiTimer) return;
  clockUiTimer = setInterval(() => {
    if (!audioCtx) return;
    const now = audioCtx.currentTime;
    let changed = false;
    for (const [id, p] of Object.entries(trackLoops)) {
      if (p.state === 'queued' && now >= p.when) { p.state = 'playing'; changed = true; }
      if (p.state === 'stopping' && p.stopAt && now >= p.stopAt) { delete trackLoops[id]; changed = true; }
    }
    if (clockRunning && timeEl && !arrPlaying) {
      const bar = barDur();
      const beats = Math.max(0, (now - clockT0) / (bar / 4));
      if (now >= clockT0) {
        timeEl.textContent = `${Math.floor(beats / 4) + 1}.${Math.floor(beats % 4) + 1} · ${fmtTime(now - clockT0)}`;
        const bi = Math.floor(beats);
        if (bi !== lastClockBeat) { lastClockBeat = bi; flashBeat(bi % 4 === 0); }
      }
    }
    if (changed) { renderLauncher(); updateTransport(); }
    if (!clockRunning && !Object.keys(trackLoops).length) { clearInterval(clockUiTimer); clockUiTimer = 0; lastClockBeat = -1; }
  }, 80);
}
function anyLauncherPlaying() {
  return Object.keys(trackLoops).length > 0;
}

/* ── persistence (patterns / arrangements / presets) ────────── */

async function refreshTracks() {
  const data = await api('/api/studio');
  tracks = data?.tracks || [];
}
async function refreshArrangements() {
  const data = await api('/api/studio/arrangement');
  arrangements = data?.arrangements || [];
}
async function refreshPresets() {
  const data = await api('/api/studio/presets');
  presets = data?.presets || [];
}
async function deleteTrack(id) {
  await api(`/api/studio/${id}`, { method: 'DELETE' });
  await refreshTracks();
  renderBrowser();
}

let editSeq = 0;                  // bumps on every edit (autosave race guard)
async function saveArrangement() {
  const payload = arrangementPayload();
  const seq = editSeq;
  let data;
  if (arrangement.id) {
    data = await api(`/api/studio/arrangement/${arrangement.id}`, { method: 'PUT', body: JSON.stringify(payload) });
  } else {
    data = await api('/api/studio/arrangement', { method: 'POST', body: JSON.stringify(payload) });
  }
  arrangement.id = data?.id || arrangement.id;
  // Edits made while the request was in flight keep the project dirty.
  if (seq === editSeq) dirty = false; else scheduleAutosave();
  await refreshArrangements();
  renderBrowser();
  return data;
}
async function loadArrangement(id) {
  const data = await api(`/api/studio/arrangement/${id}`);
  arrangement = normalizeArrangement(data);
  soloTracks = new Set(arrangement.tracks.filter((t) => t.solo).map((t) => t.id));
  invalidateMix();
  sel = arrangement.clips.length ? { area: 'arr', clipIndex: 0 } : null;
  syncSelection();
  dirty = false;
  if (titleInput && panels.arranger) titleInput.value = arrangement.title;
  renderArranger();
  renderBrowser();
  setStatus('saved');
}
/* ── projects ── */
async function confirmDiscard() {
  if (!dirty || !arrangement?.clips?.length) return true;
  return confirmDialog(t('plugin.studio.project.discard', { name: arrangement.title }, `"${arrangement.title}" has unsaved changes. Discard them?`));
}
async function openProject(id) {
  if (arrangement?.id === id && !dirty) return;
  if (!(await confirmDiscard())) return;
  if (arrPlaying) stopPlayback();
  try {
    await loadArrangement(id);
    if (titleInput) titleInput.value = arrangement.title;
    if (bpmInput) bpmInput.value = String(Math.round(arrangement.bpm * 100) / 100);
    viewExtraBeats = 0;
    requestAnimationFrame(() => zoomToFit());
  } catch (e) {
    toast(e.message, { type: 'error' });
  }
}
async function newProject() {
  if (!(await confirmDiscard())) return;
  if (arrPlaying) stopPlayback();
  arrangement = blankArrangement();
  arrangement.title = t('plugin.studio.project.untitled', null, 'Untitled project');
  soloTracks = new Set();
  invalidateMix();
  sel = null;
  syncSelection();
  dirty = false;
  if (titleInput) titleInput.value = arrangement.title;
  PPB = 56;
  viewExtraBeats = 0;
  renderArranger();
  renderBrowser();
  setStatus('saved');
}
async function saveProjectAs() {
  const base = arrangement.title || 'Untitled';
  arrangement.id = null;
  arrangement.title = arrangements.some((a) => a.title === base) ? `${base} (2)` : base;
  if (titleInput) titleInput.value = arrangement.title;
  await saveArrangement();
  setStatus('saved');
  toast(t('plugin.studio.project.savedAs', { name: arrangement.title }, `Saved as "${arrangement.title}"`), { type: 'success' });
}
async function renameProject(a, title) {
  const name = title.trim();
  if (!name || name === a.title) { renderBrowser(); return; }
  try {
    const data = await api(`/api/studio/arrangement/${a.id}`);
    data.title = name;
    await api(`/api/studio/arrangement/${a.id}`, { method: 'PUT', body: JSON.stringify(data) });
    if (arrangement?.id === a.id) { arrangement.title = name; if (titleInput) titleInput.value = name; }
    await refreshArrangements();
  } catch (e) { toast(e.message, { type: 'error' }); }
  renderBrowser();
}
async function duplicateProject(a) {
  try {
    const data = await api(`/api/studio/arrangement/${a.id}`);
    delete data.id;
    data.title = `${a.title} ${t('plugin.studio.project.copySuffix', null, '(copy)')}`;
    await api('/api/studio/arrangement', { method: 'POST', body: JSON.stringify(data) });
    await refreshArrangements();
  } catch (e) { toast(e.message, { type: 'error' }); }
  renderBrowser();
}
/* Autosave: a project that already lives in the Studio saves itself 3 s
   after the last edit (new projects wait for an explicit Save). */
let autosaveTimer = 0;
function scheduleAutosave() {
  if (autosaveTimer) clearTimeout(autosaveTimer);
  if (!arrangement?.id) return;
  autosaveTimer = setTimeout(() => {
    autosaveTimer = 0;
    if (!dirty || !arrangement?.id || busy) return;
    void saveArrangement().then(() => setStatus('saved')).catch((e) => console.warn('studio: autosave failed', e));
  }, 3000);
}
async function deleteArrangement(id) {
  await api(`/api/studio/arrangement/${id}`, { method: 'DELETE' });
  await refreshArrangements();
  if (arrangement?.id === id) {
    arrangement = blankArrangement();
    sel = arrangement.clips.length ? { area: 'arr', clipIndex: 0 } : null;
    syncSelection();
    renderArranger();
  }
  renderBrowser();
}

/* Save the pattern being edited into the pattern library.
   Non-blocking: the name comes from the clip's title field (editable in the
   Editor toolbar) instead of a native prompt(). */
async function savePatternToLibrary() {
  if (!current) { toast(t('plugin.studio.toast.selectClipFirst', null, 'Select a clip first'), { type: 'error' }); return; }
  const title = (current.title || '').trim() || 'Pattern';
  current.title = title;
  const data = await api('/api/studio', { method: 'POST', body: JSON.stringify(serializePattern(current)) });
  await refreshTracks();
  renderBrowser();
  toast(t('plugin.studio.toast.patternSaved', { name: data?.title || title }, `Saved pattern “${data?.title || title}”`), { type: 'success' });
}
async function savePreset(v) {
  const sameKind = presets.filter((p) => p.kind === v.kind).length;
  const name = `${KIND_LABELS[v.kind] || v.kind} ${sameKind + 1}`;
  const params = {
    wave: v.wave,
    synth: { ...v.synth },
    fx: v.fx.map((f) => ({ kind: f.kind, params: f.params, bypass: f.bypass })),
    level: v.level,
    pan: v.pan,
    accent: v.accent ?? 0,
  };
  await api('/api/studio/presets', { method: 'POST', body: JSON.stringify({ kind: v.kind, name, params }) });
  await refreshPresets();
  renderDevices();
  renderBrowser();
  toast(t('plugin.studio.toast.presetSaved', { name }, `Saved preset “${name}”`), { type: 'success' });
}

/* ── selection ──────────────────────────────────────────────── */

/* Resolve `sel` → `current` (live reference into the clip's pattern). */
function syncSelection() {
  clearPrevPlayhead();
  let pat = null;
  if (sel?.area === 'arr') pat = arrangement.clips[sel.clipIndex]?.pattern || null;
  else if (sel?.area === 'lch') pat = launcher.clips[`${sel.trackId}:${sel.scene}`]?.pattern || null;
  current = pat;
  if (current && !Array.isArray(current.voices)) current = null;
  if (current) {
    current.voices = current.voices.map((v) => (v.macros ? v : normalizeVoice(v)));
    selectedVoice = Math.min(selectedVoice, current.voices.length - 1);
    if (selectedVoice < 0) selectedVoice = 0;
  }
  renderDetail();
  updateStudioGlow();
}
function selectArrClip(clipIndex, { openEditor = false } = {}) {
  sel = { area: 'arr', clipIndex };
  if (openEditor) { detailOpen = true; detailPage = 'editor'; }
  syncSelection();
  renderArranger();
}
function selectLauncherClip(trackId, scene, { openEditor = false } = {}) {
  sel = { area: 'lch', trackId, scene };
  if (openEditor) { detailOpen = true; detailPage = 'editor'; }
  syncSelection();
  renderLauncher();
}

/* ── status / transport / footer ────────────────────────────── */

function setStatus(s) {
  // The save dot replaces the "saved"/"dirty" text: it glows with the accent
  // while there are unsaved changes and stays muted once saved.
  const LABELS = { 'Rendering…': 'plugin.studio.status.rendering', Playing: 'plugin.studio.status.playing', Queued: 'plugin.studio.status.queued', error: 'plugin.studio.status.error' };
  const busy = s === 'Rendering…' || s === 'Playing' || s === 'Queued';
  const isDirty = dirty && s !== 'saved';
  if (saveDotEl) {
    saveDotEl.classList.toggle('is-active', isDirty);
    saveDotEl.title = t(isDirty ? 'plugin.studio.status.unsaved' : 'plugin.studio.status.saved', null, isDirty ? 'Unsaved changes' : 'Saved');
  }
  if (!statusEl) return;
  // Mono capitals next to the dot: SAVED / UNSAVED, or the busy/error state.
  const text = busy || s === 'error'
    ? (LABELS[s] ? t(LABELS[s], null, s) : s)
    : t(isDirty ? 'plugin.studio.status.unsavedShort' : 'plugin.studio.status.saved', null, isDirty ? 'Unsaved' : 'Saved');
  if (statusEl.dataset.text !== text) {
    statusEl.dataset.text = text;
    decodeText(statusEl, text, { duration: 360 });
  }
  statusEl.hidden = false;
  statusEl.dataset.state = s === 'error' ? 'error' : (busy ? 'busy' : (isDirty ? 'dirty' : 'ok'));
}
function markDirty() {
  editSeq += 1;
  dirty = true;
  setStatus('dirty');
  scheduleMixRefresh();
  scheduleAutosave();
}
function updateTransport() {
  if (playBtn) {
    playBtn.classList.toggle('studio-transport--on', arrPlaying);
    playBtn.setAttribute('aria-pressed', String(!!arrPlaying));
  }
  if (loopBtn) {
    loopBtn.classList.toggle('studio-transport--on', arrLoop);
    loopBtn.setAttribute('aria-pressed', String(!!arrLoop));
  }
  if (metroBtn) {
    metroBtn.classList.toggle('studio-transport--on', metroOn);
    metroBtn.setAttribute('aria-pressed', String(!!metroOn));
  }
}
function setReadout(text) {
  if (footerParamEl) footerParamEl.textContent = text || '';
}
function setHint(text) {
  if (footerHintEl) footerHintEl.textContent = text || '';
}
/* Attach a footer readout string to any interactive element. */
function withHint(el, text) {
  el.dataset.hint = text;
  return el;
}

/* ── canvas colours (clip previews) ─────────────────────────── */

/* Canvas colours come from the theme: `--studio-scope-trace` (the accent)
   and `--studio-scope-ink` (the text colour; grid, peaks and clip previews
   use it at a low alpha). Re-read on theme/appearance changes. */
let canvasInk = { trace: '#5ad1a8', ink: '#ffffff' };
let canvasInkRead = false;
function readCanvasColors() {
  canvasInkRead = !!tileEl?.isConnected;
  const cs = tileEl ? getComputedStyle(tileEl) : null;
  const v = (name, fallback) => (cs?.getPropertyValue(name).trim()) || fallback;
  canvasInk = { trace: v('--studio-scope-trace', '#5ad1a8'), ink: v('--studio-scope-ink', '#ffffff') };
}
function onStudioThemeChange() {
  readCanvasColors();
  document.getElementById('theme-tokens')?.addEventListener('load', () => { readCanvasColors(); redrawClipPreviews(); }, { once: true });
  redrawClipPreviews();
}
function redrawClipPreviews() {
  if (!arrGridEl || !arrangement) return;
  arrangement.clips.forEach((clip, i) => {
    if (clip.audio) drawAudioClip(i);
    else drawPatternClip(i);
  });
}

function flashBeat(accent) {
  if (!beatDotEl) return;
  beatDotEl.classList.add('studio-beat--hit');
  beatDotEl.classList.toggle('studio-beat--bar', !!accent);
  if (beatFlashTimer) clearTimeout(beatFlashTimer);
  beatFlashTimer = setTimeout(() => {
    beatDotEl?.classList.remove('studio-beat--hit', 'studio-beat--bar');
  }, 140);
}

/* ── native-ish controls ────────────────────────────────────── */

function nativeSelect(options, value, onchange, hint) {
  const selEl = selectControl({ options: options.map(([value, label]) => ({ value, label })), value, onChange: onchange });
  selEl.classList.add('studio-select');
  if (hint) withHint(selEl, hint);
  return selEl;
}

function numberField(placeholder, value, onchange, hint) {
  const input = uiInput();
  input.type = 'number';
  input.classList.add(...('studio-number').split(/\s+/));
  input.placeholder = placeholder;
  input.value = String(value);
  input.addEventListener('change', () => { onchange(parseInt(input.value, 10) || 0); });
  if (hint) withHint(input, hint);
  return input;
}

/* Arc knob: 270° ring from the neutral "0" to the value (bipolar-aware). */
function knob(label, min, max, step, value, onchange, resetTo) {
  const k = dial({ label, min, max, step, value, resetValue: resetTo ?? min, format: fmt,
    onInput: next => {
      onchange(next);
      setReadout(t('plugin.studio.readout.param', { label, value: fmt(next) }, `${label} — ${fmt(next)}`));
    },
  });
  k.classList.add('studio-knob');
  k.dataset.param = label;
  k.title = label;
  k._dial = k.dial;
  k._set = k.setValue;
  k.addEventListener('mouseenter', () => setReadout(t('plugin.studio.readout.param', { label, value: fmt(k.value) }, `${label} — ${fmt(k.value)}`)));
  return k;
}

/* ── arranger (timeline) ────────────────────────────────────── */

/* Beats in one pass of a clip's pattern. */
function patternBeats(c) {
  return Math.max(1, (c.pattern?.steps || 16) / 4);
}
/* Audio file metadata (duration, rate, tempo, peaks) cached by path. */
const audioInfoCache = new Map();
function audioInfo(path) {
  return audioInfoCache.get(path)?.data || null;
}
async function loadAudioInfo(path) {
  const hit = audioInfoCache.get(path);
  if (hit) return hit.promise;
  const promise = api(`/api/studio/audio/info?path=${encodeURIComponent(path)}&buckets=2000`)
    .then((data) => { audioInfoCache.set(path, { data, promise }); return data; })
    .catch((e) => { audioInfoCache.delete(path); throw e; });
  audioInfoCache.set(path, { data: null, promise });
  return promise;
}
/* Clip length in beats: explicit, else one pattern pass / the rest of the file. */
function clipDurBeats(c) {
  if (Number.isFinite(c.length_beats) && c.length_beats > 0) return c.length_beats;
  if (c.audio) {
    const info = audioInfo(c.audio.src);
    if (info?.duration) return Math.max(0.25, (info.duration - (c.audio.offset || 0)) * (arrangement?.bpm || 120) / 60);
    return 4;
  }
  return Math.max(0.25, patternBeats(c) - (c.loop === false ? (c.offset_beats || 0) : 0));
}
/* Grow the arrangement so every clip fits. */
function fitArrangementLength() {
  let end = 4;
  for (const c of arrangement.clips) end = Math.max(end, c.start + clipDurBeats(c));
  const need = Math.ceil(end / 4) * 4;
  if (need > arrangement.length_beats) arrangement.length_beats = need;
}
/* A pattern is "MIDI" when it carries melodic voices (piano-roll preview). */
function patternIsMelodic(p) {
  return (p?.voices || []).some((v) => MELODIC.has(v.kind));
}
/* Collect note events across all melodic voices (deriving from euclidean
   rhythm when a voice has no explicit notes), for the mini piano roll. */
function clipMelodicNotes(p) {
  const steps = Math.max(1, p?.steps || 16);
  const out = [];
  for (const v of (p?.voices || [])) {
    if (!MELODIC.has(v.kind)) continue;
    let notes = v.notes || [];
    if (!notes.length && v.rhythm) {
      notes = rhythmToCells(v.rhythm, steps)
        .map((on, step) => (on ? { step, length: 1, degree: v.degree, octave: v.octave } : null))
        .filter(Boolean);
    }
    for (const n of notes) out.push({ step: n.step, length: n.length || 1, degree: n.degree });
  }
  return out;
}
function arrTrackIndex(id) {
  return arrangement.tracks.findIndex((t) => t.id === id);
}
function arrTrackAudible(tr) {
  const anySolo = arrangement.tracks.some((t) => soloTracks.has(t.id));
  return anySolo ? (soloTracks.has(tr.id) && !tr.mute) : !tr.mute;
}

/* ── automation (Bitwig-style breakpoint envelopes) ─────────── */

function normalizeAutomation(a) {
  const pts = (arr) => (Array.isArray(arr) ? arr
    .filter((p) => p && Number.isFinite(p.beat) && Number.isFinite(p.value))
    .map((p) => ({ beat: p.beat, value: p.value }))
    .sort((x, y) => x.beat - y.beat) : []);
  const lanes = Array.isArray(a?.lanes)
    ? a.lanes.filter((l) => l && typeof l.param === 'string').map((l) => ({ param: l.param, points: pts(l.points) }))
    : [];
  return { lanes };
}

function autoLanes(tr) {
  if (!tr.automation || !Array.isArray(tr.automation.lanes)) tr.automation = { lanes: [] };
  return tr.automation.lanes;
}
function autoLane(tr, param) {
  const lanes = autoLanes(tr);
  let lane = lanes.find((l) => l.param === param);
  if (!lane) { lane = { param, points: [] }; lanes.push(lane); }
  return lane;
}

/* All automatable params for a track (representative voices from its clips). */
function trackParamCatalog(tr) {
  const out = [
    { path: 'track.level', label: t('plugin.studio.automation.trackLevel', null, 'Track Level'), min: 0, max: 2 },
    { path: 'track.pan', label: t('plugin.studio.automation.trackPan', null, 'Track Pan'), min: -1, max: 1 },
  ];
  const rep = arrangement.clips.find((c) => c.track === tr.id && (c.pattern?.voices || []).length);
  if (rep) {
    rep.pattern.voices.forEach((v, vi) => {
      const kl = KIND_LABELS[v.kind] || v.kind;
      for (const p of (SYNTH[v.kind] || [])) out.push({ path: `voice.${vi}.${p.key}`, label: `${kl} ${p.label}`, min: p.min, max: p.max });
      out.push({ path: `voice.${vi}.level`, label: t('plugin.studio.automation.voiceLevel', { voice: kl }, `${kl} Level`), min: 0, max: 2 });
      out.push({ path: `voice.${vi}.pan`, label: t('plugin.studio.automation.voicePan', { voice: kl }, `${kl} Pan`), min: -1, max: 1 });
      // `accent` shapes note velocities when the pattern is scheduled, so it
      // is not a live automation target — it stays a device knob only.
      v.fx.forEach((f, fi) => {
        for (const p of (EFFECTS[f.kind]?.params || [])) out.push({ path: `voice.${vi}.fx.${fi}.${p.key}`, label: t('plugin.studio.automation.voiceFxParam', { voice: kl, n: fi + 1, param: p.label }, `${kl} FX${fi + 1} ${p.label}`), min: p.min, max: p.max });
      });
    });
  }
  for (const p of FX) out.push({ path: `master.${p.key}`, label: t('plugin.studio.automation.masterParam', { param: p.label }, `Master ${p.label}`), min: p.min, max: p.max });
  return out;
}
function autoRange(tr, param) {
  return trackParamCatalog(tr).find((c) => c.path === param) || { min: 0, max: 1, label: param };
}
function autoBaseValue(tr, param) {
  if (param === 'track.level') return tr.level ?? 0.8;
  if (param === 'track.pan') return tr.pan ?? 0;
  const rep = arrangement.clips.find((c) => c.track === tr.id && (c.pattern?.voices || []).length);
  const m = param.match(/^voice\.(\d+)\.(?:fx\.(\d+)\.)?(.+)$/);
  if (m && rep) {
    const v = rep.pattern.voices[parseInt(m[1], 10)];
    if (v) {
      if (m[2] != null) {
        const f = v.fx[parseInt(m[2], 10)];
        if (f) { const def = (EFFECTS[f.kind]?.params || []).find((p) => p.key === m[3]); return f.params[m[3]] != null ? f.params[m[3]] : (def?.def ?? 0.5); }
      } else if (m[3] === 'level') return v.level != null ? v.level : (DEFAULT_LEVEL[v.kind] ?? 0.5);
      else if (m[3] === 'pan') return v.pan != null ? v.pan : (DEFAULT_PAN[v.kind] ?? 0);
      else if (m[3] === 'accent') return v.accent ?? 0;   // display-only
      else { const def = (SYNTH[v.kind] || []).find((p) => p.key === m[3]); return v.synth[m[3]] != null ? v.synth[m[3]] : (def?.def ?? 0.5); }
    }
  }
  const mm = param.match(/^master\.(.+)$/);
  if (mm && rep) { const def = FX.find((p) => p.key === mm[1]); return rep.pattern.fx?.[mm[1]] != null ? rep.pattern.fx[mm[1]] : (def?.def ?? 0.5); }
  return 0.5;
}

function autoY(min, max, value, h) {
  return h - ((clamp(value, min, max) - min) / (max - min)) * h;
}
function autoValueY(min, max, y, h) {
  return min + ((h - y) / h) * (max - min);
}
function svgEl(tag, attrs) {
  const el = document.createElementNS('http://www.w3.org/2000/svg', tag);
  for (const k in attrs) el.setAttribute(k, attrs[k]);
  return el;
}
function drawAutomationSvg(svg, tr, lane) {
  const w = svg.viewBox.baseVal.width;
  const h = svg.viewBox.baseVal.height;
  const { min, max } = autoRange(tr, lane.param);
  const pts = lane.points;
  pts.sort((a, b) => a.beat - b.beat);
  svg.textContent = '';

  const by = autoY(min, max, clamp(autoBaseValue(tr, lane.param), min, max), h);
  svg.appendChild(svgEl('line', { x1: 0, y1: by, x2: w, y2: by, 'class': 'studio-auto-base' }));

  if (pts.length) {
    const pl = svgEl('polyline', { 'class': 'studio-auto-line' });
    const d = [];
    for (const p of pts) d.push(`${(p.beat * PPB).toFixed(2)},${autoY(min, max, p.value, h).toFixed(2)}`);
    pl.setAttribute('points', d.join(' '));
    svg.appendChild(pl);
  }

  pts.forEach((p, idx) => {
    const x = p.beat * PPB;
    const y = autoY(min, max, p.value, h);
    const c = svgEl('circle', { cx: x, cy: y, r: 4, 'class': 'studio-auto-handle' });
    c.addEventListener('pointerdown', (e) => startAutoDrag(e, svg, tr, lane, pts, idx));
    c.addEventListener('dblclick', (e) => {
      e.stopPropagation();
      pts.splice(idx, 1);
      markDirty();
      renderArranger();
    });
    svg.appendChild(c);
  });
}
function startAutoDrag(e, svg, tr, lane, pts, idx) {
  e.stopPropagation();
  e.preventDefault();
  const h = svg.viewBox.baseVal.height;
  const { min, max } = autoRange(tr, lane.param);
  const pt = pts[idx];
  const move = (ev) => {
    const rect = svg.getBoundingClientRect();
    const x = ev.clientX - rect.left;
    const y = ev.clientY - rect.top;
    pt.beat = Math.max(0, Math.round((x / PPB) * 4) / 4);
    pt.value = clamp(Math.round(autoValueY(min, max, y, h) * 100) / 100, min, max);
    drawAutomationSvg(svg, tr, lane);
  };
  const up = () => {
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    markDirty();
    renderArranger();
  };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
}
/* Build one automation lane row (header + envelope SVG). */
function renderAutomationLaneRow(tr, lane, catalog, beats, autoRow, isFirst, lanes) {
  const hd = h('div', 'studio-auto-head');
  hd.style.gridColumn = '1';
  hd.style.gridRow = String(autoRow);
  hd.style.setProperty('--track', trackColor(tr.color));

  const param = lane ? lane.param : 'track.level';
  hd.appendChild(nativeSelect(catalog.map((c) => [c.path, c.label]), param, (val) => {
    if (lane) lane.param = val;
    else lanes.push({ param: val, points: [] });
    markDirty();
    renderArranger();
  }, t('plugin.studio.hint.automationParam', null, 'Automation parameter')));

  if (isFirst) {
    const add = iconCtl('ui/plus');
    add.type = 'button';
    add.title = t('plugin.studio.automation.addLane', null, 'Add another automation lane');
    add.addEventListener('click', () => { lanes.push({ param: 'track.level', points: [] }); markDirty(); renderArranger(); });
    hd.appendChild(add);
  }
  if (lane) {
    const clear = iconCtl('ui/eraser');
    clear.type = 'button';
    clear.title = t('plugin.studio.automation.clear', null, 'Clear breakpoints');
    clear.addEventListener('click', () => { lane.points = []; markDirty(); renderArranger(); });
    hd.appendChild(clear);
    const rm = iconCtl('ui/close');
    rm.type = 'button';
    rm.title = t('plugin.studio.automation.removeLane', null, 'Remove lane');
    rm.addEventListener('click', () => { lanes.splice(lanes.indexOf(lane), 1); markDirty(); renderArranger(); });
    hd.appendChild(rm);
  }
  arrGridEl.appendChild(hd);

  const laneEl = h('div', 'studio-auto-lane');
  laneEl.style.gridColumn = '2';
  laneEl.style.gridRow = String(autoRow);
  laneEl.style.setProperty('--track', trackColor(tr.color));
  if (lane) {
    const svg = svgEl('svg', { 'class': 'studio-auto-svg', width: String(beats * PPB), height: String(AUTO_H), viewBox: `0 0 ${beats * PPB} ${AUTO_H}` });
    drawAutomationSvg(svg, tr, lane);
    svg.addEventListener('pointerdown', (e) => {
      if (e.target !== svg) return;
      const rect = svg.getBoundingClientRect();
      const x = e.clientX - rect.left;
      const y = e.clientY - rect.top;
      const { min, max } = autoRange(tr, lane.param);
      const beat = Math.max(0, Math.round((x / PPB) * 4) / 4);
      const value = clamp(Math.round(autoValueY(min, max, y, AUTO_H) * 100) / 100, min, max);
      lane.points.push({ beat, value });
      markDirty();
      renderArranger();
    });
    laneEl.appendChild(svg);
  }
  arrGridEl.appendChild(laneEl);
}
function renderAutomationSection(tr, beats, autoRows) {
  const lanes = autoLanes(tr);
  const catalog = trackParamCatalog(tr);
  const count = Math.max(1, lanes.length);
  for (let li = 0; li < count; li++) {
    renderAutomationLaneRow(tr, lanes[li] || null, catalog, beats, autoRows[li], li === 0, lanes);
  }
}

/* Connect a device knob to a (new or existing) automation lane. */
function connectKnobToAutomation(path, label) {
  let trackId = null;
  if (sel?.area === 'arr') trackId = arrangement.clips[sel.clipIndex]?.track;
  let tr = arrangement.tracks.find((t) => t.id === trackId);
  if (!tr) tr = arrangement.tracks[0];
  if (!tr) { toast(t('plugin.studio.toast.noTrackToAutomate', null, 'No arrangement track to automate'), { type: 'error' }); return; }
  tr.autoOpen = true;
  autoLane(tr, path);
  markDirty();
  renderArranger();
  toast(t('plugin.studio.toast.automating', { label, track: tr.name }, `Automating ${label} on “${tr.name}” — click the lane to add points`), { type: 'success' });
}

/* A device knob with an ⌁ button that connects it to an automation lane. */
function automatableKnob(label, path, min, max, step, value, onchange, resetTo) {
  const wrap = h('div', 'studio-knob-wrap');
  const k = knob(label, min, max, step, value, onchange, resetTo);
  wrap.append(k);
  const auto = iconCtl('ui/curves', 'studio-knob-auto');
  auto.type = 'button';
  auto.title = t('plugin.studio.automation.automate', { label }, `Automate ${label}`);
  auto.addEventListener('click', (e) => { e.stopPropagation(); connectKnobToAutomation(path, label); });
  wrap.appendChild(auto);
  /* Register for automation-follow: during playback the dial mirrors the lane. */
  const trackId = sel?.area === 'arr' ? arrangement.clips[sel.clipIndex]?.track : null;
  autoKnobs.push({ path, trackId, min, max, dial: k._dial, set: k._set });
  return wrap;
}

/* Linear interpolation of an automation envelope at a beat (holds ends). */
function sampleEnv(points, beat, fallback) {
  if (!points.length) return fallback;
  if (beat <= points[0].beat) return points[0].value;
  if (beat >= points[points.length - 1].beat) return points[points.length - 1].value;
  for (let i = 0; i < points.length - 1; i++) {
    const a = points[i], b = points[i + 1];
    if (beat >= a.beat && beat <= b.beat) {
      const span = b.beat - a.beat;
      if (span <= 0) return b.value;
      return a.value + (b.value - a.value) * ((beat - a.beat) / span);
    }
  }
  return fallback;
}
/* Move registered knobs to the automation value at the playhead position. */
function updateAutoKnobs(beat) {
  for (const ak of autoKnobs) {
    if (!ak.trackId || !ak.dial || !ak.dial.isConnected) continue;
    const tr = arrangement.tracks.find((t) => t.id === ak.trackId);
    if (!tr) continue;
    const lane = autoLanes(tr).find((l) => l.param === ak.path);
    if (!lane || !lane.points.length) continue;
    const v = sampleEnv(lane.points, beat, autoBaseValue(tr, ak.path));
    ak.set(Math.min(ak.max, Math.max(ak.min, v)));
  }
}

/* Beats the timeline shows: the arrangement, every clip, and at least 16
   bars of open space after them (more as the user scrolls right). */
function contentEndBeats() {
  let end = arrangement.length_beats;
  for (const c of arrangement.clips) end = Math.max(end, c.start + clipDurBeats(c));
  return end;
}
function viewBeats() {
  const room = Math.max(64, arrScrollEl ? Math.ceil((arrScrollEl.clientWidth - HEAD_W) / PPB) : 64);
  return Math.ceil((contentEndBeats() + room + viewExtraBeats) / 4) * 4;
}
/* Horizontal zoom keeping the beat under `clientX` (default: view centre) still. */
function zoomArranger(next, clientX = null) {
  if (!arrScrollEl) return;
  const ppb = clamp(next, PPB_MIN, PPB_MAX);
  if (Math.abs(ppb - PPB) < 0.01) return;
  const r = arrScrollEl.getBoundingClientRect();
  const x = (clientX ?? (r.left + HEAD_W + (r.width - HEAD_W) / 2)) - r.left;
  const beat = (arrScrollEl.scrollLeft + x - HEAD_W) / PPB;
  PPB = ppb;
  renderArranger();
  if (arrPlaying) { const b = currentBeat(); if (arrPlayheadEl) { arrPlayheadEl.style.opacity = '1'; arrPlayheadEl.style.transform = `translateX(${HEAD_W + b * PPB}px)`; } }
  arrScrollEl.scrollLeft = Math.max(0, beat * PPB + HEAD_W - x);
}
function zoomToFit() {
  if (!arrScrollEl) return;
  const end = Math.max(4, contentEndBeats());
  viewExtraBeats = 0;
  PPB = clamp((arrScrollEl.clientWidth - HEAD_W - 24) / end, PPB_MIN, PPB_MAX);
  renderArranger();
  arrScrollEl.scrollLeft = 0;
}
function onArrWheel(e) {
  if (e.ctrlKey || e.metaKey) {
    // Ctrl+wheel and trackpad pinch (Chromium sends pinch as ctrl+wheel).
    e.preventDefault();
    const factor = Math.exp(-e.deltaY * (e.deltaMode === 1 ? 0.05 : 0.004));
    zoomArranger(PPB * factor, e.clientX);
  } else if (e.shiftKey && Math.abs(e.deltaX) < Math.abs(e.deltaY)) {
    e.preventDefault();
    arrScrollEl.scrollLeft += e.deltaY;
  }
}
/* ── pinch zoom: every path a pinch can arrive by ──
   1. ctrl+wheel (mouse, and Chromium's trackpad pinch where the platform
      delivers it) — onArrWheel;
   2. WebKit `gesturestart/gesturechange` (scale);
   3. two touch pointers (touchscreens) via Pointer Events;
   4. a shell-sent `trackpad:pinch` event { phase, scale } (the PEAK'D kiosk
      runs Qt on X11, where touchpad pinches never reach the page);
   5. last resort: visualViewport scale changes. */
let arrPointer = { x: null, inside: false };
const arrTouches = new Map();     // pointerId → { x, y }
let pinchBase = null;             // { ppb, dist|scale, x }
function arrPointerX() {
  if (arrPointer.x != null) return arrPointer.x;
  const r = arrScrollEl?.getBoundingClientRect();
  return r ? r.left + HEAD_W + (r.width - HEAD_W) / 2 : 0;
}
function pinchBegin(x) { pinchBase = { ppb: PPB, x }; }
function pinchScale(scale) {
  if (!pinchBase || !Number.isFinite(scale) || scale <= 0) return;
  zoomArranger(pinchBase.ppb * scale, pinchBase.x);
}
function wireArrPinch(el) {
  el.addEventListener('pointermove', (e) => { arrPointer = { x: e.clientX, inside: true }; }, { passive: true });
  el.addEventListener('pointerleave', () => { arrPointer.inside = false; }, { passive: true });
  // WebKit gesture events.
  el.addEventListener('gesturestart', (e) => { e.preventDefault(); pinchBegin(e.clientX ?? arrPointerX()); }, { passive: false });
  el.addEventListener('gesturechange', (e) => { e.preventDefault(); pinchScale(e.scale); }, { passive: false });
  el.addEventListener('gestureend', (e) => { e.preventDefault(); pinchBase = null; }, { passive: false });
  // Two-finger touch pinch.
  const dist = () => { const [a, b] = [...arrTouches.values()]; return Math.hypot(a.x - b.x, a.y - b.y); };
  el.addEventListener('pointerdown', (e) => {
    if (e.pointerType !== 'touch') return;
    arrTouches.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (arrTouches.size === 2) {
      const [a, b] = [...arrTouches.values()];
      pinchBegin((a.x + b.x) / 2);
      pinchBase.dist = Math.max(1, dist());
    }
  }, true);
  el.addEventListener('pointermove', (e) => {
    if (e.pointerType !== 'touch' || !arrTouches.has(e.pointerId)) return;
    arrTouches.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (arrTouches.size === 2 && pinchBase?.dist) { e.preventDefault(); pinchScale(dist() / pinchBase.dist); }
  }, true);
  const lift = (e) => { if (arrTouches.delete(e.pointerId) && arrTouches.size < 2) pinchBase = null; };
  el.addEventListener('pointerup', lift, true);
  el.addEventListener('pointercancel', lift, true);
}
/* Shell bridge: window `trackpad:pinch` { phase: begin|update|end, scale }. */
function onShellPinch(e) {
  if (!arrScrollEl?.isConnected || !arrPointer.inside) return;
  const d = e.detail || {};
  if (d.phase === 'begin') pinchBegin(arrPointerX());
  else if (d.phase === 'update') { if (!pinchBase) pinchBegin(arrPointerX()); pinchScale(Number(d.scale)); }
  else pinchBase = null;
}
/* Last resort: the page itself got pinch-zoomed — follow it on the timeline. */
let lastViewportScale = window.visualViewport?.scale || 1;
function onViewportResize() {
  const sc = window.visualViewport?.scale || 1;
  if (sc === lastViewportScale) return;
  const ratio = sc / lastViewportScale;
  lastViewportScale = sc;
  if (arrPointer.inside && arrScrollEl?.isConnected) zoomArranger(PPB * ratio, arrPointerX());
}

let arrExtendTimer = 0;
function onArrScroll() {
  const el = arrScrollEl;
  if (!el || arrExtendTimer) return;
  if (el.scrollLeft + el.clientWidth > el.scrollWidth - PPB * 8) {
    arrExtendTimer = setTimeout(() => {
      arrExtendTimer = 0;
      const left = el.scrollLeft;
      viewExtraBeats += 64;
      renderArranger();
      el.scrollLeft = left;
    }, 60);
  }
}
function startEndMarkerDrag(e, mk) {
  e.preventDefault();
  e.stopPropagation();
  const startX = e.clientX;
  const l0 = arrangement.length_beats;
  try { mk.setPointerCapture(e.pointerId); } catch (_) { /* noop */ }
  const move = (ev) => {
    const q = PPB >= 24 ? 1 : 4;
    arrangement.length_beats = Math.max(4, Math.round((l0 + (ev.clientX - startX) / PPB) / q) * q);
    mk.style.left = `${arrangement.length_beats * PPB}px`;
  };
  const up = (ev) => {
    mk.removeEventListener('pointermove', move);
    mk.removeEventListener('pointerup', up);
    try { mk.releasePointerCapture(ev.pointerId); } catch (_) { /* noop */ }
    if (arrangement.length_beats !== l0) { markDirty(); renderArranger(); }
  };
  mk.addEventListener('pointermove', move);
  mk.addEventListener('pointerup', up);
}

function renderArranger() {
  if (!arrGridEl || !arrangement) return;
  arrGridEl.textContent = '';
  const beats = viewBeats();
  const n = arrangement.tracks.length;
  /* Rows: ruler, then each track's lane (+ automation lane when open), then add-track. */
  const rowFor = [];
  const rowHeights = [RULER_H];
  for (let t = 0; t < n; t++) {
    const trackRow = rowHeights.length + 1;
    rowHeights.push(TRACK_H);
    const autoRows = [];
    if (arrangement.tracks[t].autoOpen) {
      const laneCount = Math.max(1, autoLanes(arrangement.tracks[t]).length);
      for (let k = 0; k < laneCount; k++) {
        rowHeights.push(AUTO_H);
        autoRows.push(rowHeights.length);   // 1-based row of the just-pushed lane
      }
    }
    rowFor.push({ track: trackRow, autoRows });
  }
  const addRow = rowHeights.length + 1;
  rowHeights.push(TRACK_H);
  arrGridEl.style.gridTemplateColumns = `${HEAD_W}px ${beats * PPB}px`;
  arrGridEl.style.setProperty('--ppb', `${PPB}px`);
  arrGridEl.style.setProperty('--ppbar', `${PPB * 4}px`);
  arrGridEl.style.gridTemplateRows = rowHeights.map((h) => `${h}px`).join(' ');

  arrGridEl.appendChild(h('div', 'studio-arr-corner'));

  /* ruler — labels adapt to the zoom; click sets the play start, the end
     marker (draggable) sets the arrangement/export length */
  const ruler = h('div', 'studio-arr-ruler');
  ruler.style.gridColumn = '2';
  ruler.style.gridRow = '1';
  ruler.title = t('plugin.studio.arranger.playFromHere', null, 'Play from here');
  const barPx = PPB * 4;
  const every = [1, 2, 4, 8, 16, 32, 64].find((n) => n * barPx >= 44) || 64;
  for (let bar = 0; bar * 4 < beats; bar += 1) {
    const major = bar % every === 0;
    if (!major && barPx < 12) continue;
    const tick = h('span', `studio-arr-ruler-tick${major ? ' is-major' : ''}`, major ? String(bar + 1) : '');
    tick.style.left = `${bar * barPx}px`;
    ruler.appendChild(tick);
    if (PPB >= 36) {
      for (let bt = 1; bt < 4; bt++) {
        const sub = h('span', 'studio-arr-ruler-tick is-beat', PPB >= 70 ? `${bar + 1}.${bt + 1}` : '');
        sub.style.left = `${(bar * 4 + bt) * PPB}px`;
        ruler.appendChild(sub);
      }
    }
  }
  const lenShade = h('span', 'studio-arr-ruler-after');
  lenShade.style.left = `${arrangement.length_beats * PPB}px`;
  ruler.appendChild(lenShade);
  if (arrLoop) {
    const lr = h('span', 'studio-arr-ruler-loop');
    lr.style.width = `${arrangement.length_beats * PPB}px`;
    ruler.appendChild(lr);
  }
  const endMk = h('span', 'studio-arr-endmarker');
  endMk.style.left = `${arrangement.length_beats * PPB}px`;
  endMk.title = t('plugin.studio.arranger.endMarker', null, 'Arrangement end (loop and export length) — drag to change');
  endMk.addEventListener('pointerdown', (e) => startEndMarkerDrag(e, endMk));
  ruler.appendChild(endMk);
  ruler.addEventListener('click', (e) => {
    if (e.target === endMk) return;
    const r = ruler.getBoundingClientRect();
    const q = PPB >= 24 ? 1 : 4;
    arrStartBeat = Math.max(0, Math.floor(((e.clientX - r.left) / PPB) / q) * q);
    renderArranger();
    clearArrPlayhead();
  });
  arrGridEl.appendChild(ruler);

  /* track headers */
  for (let ti = 0; ti < n; ti++) {
    const tr = arrangement.tracks[ti];
    const head = h('div', 'studio-arr-head');
    head.style.gridColumn = '1';
    head.style.gridRow = String(rowFor[ti].track);
    head.style.setProperty('--track', trackColor(tr.color));
    if (sel?.area === 'arr' && arrTrackIndex(arrangement.clips[sel.clipIndex]?.track) === ti) head.classList.add('studio-arr-head--active');
    head.appendChild(h('span', 'studio-track-stripe'));

    const name = h('span', 'studio-arr-name', tr.name);
    name.title = t('plugin.studio.track.renameHint', null, 'Double-click to rename');
    name.addEventListener('dblclick', (e) => { e.stopPropagation(); inlineRename(name, tr.name, (v) => { tr.name = v; markDirty(); renderArranger(); }); });
    head.appendChild(name);

    const btns = h('span', 'studio-arr-head-btns');
    const mute = h('button', 'studio-ms', 'M');
    mute.type = 'button';
    mute.title = t('plugin.studio.track.mute', null, 'Mute');
    mute.classList.toggle('studio-ms--on', !!tr.mute);
    mute.addEventListener('click', (e) => { e.stopPropagation(); tr.mute = !tr.mute; markDirty(); renderArranger(); });
    const solo = h('button', 'studio-ms studio-ms--solo', 'S');
    solo.type = 'button';
    solo.title = t('plugin.studio.track.solo', null, 'Solo');
    solo.classList.toggle('studio-ms--on', soloTracks.has(tr.id));
    solo.addEventListener('click', (e) => { e.stopPropagation(); toggleSolo(tr.id); renderArranger(); });
    const add = iconCtl('ui/plus');
    add.type = 'button';
    add.title = t('plugin.studio.track.addClip', null, 'Add clip');
    add.addEventListener('click', (e) => { e.stopPropagation(); addArrClip(ti); });
    const auto = h('button', 'studio-head-btn', 'A');
    auto.type = 'button';
    auto.title = t('plugin.studio.track.toggleAutomation', null, 'Show/hide automation lane');
    auto.classList.toggle('studio-btn--on', !!tr.autoOpen);
    auto.addEventListener('click', (e) => { e.stopPropagation(); tr.autoOpen = !tr.autoOpen; renderArranger(); });
    const del = iconCtl('ui/close');
    del.type = 'button';
    del.title = t('plugin.studio.track.remove', null, 'Remove track');
    del.addEventListener('click', (e) => { e.stopPropagation(); removeArrTrack(ti); });
    if (tr.kind === 'audio') add.classList.add('hidden');
    btns.append(mute, solo, auto, add, del);
    head.appendChild(btns);
    head.appendChild(meterNode(() => channels.get(tr.id)?.meter, { vertical: false, compact: true }));
    arrGridEl.appendChild(head);

    /* lane background */
    const lane = h('div', 'studio-arr-lane');
    lane.dataset.trackId = tr.id;
    lane.style.gridColumn = '2';
    lane.style.gridRow = String(rowFor[ti].track);
    lane.style.setProperty('--track', trackColor(tr.color));
    if (!arrTrackAudible(tr)) lane.classList.add('studio-arr-lane--muted');
    lane.addEventListener('click', () => {
      if (!sel) return;
      sel = null;
      current = null;
      renderArranger();
      renderDetail();
    });
    arrGridEl.appendChild(lane);

    /* automation lanes (one per lane) */
    if (rowFor[ti].autoRows.length) renderAutomationSection(tr, beats, rowFor[ti].autoRows);
  }

  /* add track row */
  const addTrack = plusLabel(h('button', 'studio-arr-addtrack'), t('plugin.studio.arranger.addTrack', null, '+ Add track'));
  addTrack.type = 'button';
  addTrack.style.gridColumn = '1';
  addTrack.style.gridRow = String(addRow);
  addTrack.addEventListener('click', () => openBrowser('instruments', 'addTrack'));
  arrGridEl.appendChild(addTrack);

  /* clips */
  for (let i = 0; i < arrangement.clips.length; i++) {
    const clip = arrangement.clips[i];
    const tIndex = arrTrackIndex(clip.track);
    if (tIndex < 0) continue;
    const tr = arrangement.tracks[tIndex];
    const start = Math.max(0, clip.start);
    const dur = clipDurBeats(clip);
    const isAudio = !!clip.audio;
    const name = isAudio ? (clip.audio.name || clip.audio.src.split('/').pop()) : (clip.pattern?.title || tr.name);
    const el = h('div', `studio-arr-clip${isAudio ? ' studio-arr-clip--audio' : ''}`);
    // Absolute placement inside the lane row so fractional starts/lengths draw exactly.
    el.style.gridColumn = '2';
    el.style.gridRow = String(rowFor[tIndex].track);
    el.style.marginLeft = `${start * PPB}px`;
    el.style.width = `${Math.max(6, dur * PPB - 2)}px`;
    el.style.setProperty('--track', trackColor(tr.color));
    el.dataset.clipIndex = i;
    el.title = t('plugin.studio.clip.hint3', { name }, `${name} — drag to move (also to another track) · Ctrl-drag duplicates · edges trim · Alt-click splits · double-click edits`);
    el.addEventListener('contextmenu', (ev) => { ctxClip = { index: i, beat: snapBeat(beatAtClientX(ev.clientX), { shiftKey: true }) }; });
    if (sel?.area === 'arr' && sel.clipIndex === i) el.classList.add('studio-arr-clip--sel');
    if (!arrTrackAudible(tr)) el.classList.add('studio-arr-clip--muted');
    const cv = h('canvas', isAudio ? 'studio-arr-clip-wave studio-arr-clip-audio' : 'studio-arr-clip-wave studio-arr-clip-pattern');
    el.appendChild(cv);
    /* loop repetitions of a pattern clip */
    if (!isAudio && clip.loop !== false) {
      const pass = patternBeats(clip);
      const off = (clip.offset_beats || 0) % pass;
      for (let k = pass - off; k < dur - 0.01; k += pass) {
        if (k <= 0.01) continue;
        const mark = h('span', 'studio-arr-clip-loopmark');
        mark.style.left = `${k * PPB}px`;
        el.appendChild(mark);
      }
    }
    if (isAudio && (clip.audio.fade_in > 0.02 || clip.audio.fade_out > 0.06)) {
      const spb = 60 / arrangement.bpm;
      if (clip.audio.fade_in > 0.02) { const f = h('span', 'studio-arr-clip-fade studio-arr-clip-fade--in'); f.style.width = `${(clip.audio.fade_in / spb) * PPB}px`; el.appendChild(f); }
      if (clip.audio.fade_out > 0.06) { const f = h('span', 'studio-arr-clip-fade studio-arr-clip-fade--out'); f.style.width = `${(clip.audio.fade_out / spb) * PPB}px`; el.appendChild(f); }
    }
    const label = h('span', 'studio-arr-clip-name', name);
    if (clip.gain_db) label.appendChild(h('span', 'studio-arr-clip-gain', `${clip.gain_db > 0 ? '+' : ''}${clip.gain_db.toFixed(1)} dB`));
    el.appendChild(label);
    if (!isAudio) {
      const send = iconCtl('ui/launcher', 'studio-arr-clip-x');
      send.type = 'button';
      send.style.right = '22px';
      send.title = t('plugin.studio.clip.sendToLauncher', null, 'Send to Launcher');
      send.addEventListener('pointerdown', (e) => e.stopPropagation());
      send.addEventListener('click', (e) => { e.stopPropagation(); sendArrangementToLauncher(i); });
      el.appendChild(send);
    }
    const rm = iconCtl('ui/close', 'studio-arr-clip-x');
    rm.type = 'button';
    rm.title = t('plugin.studio.clip.remove', null, 'Remove clip');
    rm.addEventListener('pointerdown', (e) => e.stopPropagation());
    rm.addEventListener('click', (e) => { e.stopPropagation(); removeArrClip(i); });
    el.appendChild(rm);
    const grip = h('span', 'studio-arr-clip-resize');
    grip.title = t('plugin.studio.clip.resize', null, 'Drag to change the clip length');
    grip.addEventListener('pointerdown', (e) => startClipResize(e, i));
    el.appendChild(grip);
    const trim = h('span', 'studio-arr-clip-resize studio-arr-clip-resize--start');
    trim.title = t('plugin.studio.clip.trimStart', null, 'Drag to trim the clip start');
    trim.addEventListener('pointerdown', (e) => startClipTrim(e, i));
    el.appendChild(trim);
    el.addEventListener('click', () => selectArrClip(i));
    el.addEventListener('dblclick', () => selectArrClip(i, { openEditor: true }));
    el.addEventListener('pointerdown', (e) => startClipDrag(e, i));
    arrGridEl.appendChild(el);
    if (isAudio) {
      if (audioInfo(clip.audio.src)) requestAnimationFrame(() => drawAudioClip(i));
      else void loadAudioInfo(clip.audio.src).then(() => renderArranger()).catch(() => {});
    } else requestAnimationFrame(() => drawPatternClip(i));
  }

  /* playhead + start marker */
  arrPlayheadEl = h('div', 'studio-arr-playhead');
  arrPlayheadEl.style.opacity = '0';
  arrGridEl.appendChild(arrPlayheadEl);
  const startMarkerEl = h('div', 'studio-arr-startmarker');
  startMarkerEl.style.transform = `translateX(${HEAD_W + Math.min(arrStartBeat, beats - 1) * PPB}px)`;
  /* zoom controls in the corner */
  const corner = arrGridEl.querySelector('.studio-arr-corner');
  if (corner) {
    const zo = iconCtl('ui/zoom-out', 'studio-head-btn', t('plugin.studio.arranger.zoomOut', null, 'Zoom out (Ctrl+wheel)'));
    zo.addEventListener('click', () => zoomArranger(PPB / 1.5));
    const fit = h('button', 'studio-head-btn studio-zoom-fit', t('plugin.studio.arranger.fit', null, 'Fit'));
    fit.type = 'button';
    fit.title = t('plugin.studio.arranger.fitHint', null, 'Zoom to fit (Z)');
    fit.addEventListener('click', () => zoomToFit());
    const zi = iconCtl('ui/zoom-in', 'studio-head-btn', t('plugin.studio.arranger.zoomIn', null, 'Zoom in (Ctrl+wheel)'));
    zi.addEventListener('click', () => zoomArranger(PPB * 1.5));
    corner.append(zo, fit, zi);
  }
  startMarkerEl.title = t('plugin.studio.arranger.playStart', null, 'Play start (click the ruler to move)');
  arrGridEl.appendChild(startMarkerEl);
}

function inlineRename(span, value, done) {
  const input = uiInput();
  input.type = 'text';
  input.classList.add(...('studio-rename').split(/\s+/));
  input.value = value;
  span.replaceWith(input);
  input.focus();
  input.select();
  const commit = () => done(input.value.trim() || value);
  input.addEventListener('blur', commit);
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') input.blur();
    if (e.key === 'Escape') { input.value = value; input.blur(); }
    e.stopPropagation();
  });
}

function toggleSolo(id) {
  if (soloTracks.has(id)) soloTracks.delete(id);
  else soloTracks.add(id);
  for (const tr of launcher.tracks) ensureTrackAudio(tr);
  refreshLiveMix();
}

function addArrTrack(kind) {
  const i = arrangement.tracks.length;
  const label = kind ? KIND_LABELS[kind] : `Track ${i + 1}`;
  const tr = { id: `t${Date.now()}`, name: label, kind: 'instrument', color: i % TRACK_COLORS.length, mute: false, level: 0.8, pan: 0, fx: [], sends: null, automation: { lanes: [] } };
  arrangement.tracks.push(tr);
  const pattern = kind ? simplePattern(kind, 16) : (i % 3 === 0 ? kitPattern(16) : (i % 3 === 1 ? bassPattern(16) : leadPattern(16)));
  arrangement.clips.push({ track: tr.id, start: 0, length_beats: null, loop: true, gain_db: 0, audio: null, pattern });
  markDirty();
  renderArranger();
}
function removeArrTrack(t) {
  const id = arrangement.tracks[t].id;
  arrangement.tracks.splice(t, 1);
  arrangement.clips = arrangement.clips.filter((c) => c.track !== id);
  if (sel?.area === 'arr') { sel = null; }
  markDirty();
  syncSelection();
  renderArranger();
}
/* Put a WAV (Files-relative path) on a new audio track at `startBeat`. */
async function addAudioTrack(path, { startBeat = null, name = null, matchTempo = false } = {}) {
  let info = null;
  try { info = await loadAudioInfo(path); } catch (e) {
    toast(e.message || t('plugin.studio.toast.audioFailed', null, 'Could not read the audio file'), { type: 'error' });
    return;
  }
  if (matchTempo && Number.isFinite(info?.bpm)) {
    arrangement.bpm = info.bpm;
    if (launcher) launcher.bpm = info.bpm;
    if (bpmInput) bpmInput.value = String(Math.round(info.bpm * 100) / 100);
  }
  const i = arrangement.tracks.length;
  const label = name || path.split('/').pop().replace(/\.wav$/i, '');
  const tr = { id: `a${Date.now()}`, name: label, kind: 'audio', color: i % TRACK_COLORS.length, mute: false, level: 0.8, pan: 0, fx: [], sends: { reverb: 0, delay: 0 }, automation: { lanes: [] } };
  arrangement.tracks.push(tr);
  const start = startBeat ?? (arrPlaying ? Math.floor(currentBeat()) : arrStartBeat);
  arrangement.clips.push({
    track: tr.id, start, length_beats: null, loop: false, gain_db: 0, pattern: null,
    audio: { src: path, name: label, offset: 0, fade_in: 0.005, fade_out: 0.05, bpm: Number.isFinite(info?.bpm) ? info.bpm : null },
  });
  fitArrangementLength();
  selectArrClip(arrangement.clips.length - 1);
  markDirty();
  renderArranger();
  if (detailPage === 'mixer') renderMixer();
  toast(t('plugin.studio.toast.audioAdded', { name: label }, `Added “${label}”`), { type: 'success' });
}
function addArrClip(t) {
  const tr = arrangement.tracks[t];
  if (!tr) return;
  const start = arrangement.clips.filter((c) => c.track === tr.id).length * 4;
  const pattern = t % 3 === 0 ? kitPattern(16) : (t % 3 === 1 ? bassPattern(16) : leadPattern(16));
  arrangement.clips.push({ track: tr.id, start, length_beats: null, loop: true, gain_db: 0, audio: null, pattern });
  fitArrangementLength();
  markDirty();
  renderArranger();
}
function removeArrClip(i) {
  arrangement.clips.splice(i, 1);
  if (sel?.area === 'arr') {
    if (sel.clipIndex === i) sel = null;
    else if (sel.clipIndex > i) sel.clipIndex -= 1;
  }
  markDirty();
  syncSelection();
  renderArranger();
}

function snapBeat(v, e) {
  const q = e.shiftKey ? 0.25 : 1;
  return Math.round(v / q) * q;
}
/* Beat under a client X inside the arranger grid. */
function beatAtClientX(x) {
  const r = arrGridEl.getBoundingClientRect();
  return Math.max(0, (x - r.left - HEAD_W) / PPB);
}
/* The track lane whose row contains client Y (automation rows excluded). */
function laneAtClientY(y) {
  for (const lane of arrGridEl.querySelectorAll('.studio-arr-lane')) {
    const r = lane.getBoundingClientRect();
    if (y >= r.top && y < r.bottom) return lane;
  }
  return null;
}
function cloneClip(c) {
  return normalizeArrangement({ tracks: [], clips: [JSON.parse(JSON.stringify(serializeClip(c)))] }).clips[0];
}
/* Split a clip at an arrangement beat: the second half continues the
   pattern (offset_beats) or the file (audio offset) from the split point. */
function splitClipAt(i, beat) {
  const c = arrangement.clips[i];
  if (!c) return false;
  const dur = clipDurBeats(c);
  const at = Math.round(beat * 4) / 4;
  if (at <= c.start + 0.1 || at >= c.start + dur - 0.1) return false;
  const second = cloneClip(c);
  const cut = at - c.start;
  c.length_beats = cut;
  second.start = at;
  second.length_beats = dur - cut;
  if (c.audio) {
    second.audio.offset = (c.audio.offset || 0) + cut * 60 / arrangement.bpm;
    second.audio.fade_in = 0.005;
    c.audio.fade_out = 0.005;
  } else {
    second.offset_beats = (c.offset_beats || 0) + cut;
    second.loop = true;
    if (c.loop === false) { c.loop = true; }
  }
  arrangement.clips.splice(i + 1, 0, second);
  if (sel?.area === 'arr' && sel.clipIndex > i) sel.clipIndex += 1;
  selectArrClip(i + 1);
  markDirty();
  return true;
}
function playheadBeat() {
  return arrPlaying ? currentBeat() : arrStartBeat;
}
function splitSelectedAtPlayhead() {
  if (sel?.area !== 'arr') return;
  if (!splitClipAt(sel.clipIndex, playheadBeat())) toast(t('plugin.studio.toast.splitOutside', null, 'Move the playhead inside the selected clip to split it'), { type: 'error' });
}

function startClipDrag(e, clipIndex) {
  if (e.button !== 0) return;
  e.preventDefault();
  e.stopPropagation();
  let clip = arrangement.clips[clipIndex];
  let el = e.currentTarget;
  const startX = e.clientX;
  const startY = e.clientY;
  const startBeat = clip.start;
  const clickBeat = beatAtClientX(e.clientX);
  const copy = e.ctrlKey || e.metaKey;
  const alt = e.altKey;
  let duplicated = false;
  let moved = false;
  let hot = null;
  const target = el;   // keeps the pointer capture and the listeners
  try { target.setPointerCapture(e.pointerId); } catch (_) { /* noop */ }
  const move = (ev) => {
    if (!moved && Math.abs(ev.clientX - startX) < 4 && Math.abs(ev.clientY - startY) < 4) return;
    moved = true;
    if ((copy || alt) && !duplicated) {
      // Alt/Ctrl-drag: leave the original, drag a copy.
      duplicated = true;
      const dup = cloneClip(clip);
      arrangement.clips.push(dup);
      const ghost = el.cloneNode(true);
      ghost.classList.add('studio-arr-clip--ghost');
      el.parentNode.appendChild(ghost);
      el = ghost;
      clip = dup;
    }
    clip.start = Math.max(0, snapBeat(startBeat + (ev.clientX - startX) / PPB, ev));
    el.style.marginLeft = `${clip.start * PPB}px`;
    const lane = laneAtClientY(ev.clientY);
    if (hot && hot !== lane) hot.classList.remove('studio-arr-lane--target');
    hot = lane;
    if (lane) {
      lane.classList.add('studio-arr-lane--target');
      el.style.gridRow = lane.style.gridRow;
      const id = lane.dataset.trackId;
      if (id) clip.track = id;
    }
  };
  const up = (ev) => {
    target.removeEventListener('pointermove', move);
    target.removeEventListener('pointerup', up);
    target.removeEventListener('pointercancel', up);
    try { target.releasePointerCapture(ev.pointerId); } catch (_) { /* noop */ }
    if (hot) hot.classList.remove('studio-arr-lane--target');
    if (!moved) {
      if (alt) splitClipAt(clipIndex, snapBeat(clickBeat, ev));
      return;
    }
    fitArrangementLength();
    if (duplicated) selectArrClip(arrangement.clips.length - 1);
    markDirty();
    renderArranger();
  };
  target.addEventListener('pointermove', move);
  target.addEventListener('pointerup', up);
  target.addEventListener('pointercancel', up);
}
/* Right edge: change the length. */
function startClipResize(e, clipIndex) {
  if (e.button !== 0) return;
  e.preventDefault();
  e.stopPropagation();
  const clip = arrangement.clips[clipIndex];
  const grip = e.currentTarget;
  const el = grip.parentElement;
  const startX = e.clientX;
  const startDur = clipDurBeats(clip);
  let maxDur = Infinity;
  if (clip.audio) {
    const info = audioInfo(clip.audio.src);
    if (info?.duration) maxDur = (info.duration - (clip.audio.offset || 0)) * arrangement.bpm / 60;
  } else if (clip.loop === false) {
    maxDur = Math.max(0.25, patternBeats(clip) - (clip.offset_beats || 0));
  }
  let changed = false;
  try { grip.setPointerCapture(e.pointerId); } catch (_) { /* noop */ }
  const move = (ev) => {
    const d = clamp(snapBeat(startDur + (ev.clientX - startX) / PPB, ev), 0.25, maxDur);
    if (d !== clip.length_beats) { clip.length_beats = d; changed = true; }
    el.style.width = `${Math.max(6, d * PPB - 2)}px`;
    if (clip.audio) drawAudioClip(clipIndex); else drawPatternClip(clipIndex);
  };
  const up = (ev) => {
    grip.removeEventListener('pointermove', move);
    grip.removeEventListener('pointerup', up);
    try { grip.releasePointerCapture(ev.pointerId); } catch (_) { /* noop */ }
    if (changed) { fitArrangementLength(); markDirty(); renderArranger(); }
  };
  grip.addEventListener('pointermove', move);
  grip.addEventListener('pointerup', up);
}
/* Left edge: trim the start (content stays in place on the timeline). */
function startClipTrim(e, clipIndex) {
  if (e.button !== 0) return;
  e.preventDefault();
  e.stopPropagation();
  const clip = arrangement.clips[clipIndex];
  const grip = e.currentTarget;
  const el = grip.parentElement;
  const startX = e.clientX;
  const s0 = clip.start;
  const d0 = clipDurBeats(clip);
  const spb = 60 / arrangement.bpm;
  const off0 = clip.audio ? (clip.audio.offset || 0) / spb : (clip.offset_beats || 0);
  // How far left we may extend: to the beginning of the content.
  const minDelta = Math.max(-s0, -off0);
  let changed = false;
  try { grip.setPointerCapture(e.pointerId); } catch (_) { /* noop */ }
  const move = (ev) => {
    const delta = clamp(snapBeat((ev.clientX - startX) / PPB, ev), minDelta, d0 - 0.25);
    clip.start = s0 + delta;
    clip.length_beats = d0 - delta;
    if (clip.audio) clip.audio.offset = Math.max(0, (off0 + delta) * spb);
    else clip.offset_beats = Math.max(0, off0 + delta);
    changed = true;
    el.style.marginLeft = `${clip.start * PPB}px`;
    el.style.width = `${Math.max(6, clip.length_beats * PPB - 2)}px`;
    if (clip.audio) drawAudioClip(clipIndex); else drawPatternClip(clipIndex);
  };
  const up = (ev) => {
    grip.removeEventListener('pointermove', move);
    grip.removeEventListener('pointerup', up);
    try { grip.releasePointerCapture(ev.pointerId); } catch (_) { /* noop */ }
    if (changed) { markDirty(); renderArranger(); }
  };
  grip.addEventListener('pointermove', move);
  grip.addEventListener('pointerup', up);
}

/* Pattern clip content in one pass, in beats: melodic notes and drum hits
   per lane, the way the engine schedules them. */
function patternContent(p) {
  const steps = Math.max(1, p?.steps || 16);
  const notes = [];
  const lanes = [];
  for (const v of (p?.voices || [])) {
    if (MELODIC.has(v.kind) || v.kind === 'grid') {
      let ns = v.notes || [];
      if (!ns.length && v.rhythm) {
        ns = rhythmToCells(v.rhythm, steps).map((on, step) => (on ? { step, length: 1, degree: v.degree, octave: v.octave } : null)).filter(Boolean);
      }
      for (const n of ns) notes.push({ beat: n.step / 4, len: Math.max(1, n.length || 1) / 4, pitch: (n.degree || 0) + 12 * (n.octave || 0), vel: n.velocity ?? 0.8 });
    } else if (v.kind === 'drumkit') {
      const byPad = new Map();
      if ((v.notes || []).length) {
        for (const n of v.notes) { const pad = ((n.degree % 16) + 16) % 16; if (!byPad.has(pad)) byPad.set(pad, []); byPad.get(pad).push({ beat: n.step / 4, vel: n.velocity ?? 0.85 }); }
      } else {
        byPad.set(0, rhythmToCells(v.rhythm || '', steps).map((on, s) => (on ? { beat: s / 4, vel: 0.85 } : null)).filter(Boolean));
      }
      for (const pad of [...byPad.keys()].sort((a, b) => a - b)) lanes.push(byPad.get(pad));
    } else {
      lanes.push(rhythmToCells(v.rhythm || '', steps).map((on, s) => (on ? { beat: s / 4, vel: 0.85 } : null)).filter(Boolean));
    }
  }
  return { notes, lanes, beats: steps / 4 };
}
/* Draw a pattern clip at true scale: its content repeated per loop pass
   (starting `offset_beats` into the pattern), never stretched. */
function drawPatternClip(i) {
  const clip = arrangement.clips[i];
  if (!clip?.pattern) return;
  const cv = arrGridEl?.querySelector(`.studio-arr-clip[data-clip-index="${i}"] .studio-arr-clip-pattern`);
  if (!cv) return;
  const dpr = window.devicePixelRatio || 1;
  const w = cv.clientWidth || 60;
  const hgt = cv.clientHeight || 24;
  cv.width = Math.max(1, Math.round(w * dpr));
  cv.height = Math.max(1, Math.round(hgt * dpr));
  const ctx = cv.getContext('2d');
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, w, hgt);
  const { notes, lanes, beats: pass } = patternContent(clip.pattern);
  const dur = clipDurBeats(clip);
  const off = clip.offset_beats || 0;
  const loop = clip.loop !== false;
  const top = 14;                       // leave room for the name
  const innerH = Math.max(6, hgt - top - 3);
  const melH = notes.length ? (lanes.length ? innerH * 0.6 : innerH) : 0;
  const drumH = innerH - melH;
  const passes = loop ? Math.ceil((dur + off) / pass) + 1 : 1;
  const place = (beat, len, draw) => {
    for (let n = 0; n < passes; n++) {
      const local = beat + n * pass - off;
      if (local >= dur || local + len <= 0) continue;
      const x0 = Math.max(0, local) * PPB;
      const x1 = Math.min(dur, local + len) * PPB;
      draw(x0, Math.max(1, x1 - x0));
    }
  };
  ctx.fillStyle = canvasInk.ink;
  if (notes.length) {
    let lo = Infinity, hi = -Infinity;
    for (const n of notes) { lo = Math.min(lo, n.pitch); hi = Math.max(hi, n.pitch); }
    const span = Math.max(1, hi - lo + 1);
    const nh = Math.max(1.5, Math.min(5, melH / span - 0.5));
    for (const n of notes) {
      const y = top + ((hi - n.pitch) / span) * (melH - nh);
      ctx.globalAlpha = 0.45 + 0.4 * Math.min(1, n.vel);
      place(n.beat, n.len, (x, wd) => ctx.fillRect(x, y, Math.max(1.5, wd - 1), nh));
    }
  }
  if (lanes.length) {
    const lh = drumH / lanes.length;
    const hw = Math.max(1.5, Math.min(4, PPB / 8 - 1));
    lanes.forEach((hits, li) => {
      const y = top + melH + li * lh;
      for (const hit of hits) {
        ctx.globalAlpha = 0.35 + 0.5 * Math.min(1, hit.vel);
        place(hit.beat, 0.0001, (x) => ctx.fillRect(x, y + lh * 0.15, hw, Math.max(1, lh * 0.7)));
      }
    });
  }
  ctx.globalAlpha = 1;
}

/* Audio clip: the file's waveform from `offset` for the clip's length. */
function drawAudioClip(i) {
  const clip = arrangement.clips[i];
  const info = clip?.audio && audioInfo(clip.audio.src);
  if (!info?.peaks?.length) return;
  const cv = arrGridEl?.querySelector(`.studio-arr-clip[data-clip-index="${i}"] .studio-arr-clip-audio`);
  if (!cv) return;
  const dpr = window.devicePixelRatio || 1;
  const w = cv.clientWidth || 100;
  const hgt = cv.clientHeight || 24;
  cv.width = Math.max(1, Math.round(w * dpr));
  cv.height = Math.max(1, Math.round(hgt * dpr));
  const ctx = cv.getContext('2d');
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, w, hgt);
  const peaks = info.peaks;
  const secs = info.duration || 1;
  const spb = 60 / arrangement.bpm;
  const t0 = clip.audio.offset || 0;
  const t1 = t0 + (w / PPB) * spb;   // true time scale: one beat = PPB px
  const gain = dbToGain(clip.gain_db || 0);
  ctx.fillStyle = canvasInk.ink;
  ctx.globalAlpha = 0.42;
  const mid = hgt / 2;
  for (let x = 0; x < w; x++) {
    const a = t0 + ((t1 - t0) * x) / w;
    const b = t0 + ((t1 - t0) * (x + 1)) / w;
    const k0 = Math.floor((a / secs) * peaks.length);
    const k1 = Math.max(k0 + 1, Math.ceil((b / secs) * peaks.length));
    let mn = 0, mx = 0;
    for (let k = k0; k < k1 && k < peaks.length; k++) { if (k < 0) continue; mn = Math.min(mn, peaks[k][0]); mx = Math.max(mx, peaks[k][1]); }
    mn = Math.max(-1, mn * gain); mx = Math.min(1, mx * gain);
    ctx.fillRect(x, mid - mx * mid, 1, Math.max(1, (mx - mn) * mid));
  }
}

/* Redraw the selected arranger clip's preview after an edit (cheap; MIDI is
   synchronous, waveform reuses cached peaks). */
function refreshSelectedClipPreview() {
  if (sel?.area !== 'arr' || !arrGridEl) return;
  const i = sel.clipIndex;
  const clip = arrangement.clips[i];
  if (!clip) return;
  if (clip.audio) drawAudioClip(i);
  else drawPatternClip(i);
}

/* ── clip launcher (tracks × scenes) ────────────────────────── */

function renderLauncher() {
  if (!lchGridEl || !launcher) return;
  lchGridEl.textContent = '';
  const n = launcher.tracks.length;
  const s = launcher.scenes;
  const CELL = 132;
  const ROW = 48;
  lchGridEl.style.gridTemplateColumns = `${HEAD_W}px repeat(${s}, ${CELL}px) ${CELL}px`;
  lchGridEl.style.gridTemplateRows = `30px repeat(${n}, ${ROW}px) ${ROW}px`;

  lchGridEl.appendChild(h('div', 'studio-arr-corner'));

  /* scene header: one launch button per scene */
  for (let c = 0; c < s; c++) {
    const head = h('button', 'studio-launch-scene');
    head.type = 'button';
    head.style.gridColumn = String(c + 2);
    head.style.gridRow = '1';
    head.appendChild(icon('ui/play', { size: 11 }));
    head.appendChild(h('span', null, ` ${c + 1}`));
    head.title = t('plugin.studio.launcher.launchScene', { n: c + 1 }, `Launch scene ${c + 1}`);
    head.addEventListener('click', () => launchScene(c));
    lchGridEl.appendChild(head);
  }
  const addScene = plusLabel(h('button', 'studio-launch-scene studio-launch-scene--add'), t('plugin.studio.launcher.addScene', null, '+ Scene'));
  addScene.type = 'button';
  addScene.style.gridColumn = String(s + 2);
  addScene.style.gridRow = '1';
  addScene.addEventListener('click', () => { launcher.scenes += 1; renderLauncher(); });
  lchGridEl.appendChild(addScene);

  /* track headers */
  for (let ti = 0; ti < n; ti++) {
    const tr = launcher.tracks[ti];
    const head = h('div', 'studio-arr-head');
    head.style.gridColumn = '1';
    head.style.gridRow = String(ti + 2);
    head.style.setProperty('--track', trackColor(tr.color));
    head.appendChild(h('span', 'studio-track-stripe'));
    const name = h('span', 'studio-arr-name', tr.name);
    name.title = t('plugin.studio.track.renameHint', null, 'Double-click to rename');
    name.addEventListener('dblclick', (e) => { e.stopPropagation(); inlineRename(name, tr.name, (v) => { tr.name = v; renderLauncher(); }); });
    head.appendChild(name);
    const btns = h('span', 'studio-arr-head-btns');
    const mute = h('button', 'studio-ms', 'M');
    mute.type = 'button';
    mute.title = t('plugin.studio.track.mute', null, 'Mute');
    mute.classList.toggle('studio-ms--on', !!tr.mute);
    mute.setAttribute('aria-pressed', String(!!tr.mute));
    mute.addEventListener('click', (e) => { e.stopPropagation(); tr.mute = !tr.mute; ensureTrackAudio(tr); renderLauncher(); });
    const solo = h('button', 'studio-ms studio-ms--solo', 'S');
    solo.type = 'button';
    solo.title = t('plugin.studio.track.solo', null, 'Solo');
    solo.classList.toggle('studio-ms--on', soloTracks.has(tr.id));
    solo.setAttribute('aria-pressed', String(soloTracks.has(tr.id)));
    solo.addEventListener('click', (e) => { e.stopPropagation(); toggleSolo(tr.id); renderLauncher(); });
    const stopBtn = iconCtl('ui/stop');
    stopBtn.type = 'button';
    stopBtn.title = t('plugin.studio.launcher.stopTrackClips', null, 'Stop clips on this track');
    stopBtn.addEventListener('click', (e) => { e.stopPropagation(); stopLauncherTrack(tr.id, true); });
    const del = iconCtl('ui/close');
    del.type = 'button';
    del.title = t('plugin.studio.track.remove', null, 'Remove track');
    del.addEventListener('click', (e) => { e.stopPropagation(); removeLauncherTrack(ti); });
    btns.append(mute, solo, stopBtn, del);
    head.appendChild(btns);
    lchGridEl.appendChild(head);
  }
  const addTrack = plusLabel(h('button', 'studio-arr-addtrack'), t('plugin.studio.arranger.addTrack', null, '+ Add track'));
  addTrack.type = 'button';
  addTrack.style.gridColumn = '1';
  addTrack.style.gridRow = String(n + 2);
  addTrack.addEventListener('click', () => openBrowser('instruments', 'addTrack'));
  lchGridEl.appendChild(addTrack);

  /* clip slots */
  for (let ti = 0; ti < n; ti++) {
    const tr = launcher.tracks[ti];
    for (let c = 0; c < s; c++) {
      const key = `${tr.id}:${c}`;
      const clip = launcher.clips[key];
      const cell = h('div', 'studio-launch-slot');
      cell.style.gridColumn = String(c + 2);
      cell.style.gridRow = String(ti + 2);
      cell.style.setProperty('--track', trackColor(tr.color));
      if (clip) {
        cell.classList.add('studio-launch-clip');
        const lp = trackLoops[tr.id];
        const isThis = lp && lp.slotKey === key;
        if (isThis && lp.state === 'playing') cell.classList.add('studio-launch-clip--playing');
        if (isThis && lp.state === 'queued') cell.classList.add('studio-launch-clip--queued');
        if (sel?.area === 'lch' && sel.trackId === tr.id && sel.scene === c) cell.classList.add('studio-launch-clip--sel');
        const playIc = h('button', 'studio-launch-play');
        playIc.type = 'button';
        playIc.title = t('plugin.studio.launcher.launchClip', null, 'Launch clip');
        playIc.setAttribute('aria-label', t('plugin.studio.launcher.launchClip', null, 'Launch clip'));
        playIc.appendChild(icon('ui/play', { size: 10 }));
        playIc.addEventListener('click', (e) => { e.stopPropagation(); void launchClip(ti, c); });
        cell.appendChild(playIc);
        cell.appendChild(h('span', 'studio-arr-clip-name', clip.title || clip.pattern?.title || tr.name));
        const send = iconCtl('ui/arranger', 'studio-arr-clip-x');
        send.type = 'button';
        send.title = t('plugin.studio.clip.sendToArrangement', null, 'Send to Arranger');
        send.addEventListener('click', (e) => { e.stopPropagation(); sendLauncherToArrangement(tr.id, c); });
        cell.appendChild(send);
        const rm = iconCtl('ui/close', 'studio-arr-clip-x');
        rm.type = 'button';
        rm.style.right = '22px';
        rm.title = t('plugin.studio.clip.remove', null, 'Remove clip');
        rm.addEventListener('click', (e) => {
          e.stopPropagation();
          if (trackLoops[tr.id]?.slotKey === key) stopLauncherTrack(tr.id, false);
          delete launcher.clips[key];
          if (sel?.area === 'lch' && sel.trackId === tr.id && sel.scene === c) { sel = null; syncSelection(); }
          renderLauncher();
        });
        cell.appendChild(rm);
        cell.addEventListener('click', () => selectLauncherClip(tr.id, c));
        cell.addEventListener('dblclick', () => selectLauncherClip(tr.id, c, { openEditor: true }));
        cell.title = t('plugin.studio.launcher.clipHint', null, 'Click to select · ▶ to launch · double-click to edit');
      } else {
        cell.classList.add('studio-launch-empty');
        cell.title = t('plugin.studio.track.addClip', null, 'Add clip');
        cell.appendChild(icon('ui/plus', { size: 14 }));
        cell.addEventListener('click', () => addLauncherClip(ti, c));
      }
      lchGridEl.appendChild(cell);
    }
  }
}

function removeLauncherTrack(t) {
  const id = launcher.tracks[t].id;
  stopLauncherTrack(id, false);
  launcher.tracks.splice(t, 1);
  for (const key of Object.keys(launcher.clips)) if (key.startsWith(id + ':')) delete launcher.clips[key];
  if (sel?.area === 'lch' && sel.trackId === id) { sel = null; syncSelection(); }
  renderLauncher();
}
function addLauncherTrackWithKind(kind) {
  const i = launcher.tracks.length;
  const tr = { id: `lt${Date.now()}`, name: KIND_LABELS[kind] || `Track ${i + 1}`, color: i % TRACK_COLORS.length, mute: false, level: 0.8, pan: 0 };
  launcher.tracks.push(tr);
  launcher.clips[`${tr.id}:0`] = { pattern: simplePattern(kind, 16), title: KIND_LABELS[kind] };
  renderLauncher();
}
function addLauncherClip(ti, si) {
  const tr = launcher.tracks[ti];
  const fns = [kitPattern, bassPattern, leadPattern, padPattern];
  const fn = fns[ti % fns.length];
  launcher.clips[`${tr.id}:${si}`] = { pattern: fn(16), title: tr.name };
  selectLauncherClip(tr.id, si);
}

function sendLauncherToArrangement(trackId, sceneIdx) {
  const clip = launcher.clips[`${trackId}:${sceneIdx}`];
  if (!clip) return;
  const destTrack = arrangement.tracks[0] || { id: 't0' };
  const dur = Math.max(1, Math.round((clip.pattern.steps || 16) / 4));
  const start = arrangement.clips.filter((c) => c.track === destTrack.id).length * 4;
  arrangement.clips.push({ track: destTrack.id, start: Math.min(start, Math.round(arrangement.length_beats) - dur), pattern: normalizePattern(clip.pattern) });
  markDirty();
  renderArranger();
  toast(t('plugin.studio.toast.sentToArrangement', null, 'Sent clip to Arranger'), { type: 'success' });
}
function sendArrangementToLauncher(clipIndex) {
  const c = arrangement.clips[clipIndex];
  if (!c) return;
  const destTrack = launcher.tracks[0];
  if (!destTrack) return;
  let slot = 0;
  while (launcher.clips[`${destTrack.id}:${slot}`]) slot++;
  if (slot >= launcher.scenes) launcher.scenes = slot + 1;
  launcher.clips[`${destTrack.id}:${slot}`] = { pattern: normalizePattern(c.pattern), title: c.pattern.title || destTrack.name };
  renderLauncher();
  toast(t('plugin.studio.toast.sentToLauncher', null, 'Sent clip to Launcher'), { type: 'success' });
}

/* ── detail panel (Editor / Devices / Mixer) ────────────────── */

function renderDetail() {
  if (!detailEl) return;
  // Collapse only the body — the rail (page buttons + toggle) stays visible so
  // the panel can always be re-expanded.
  if (detailBodyEl) detailBodyEl.classList.toggle('hidden', !detailOpen);
  if (collapseBtn) collapseBtn.replaceChildren(icon(detailOpen ? 'ui/chevron-down' : 'ui/chevron-up', { size: 13 }));
  pageSeg?.setValue(detailPage, { silent: true });
  if (!detailOpen) return;
  if (editorPageEl) editorPageEl.classList.toggle('hidden', detailPage !== 'editor');
  if (devicesEl) devicesEl.classList.toggle('hidden', detailPage !== 'devices');
  if (mixerEl) mixerEl.classList.toggle('hidden', detailPage !== 'mixer');
  if (detailPage === 'editor') renderEditor();
  else if (detailPage === 'devices') renderDevices();
  else renderMixer();
  if (detailPage === 'editor') {
    const kind = current?.voices?.[selectedVoice]?.kind;
    const base = !current ? t('plugin.studio.hint.selectClip', null, 'Select a clip in the Arranger or Launcher to edit it')
      : MELODIC.has(kind) ? t('plugin.studio.hint.editorNotes', null, 'Click the grid to add notes · drag to move · drag the right edge to resize')
        : kind === 'drumkit' ? t('plugin.studio.hint.editorPads', null, 'Click a pad row to place hits · Fill = Euclidean rhythm')
          : t('plugin.studio.hint.editorSteps', null, 'Click cells to toggle hits · Fill = Euclidean rhythm');
    setHint(`${base}${keysHint()}`);
  } else {
    setHint((detailPage === 'devices'
      ? t('plugin.studio.hint.devices', null, 'Drag knobs up/down · double-click a knob resets it')
      : t('plugin.studio.hint.mixer', null, 'Click a strip to select its track · M/S mute & solo')) + keysHint());
  }
}
function keysHint() { return t('plugin.studio.hint.keys3', null, ' — Space play · Enter back to start · L loop · M/S mute/solo · Ctrl+E split · Ctrl+D duplicate · Ctrl+S save · Ctrl+Shift+E export'); }

/* ── editor page ────────────────────────────────────────────── */

let edToolbarEl = null;

function selectedArrClip() {
  return sel?.area === 'arr' ? arrangement.clips[sel.clipIndex] || null : null;
}
/* Inspector for an audio clip: gain, trim offset, fades, length, tempo. */
function renderAudioClipEditor(clip) {
  edToolbarEl.textContent = '';
  gridEl.textContent = '';
  gridEl.classList.remove('hidden');
  pianoEl?.classList.add('hidden');
  const a = clip.audio;
  const info = audioInfo(a.src);
  const name = uiInput();
  name.type = 'text';
  name.classList.add('studio-title', 'studio-title--clip');
  name.value = a.name || '';
  name.title = t('plugin.studio.editor.clipName', null, 'Clip name');
  name.addEventListener('change', () => { a.name = name.value.trim() || a.src.split('/').pop(); markDirty(); renderArranger(); });
  edToolbarEl.appendChild(name);
  edToolbarEl.appendChild(h('span', 'studio-audio-src', a.src));
  const wrap = h('div', 'studio-audio-inspector');
  const spb = 60 / arrangement.bpm;
  const dur = info?.duration || 0;
  const row = h('div', 'studio-knob-row');
  const change = () => { markDirty(); renderArranger(); };
  row.appendChild(knob(t('plugin.studio.clip.gain', null, 'Gain (dB)'), -24, 12, 0.1, clip.gain_db || 0, (n) => { clip.gain_db = n; change(); }, 0));
  row.appendChild(knob(t('plugin.studio.clip.offset', null, 'Start (s)'), 0, Math.max(0.01, dur), 0.01, a.offset || 0, (n) => { a.offset = n; change(); }, 0));
  row.appendChild(knob(t('plugin.studio.clip.fadeIn', null, 'Fade in (s)'), 0, 10, 0.01, a.fade_in || 0, (n) => { a.fade_in = n; change(); }, 0.005));
  row.appendChild(knob(t('plugin.studio.clip.fadeOut', null, 'Fade out (s)'), 0, 10, 0.01, a.fade_out || 0, (n) => { a.fade_out = n; change(); }, 0.05));
  wrap.appendChild(row);
  const facts = h('div', 'studio-audio-facts');
  const fact = (label, value) => { const f = h('span', 'studio-fact'); f.append(h('span', 'studio-fact-label', label), h('span', 'studio-fact-value', value)); facts.appendChild(f); };
  fact(t('plugin.studio.clip.duration', null, 'Duration'), dur ? fmtTime(dur) : '—');
  fact(t('plugin.studio.clip.length', null, 'Clip length'), `${(clipDurBeats(clip) / 4).toFixed(2)} ${t('plugin.studio.clip.bars', null, 'bars')}`);
  fact(t('plugin.studio.clip.tempo', null, 'Detected tempo'), Number.isFinite(info?.bpm) ? `${info.bpm} BPM` : '—');
  fact(t('plugin.studio.clip.loudness', null, 'Loudness'), Number.isFinite(info?.lufs) ? `${info.lufs.toFixed(1)} LUFS` : '—');
  wrap.appendChild(facts);
  const actions = h('div', 'studio-audio-actions');
  if (Number.isFinite(info?.bpm) && Math.abs(info.bpm - arrangement.bpm) > 0.05) {
    actions.appendChild(button({ variant: 'ghost', label: t('plugin.studio.clip.matchTempo', { bpm: info.bpm }, `Set project tempo to ${info.bpm} BPM`), onClick: () => {
      arrangement.bpm = info.bpm; if (launcher) launcher.bpm = info.bpm;
      if (bpmInput) bpmInput.value = String(info.bpm);
      markDirty(); renderArranger(); renderEditor();
    } }));
  }
  actions.appendChild(button({ variant: 'ghost', label: t('plugin.studio.clip.fullLength', null, 'Use the whole file'), onClick: () => { clip.length_beats = null; a.offset = 0; fitArrangementLength(); change(); renderEditor(); } }));
  wrap.appendChild(actions);
  gridEl.appendChild(wrap);
  if (!info) void loadAudioInfo(a.src).then(() => { if (selectedArrClip() === clip) renderEditor(); }).catch(() => {});
}
function renderEditor() {
  if (!gridEl) return;
  const audioClip = selectedArrClip();
  if (audioClip?.audio) { renderAudioClipEditor(audioClip); return; }
  renderEditorToolbar();
  if (!current) {
    gridEl.textContent = '';
    gridEl.appendChild(h('div', 'studio-empty', t('plugin.studio.empty.selectClip', null, 'Select a clip in the Arranger or Launcher to edit it.')));
    gridEl.classList.remove('hidden');
    pianoEl?.classList.add('hidden');
    return;
  }
  /* The editor auto-selects its view from the selected voice's kind:
     melodic → piano roll · drumkit → drum machine · drums → step grid. */
  const showPiano = MELODIC.has(current.voices[selectedVoice]?.kind);
  gridEl.classList.toggle('hidden', showPiano);
  pianoEl?.classList.toggle('hidden', !showPiano);
  if (showPiano) renderPianoRoll();
  else renderStepGrid();
}

function renderEditorToolbar() {
  if (!edToolbarEl) return;
  edToolbarEl.textContent = '';
  if (!current) return;

  const name = uiInput();
  name.type = 'text';
  name.classList.add(...('studio-title studio-title--clip').split(/\s+/));
  name.value = current.title || 'Clip';
  name.maxLength = 120;
  name.title = t('plugin.studio.editor.clipName', null, 'Clip name');
  name.addEventListener('change', () => { current.title = name.value.trim() || 'Clip'; markDirty(); renderArranger(); renderLauncher(); });
  edToolbarEl.appendChild(name);

  edToolbarEl.appendChild(nativeSelect(['8', '16', '32', '64'].map((s) => [s, tp('plugin.studio.editor.steps', Number(s), null, `${s} steps`)]), String(current.steps), (val) => {
    current.steps = parseInt(val, 10);
    for (const v of current.voices) v.notes = (v.notes || []).filter((nt) => nt.step < current.steps);
    markDirty();
    renderEditor();
    renderArranger();
    renderLauncher();
  }, t('plugin.studio.hint.patternLength', null, 'Pattern length')));

  const arrClip = selectedArrClip();
  if (arrClip && !arrClip.audio) {
    const loopBtn = button({ variant: 'ghost', label: t('plugin.studio.clip.loop', null, 'Loop'), onClick: () => {
      arrClip.loop = arrClip.loop === false; markDirty(); renderArranger(); renderEditorToolbar();
    } });
    loopBtn.classList.add('studio-toggle');
    loopBtn.classList.toggle('studio-btn--on', arrClip.loop !== false);
    loopBtn.title = t('plugin.studio.clip.loopHint', null, 'Repeat the pattern to fill the clip length');
    edToolbarEl.appendChild(loopBtn);
  }
  edToolbarEl.appendChild(nativeSelect(TUNINGS, current.tuning, (val) => { current.tuning = val; markDirty(); renderEditor(); }, t('plugin.studio.hint.tuning', null, 'Tuning')));

  /* Swing — groove that delays every other 16th-note step (1 = triplet) */
  const swingSel = nativeSelect(SWING_OPTS.map(([v, k]) => [v, t(k)]), String(current.swing ?? 0), (val) => {
    current.swing = parseFloat(val) || 0;
    markDirty();
    renderEditor();
  }, t('plugin.studio.hint.swing', null, 'Swing — delays every other 16th (groove)'));
  edToolbarEl.appendChild(swingSel);

  const v = current.voices[selectedVoice];
  if (v) {
    /* Euclidean fill — the studio's signature rhythm tool */
    const hitsSel = nativeSelect(Array.from({ length: current.steps + 1 }, (_, i) => [String(i), String(i)]), '4', () => {}, t('plugin.studio.hint.euclideanHits', null, 'Euclidean hits'));
    hitsSel.classList.add('studio-select--narrow');
    const rotField = numberField(t('plugin.studio.editor.rotationPlaceholder', null, 'rot'), 0, () => {}, t('plugin.studio.hint.rotation', null, 'Rotation'));
    const fill = h('button', 'studio-btn', t('plugin.studio.editor.fill', null, 'Fill'));
    fill.type = 'button';
    fill.title = t('plugin.studio.editor.fillHint', null, 'Fill the selected voice with a Euclidean rhythm');
    fill.addEventListener('click', () => {
      const hits = parseInt(hitsSel.value, 10) || 0;
      const rot = parseInt(rotField.value, 10) || 0;
      v.rhythm = `e${hits},${rot}`;
      if (MELODIC.has(v.kind)) {
        v.notes = rhythmToCells(v.rhythm, current.steps)
          .map((on, step) => (on ? { step, length: 1, degree: v.degree, octave: v.octave } : null))
          .filter(Boolean);
      }
      markDirty();
      renderEditor();
    });
    const clear = h('button', 'studio-btn', t('plugin.studio.editor.clear', null, 'Clear'));
    clear.type = 'button';
    clear.title = t('plugin.studio.editor.clearHint', null, 'Clear the selected voice');
    clear.addEventListener('click', () => {
      v.rhythm = '.'.repeat(current.steps);
      v.notes = [];
      markDirty();
      renderEditor();
    });
    edToolbarEl.append(hitsSel, rotField, fill, clear);
  }
  const prev = h('button', 'studio-btn studio-btn--play', '');
  prev.type = 'button';
  prev.title = t('plugin.studio.editor.preview', null, 'Preview clip');
  prev.appendChild(icon(prevPlaying ? 'ui/stop' : 'ui/play', { size: 12 }));
  prev.addEventListener('click', () => void previewClipInEditor());
  edToolbarEl.appendChild(prev);

  const savePat = h('button', 'studio-btn', t('plugin.studio.editor.savePattern', null, 'Save pattern'));
  savePat.type = 'button';
  savePat.title = t('plugin.studio.editor.savePatternHint', null, 'Save clip to the pattern library');
  savePat.addEventListener('click', () => void savePatternToLibrary().catch((e) => toast(e.message, { type: 'error' })));
  edToolbarEl.appendChild(savePat);
}

function renderStepGrid() {
  gridEl.textContent = '';
  const selVoice = current.voices[selectedVoice];
  if (selVoice?.kind === 'drumkit') { renderDrumkitGrid(selVoice); return; }
  const steps = current.steps;
  const n = current.voices.length;
  const CELL = 36;
  gridEl.style.gridTemplateColumns = `132px repeat(${steps}, ${CELL}px)`;
  gridEl.style.gridTemplateRows = `26px repeat(${n}, ${CELL}px) ${CELL}px`;

  gridEl.appendChild(h('div', 'studio-corner'));

  for (let s = 0; s < steps; s++) {
    const scene = h('div', 'studio-scene', String(s + 1));
    scene.dataset.step = s;
    scene.style.gridColumn = String(s + 2);
    scene.style.gridRow = '1';
    if (s % 4 === 0) scene.classList.add('studio-scene--bar');
    gridEl.appendChild(scene);
  }

  for (let v = 0; v < n; v++) {
    const head = h('button', 'studio-track-head');
    head.type = 'button';
    head.dataset.voice = v;
    head.style.gridColumn = '1';
    head.style.gridRow = String(v + 2);
    head.style.setProperty('--track', trackColor(v));
    head.classList.toggle('studio-track-head--active', v === selectedVoice);
    head.title = t('plugin.studio.editor.voiceHint', { name: KIND_LABELS[current.voices[v].kind] }, `${KIND_LABELS[current.voices[v].kind]} — click to edit`);
    head.addEventListener('click', () => {
      selectedVoice = v;
      renderEditor();
      renderDevices();
    });
    const dot = h('span', 'studio-track-dot');
    head.appendChild(dot);
    head.appendChild(h('span', 'studio-track-name', KIND_INITIALS[current.voices[v].kind] || '?'));
    if (n > 1) {
      const rm = h('span', 'studio-track-x');
      rm.appendChild(icon('ui/close', { size: 11 }));
      rm.title = t('plugin.studio.editor.removeVoice', null, 'Remove voice');
      rm.addEventListener('click', (e) => { e.stopPropagation(); removeVoice(v); });
      head.appendChild(rm);
    }
    gridEl.appendChild(head);
  }
  const add = h('button', 'studio-track-add');
  add.type = 'button';
  plusLabel(add, t('plugin.studio.editor.addVoice', null, '+ Voice'));
  add.style.gridColumn = '1';
  add.style.gridRow = String(n + 2);
  add.title = t('plugin.studio.editor.addVoiceHint', null, 'Add voice');
  add.addEventListener('click', () => {
    if (current.voices.length >= 12) { toast(t('plugin.studio.toast.maxVoices', null, 'Max 12 voices'), { type: 'error' }); return; }
    openBrowser('instruments', 'addVoice');
  });
  gridEl.appendChild(add);

  for (let v = 0; v < n; v++) {
    const vc = current.voices[v];
    for (let s = 0; s < steps; s++) {
      const cell = h('button', 'studio-cell');
      cell.type = 'button';
      cell.dataset.step = s;
      cell.dataset.voice = v;
      cell.style.gridColumn = String(s + 2);
      cell.style.gridRow = String(v + 2);
      cell.style.setProperty('--track', trackColor(v));
      if (s % 4 === 0) cell.classList.add('studio-cell--bar');
      const on = rhythmToCells(vc.rhythm, steps)[s];
      cell.classList.toggle('studio-cell--on', on);
      if (on && MELODIC.has(vc.kind)) {
        const note = noteAt(vc, s);
        cell.textContent = note ? String(note.degree) : '';
      }
      cell.addEventListener('click', () => toggleCell(s, v));
      gridEl.appendChild(cell);
    }
  }
}

function toggleCell(s, v) {
  const vc = current.voices[v];
  if (MELODIC.has(vc.kind)) {
    const existing = noteAt(vc, s);
    if (existing) vc.notes = vc.notes.filter((nt) => nt !== existing);
    else { vc.notes.push({ step: s, length: 1, degree: vc.degree, octave: vc.octave }); auditionNote(vc, vc.degree, vc.octave); }
    recomputeRhythmFromNotes(vc);
  } else {
    const cells = rhythmToCells(vc.rhythm, current.steps);
    cells[s] = !cells[s];
    if (cells[s]) auditionNote(vc);
    vc.rhythm = cellsToRhythm(cells);
  }
  markDirty();
  renderEditor();
  refreshSelectedClipPreview();
}

function removeVoice(v) {
  current.voices.splice(v, 1);
  selectedVoice = Math.min(selectedVoice, current.voices.length - 1);
  markDirty();
  renderEditor();
  renderDevices();
}
function addVoiceWithKind(kind) {
  const v = voice(kind, 'x...');
  if (kind === 'drumkit') v.pads = defaultPads();
  current.voices.push(v);
  selectedVoice = current.voices.length - 1;
  markDirty();
  renderEditor();
  renderDevices();
}

/* drumkit: 16 pads × steps */
function renderDrumkitGrid(v) {
  const pads = (v.pads?.length ? v.pads : defaultPads()).slice(0, 16);
  const steps = current.steps;
  const CELL = 36;
  gridEl.style.gridTemplateColumns = `132px repeat(${steps}, ${CELL}px)`;
  gridEl.style.gridTemplateRows = `26px repeat(${pads.length}, ${CELL}px)`;
  gridEl.appendChild(h('div', 'studio-corner'));
  for (let s = 0; s < steps; s++) {
    const scene = h('div', 'studio-scene', String(s + 1));
    scene.dataset.step = s;
    scene.style.gridColumn = String(s + 2);
    scene.style.gridRow = '1';
    if (s % 4 === 0) scene.classList.add('studio-scene--bar');
    gridEl.appendChild(scene);
  }
  for (let p = 0; p < pads.length; p++) {
    const pad = pads[p];
    const colorIdx = KINDS.indexOf(pad.kind);
    const head = h('button', 'studio-track-head');
    head.type = 'button';
    head.style.gridColumn = '1';
    head.style.gridRow = String(p + 2);
    head.style.setProperty('--track', trackColor(colorIdx));
    head.title = `${pad.name} (${KIND_LABELS[pad.kind]})`;
    head.addEventListener('pointerdown', () => auditionNote(v, 0, 0, p));
    head.appendChild(h('span', 'studio-track-dot'));
    head.appendChild(h('span', 'studio-track-name', pad.name));
    gridEl.appendChild(head);
    for (let s = 0; s < steps; s++) {
      const cell = h('button', 'studio-cell');
      cell.type = 'button';
      cell.dataset.step = s;
      cell.dataset.pad = p;
      cell.style.gridColumn = String(s + 2);
      cell.style.gridRow = String(p + 2);
      cell.style.setProperty('--track', trackColor(colorIdx));
      if (s % 4 === 0) cell.classList.add('studio-cell--bar');
      cell.classList.toggle('studio-cell--on', !!drumkitNoteAt(v, p, s));
      cell.addEventListener('click', () => toggleDrumkitCell(v, p, s));
      gridEl.appendChild(cell);
    }
  }
}
function toggleDrumkitCell(v, pad, step) {
  const existing = drumkitNoteAt(v, pad, step);
  if (existing) v.notes = v.notes.filter((nt) => nt !== existing);
  else { v.notes.push({ step, length: 1, degree: pad, octave: 0 }); auditionNote(v, 0, 0, pad); }
  markDirty();
  renderEditor();
}

function recomputeRhythmFromNotes(v) {
  const cells = new Array(current.steps).fill(false);
  for (const nt of v.notes) {
    for (let s = nt.step; s < Math.min(current.steps, nt.step + (nt.length || 1)); s++) cells[s] = true;
  }
  v.rhythm = cellsToRhythm(cells);
}

/* ── piano roll ─────────────────────────────────────────────── */

const PR_KEY_W = 64;
const PR_RULER_H = 24;
const PR_ROW = 20;
const PR_COL = 28;
const PR_DEG_LO = -36;  // A1 (around 55 Hz)
const PR_DEG_HI = 48;   // C8

function prRowOf(degree) {
  return PR_DEG_HI - degree;
}

/* ── click-to-audition (piano keys) ─────────────────────────── */

/* ── note audition with the voice's real sound ────────────────
   A one-note, one-beat copy of the voice (same kind, synth, inserts; no
   pattern bus) is rendered by /api/studio/preview and cached as a decoded
   AudioBuffer per (voice sound, pitch/pad). Cached notes play at once;
   uncached ones play as soon as they arrive (never blocking the UI). */
const auditionCache = new Map();   // key → Promise<AudioBuffer>
let auditionVoice = null;          // { src, gain }
let auditionSeq = 0;
let auditionFetching = false;
let auditionNext = null;          // [voice, degree, octave, pad] asked for while fetching
function auditionKey(v, degree, octave, pad) {
  const { notes, rhythm, level, pan, accent, macros, midi, ...sound } = v;
  return JSON.stringify([sound, macros, degree, octave, pad, current?.tuning, current?.bpm]);
}
function auditionConfig(v, degree, octave, pad) {
  const voiceCfg = serializePattern({ title: 'audition', bpm: current?.bpm || 120, steps: 4, swing: 0, tuning: current?.tuning || 'edo12', voices: [v], fx: {} }).voices[0];
  voiceCfg.midi = [];
  voiceCfg.accent = 0;
  voiceCfg.level = 0.8;
  voiceCfg.pan = 0;
  if (v.kind === 'drumkit') { voiceCfg.rhythm = ''; voiceCfg.notes = [{ step: 0, length: 1, degree: pad, octave: 0, velocity: 0.9 }]; }
  else if (MELODIC.has(v.kind) || v.kind === 'grid') { voiceCfg.rhythm = ''; voiceCfg.notes = [{ step: 0, length: 4, degree, octave, velocity: 0.85 }]; }
  else { voiceCfg.rhythm = 'x...'; voiceCfg.notes = []; }
  return { title: 'audition', bpm: current?.bpm || 120, steps: 4, swing: 0, tuning: current?.tuning || 'edo12', voices: [voiceCfg], fx: {} };
}
function auditionNote(v, degree = 0, octave = 0, pad = 0) {
  if (!v) return;
  const ctx = audioCtxOr();
  if (ctx.state === 'suspended') void ctx.resume();
  const key = auditionKey(v, degree, octave, pad);
  let p = auditionCache.get(key);
  if (!p && auditionFetching) {
    // One audition render at a time: keep only the latest wish.
    auditionNext = [v, degree, octave, pad];
    auditionSeq += 1;
    return;
  }
  if (!p) {
    auditionFetching = true;
    p = apiFetch('/api/studio/preview', { method: 'POST', body: JSON.stringify(auditionConfig(v, degree, octave, pad)), responseType: 'blob' })
      .then((blob) => blob.arrayBuffer())
      .then((buf) => ctx.decodeAudioData(buf));
    p.catch(() => auditionCache.delete(key));
    p.catch(() => {}).finally(() => {
      auditionFetching = false;
      const next = auditionNext;
      auditionNext = null;
      if (next) auditionNote(...next);
    });
    auditionCache.set(key, p);
    if (auditionCache.size > 64) auditionCache.delete(auditionCache.keys().next().value);
  }
  const seq = ++auditionSeq;
  void p.then((buffer) => {
    if (seq !== auditionSeq) return;      // a newer note was asked for meanwhile
    const now = ctx.currentTime;
    if (auditionVoice) {
      // 8 ms fade on the previous audition: no click when notes overlap.
      try { auditionVoice.gain.gain.setTargetAtTime(0, now, 0.003); auditionVoice.src.stop(now + 0.03); } catch (_) { /* noop */ }
    }
    const src = ctx.createBufferSource();
    src.buffer = buffer;
    const gain = ctx.createGain();
    src.connect(gain).connect(masterOut());
    src.start(now);
    auditionVoice = { src, gain };
  }).catch(() => {});
  if (MELODIC.has(v.kind)) setReadout(t('plugin.studio.readout.noteOnly', { note: degreeLabel(degree + 12 * octave, current?.tuning || 'edo12') }, `Note — ${degreeLabel(degree + 12 * octave, current?.tuning || 'edo12')}`));
}
/* Kept name for the piano-roll keys. */
function auditionKeyDegree(degree) {
  const v = current?.voices?.[selectedVoice];
  auditionNote(v, degree, v?.octave ?? 0);
}

function renderPianoRoll() {
  if (!pianoEl) return;
  pianoEl.textContent = '';
  const v = current.voices[selectedVoice];
  if (!v || !MELODIC.has(v.kind)) { pianoEl.classList.add('hidden'); return; }
  const steps = current.steps;
  const rows = PR_DEG_HI - PR_DEG_LO + 1;

  const grid = h('div', 'studio-pr-grid');
  grid.style.gridTemplateColumns = `${PR_KEY_W}px repeat(${steps}, ${PR_COL}px)`;
  grid.style.gridTemplateRows = `${PR_RULER_H}px repeat(${rows}, ${PR_ROW}px)`;
  grid.appendChild(h('div', 'studio-pr-corner'));

  for (let s = 0; s < steps; s++) {
    const c = h('div', 'studio-pr-step', String(s + 1));
    c.dataset.step = s;
    c.style.gridColumn = String(s + 2);
    c.style.gridRow = '1';
    grid.appendChild(c);
  }
  for (let d = PR_DEG_LO; d <= PR_DEG_HI; d++) {
    const k = h('div', 'studio-pr-key', degreeLabel(d, current.tuning));
    k.classList.toggle('studio-pr-key--black', isBlack(d, current.tuning));
    k.style.gridColumn = '1';
    k.style.gridRow = String(prRowOf(d) + 2);
    k.title = t('plugin.studio.piano.audition', null, 'Click to audition');
    k.addEventListener('pointerdown', () => {
      auditionKeyDegree(d);
      k.classList.add('studio-pr-key--pressed');
      setTimeout(() => k.classList.remove('studio-pr-key--pressed'), 160);
    });
    grid.appendChild(k);
  }

  const notes = h('div', 'studio-pr-notes');
  notes.style.left = `${PR_KEY_W}px`;
  notes.style.top = `${PR_RULER_H}px`;
  notes.style.width = `${steps * PR_COL}px`;
  notes.style.height = `${rows * PR_ROW}px`;
  for (let i = 0; i < v.notes.length; i++) {
    const nt = v.notes[i];
    const bar = h('div', 'studio-pr-note', degreeLabel(nt.degree, current.tuning));
    bar.dataset.noteIndex = i;
    bar.style.left = `${nt.step * PR_COL + 1}px`;
    bar.style.top = `${prRowOf(nt.degree) * PR_ROW + 1}px`;
    bar.style.width = `${Math.max(1, nt.length || 1) * PR_COL - 2}px`;
    bar.style.height = `${PR_ROW - 2}px`;
    bar.addEventListener('pointerdown', (e) => { auditionNote(v, nt.degree, nt.octave ?? v.octave); startNoteDrag(e, i, v); });
    bar.addEventListener('dblclick', (e) => { e.stopPropagation(); deleteNote(i); });
    notes.appendChild(bar);
  }
  grid.appendChild(notes);
  notes.addEventListener('pointerdown', (e) => {
    if (e.target !== notes) return;
    const rect = notes.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    const step = Math.floor(x / PR_COL);
    const degree = PR_DEG_HI - Math.floor(y / PR_ROW);
    if (step >= 0 && step < steps && degree >= PR_DEG_LO && degree <= PR_DEG_HI) addNote(step, degree);
  });

  pianoEl.appendChild(grid);
  requestAnimationFrame(() => {
    pianoEl.scrollTop = Math.max(0, prRowOf(0) * PR_ROW - pianoEl.clientHeight / 2 + PR_ROW);
  });
}

function addNote(step, degree) {
  const v = current.voices[selectedVoice];
  if (!v) return;
  v.notes.push({ step, length: 1, degree, octave: v.octave });
  auditionNote(v, degree, v.octave);
  recomputeRhythmFromNotes(v);
  markDirty();
  renderPianoRoll();
  refreshSelectedClipPreview();
}
function deleteNote(i) {
  const v = current.voices[selectedVoice];
  if (!v) return;
  v.notes.splice(i, 1);
  recomputeRhythmFromNotes(v);
  markDirty();
  renderPianoRoll();
  refreshSelectedClipPreview();
}
function startNoteDrag(e, i, v) {
  e.stopPropagation();
  e.preventDefault();
  const nt = v.notes[i];
  const bar = e.currentTarget;
  const startX = e.clientX;
  const startY = e.clientY;
  const startStep = nt.step;
  const startDeg = nt.degree;
  const startLen = nt.length || 1;
  const rect = bar.getBoundingClientRect();
  const nearRightEdge = (e.clientX - rect.right) > -6;
  const mode = nearRightEdge ? 'resize' : 'move';
  const move = (ev) => {
    const dx = ev.clientX - startX;
    const dy = ev.clientY - startY;
    if (mode === 'resize') {
      nt.length = Math.max(1, Math.min(current.steps - nt.step, startLen + Math.round(dx / PR_COL)));
    } else {
      nt.step = Math.max(0, Math.min(current.steps - (nt.length || 1), startStep + Math.round(dx / PR_COL)));
      const nd = Math.max(PR_DEG_LO, Math.min(PR_DEG_HI, startDeg - Math.round(dy / PR_ROW)));
      if (nd !== nt.degree) { nt.degree = nd; auditionNote(v, nd, nt.octave ?? v.octave); }
    }
    bar.style.left = `${nt.step * PR_COL + 1}px`;
    bar.style.top = `${prRowOf(nt.degree) * PR_ROW + 1}px`;
    bar.style.width = `${Math.max(1, nt.length || 1) * PR_COL - 2}px`;
    bar.textContent = degreeLabel(nt.degree, current.tuning);
  };
  const up = () => {
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    recomputeRhythmFromNotes(v);
    markDirty();
    refreshSelectedClipPreview();
  };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
}

/* ── devices page ───────────────────────────────────────────── */

function flattenVoiceParams(v) {
  const out = [];
  for (const p of (SYNTH[v.kind] || [])) {
    out.push({ path: p.key, label: `${KIND_LABELS[v.kind]} ${p.label}`, min: p.min, max: p.max, step: p.step, def: p.def,
      get: () => (v.synth[p.key] != null ? v.synth[p.key] : p.def), set: (n) => { v.synth[p.key] = n; } });
  }
  out.push({ path: 'level', label: t('plugin.studio.param.level', null, 'Level'), min: 0, max: 2, step: 0.01, def: (DEFAULT_LEVEL[v.kind] ?? 0.5),
    get: () => (v.level != null ? v.level : (DEFAULT_LEVEL[v.kind] ?? 0.5)), set: (n) => { v.level = n; } });
  out.push({ path: 'pan', label: t('plugin.studio.param.pan', null, 'Pan'), min: -1, max: 1, step: 0.01, def: 0,
    get: () => (v.pan != null ? v.pan : 0), set: (n) => { v.pan = n; } });
  out.push({ path: 'accent', label: t('plugin.studio.param.accent', null, 'Accent'), min: 0, max: 0.6, step: 0.01, def: 0,
    get: () => (v.accent ?? 0), set: (n) => { v.accent = n; } });
  v.fx.forEach((f, i) => {
    for (const p of (EFFECTS[f.kind]?.params || [])) {
      out.push({ path: `fx${i}.${p.key}`, label: `FX${i + 1} ${p.label}`, min: p.min, max: p.max, step: p.step, def: p.def,
        get: () => (f.params[p.key] != null ? f.params[p.key] : p.def), set: (n) => { f.params[p.key] = n; } });
    }
  });
  return out;
}

function clamp(n, lo, hi) {
  return Math.min(hi, Math.max(lo, n));
}
/* Apply one macro knob to every mapped parameter (bipolar depth around base). */
function applyMacro(mac, flat, k) {
  mac.value = k;
  for (const e of (mac.entries || [])) {
    const t = flat.find((f) => f.path === e.path);
    if (!t) continue;
    const span = t.max - t.min;
    const offset = (k - 0.5) * 2 * (e.amount ?? 1) * span;
    t.set(clamp(e.base + offset, t.min, t.max));
  }
  markDirty();
}
function macroValue(mac) {
  return mac && Number.isFinite(mac.value) ? mac.value : 0.5;
}

function applyPreset(v, preset) {
  const p = preset.params || preset || {};
  if (p.wave) v.wave = p.wave;
  if (p.synth && typeof p.synth === 'object') { for (const k in p.synth) v.synth[k] = p.synth[k]; }
  if (Array.isArray(p.fx)) v.fx = p.fx.map((f) => ({ kind: f.kind, params: { ...(f.params || {}) }, bypass: !!f.bypass }));
  if (typeof p.level === 'number') v.level = p.level;
  if (typeof p.pan === 'number') v.pan = p.pan;
  if (Number.isFinite(p.accent)) v.accent = clamp(p.accent, 0, 0.6);
  markDirty();
  renderDevices();
}

/* ── devices page: a Bitwig-style device chain ────────────────
   Each device is a card: a coloured header (type colour, name, on/off,
   remove), then its parameters — the main one as a large knob, the rest in
   a wrapping grid. The instrument is split into one card per parameter
   group (Oscillator, Filter, Envelope, …) so every section has room. Card
   colours come from the `--studio-dev-*` tokens (tiles.css). */

const GROUP_DEV = { Oscillator: 'osc', Filter: 'filter', Envelope: 'env', LFO: 'lfo', Voice: 'voice', Drum: 'drum', Sampler: 'osc', Drive: 'dist' };
const EFFECT_DEV = { distortion: 'dist', bitcrush: 'dist', filter: 'filter', eq: 'eq', compressor: 'dyn', delay: 'delay', reverb: 'reverb', chorus: 'mod', phaser: 'mod' };
/* The parameter that gets the big knob, by group / effect kind. */
const HERO_KEYS = {
  Oscillator: ['a_level', 'fm_index'], Filter: ['cutoff'], Envelope: ['decay', 'release', 'attack'], LFO: ['lfo_rate'],
  Voice: ['glide', 'poly'], Drum: ['pitch', 'tone', 'decay'], Sampler: ['program'],
  distortion: ['drive'], bitcrush: ['bits'], filter: ['cutoff'], eq: ['mid_gain'], compressor: ['threshold'],
  delay: ['mix', 'time'], reverb: ['mix', 'size'], chorus: ['mix', 'depth'], phaser: ['mix', 'rate'],
};
/* Fine-tuning ("atomic") parameters get the extra-small knob tier. */
const FINE_KEYS = new Set([
  // filter
  'keytrack', 'fattack', 'fdecay', 'fsustain', 'frelease', 'vel_filter',
  // envelope
  'env_shape', 'vel_amp',
  // oscillator
  'pw', 'spread', 'b_semi', 'b_octave', 'fm_ratio', 'fm_index', 'ring', 'noise',
  // lfo / voice
  'lfo_pan', 'lfo_amp', 'analog', 'pan_spread', 'poly',
  // drums
  'hold', 'click', 'buzz', 'snap', 'metal', 'bell',
]);
const FINE_FX = {
  distortion: ['tone', 'out'], filter: ['drive', 'poles'], compressor: ['knee', 'makeup', 'mix'],
  delay: ['damp', 'mod', 'offset', 'ping_pong'], reverb: ['predelay', 'width', 'mod', 'damping'],
  chorus: ['spread'], phaser: ['feedback', 'stages'], bitcrush: ['downsample'],
};
function devColor(type) { return `var(--studio-dev-${type}, var(--accent))`; }
function pickHero(params, keys) {
  const usable = params.filter((p) => !p.choices?.length);
  for (const k of keys || []) { const p = usable.find((x) => x.key === k); if (p) return p; }
  return usable[0] || null;
}

/* A device card shell. */
function devCard(type, title, { bypassed = false, onBypass = null, onRemove = null, extra = [] } = {}) {
  const card = h('div', `studio-dev studio-dev--${type}`);
  card.style.setProperty('--dev', devColor(type));
  card.classList.toggle('is-bypassed', bypassed);
  const head = h('div', 'studio-dev-head');
  head.appendChild(h('span', 'studio-dev-chip'));
  const name = h('span', 'studio-dev-name', title);
  name.title = title;
  head.appendChild(name);
  for (const x of extra) head.appendChild(x);
  if (onBypass) {
    const on = h('button', 'studio-dev-power', '');
    on.type = 'button';
    on.title = t('plugin.studio.device.bypass', null, 'Bypass');
    on.setAttribute('aria-pressed', String(!bypassed));
    on.classList.toggle('is-on', !bypassed);
    on.addEventListener('click', onBypass);
    head.appendChild(on);
  }
  if (onRemove) {
    const rm = iconCtl('ui/close', 'studio-head-btn', t('plugin.studio.common.remove', null, 'Remove'));
    rm.addEventListener('click', onRemove);
    head.appendChild(rm);
  }
  card.appendChild(head);
  const body = h('div', 'studio-dev-body');
  card.appendChild(body);
  return { card, body };
}
/* Fill a card body with three tiers: the main parameter as a large knob,
   ordinary ones as small knobs, fine-tuning ones as extra-small knobs.
   Columns flow horizontally so cards grow sideways, Bitwig-style. */
function devParams(body, params, heroKeys, get, set, pathOf, fineKeys = null) {
  const hero = pickHero(params, heroKeys);
  const isFine = (p) => (fineKeys ? fineKeys.includes(p.key) : FINE_KEYS.has(p.key));
  const control = (p, tier) => {
    const value = get(p);
    if (p.choices?.length) {
      const cell = h('label', 'studio-dev-choice');
      const lab = h('span', 'studio-dev-choice-label', p.label);
      lab.title = p.label;
      cell.append(lab, nativeSelect(p.choices.map((l, i) => [String(i), l]), String(Math.round(value)), (val) => set(p, parseFloat(val)), p.label));
      return cell;
    }
    const k = pathOf
      ? automatableKnob(p.label, pathOf(p), p.min, p.max, p.step, value, (n) => set(p, n), p.def)
      : knob(p.label, p.min, p.max, p.step, value, (n) => set(p, n), p.def);
    if (tier === 'hero') k.classList.add('studio-knob--hero');
    if (tier === 'fine') k.classList.add('studio-knob--fine');
    return k;
  };
  if (hero) {
    const hb = h('div', 'studio-dev-hero');
    hb.appendChild(control(hero, 'hero'));
    body.appendChild(hb);
  }
  const rest = params.filter((p) => p !== hero);
  const choices = rest.filter((p) => p.choices?.length);
  const small = rest.filter((p) => !p.choices?.length && !isFine(p));
  const fine = rest.filter((p) => !p.choices?.length && isFine(p));
  if (choices.length) {
    const col = h('div', 'studio-dev-choices');
    for (const p of choices) col.appendChild(control(p, 'choice'));
    body.appendChild(col);
  }
  if (small.length) {
    const grid = h('div', 'studio-dev-grid');
    for (const p of small) grid.appendChild(control(p, 'small'));
    body.appendChild(grid);
  }
  if (fine.length) {
    const grid = h('div', 'studio-dev-grid studio-dev-grid--fine');
    for (const p of fine) grid.appendChild(control(p, 'fine'));
    body.appendChild(grid);
  }
}

function renderDevices() {
  if (!devicesEl || detailPage !== 'devices') return;
  devicesEl.textContent = '';
  autoKnobs = [];
  if (!current) {
    devicesEl.appendChild(h('div', 'studio-empty', t('plugin.studio.empty.selectClipDevices', null, 'Select a clip to see its devices.')));
    return;
  }
  const v = current.voices[selectedVoice];
  if (!v) return;

  /* voice picker */
  const strip = h('div', 'studio-device-voices');
  current.voices.forEach((vc, i) => {
    const b = h('button', 'studio-device-voice', KIND_LABELS[vc.kind] || vc.kind);
    b.type = 'button';
    b.style.setProperty('--track', trackColor(i));
    b.title = KIND_LABELS[vc.kind];
    b.classList.toggle('studio-device-voice--on', i === selectedVoice);
    b.addEventListener('click', () => { selectedVoice = i; renderDevices(); renderEditor(); });
    strip.appendChild(b);
  });
  devicesEl.appendChild(strip);

  const chain = h('div', 'studio-devchain');
  const vi = selectedVoice;
  const synthGet = (p) => (v.synth[p.key] != null ? v.synth[p.key] : p.def);
  const synthSet = (p, n) => { v.synth[p.key] = n; markDirty(); };

  /* instrument header card: kind, pitch, presets */
  const presetBtn = iconCtl('ui/chevron-down', 'studio-head-btn', t('plugin.studio.device.presets', null, 'Presets'));
  presetBtn.addEventListener('click', () => openBrowser('presets'));
  const sp = iconCtl('ui/star', 'studio-head-btn', t('plugin.studio.device.savePreset', null, 'Save preset'));
  sp.addEventListener('click', () => void savePreset(v).catch((e) => toast(e.message, { type: 'error' })));
  const instType = voices_isDrum(v.kind) ? 'drum' : 'osc';
  const { card: inst, body: instBody } = devCard(instType, KIND_LABELS[v.kind] || v.kind, { extra: [presetBtn, sp] });
  inst.classList.add('studio-dev--instrument');
  const kindCell = h('label', 'studio-dev-choice');
  kindCell.append(h('span', 'studio-dev-choice-label', t('plugin.studio.device.instrument', null, 'Instrument')), nativeSelect(KINDS.map((k) => [k, KIND_LABELS[k]]), v.kind, (val) => {
    v.kind = val;
    v.synth = { ...(CATALOG?.kinds?.find((k) => k.kind === val)?.defaults || {}) };
    markDirty();
    renderEditor();
    renderDevices();
  }, t('plugin.studio.device.instrument', null, 'Instrument')));
  const ig = h('div', 'studio-dev-grid');
  ig.appendChild(kindCell);
  if (v.kind === 'bass') {
    const c = h('label', 'studio-dev-choice');
    c.append(h('span', 'studio-dev-choice-label', t('plugin.studio.param.wave', null, 'Wave')), nativeSelect(WAVES.map(([w, k]) => [w, t(k)]), v.wave, (val) => { v.wave = val; markDirty(); }, t('plugin.studio.param.wave', null, 'Wave')));
    ig.appendChild(c);
  }
  if (MELODIC.has(v.kind)) {
    ig.appendChild(knob(t('plugin.studio.device.octave', null, 'Octave'), -4, 4, 1, v.octave ?? 0, (n) => { v.octave = n; markDirty(); renderEditor(); }, 0));
    ig.appendChild(knob(t('plugin.studio.device.degree', null, 'Degree'), -24, 24, 1, v.degree ?? 0, (n) => { v.degree = n; markDirty(); renderEditor(); }, 0));
  }
  instBody.appendChild(ig);
  if (v.kind === 'drumkit') renderPadGrid(v, instBody);
  chain.appendChild(inst);

  if (v.kind !== 'drumkit') {
    const groups = {};
    for (const p of (SYNTH[v.kind] || [])) {
      const g = p.group || PARAM_GROUP[p.key] || 'Other';
      (groups[g] = groups[g] || []).push(p);
    }
    const order = [...GROUP_ORDER.filter((g) => groups[g]), ...Object.keys(groups).filter((g) => !GROUP_ORDER.includes(g))];
    for (const g of order) {
      const params = groups[g];
      if (!params?.length) continue;
      const type = GROUP_DEV[g] || (voices_isDrum(v.kind) ? 'drum' : 'util');
      const { card, body } = devCard(type, g);
      devParams(body, params, HERO_KEYS[g], synthGet, synthSet, (p) => `voice.${vi}.${p.key}`);
      chain.appendChild(card);
    }
  }

  /* insert effects */
  v.fx.forEach((f, i) => {
    const type = EFFECT_DEV[f.kind] || 'util';
    const { card, body } = devCard(type, EFFECTS[f.kind]?.label || f.kind, {
      bypassed: !!f.bypass,
      onBypass: () => { f.bypass = !f.bypass; markDirty(); renderDevices(); },
      onRemove: () => { v.fx.splice(i, 1); markDirty(); renderDevices(); },
    });
    devParams(body, EFFECTS[f.kind]?.params || [], HERO_KEYS[f.kind],
      (p) => (f.params[p.key] != null ? f.params[p.key] : p.def),
      (p, n) => { f.params[p.key] = n; markDirty(); },
      (p) => `voice.${vi}.fx.${i}.${p.key}`, FINE_FX[f.kind] || []);
    chain.appendChild(card);
  });

  /* add effect */
  const add = h('div', 'studio-dev studio-dev--add');
  const addSel = nativeSelect([['', t('plugin.studio.device.addEffect', null, '+ Add effect')], ...EFFECT_KINDS.map((k) => [k, EFFECTS[k].label])], '', (kind) => {
    if (!kind) return;
    if (v.fx.length >= 8) { toast(t('plugin.studio.toast.maxFx8', null, 'Max 8 effects per voice'), { type: 'error' }); return; }
    v.fx.push({ kind, params: { ...(EFFECTS[kind]?.defaults || {}) }, bypass: false });
    markDirty();
    renderDevices();
  }, t('plugin.studio.device.addEffect', null, '+ Add effect'));
  add.appendChild(addSel);
  chain.appendChild(add);

  /* output */
  const { card: out, body: outBody } = devCard('util', t('plugin.studio.device.output', null, 'Output'));
  const outParams = [
    { key: 'level', label: t('plugin.studio.param.level', null, 'Level'), min: 0, max: 2, step: 0.01, def: DEFAULT_LEVEL[v.kind] ?? 0.5 },
    { key: 'pan', label: t('plugin.studio.param.pan', null, 'Pan'), min: -1, max: 1, step: 0.01, def: DEFAULT_PAN[v.kind] ?? 0 },
    { key: 'accent', label: t('plugin.studio.param.accent', null, 'Accent'), min: 0, max: 0.6, step: 0.01, def: 0 },
  ];
  devParams(outBody, outParams, ['level'],
    (p) => (v[p.key] != null ? v[p.key] : p.def),
    (p, n) => { v[p.key] = n; markDirty(); },
    (p) => `voice.${vi}.${p.key}`, ['accent']);
  chain.appendChild(out);

  /* pattern bus (reverb/delay mix → the track's sends in an arrangement) */
  const { card: bus, body: busBody } = devCard('reverb', t('plugin.studio.device.patternBus', null, 'Pattern bus'));
  bus.title = t('plugin.studio.device.patternBusHint', null, 'Pattern-level reverb/delay and bus settings. In the Arranger, reverb/delay mix become the track sends unless the mixer sets them.');
  devParams(busBody, FX, ['reverb_mix'],
    (p) => (current.fx[p.key] != null ? current.fx[p.key] : p.def),
    (p, n) => { current.fx[p.key] = n; markDirty(); },
    (p) => `master.${p.key}`);
  chain.appendChild(bus);

  chain.addEventListener('wheel', (e) => {
    // Plain wheel scrolls the chain sideways (knobs keep their own wheel).
    if (e.ctrlKey || e.metaKey || e.target.closest?.('.ui-dial, select')) return;
    if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) { chain.scrollLeft += e.deltaY; e.preventDefault(); }
  }, { passive: false });
  devicesEl.appendChild(chain);

  /* macro rack */
  const { card: macros, body: macBody } = devCard('mod', t('plugin.studio.device.macros', null, 'Macros'));
  macros.classList.add('studio-dev--macros');
  const flat = flattenVoiceParams(v);
  const mgrid = h('div', 'studio-macro-grid');
  for (let i = 0; i < 8; i++) {
    const mac = v.macros[i] || (v.macros[i] = { value: 0.5, entries: [] });
    const wrap = h('div', 'studio-macro');
    const k = knob(`M${i + 1}`, 0, 1, 0.01, macroValue(mac), (nv) => applyMacro(mac, flat, nv), 0.5);
    wrap.appendChild(k);
    const list = h('div', 'studio-macro-entries');
    mac.entries.forEach((e, ei) => {
      const row = h('div', 'studio-macro-entry');
      row.appendChild(nativeSelect([['', '—'], ...flat.map((f) => [f.path, f.label])], e.path, (path) => {
        const tt = flat.find((f) => f.path === path);
        e.path = path;
        e.amount = 1;
        e.base = tt ? tt.get() : 0;
        markDirty();
        renderDevices();
      }, t('plugin.studio.hint.destination', null, 'Destination')));
      row.appendChild(knob(t('plugin.studio.device.amount', null, 'Amt'), -1, 1, 0.01, e.amount != null ? e.amount : 1, (n) => { e.amount = n; markDirty(); }, 1));
      const rm = iconCtl('ui/close', 'studio-head-btn', t('plugin.studio.device.removeMapping', null, 'Remove mapping'));
      rm.addEventListener('click', () => { mac.entries.splice(ei, 1); markDirty(); renderDevices(); });
      row.appendChild(rm);
      list.appendChild(row);
    });
    const addEntry = iconCtl('ui/plus', 'studio-head-btn', t('plugin.studio.device.addMapping', null, 'Add a parameter to this macro'));
    addEntry.addEventListener('click', () => {
      const first = flat[0];
      mac.entries.push({ path: first ? first.path : '', amount: 1, base: first ? first.get() : 0 });
      markDirty();
      renderDevices();
    });
    list.appendChild(addEntry);
    wrap.appendChild(list);
    mgrid.appendChild(wrap);
  }
  macBody.appendChild(mgrid);
  chain.appendChild(macros);
}
function voices_isDrum(kind) {
  return ['kick', 'snare', 'hat', 'clap', 'tom', 'perc', 'rim', 'cowbell', 'shaker', 'crash', 'ride', 'drumkit', 'sfkit'].includes(kind);
}

function renderPadGrid(v, container) {
  if (!v.pads?.length) v.pads = defaultPads();
  const pads = v.pads.slice(0, 16);
  const grid = h('div', 'studio-pads');
  for (let i = 0; i < 16; i++) {
    const pad = pads[i] || { name: `Pad ${i + 1}`, kind: 'kick' };
    const b = h('button', 'studio-pad');
    b.type = 'button';
    b.classList.toggle('studio-pad--active', i === selectedPad);
    b.style.setProperty('--track', trackColor(KINDS.indexOf(pad.kind)));
    b.appendChild(h('span', null, pad.name));
    b.addEventListener('click', () => { selectedPad = i; auditionNote(v, 0, 0, i); renderDevices(); });
    grid.appendChild(b);
  }
  container.appendChild(grid);
  const pad = pads[selectedPad] || { name: 'Pad', kind: 'kick' };
  const ed = h('div', 'studio-pad-editor');
  ed.appendChild(h('span', 'studio-rack-label', t('plugin.studio.device.pad', { n: selectedPad + 1 }, `Pad ${selectedPad + 1}`)));
  const nameInput = uiInput();
  nameInput.type = 'text';
  nameInput.classList.add(...('studio-number').split(/\s+/));
  nameInput.value = pad.name;
  nameInput.addEventListener('change', () => { pad.name = nameInput.value || 'Pad'; markDirty(); renderDevices(); });
  ed.appendChild(nameInput);
  ed.appendChild(nativeSelect(KINDS.filter((k) => k !== 'drumkit').map((k) => [k, KIND_LABELS[k]]), pad.kind, (val) => { pad.kind = val; markDirty(); renderDevices(); }));
  container.appendChild(ed);
}

/* ── mixer page ─────────────────────────────────────────────── */

/* A mixer group's title doubles as its identity (the Launcher tap below), so only the display is translated. */
/* ── mixer page ───────────────────────────────────────────────
   Console-style strips: inserts, sends, pan, a dB fader with a peak meter,
   mute/solo. Then the Reverb and Delay returns and the Master (fader,
   loudness target, ceiling, glue, width, measured LUFS). Fader, pan, mute,
   solo and return levels are live; everything else re-renders the stems. */

/* Audio-taper fader: position 0..1 ↔ gain 0..2 (+6 dB); 0 dB sits at ~79 %. */
function faderPosToGain(p) { return Math.pow(clamp(p, 0, 1), 3) * 2; }
function faderGainToPos(g) { return Math.cbrt(clamp(g, 0, 2) / 2); }

function dbFader(label, value, onchange, { meter = null } = {}) {
  const wrap = h('div', 'studio-dbfader');
  wrap.dataset.param = label;
  const track = h('div', 'studio-dbfader-track');
  const thumb = h('div', 'studio-dbfader-thumb');
  thumb.tabIndex = 0;
  thumb.setAttribute('role', 'slider');
  thumb.setAttribute('aria-label', label);
  const scale = h('div', 'studio-dbfader-scale');
  for (const db of [6, 0, -6, -12, -24, -48]) {
    const tick = h('span', 'studio-dbfader-tick', db > 0 ? `+${db}` : String(db));
    tick.style.bottom = `${faderGainToPos(dbToGain(db)) * 100}%`;
    scale.appendChild(tick);
  }
  track.appendChild(thumb);
  const body = h('div', 'studio-dbfader-body');
  body.append(scale, track);
  if (meter) body.appendChild(meter);
  const val = h('span', 'studio-dbfader-val');
  let cur = value;
  const paint = () => {
    thumb.style.bottom = `${faderGainToPos(cur) * 100}%`;
    val.textContent = `${fmtDb(cur)} dB`;
    thumb.setAttribute('aria-valuetext', `${fmtDb(cur)} dB`);
  };
  const set = (g, fire = true) => {
    cur = clamp(g, 0, 2);
    paint();
    if (fire) { onchange(cur); setReadout(t('plugin.studio.readout.param', { label, value: `${fmtDb(cur)} dB` }, `${label} — ${fmtDb(cur)} dB`)); }
  };
  const fromEvent = (e) => {
    const r = track.getBoundingClientRect();
    return faderPosToGain(1 - (e.clientY - r.top) / Math.max(1, r.height));
  };
  track.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const fine = e.shiftKey;
    const startY = e.clientY;
    const startPos = faderGainToPos(cur);
    if (e.target !== thumb) set(fromEvent(e));
    const move = (ev) => {
      if (fine || ev.shiftKey) set(faderPosToGain(startPos - (ev.clientY - startY) / 600));
      else set(fromEvent(ev));
    };
    const up = () => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up); };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  });
  track.addEventListener('dblclick', () => set(1));
  val.addEventListener('dblclick', () => set(1));
  thumb.addEventListener('keydown', (e) => {
    const step = e.shiftKey ? 0.1 : 1;
    const db = gainToDb(cur);
    if (e.key === 'ArrowUp') { e.preventDefault(); set(dbToGain(Math.min(6, (Number.isFinite(db) ? db : -60) + step))); }
    if (e.key === 'ArrowDown') { e.preventDefault(); set(db <= -59 ? 0 : dbToGain(db - step)); }
  });
  wrap.title = t('plugin.studio.mixer.faderHint', null, 'Drag (Shift = fine) · double-click = 0 dB');
  wrap.append(body, val);
  paint();
  return wrap;
}

function msButtons(tr, after) {
  const ms = h('span', 'studio-mixer-ms');
  const m = h('button', 'studio-ms', 'M');
  m.type = 'button';
  m.title = t('plugin.studio.track.mute', null, 'Mute');
  m.classList.toggle('studio-ms--on', !!tr.mute);
  m.addEventListener('click', () => { tr.mute = !tr.mute; markDirty(); after(); });
  const s = h('button', 'studio-ms studio-ms--solo', 'S');
  s.type = 'button';
  s.title = t('plugin.studio.track.solo', null, 'Solo');
  s.classList.toggle('studio-ms--on', soloTracks.has(tr.id));
  s.addEventListener('click', () => { toggleSolo(tr.id); after(); });
  ms.append(m, s);
  return ms;
}

let openInsert = null;   // { owner: 'track:<id>' | 'master', index }
/* Insert slots for a chain (track inserts or master inserts). */
function insertSlots(owner, list, onChange) {
  const box = h('div', 'studio-inserts');
  list.forEach((f, i) => {
    const slot = h('div', 'studio-insert');
    slot.classList.toggle('is-bypassed', !!f.bypass);
    const isOpen = openInsert && openInsert.owner === owner && openInsert.index === i;
    slot.classList.toggle('is-open', !!isOpen);
    const name = h('button', 'studio-insert-name', EFFECTS[f.kind]?.label || f.kind);
    name.type = 'button';
    name.title = t('plugin.studio.mixer.insertEdit', null, 'Edit parameters');
    name.addEventListener('click', () => { openInsert = isOpen ? null : { owner, index: i }; renderMixer(); });
    const by = h('button', 'studio-insert-btn', f.bypass ? '○' : '●');
    by.type = 'button';
    by.title = t('plugin.studio.mixer.bypass', null, 'Bypass');
    by.addEventListener('click', () => { f.bypass = !f.bypass; onChange(); renderMixer(); });
    const rm = iconCtl('ui/close', 'studio-insert-btn', t('plugin.studio.mixer.removeInsert', null, 'Remove effect'));
    rm.addEventListener('click', () => { list.splice(i, 1); openInsert = null; onChange(); renderMixer(); });
    slot.append(by, name, rm);
    box.appendChild(slot);
  });
  if (list.length < 8) {
    const add = nativeSelect([['', t('plugin.studio.mixer.addInsert', null, '+ Insert')], ...EFFECT_KINDS.map((k) => [k, EFFECTS[k]?.label || k])], '', (kind) => {
      if (!kind) return;
      list.push({ kind, params: { ...(EFFECTS[kind]?.defaults || {}) }, bypass: false });
      openInsert = { owner, index: list.length - 1 };
      onChange();
      renderMixer();
    }, t('plugin.studio.mixer.addInsertHint', null, 'Add an insert effect'));
    add.classList.add('studio-insert-add');
    box.appendChild(add);
  }
  return box;
}
/* Parameter knobs of the open insert, shown under the console. */
function insertEditor(list, index, title, onChange) {
  const f = list[index];
  if (!f) return null;
  const panel = h('div', 'studio-insert-editor');
  const head = h('div', 'studio-insert-editor-head');
  head.append(eyebrow(`${title} · ${EFFECTS[f.kind]?.label || f.kind}`));
  const close = iconCtl('ui/close', 'studio-head-btn', t('plugin.studio.common.close', null, 'Close'));
  close.addEventListener('click', () => { openInsert = null; renderMixer(); });
  head.appendChild(close);
  panel.appendChild(head);
  const row = h('div', 'studio-knob-row');
  for (const p of (EFFECTS[f.kind]?.params || [])) {
    const v = f.params[p.key] ?? p.def;
    if (p.choices?.length) {
      row.appendChild(nativeSelect(p.choices.map((c, ci) => [String(ci), c]), String(Math.round(v)), (val) => { f.params[p.key] = Number(val); onChange(); }, p.label));
    } else {
      row.appendChild(knob(p.label, p.min, p.max, p.step, v, (n) => { f.params[p.key] = n; onChange(); }, p.def));
    }
  }
  panel.appendChild(row);
  return panel;
}

function sendKnob(label, value, onchange) {
  return knob(label, 0, 1, 0.01, value, onchange, 0);
}

const DELAY_SYNC = [['0.25', '1/16'], ['0.5', '1/8'], ['0.75', '1/8 ·'], ['1', '1/4'], ['1.5', '1/4 ·'], ['2', '1/2'], ['0.333', '1/8 T']];
const LOUDNESS_TARGETS = [['0', 'plugin.studio.mixer.loudnessOff'], ['-14', '−14 LUFS'], ['-11', '−11 LUFS'], ['-9', '−9 LUFS']];

let masterReadoutEl = null;
function updateMasterReadout() {
  if (!masterReadoutEl?.isConnected) return;
  const m = mixManifest?.master;
  masterReadoutEl.textContent = '';
  const fact = (label, value) => { const f = h('span', 'studio-fact'); f.append(h('span', 'studio-fact-label', label), h('span', 'studio-fact-value', value)); masterReadoutEl.appendChild(f); };
  fact(t('plugin.studio.mixer.integrated', null, 'Integrated'), m && Number.isFinite(m.lufs) && m.lufs > -69 ? `${m.lufs.toFixed(1)} LUFS` : '—');
  fact(t('plugin.studio.mixer.peak', null, 'Peak'), m && m.peak > 0 ? `${fmtDb(m.peak)} dBFS` : '—');
  if (m && Math.abs(m.trim_db || 0) > 0.05) fact(t('plugin.studio.mixer.trim', null, 'Loudness trim'), `${m.trim_db > 0 ? '+' : ''}${m.trim_db.toFixed(1)} dB`);
}
function renderMixerReadouts() {
  updateMasterReadout();
  if (!mixerEl || !mixManifest) return;
  for (const r of (mixManifest.tracks || [])) {
    const el = mixerEl.querySelector(`[data-lufs-track="${CSS.escape(r.track_id)}"]`);
    if (el) el.textContent = r.muted || !Number.isFinite(r.lufs) || r.lufs <= -69 ? '' : `${r.lufs.toFixed(1)} LU`;
  }
}

function renderMixer() {
  if (!mixerEl || detailPage !== 'mixer') return;
  mixerEl.textContent = '';
  const content = () => { markDirty(); };
  const live = () => { markDirty(); renderArranger(); };
  const console_ = h('div', 'studio-console');

  /* Arranger tracks */
  const row = h('div', 'studio-mixer-row');
  if (!arrangement.tracks.length) row.appendChild(h('div', 'studio-empty', t('plugin.studio.mixer.empty', null, 'No tracks yet — add one in the Arranger.')));
  for (const tr of arrangement.tracks) {
    if (!Array.isArray(tr.fx)) tr.fx = [];
    const strip = h('div', 'studio-mixer-strip');
    strip.style.setProperty('--track', trackColor(tr.color));
    strip.classList.toggle('is-muted', !arrTrackAudible(tr));
    const name = h('span', 'studio-mixer-name', tr.name);
    name.title = t('plugin.studio.track.renameHint', null, 'Double-click to rename');
    name.addEventListener('dblclick', () => inlineRename(name, tr.name, (v) => { tr.name = v; markDirty(); renderMixer(); renderArranger(); }));
    strip.appendChild(h('span', 'studio-track-stripe'));
    strip.appendChild(name);
    strip.appendChild(h('span', 'studio-mixer-kind', tr.kind === 'audio' ? t('plugin.studio.mixer.audioTrack', null, 'Audio') : t('plugin.studio.mixer.instrumentTrack', null, 'Instrument')));
    strip.appendChild(insertSlots(`track:${tr.id}`, tr.fx, content));
    const sends = h('div', 'studio-mixer-sends');
    const cur = tr.sends || effectiveSends(tr);
    sends.appendChild(sendKnob(t('plugin.studio.mixer.sendReverb', null, 'Reverb'), cur.reverb, (n) => { tr.sends = { ...(tr.sends || effectiveSends(tr)), reverb: n }; content(); }));
    sends.appendChild(sendKnob(t('plugin.studio.mixer.sendDelay', null, 'Delay'), cur.delay, (n) => { tr.sends = { ...(tr.sends || effectiveSends(tr)), delay: n }; content(); }));
    strip.appendChild(sends);
    strip.appendChild(knob(t('plugin.studio.param.pan', null, 'Pan'), -1, 1, 0.01, tr.pan ?? 0, (n) => { tr.pan = n; markDirty(); }, 0));
    strip.appendChild(dbFader(t('plugin.studio.mixer.trackLevel', { name: tr.name }, `${tr.name} Level`), tr.level ?? 0.8, (g) => { tr.level = g; markDirty(); }, { meter: meterNode(() => channels.get(tr.id)?.meter) }));
    strip.appendChild(msButtons(tr, () => { renderMixer(); renderArranger(); }));
    const lu = h('span', 'studio-mixer-lufs');
    lu.dataset.lufsTrack = tr.id;
    lu.title = t('plugin.studio.mixer.trackLoudness', null, 'Track loudness (post-fader, last render)');
    strip.appendChild(lu);
    row.appendChild(strip);
  }

  /* Returns */
  const ret = arrangement.returns || (arrangement.returns = { reverb: {}, delay: {} });
  const rv = ret.reverb || (ret.reverb = {});
  const dl = ret.delay || (ret.delay = {});
  const rstrip = h('div', 'studio-mixer-strip studio-mixer-strip--return');
  rstrip.appendChild(h('span', 'studio-mixer-name', t('plugin.studio.mixer.reverbReturn', null, 'Reverb')));
  const rk = h('div', 'studio-mixer-sends studio-mixer-sends--grid');
  rk.appendChild(knob(t('plugin.studio.mixer.size', null, 'Size'), 0.05, 1.6, 0.01, rv.size ?? 0.55, (n) => { rv.size = n; content(); }, 0.55));
  rk.appendChild(knob(t('plugin.studio.mixer.damping', null, 'Damp'), 0, 1, 0.01, rv.damping ?? 0.45, (n) => { rv.damping = n; content(); }, 0.45));
  rk.appendChild(knob(t('plugin.studio.mixer.predelay', null, 'Pre-delay (ms)'), 0, 250, 1, rv.predelay ?? 18, (n) => { rv.predelay = n; content(); }, 18));
  rk.appendChild(knob(t('plugin.studio.mixer.width', null, 'Width'), 0, 2, 0.01, rv.width ?? 1, (n) => { rv.width = n; content(); }, 1));
  rstrip.appendChild(rk);
  rstrip.appendChild(dbFader(t('plugin.studio.mixer.reverbReturn', null, 'Reverb'), returnLevel('reverb'), (g) => { ret.reverb_level = g; markDirty(); }, { meter: meterNode(() => channels.get(RETURN_IDS.reverb)?.meter) }));
  row.appendChild(rstrip);

  const dstrip = h('div', 'studio-mixer-strip studio-mixer-strip--return');
  dstrip.appendChild(h('span', 'studio-mixer-name', t('plugin.studio.mixer.delayReturn', null, 'Delay')));
  const dk = h('div', 'studio-mixer-sends studio-mixer-sends--grid');
  const syncVal = dl.sync != null ? String(dl.sync) : (dl.time != null ? '' : '0.75');
  dk.appendChild(nativeSelect([...(syncVal === '' ? [['', `${Math.round(dl.time)} ms`]] : []), ...DELAY_SYNC], syncVal, (v) => { if (v) { dl.sync = Number(v); delete dl.time; } content(); }, t('plugin.studio.mixer.delayTime', null, 'Delay time (tempo-synced)')));
  dk.appendChild(knob(t('plugin.studio.mixer.feedback', null, 'Feedback'), 0, 0.95, 0.01, dl.feedback ?? 0.35, (n) => { dl.feedback = n; content(); }, 0.35));
  dk.appendChild(knob(t('plugin.studio.mixer.damping', null, 'Damp'), 0, 1, 0.01, dl.damp ?? 0.4, (n) => { dl.damp = n; content(); }, 0.4));
  dk.appendChild(knob(t('plugin.studio.mixer.pingPong', null, 'Ping-pong'), 0, 1, 0.01, dl.ping_pong ?? 0.5, (n) => { dl.ping_pong = n; content(); }, 0.5));
  dstrip.appendChild(dk);
  dstrip.appendChild(dbFader(t('plugin.studio.mixer.delayReturn', null, 'Delay'), returnLevel('delay'), (g) => { ret.delay_level = g; markDirty(); }, { meter: meterNode(() => channels.get(RETURN_IDS.delay)?.meter) }));
  row.appendChild(dstrip);

  /* Master */
  if (!Array.isArray(arrangement.master_fx)) arrangement.master_fx = [];
  if (!arrangement.fx) arrangement.fx = {};
  const fx = arrangement.fx;
  const mstrip = h('div', 'studio-mixer-strip studio-mixer-strip--master');
  mstrip.appendChild(h('span', 'studio-mixer-name', t('plugin.studio.mixer.mainName', null, 'MASTER')));
  mstrip.appendChild(insertSlots('master', arrangement.master_fx, content));
  const mk = h('div', 'studio-mixer-sends studio-mixer-sends--grid');
  mk.appendChild(knob(t('plugin.studio.mixer.glue', null, 'Glue'), 0, 1, 0.01, fx.glue ?? 0, (n) => { fx.glue = n; markDirty(); }, 0));
  mk.appendChild(knob(t('plugin.studio.mixer.ceiling', null, 'Ceiling'), -6, 0, 0.1, fx.ceiling ?? -1, (n) => { fx.ceiling = n; markDirty(); }, -1));
  mk.appendChild(knob(t('plugin.studio.mixer.width', null, 'Width'), 0, 2, 0.01, fx.master_width ?? 1, (n) => { fx.master_width = n; content(); }, 1));
  mk.appendChild(knob(t('plugin.studio.mixer.drive', null, 'Drive'), 0, 1, 0.01, fx.master_drive ?? 0, (n) => { fx.master_drive = n; content(); }, 0));
  mstrip.appendChild(mk);
  const lsel = nativeSelect(LOUDNESS_TARGETS.map(([v, k]) => [v, k.startsWith('plugin.') ? t(k, null, 'Off') : k]), String(Math.round(fx.loudness || 0)) === '0' ? '0' : String(Math.round(fx.loudness)), (v) => {
    const n = Number(v);
    if (n) fx.loudness = n; else delete fx.loudness;
    content();
  }, t('plugin.studio.mixer.loudnessHint', null, 'Loudness target — the export is trimmed to this integrated loudness'));
  lsel.classList.add('studio-mixer-loudness');
  mstrip.appendChild(lsel);
  mstrip.appendChild(dbFader(t('plugin.studio.mixer.mainGain', null, 'Master'), arrangement.master ?? 0.9, (g) => { arrangement.master = g; markDirty(); }, { meter: meterNode(() => masterNodes?.meter) }));
  masterReadoutEl = h('div', 'studio-master-readout');
  mstrip.appendChild(masterReadoutEl);
  row.appendChild(mstrip);

  console_.appendChild(row);
  mixerEl.appendChild(console_);

  /* Open insert editor */
  if (openInsert) {
    let ed = null;
    if (openInsert.owner === 'master') ed = insertEditor(arrangement.master_fx, openInsert.index, t('plugin.studio.mixer.mainName', null, 'MASTER'), content);
    else {
      const tr = arrangement.tracks.find((x) => `track:${x.id}` === openInsert.owner);
      if (tr) ed = insertEditor(tr.fx, openInsert.index, tr.name, content);
    }
    if (ed) mixerEl.appendChild(ed); else openInsert = null;
  }

  /* Launcher strips (when the Launcher is visible): simple level/pan/M/S. */
  if (panels.launcher && launcher?.tracks?.length) {
    const sec = h('div', 'studio-mixer-group');
    sec.appendChild(h('span', 'studio-rack-label', t('plugin.studio.mixer.launcher', null, 'Launcher')));
    const lrow = h('div', 'studio-mixer-row');
    for (const tr of launcher.tracks) {
      const strip = h('div', 'studio-mixer-strip studio-mixer-strip--compact');
      strip.style.setProperty('--track', trackColor(tr.color));
      strip.appendChild(h('span', 'studio-track-stripe'));
      strip.appendChild(h('span', 'studio-mixer-name', tr.name));
      strip.appendChild(knob(t('plugin.studio.param.pan', null, 'Pan'), -1, 1, 0.01, tr.pan ?? 0, (n) => { tr.pan = n; ensureTrackAudio(tr); }, 0));
      strip.appendChild(dbFader(tr.name, tr.level ?? 0.8, (g) => { tr.level = g; ensureTrackAudio(tr); }, { meter: meterNode(() => trackAudio[tr.id]?.analyser) }));
      strip.appendChild(msButtons(tr, () => { for (const x of launcher.tracks) ensureTrackAudio(x); renderMixer(); renderLauncher(); }));
      lrow.appendChild(strip);
    }
    sec.appendChild(lrow);
    mixerEl.appendChild(sec);
  }
  renderMixerReadouts();
  paintMeters();
}

/* Sends a track gets when it has none set: the max pattern reverb/delay mix
   of its clips (how projects stored their sends before the mixer had them). */
function effectiveSends(tr) {
  let reverb = 0;
  let delay = 0;
  for (const c of arrangement.clips) {
    if (c.track !== tr.id || !c.pattern) continue;
    reverb = Math.max(reverb, Number(c.pattern.fx?.reverb_mix) || 0);
    delay = Math.max(delay, Number(c.pattern.fx?.delay_mix) || 0);
  }
  return { reverb, delay };
}

/* ── pop-up browser ─────────────────────────────────────────── */

let browserQuery = '';
const BROWSER_TABS = [['arrangements', 'plugin.studio.browser.tab.projects'], ['audio', 'plugin.studio.browser.tab.audio'], ['patterns', 'plugin.studio.browser.tab.patterns'], ['instruments', 'plugin.studio.browser.tab.instruments'], ['effects', 'plugin.studio.browser.tab.effects'], ['presets', 'plugin.studio.browser.tab.presets']];

function openBrowser(tab, mode = null) {
  browserTab = tab || browserTab;
  if (browserTab === 'audio') libraryFiles = null;   // re-list on open
  browserMode = mode ? { type: mode, area: panels.launcher && !panels.arranger ? 'lch' : 'arr' } : null;
  browserEl?.classList.remove('hidden');
  browserBtn?.setAttribute('aria-expanded', 'true');
  browserReveal = true;
  renderBrowser();
  browserEl?.querySelector('input')?.focus();
}
function closeBrowser() {
  stopLibraryPreview();
  browserEl?.classList.add('hidden');
  browserBtn?.setAttribute('aria-expanded', 'false');
  browserMode = null;
}
function toggleBrowser() {
  if (browserEl?.classList.contains('hidden')) openBrowser(browserTab);
  else closeBrowser();
}

/* ── SynthMe — build & save custom instruments ───────────────── */

const SYNTHME_WAVES = [[0, 'plugin.studio.wave.sine'], [1, 'plugin.studio.wave.triangle'], [2, 'plugin.studio.wave.saw'], [3, 'plugin.studio.wave.square']];
const SYNTHME_FILTERS = [[0, 'plugin.studio.filter.lowpass'], [1, 'plugin.studio.filter.highpass'], [2, 'plugin.studio.filter.bandpass']];

function defaultSynthmeDraft() {
  return {
    name: 'My Synth',
    synth: { o1w: 2, o2w: 3, detune: 0.1, mix: 0.5, noise: 0, ftype: 0, cutoff: 2000, res: 1, drive: 2, attack: 0.005, decay: 0.2, sustain: 0.6, release: 0.3 },
    midi: [],
    fx: [],
  };
}
function synthmeFxParamDefs(kind) {
  return EFFECTS[kind]?.params || [];
}
function renderSynthme() {
  if (!synthmeWrapEl) return;
  if (!synthmeDraft) synthmeDraft = defaultSynthmeDraft();
  synthmeWrapEl.textContent = '';
  const d = synthmeDraft;

  const head = h('div', 'studio-synthme-head');
  head.appendChild(h('span', 'studio-synthme-title', t('plugin.studio.synthme.title', null, 'SynthMe — instrument creator')));
  const name = uiInput();
  name.type = 'text';
  name.classList.add(...('studio-title studio-title--clip').split(/\s+/));
  name.value = d.name;
  name.maxLength = 80;
  name.placeholder = t('plugin.studio.synthme.namePlaceholder', null, 'Instrument name');
  name.addEventListener('change', () => { d.name = name.value.trim() || 'My Synth'; });
  head.appendChild(name);
  const preview = h('button', 'studio-btn studio-btn--play', '');
  preview.type = 'button';
  preview.title = t(auditionSource ? 'plugin.studio.common.stopPreview' : 'plugin.studio.synthme.preview', null, auditionSource ? 'Stop preview' : 'Preview this instrument');
  preview.appendChild(icon(auditionSource ? 'ui/stop' : 'ui/play', { size: 12 }));
  preview.addEventListener('click', () => void previewSynthmeDraft());
  const reset = h('button', 'studio-btn', t('plugin.studio.synthme.reset', null, 'Reset'));
  reset.type = 'button';
  reset.title = t('plugin.studio.synthme.resetHint', null, 'Reset the instrument to defaults');
  reset.addEventListener('click', () => { synthmeDraft = defaultSynthmeDraft(); renderSynthme(); });
  const close = iconCtl('ui/close');
  close.type = 'button';
  close.title = t('plugin.studio.common.backToPanels', null, 'Back to Arranger/Launcher');
  close.addEventListener('click', () => togglePanel('synthme'));
  head.append(preview, reset, close);
  synthmeWrapEl.appendChild(head);

  const body = h('div', 'studio-synthme-body');

  /* oscillators + noise */
  body.appendChild(h('span', 'studio-rack-label', t('plugin.studio.synthme.oscillators', null, 'Oscillators & Noise')));
  const osc = h('div', 'studio-synthme-row');
  osc.appendChild(nativeSelect(SYNTHME_WAVES.map(([w, k]) => [String(w), t(k)]), String(d.synth.o1w), (val) => { d.synth.o1w = parseInt(val, 10); }, t('plugin.studio.synthme.osc1Wave', null, 'Osc 1 wave')));
  osc.appendChild(nativeSelect(SYNTHME_WAVES.map(([w, k]) => [String(w), t(k)]), String(d.synth.o2w), (val) => { d.synth.o2w = parseInt(val, 10); }, t('plugin.studio.synthme.osc2Wave', null, 'Osc 2 wave')));
  osc.appendChild(knob(t('plugin.studio.param.detune', null, 'Detune'), -24, 24, 0.1, d.synth.detune, (n) => { d.synth.detune = n; }, 0.1));
  osc.appendChild(knob(t('plugin.studio.param.mix', null, 'Mix'), 0, 1, 0.05, d.synth.mix, (n) => { d.synth.mix = n; }, 0.5));
  osc.appendChild(knob(t('plugin.studio.param.noise', null, 'Noise'), 0, 1, 0.05, d.synth.noise, (n) => { d.synth.noise = n; }, 0));
  body.appendChild(osc);

  /* filter + drive */
  body.appendChild(h('span', 'studio-rack-label', t('plugin.studio.synthme.filterDrive', null, 'Filter & Drive')));
  const filt = h('div', 'studio-synthme-row');
  filt.appendChild(nativeSelect(SYNTHME_FILTERS.map(([w, k]) => [String(w), t(k)]), String(d.synth.ftype), (val) => { d.synth.ftype = parseInt(val, 10); }, t('plugin.studio.hint.filterType', null, 'Filter type')));
  filt.appendChild(knob(t('plugin.studio.param.cutoff', null, 'Cutoff'), 20, 20000, 10, d.synth.cutoff, (n) => { d.synth.cutoff = n; }, 2000));
  filt.appendChild(knob(t('plugin.studio.param.res', null, 'Res'), 0.1, 20, 0.1, d.synth.res, (n) => { d.synth.res = n; }, 1));
  filt.appendChild(knob(t('plugin.studio.param.drive', null, 'Drive'), 0.25, 24, 0.05, d.synth.drive, (n) => { d.synth.drive = n; }, 2));
  body.appendChild(filt);

  /* envelope */
  body.appendChild(h('span', 'studio-rack-label', t('plugin.studio.synthme.envelope', null, 'Envelope')));
  const env = h('div', 'studio-synthme-row');
  env.appendChild(knob(t('plugin.studio.param.attack', null, 'Attack'), 0.001, 5, 0.005, d.synth.attack, (n) => { d.synth.attack = n; }, 0.005));
  env.appendChild(knob(t('plugin.studio.param.decay', null, 'Decay'), 0.001, 5, 0.01, d.synth.decay, (n) => { d.synth.decay = n; }, 0.2));
  env.appendChild(knob(t('plugin.studio.param.sustain', null, 'Sustain'), 0, 1, 0.05, d.synth.sustain, (n) => { d.synth.sustain = n; }, 0.6));
  env.appendChild(knob(t('plugin.studio.param.release', null, 'Release'), 0.001, 5, 0.01, d.synth.release, (n) => { d.synth.release = n; }, 0.3));
  body.appendChild(env);

  /* MIDI effects */
  body.appendChild(h('span', 'studio-rack-label', t('plugin.studio.synthme.midiEffects', null, 'MIDI Effects')));
  const midiWrap = h('div', 'studio-synthme-fx');
  d.midi.forEach((m, i) => {
    const row = h('div', 'studio-synthme-fx-row');
    const sel = nativeSelect(MIDI_FX_KINDS.map((k) => [k, MIDI_FX[k].label]), m.kind, (val) => { m.kind = val; m.params = {}; renderSynthme(); }, t('plugin.studio.hint.midiEffect', null, 'MIDI effect'));
    row.appendChild(sel);
    const rm = iconCtl('ui/close');
    rm.type = 'button';
    rm.title = t('plugin.studio.synthme.removeMidiFx', null, 'Remove MIDI effect');
    rm.addEventListener('click', () => { d.midi.splice(i, 1); renderSynthme(); });
    row.appendChild(rm);
    midiWrap.appendChild(row);
    for (const p of (MIDI_FX[m.kind]?.params || [])) {
      const val = m.params[p.key] != null ? m.params[p.key] : p.def;
      midiWrap.appendChild(knob(p.label, p.min, p.max, p.step, val, (n) => { m.params[p.key] = n; }, p.def));
    }
  });
  const midiAddRow = h('div', 'studio-synthme-fx-row');
  const midiAddSel = nativeSelect(MIDI_FX_KINDS.map((k) => [k, MIDI_FX[k].label]), 'transpose', () => {});
  const midiAddBtn = plusLabel(h('button', 'studio-btn'), t('plugin.studio.synthme.addMidiFx', null, '+ MIDI FX'));
  midiAddBtn.type = 'button';
  midiAddBtn.addEventListener('click', () => {
    if (d.midi.length >= 6) { toast(t('plugin.studio.toast.maxMidiFx', null, 'Max 6 MIDI effects'), { type: 'error' }); return; }
    d.midi.push({ kind: midiAddSel.value, params: {} });
    renderSynthme();
  });
  midiAddRow.append(midiAddSel, midiAddBtn);
  midiWrap.appendChild(midiAddRow);
  body.appendChild(midiWrap);

  /* effects */
  body.appendChild(h('span', 'studio-rack-label', t('plugin.studio.synthme.effects', null, 'Effects')));
  const fxWrap = h('div', 'studio-synthme-fx');
  d.fx.forEach((f, i) => {
    const row = h('div', 'studio-synthme-fx-row');
    row.appendChild(h('span', 'studio-device-name', EFFECTS[f.kind]?.label || f.kind));
    const rm = iconCtl('ui/close');
    rm.type = 'button';
    rm.title = t('plugin.studio.synthme.removeFx', null, 'Remove effect');
    rm.addEventListener('click', () => { d.fx.splice(i, 1); renderSynthme(); });
    row.appendChild(rm);
    fxWrap.appendChild(row);
    for (const p of (synthmeFxParamDefs(f.kind))) {
      const val = f.params[p.key] != null ? f.params[p.key] : p.def;
      fxWrap.appendChild(knob(p.label, p.min, p.max, p.step, val, (n) => { f.params[p.key] = n; }, p.def));
    }
  });
  const addRow = h('div', 'studio-synthme-fx-row');
  const addSel = nativeSelect(EFFECT_KINDS.map((k) => [k, EFFECTS[k].label]), 'distortion', () => {});
  const addBtn = plusLabel(h('button', 'studio-btn'), t('plugin.studio.synthme.addFx', null, '+ FX'));
  addBtn.type = 'button';
  addBtn.addEventListener('click', () => {
    if (d.fx.length >= 6) { toast(t('plugin.studio.toast.maxFx6', null, 'Max 6 effects'), { type: 'error' }); return; }
    d.fx.push({ kind: addSel.value, params: {}, bypass: false });
    renderSynthme();
  });
  addRow.append(addSel, addBtn);
  fxWrap.appendChild(addRow);
  body.appendChild(fxWrap);

  synthmeWrapEl.appendChild(body);

  /* footer actions */
  const foot = h('div', 'studio-synthme-foot');
  const applyBtn = h('button', 'studio-btn', t('plugin.studio.common.applyToVoice', null, 'Apply to voice'));
  applyBtn.type = 'button';
  applyBtn.title = t('plugin.studio.synthme.applyHint', null, 'Use this instrument on the selected voice');
  applyBtn.addEventListener('click', () => applySynthmeDraft());
  const saveBtn = h('button', 'studio-btn', t('plugin.studio.synthme.saveInstrument', null, 'Save instrument'));
  saveBtn.type = 'button';
  saveBtn.title = t('plugin.studio.synthme.saveHint', null, 'Save as a reusable instrument');
  saveBtn.addEventListener('click', () => void saveSynthmeInstrument());
  foot.append(applyBtn, saveBtn);
  synthmeWrapEl.appendChild(foot);
}
function applySynthmeDraft() {
  const v = current?.voices[selectedVoice];
  if (!v) { toast(t('plugin.studio.toast.selectClipFirst', null, 'Select a clip first'), { type: 'error' }); return; }
  v.kind = 'synthme';
  v.synth = { ...synthmeDraft.synth };
  v.fx = synthmeDraft.fx.map((f) => ({ kind: f.kind, params: { ...f.params }, bypass: !!f.bypass }));
  v.midi = synthmeDraft.midi.map((m) => ({ kind: m.kind, params: { ...m.params } }));
  markDirty();
  renderEditor();
  renderDevices();
  toast(t('plugin.studio.toast.synthmeApplied', null, 'SynthMe instrument applied'), { type: 'success' });
}
async function previewSynthmeDraft() {
  if (auditionSource) { stopAudition(); setStatus(dirty ? 'dirty' : 'saved'); renderSynthme(); return; }
  try {
    const d = synthmeDraft;
    const cfg = {
      title: d.name || 'SynthMe',
      bpm: arrangement.bpm,
      steps: 16,
      tuning: 'edo12',
      voices: [{
        kind: 'synthme', rhythm: 'x...x...x...x...', degree: 0, octave: 3,
        synth: { ...d.synth },
        midi: d.midi.map((m) => ({ kind: m.kind, params: m.params })),
        fx: d.fx.map((f) => ({ kind: f.kind, params: f.params, bypass: f.bypass })),
      }],
      fx: {},
    };
    await previewAudition(cfg);
    renderSynthme();
  } catch (e) {
    toast(e.message || t('plugin.studio.toast.previewFailed', null, 'Preview failed'), { type: 'error' });
  }
}
async function saveSynthmeInstrument() {
  const d = synthmeDraft;
  if (!d.name.trim()) { toast(t('plugin.studio.toast.nameInstrument', null, 'Name the instrument first'), { type: 'error' }); return; }
  await api('/api/studio/presets', {
    method: 'POST',
    body: JSON.stringify({ kind: 'synthme', name: d.name.trim(), params: { synth: d.synth, midi: d.midi.map((m) => ({ kind: m.kind, params: m.params })), fx: d.fx.map((f) => ({ kind: f.kind, params: f.params, bypass: f.bypass })) } }),
  });
  await refreshPresets();
  renderBrowser();
  toast(t('plugin.studio.toast.instrumentSaved', { name: d.name.trim() }, `Saved instrument “${d.name.trim()}”`), { type: 'success' });
}
/* Turn a saved SynthMe preset into a full voice / pattern. */
function synthmePresetVoice(p) {
  const pr = p.params || {};
  const v = voice('synthme', 'x...', {});
  v.synth = { ...(pr.synth || {}) };
  v.fx = (pr.fx || []).map((f) => ({ kind: f.kind, params: { ...(f.params || {}) }, bypass: !!f.bypass }));
  v.midi = (pr.midi || []).map((m) => ({ kind: m.kind, params: { ...(m.params || {}) } }));
  return v;
}
function synthmePresetPattern(p) {
  const pat = kitPattern(16);
  pat.title = p.name || 'SynthMe';
  pat.voices = [synthmePresetVoice(p)];
  return pat;
}
function gridPresetVoice(p) {
  const pr = p.params || {};
  const v = voice('grid', 'x...', {});
  v.grid = pr.grid ? { modules: (pr.grid.modules || []).map((m) => ({ id: m.id, kind: m.kind, params: { ...(m.params || {}) } })), cables: (pr.grid.cables || []).map((c) => ({ from: [c.from[0], c.from[1]], to: [c.to[0], c.to[1]] })) } : null;
  return v;
}

/* ── WaveMe — modular patch editor (The Grid) ────────────────── */

const GRID_SIG = { audio: '#ff9f43', ctrl: '#57a0ff', pitch: '#ffe156', gate: '#8dff9e' };
const GRID_MODULE_LABELS = { osc: 'plugin.studio.grid.module.osc', noise: 'plugin.studio.grid.module.noise', filter: 'plugin.studio.grid.module.filter', drive: 'plugin.studio.grid.module.drive', gain: 'plugin.studio.grid.module.gain', mixer: 'plugin.studio.grid.module.mixer', env: 'plugin.studio.grid.module.env', lfo: 'plugin.studio.grid.module.lfo', out: 'plugin.studio.grid.module.out' };

function gridPorts(kind) {
  switch (kind) {
    case 'osc': return { inputs: [], outputs: [{ name: 'out', type: 'audio' }], precord: 'pitch' };
    case 'noise': return { inputs: [], outputs: [{ name: 'out', type: 'audio' }], precord: null };
    case 'filter': return { inputs: [{ name: 'in', type: 'audio' }, { name: 'mod', type: 'ctrl' }], outputs: [{ name: 'out', type: 'audio' }], precord: null };
    case 'drive': return { inputs: [{ name: 'in', type: 'audio' }, { name: 'mod', type: 'ctrl' }], outputs: [{ name: 'out', type: 'audio' }], precord: null };
    case 'gain': return { inputs: [{ name: 'in', type: 'audio' }, { name: 'mod', type: 'ctrl' }], outputs: [{ name: 'out', type: 'audio' }], precord: null };
    case 'mixer': return { inputs: [{ name: 'a', type: 'audio' }, { name: 'b', type: 'audio' }], outputs: [{ name: 'out', type: 'audio' }], precord: null };
    case 'env': return { inputs: [{ name: 'in', type: 'audio' }], outputs: [{ name: 'out', type: 'audio' }, { name: 'ctrl', type: 'ctrl' }], precord: 'gate' };
    case 'lfo': return { inputs: [], outputs: [{ name: 'ctrl', type: 'ctrl' }], precord: null };
    case 'out': return { inputs: [{ name: 'in', type: 'audio' }], outputs: [], precord: null };
    default: return { inputs: [], outputs: [], precord: null };
  }
}
function gridModuleParams(kind) {
  switch (kind) {
    case 'osc': return [{ key: 'wave', label: t('plugin.studio.param.wave', null, 'Wave'), min: 0, max: 3, step: 1, def: 2 }, { key: 'detune', label: t('plugin.studio.param.detune', null, 'Detune'), min: -24, max: 24, step: 0.1, def: 0 }];
    case 'filter': return [{ key: 'type', label: t('plugin.studio.param.type', null, 'Type'), min: 0, max: 2, step: 1, def: 0 }, { key: 'cutoff', label: t('plugin.studio.param.cutoff', null, 'Cutoff'), min: 20, max: 20000, step: 10, def: 2000 }, { key: 'res', label: t('plugin.studio.param.res', null, 'Res'), min: 0.1, max: 20, step: 0.1, def: 1 }];
    case 'drive': return [{ key: 'drive', label: t('plugin.studio.param.drive', null, 'Drive'), min: 0.25, max: 24, step: 0.05, def: 2 }];
    case 'gain': return [{ key: 'level', label: t('plugin.studio.param.level', null, 'Level'), min: 0, max: 2, step: 0.01, def: 0.8 }];
    case 'mixer': return [{ key: 'balance', label: t('plugin.studio.param.balance', null, 'Balance'), min: 0, max: 1, step: 0.05, def: 0.5 }];
    case 'env': return [{ key: 'attack', label: 'A', min: 0.001, max: 5, step: 0.005, def: 0.005 }, { key: 'decay', label: 'D', min: 0.001, max: 5, step: 0.01, def: 0.2 }, { key: 'sustain', label: 'S', min: 0, max: 1, step: 0.05, def: 0.6 }, { key: 'release', label: 'R', min: 0.001, max: 5, step: 0.01, def: 0.3 }];
    case 'lfo': return [{ key: 'rate', label: t('plugin.studio.param.rate', null, 'Rate'), min: 0.05, max: 20, step: 0.01, def: 1 }, { key: 'depth', label: t('plugin.studio.param.depth', null, 'Depth'), min: 0, max: 1, step: 0.01, def: 0.5 }, { key: 'wave', label: t('plugin.studio.param.wave', null, 'Wave'), min: 0, max: 2, step: 1, def: 0 }];
    default: return [];
  }
}
function gridDefaultDraft() {
  return {
    name: 'My Grid',
    modules: [
      { id: 'm1', kind: 'osc', x: 16, y: 60, params: {} },
      { id: 'm2', kind: 'filter', x: 150, y: 60, params: {} },
      { id: 'm3', kind: 'env', x: 300, y: 60, params: {} },
      { id: 'm4', kind: 'out', x: 440, y: 60, params: {} },
    ],
    cables: [
      { from: ['m1', 'out'], to: ['m2', 'in'] },
      { from: ['m2', 'out'], to: ['m3', 'in'] },
      { from: ['m3', 'out'], to: ['m4', 'in'] },
    ],
  };
}
function renderGrid() {
  if (!gridWrapEl) return;
  if (!gridDraft) gridDraft = gridDefaultDraft();
  gridWrapEl.textContent = '';
  const d = gridDraft;

  const head = h('div', 'studio-synthme-head');
  head.appendChild(h('span', 'studio-synthme-title', 'WaveMe'));
  const name = uiInput();
  name.type = 'text';
  name.classList.add(...('studio-title studio-title--clip').split(/\s+/));
  name.value = d.name;
  name.placeholder = t('plugin.studio.grid.namePlaceholder', null, 'Patch name');
  name.addEventListener('change', () => { d.name = name.value.trim() || 'My Grid'; });
  head.appendChild(name);
  const preview = h('button', 'studio-btn studio-btn--play', '');
  preview.type = 'button';
  preview.title = t(auditionSource ? 'plugin.studio.common.stopPreview' : 'plugin.studio.grid.preview', null, auditionSource ? 'Stop preview' : 'Preview patch');
  preview.appendChild(icon(auditionSource ? 'ui/stop' : 'ui/play', { size: 12 }));
  preview.addEventListener('click', () => void previewGridDraft());
  const apply = h('button', 'studio-btn', t('plugin.studio.common.applyToVoice', null, 'Apply to voice'));
  apply.type = 'button';
  apply.addEventListener('click', () => applyGridDraft());
  const save = h('button', 'studio-btn', t('plugin.studio.common.save', null, 'Save'));
  save.type = 'button';
  save.addEventListener('click', () => void saveGridInstrument());
  const close = iconCtl('ui/close');
  close.type = 'button';
  close.title = t('plugin.studio.common.backToPanels', null, 'Back to Arranger/Launcher');
  close.addEventListener('click', () => togglePanel('grid'));
  head.append(preview, apply, save, close);
  gridWrapEl.appendChild(head);

  /* palette */
  const palette = h('div', 'studio-grid-palette');
  for (const k of ['osc', 'noise', 'filter', 'drive', 'gain', 'mixer', 'env', 'lfo', 'out']) {
    const b = h('button', 'studio-btn studio-grid-palette-item', t(GRID_MODULE_LABELS[k]));
    b.type = 'button';
    b.title = t('plugin.studio.grid.addModule', { name: t(GRID_MODULE_LABELS[k]) }, `Add ${GRID_MODULE_LABELS[k]}`);
    b.addEventListener('click', () => { addGridModule(k); });
    palette.appendChild(b);
  }
  gridWrapEl.appendChild(palette);

  /* canvas */
  const canvas = h('div', 'studio-grid-canvas');
  canvas.dataset.canvas = '1';
  const svg = svgEl('svg', { 'class': 'studio-grid-svg', width: '100%', height: '100%' });
  canvas.appendChild(svg);
  for (const m of d.modules) {
    canvas.appendChild(renderGridModule(m));
  }
  canvas.addEventListener('pointerdown', (e) => {
    if (e.target === canvas || e.target === svg) { gridSel = null; renderGrid(); }
  });
  gridWrapEl.appendChild(canvas);
  gridSvgEl = svg;
  gridCanvasEl = canvas;
  requestAnimationFrame(() => renderGridCables());
}
function renderGridModule(m) {
  const ports = gridPorts(m.kind);
  const el = h('div', 'studio-grid-module');
  el.dataset.mid = m.id;
  el.classList.toggle('studio-grid-module--sel', gridSel === m.id);
  el.style.left = `${m.x}px`;
  el.style.top = `${m.y}px`;
  el.addEventListener('pointerdown', (e) => {
    if (e.target.closest('.studio-grid-port') || e.target.closest('.studio-grid-module-params')) return;
    if (gridSel !== m.id) {
      gridSel = m.id;
      renderGrid();
      const fresh = gridCanvasEl?.querySelector(`.studio-grid-module[data-mid="${m.id}"]`);
      if (fresh) startGridModuleDrag(e, m, fresh);
      return;
    }
    startGridModuleDrag(e, m, el);
  });

  const head = h('div', 'studio-grid-module-head', t(GRID_MODULE_LABELS[m.kind]));
  if (ports.precord) head.appendChild(h('span', 'studio-grid-precord', ports.precord === 'pitch' ? '♪' : '▮'));
  if (m.kind !== 'out') {
    const rm = iconCtl('ui/close', 'studio-head-btn studio-grid-module-x');
    rm.type = 'button';
    rm.title = t('plugin.studio.grid.removeModule', null, 'Remove module');
    rm.addEventListener('pointerdown', (e) => e.stopPropagation());
    rm.addEventListener('click', (e) => { e.stopPropagation(); removeGridModule(m.id); });
    head.appendChild(rm);
  }
  el.appendChild(head);

  /* inputs on the left, outputs on the right */
  ports.inputs.forEach((p, i) => el.appendChild(renderGridPort(m, p, true, i)));
  ports.outputs.forEach((p, i) => el.appendChild(renderGridPort(m, p, false, i)));

  /* params — always visible on the module face */
  const body = h('div', 'studio-grid-module-params');
  for (const p of gridModuleParams(m.kind)) {
    const value = m.params[p.key] != null ? m.params[p.key] : p.def;
    if ((m.kind === 'osc' && p.key === 'wave') || (m.kind === 'filter' && p.key === 'type') || (m.kind === 'lfo' && p.key === 'wave')) {
      const opts = p.key === 'wave' ? [[0, 'plugin.studio.wave.sine'], [1, 'plugin.studio.wave.tri'], [2, 'plugin.studio.wave.saw'], [3, 'plugin.studio.wave.sq']].map(([v, k]) => [String(v), t(k)]) : [[0, 'plugin.studio.filter.lp'], [1, 'plugin.studio.filter.hp'], [2, 'plugin.studio.filter.bp']].map(([v, k]) => [String(v), t(k)]);
      body.appendChild(nativeSelect(opts, String(Math.round(value)), (val) => { m.params[p.key] = parseFloat(val); renderGrid(); }, p.label));
    } else {
      body.appendChild(knob(p.label, p.min, p.max, p.step, value, (n) => { m.params[p.key] = n; }, p.def));
    }
  }
  el.appendChild(body);
  return el;
}
function renderGridPort(m, port, isInput, idx) {
  const p = h('span', 'studio-grid-port');
  p.classList.add(isInput ? 'studio-grid-port--in' : 'studio-grid-port--out');
  p.style.setProperty('--sig', GRID_SIG[port.type] || '#888');
  p.style.top = `${26 + idx * 16}px`;
  p.title = `${m.kind} · ${port.name}`;
  p.dataset.mid = m.id;
  p.dataset.port = port.name;
  p.dataset.dir = isInput ? 'in' : 'out';
  p.addEventListener('pointerdown', (e) => startGridCable(e, p));
  p.addEventListener('dblclick', (e) => { e.stopPropagation(); removeGridCable(m.id, port.name, isInput); });
  return p;
}
function addGridModule(kind) {
  const id = 'm' + Date.now().toString(36) + Math.floor(Math.random() * 999);
  gridDraft.modules.push({ id, kind, x: 40 + (gridDraft.modules.length % 4) * 130, y: 60 + Math.floor(gridDraft.modules.length / 4) * 120, params: {} });
  gridSel = id;   // open the new module's parameter panel
  renderGrid();
}
function removeGridCable(mid, port, isInput) {
  gridDraft.cables = gridDraft.cables.filter((c) => !((isInput && c.to[0] === mid && c.to[1] === port) || (!isInput && c.from[0] === mid && c.from[1] === port)));
  renderGrid();
}
function removeGridModule(mid) {
  gridDraft.modules = gridDraft.modules.filter((m) => m.id !== mid);
  gridDraft.cables = gridDraft.cables.filter((c) => c.from[0] !== mid && c.to[0] !== mid);
  if (gridSel === mid) gridSel = null;
  renderGrid();
}
let gridDragCable = null; // { from: [mid, port], to: [mid, port] | null, x, y }
function startGridCable(e, portEl) {
  e.stopPropagation();
  e.preventDefault();
  const mid = portEl.dataset.mid;
  const port = portEl.dataset.port;
  const dir = portEl.dataset.dir;
  const rect = portEl.getBoundingClientRect();
  gridDragCable = { mid, port, dir, x: rect.left + rect.width / 2, y: rect.top + rect.height / 2, to: null };
  const move = (ev) => {
    gridDragCable.x = ev.clientX;
    gridDragCable.y = ev.clientY;
    renderGridCables();
  };
  const up = (ev) => {
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    const target = document.elementFromPoint(ev.clientX, ev.clientY)?.closest?.('.studio-grid-port');
    if (target && target.dataset.mid !== mid && target.dataset.dir !== dir) {
      const tmid = target.dataset.mid;
      const tport = target.dataset.port;
      const tdir = target.dataset.dir;
      const from = dir === 'out' ? [mid, port] : [tmid, tport];
      const to = dir === 'out' ? [tmid, tport] : [mid, port];
      // in ports accept only one cable
      gridDraft.cables = gridDraft.cables.filter((c) => !(c.to[0] === to[0] && c.to[1] === to[1]));
      gridDraft.cables.push({ from, to });
    }
    gridDragCable = null;
    renderGrid();
  };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
}
function renderGridCables() {
  if (!gridSvgEl || !gridCanvasEl || !gridDraft) return;
  const svg = gridSvgEl;
  const rect = gridCanvasEl.getBoundingClientRect();
  svg.textContent = '';
  const portPos = (mid, port) => {
    const el = gridCanvasEl.querySelector(`.studio-grid-port[data-mid="${mid}"][data-port="${port}"]`);
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { x: r.left - rect.left + r.width / 2, y: r.top - rect.top + r.height / 2 };
  };
  for (const c of gridDraft.cables) {
    const a = portPos(c.from[0], c.from[1]);
    const b = portPos(c.to[0], c.to[1]);
    if (!a || !b) continue;
    const path = svgEl('path', { d: `M ${a.x} ${a.y} C ${a.x + 40} ${a.y}, ${b.x - 40} ${b.y}, ${b.x} ${b.y}`, 'class': 'studio-grid-cable' });
    path.style.stroke = GRID_SIG[gridPortType(c.from[0], c.from[1])] || '#888';
    svg.appendChild(path);
  }
  if (gridDragCable) {
    const a = gridDragCable.dir === 'out' ? portPos(gridDragCable.mid, gridDragCable.port) : { x: gridDragCable.x - rect.left, y: gridDragCable.y - rect.top };
    const b = gridDragCable.dir === 'out' ? { x: gridDragCable.x - rect.left, y: gridDragCable.y - rect.top } : portPos(gridDragCable.mid, gridDragCable.port);
    if (a && b) svg.appendChild(svgEl('path', { d: `M ${a.x} ${a.y} L ${b.x} ${b.y}`, 'class': 'studio-grid-cable studio-grid-cable--drag' }));
  }
}
function gridPortType(mid, port) {
  const m = gridDraft.modules.find((x) => x.id === mid);
  if (!m) return 'audio';
  const ports = gridPorts(m.kind);
  const found = [...ports.inputs, ...ports.outputs].find((p) => p.name === port);
  return found ? found.type : 'audio';
}
function startGridModuleDrag(e, m, el) {
  e.preventDefault();
  const startX = e.clientX, startY = e.clientY, ox = m.x, oy = m.y;
  const move = (ev) => {
    m.x = Math.max(0, ox + ev.clientX - startX);
    m.y = Math.max(0, oy + ev.clientY - startY);
    el.style.left = `${m.x}px`;
    el.style.top = `${m.y}px`;
    renderGridCables();
  };
  const up = () => {
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    renderGrid();
  };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
}
function applyGridDraft() {
  const v = current?.voices[selectedVoice];
  if (!v) { toast(t('plugin.studio.toast.selectClipFirst', null, 'Select a clip first'), { type: 'error' }); return; }
  v.kind = 'grid';
  v.grid = gridDraftToPatch(gridDraft);
  v.synth = {};
  markDirty();
  renderEditor();
  renderDevices();
  toast(t('plugin.studio.toast.gridApplied', null, 'WaveMe patch applied'), { type: 'success' });
}
async function previewGridDraft() {
  if (auditionSource) { stopAudition(); setStatus(dirty ? 'dirty' : 'saved'); renderGrid(); return; }
  try {
    const cfg = {
      title: gridDraft.name || 'Grid', bpm: arrangement.bpm, steps: 16, tuning: 'edo12',
      voices: [{ kind: 'grid', rhythm: 'x...x...x...x...', degree: 0, octave: 3, grid: gridDraftToPatch(gridDraft) }],
      fx: {},
    };
    await previewAudition(cfg);
    renderGrid();
  } catch (e) { toast(e.message || t('plugin.studio.toast.previewFailed', null, 'Preview failed'), { type: 'error' }); }
}
function gridDraftToPatch(d) {
  return {
    modules: d.modules.map((m) => ({ id: m.id, kind: m.kind, params: { ...m.params } })),
    cables: d.cables.map((c) => ({ from: [c.from[0], c.from[1]], to: [c.to[0], c.to[1]] })),
  };
}
async function saveGridInstrument() {
  const d = gridDraft;
  if (!d.name.trim()) { toast(t('plugin.studio.toast.namePatch', null, 'Name the patch first'), { type: 'error' }); return; }
  await api('/api/studio/presets', {
    method: 'POST',
    body: JSON.stringify({ kind: 'grid', name: d.name.trim(), params: { grid: gridDraftToPatch(d) } }),
  });
  await refreshPresets();
  renderBrowser();
  toast(t('plugin.studio.toast.patchSaved', { name: d.name.trim() }, `Saved WaveMe patch “${d.name.trim()}”`), { type: 'success' });
}

function browserRow({ label, sub, color, lead, onClick, onDblClick, actions = [] }) {
  const r = h('div', 'studio-browser-item');
  r.title = label;
  if (lead) r.appendChild(icon(lead, { size: 14, className: 'studio-browser-lead' }));
  if (color) {
    const d = h('span', 'studio-track-dot');
    d.style.setProperty('--track', color);
    r.appendChild(d);
  }
  const text = h('span', 'studio-browser-text');
  text.appendChild(h('span', 'studio-browser-label', label));
  if (sub) text.appendChild(h('span', 'studio-browser-sub', sub));
  r.appendChild(text);
  for (const a of actions) r.appendChild(a);
  if (onClick) r.addEventListener('click', onClick);
  if (onDblClick) r.addEventListener('dblclick', onDblClick);
  return r;
}
function smallBtn(iconName, title, onClick) {
  const b = iconCtl(iconName, 'studio-head-btn', title);
  b.addEventListener('click', (e) => { e.stopPropagation(); onClick(e); });
  return b;
}
/* "+ Add track"-style labels (every catalog writes them with a leading "+"):
   the glyph becomes the catalog plus icon, the words stay. */
function stripPlus(text) {
  return String(text).replace(/^\+\s*/, '');
}
function plusLabel(el, text) {
  el.replaceChildren(icon('ui/plus', { size: 12 }), h('span', null, stripPlus(text)));
  return el;
}
/* A compact icon control (catalog icon, never a text glyph) on the shared
   toolButton contract; Studio's CSS sizes it for the dense DAW rows. */
function iconCtl(iconName, className = 'studio-head-btn', title = '') {
  const b = toolButton(iconName, title, null, { className, iconSize: 13 });
  b.type = 'button';
  return b;
}

/* Rows cascade in when the list is (re)loaded — the browser opening or a tab
   switch — not on every re-render (search keystrokes, a saved preset). */
/* ── audio library (WAV files under Music/, AI songs first-class) ── */

let libraryFiles = null;         // [{ path, name, bytes, modified, generated }]
let libraryLoading = false;
let libraryPreview = null;       // { path, audio, url }
async function refreshLibrary() {
  if (libraryLoading) return;
  libraryLoading = true;
  try {
    const data = await api('/api/studio/audio/library');
    libraryFiles = data?.files || [];
  } catch (e) {
    libraryFiles = [];
    console.warn('studio: audio library unavailable', e);
  } finally {
    libraryLoading = false;
  }
  if (browserTab === 'audio') renderBrowser();
}
function stopLibraryPreview() {
  if (!libraryPreview) return;
  try { libraryPreview.audio.pause(); } catch (_) { /* noop */ }
  URL.revokeObjectURL(libraryPreview.url);
  libraryPreview = null;
}
async function toggleLibraryPreview(path) {
  const same = libraryPreview?.path === path;
  stopLibraryPreview();
  if (same) { renderBrowser(); return; }
  try {
    const blob = await apiFetch(`/api/studio/audio/file?path=${encodeURIComponent(path)}`, { responseType: 'blob' });
    const url = URL.createObjectURL(blob);
    const audio = new Audio(url);
    audio.addEventListener('ended', () => { stopLibraryPreview(); renderBrowser(); });
    libraryPreview = { path, audio, url };
    await audio.play();
  } catch (e) {
    toast(e.message || t('plugin.studio.toast.playFailed', null, 'Play failed'), { type: 'error' });
  }
  renderBrowser();
}
function fmtBytes(n) {
  if (n >= 1048576) return `${(n / 1048576).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(n / 1024))} KB`;
}
function renderLibraryList(match, searching = false) {
  if (libraryFiles == null) {
    if (!searching) browserListEl.appendChild(h('div', 'studio-empty', t('plugin.studio.library.loading', null, 'Loading your audio…')));
    void refreshLibrary();
    return;
  }
  if (!searching) browserListEl.appendChild(browserRow({
    label: t('plugin.studio.library.generate', null, 'Generate a song with AI…'),
    lead: 'ui/magic-wand',
    onClick: () => { closeBrowser(); openAiPanel(); },
  }));
  if (!libraryFiles.length) {
    if (searching) return;
    browserListEl.appendChild(h('div', 'studio-empty', t('plugin.studio.library.empty', null, 'No WAV files in Music/ yet. Songs you generate with AI land in Music/Generated.')));
    return;
  }
  for (const f of libraryFiles) {
    if (!match(f.name) && !match(f.path)) continue;
    const playing = libraryPreview?.path === f.path;
    browserListEl.appendChild(browserRow({
      label: f.name.replace(/\.wav$/i, ''),
      sub: `${f.generated ? t('plugin.studio.library.aiTag', null, 'AI song') + ' · ' : ''}${f.path.replace(/\/[^/]*$/, '')} · ${fmtBytes(f.bytes)}`,
      lead: f.generated ? 'ui/magic-wand' : 'ui/music',
      onDblClick: () => { void addAudioTrack(f.path); closeBrowser(); },
      actions: [
        smallBtn(playing ? 'ui/stop' : 'ui/play', t('plugin.studio.library.preview', null, 'Preview'), (e) => { e?.stopPropagation?.(); void toggleLibraryPreview(f.path); }),
        smallBtn('ui/plus', t('plugin.studio.library.add', null, 'Add to the arrangement at the play position'), (e) => { e?.stopPropagation?.(); void addAudioTrack(f.path); }),
      ],
    }));
  }
}

/* ── AI song generation (local model, background job) ─────────
   POST /api/songgen/generate starts a render on the user's machine and
   returns a job id; we poll /api/songgen/jobs/:id. Renders take minutes, so
   the panel can be closed — finished songs are added to the arrangement. */

let aiPanelEl = null;
let aiJobs = [];                 // { id, title, status, elapsed, result, error, match, added }
let aiPollTimer = 0;
let aiDraft = { style: '', lyrics: '', title: '', duration: 90, match: true };
let aiNotice = null;             // { kind: 'download' | 'engine' | 'error', text, quality, size }

function openAiPanel() {
  if (!aiPanelEl) return;
  aiPanelEl.classList.remove('hidden');
  renderAiPanel();
  aiPanelEl.querySelector('textarea')?.focus();
}
function closeAiPanel() {
  aiPanelEl?.classList.add('hidden');
}
function aiField(label, control, hint) {
  const f = h('label', 'studio-ai-field');
  f.appendChild(h('span', 'studio-ai-label', label));
  f.appendChild(control);
  if (hint) f.appendChild(h('span', 'studio-ai-hint', hint));
  return f;
}
function renderAiPanel() {
  if (!aiPanelEl || aiPanelEl.classList.contains('hidden')) return;
  aiPanelEl.textContent = '';
  const head = h('div', 'studio-browser-head');
  head.appendChild(eyebrow(t('plugin.studio.ai.title', null, 'AI song')));
  const close = iconCtl('ui/close', 'studio-head-btn', t('plugin.studio.common.close', null, 'Close'));
  close.addEventListener('click', closeAiPanel);
  head.appendChild(close);
  aiPanelEl.appendChild(head);
  aiPanelEl.appendChild(h('p', 'studio-ai-intro', t('plugin.studio.ai.intro', null, 'Describe a song and the local music model writes it — vocals included. It runs on this computer and takes several minutes; the result lands on a new audio track.')));

  const style = document.createElement('textarea');
  style.className = 'studio-ai-text';
  style.rows = 2;
  style.placeholder = t('plugin.studio.ai.stylePlaceholder', null, 'e.g. dreamy synth-pop, female vocals, 100 BPM, warm analog pads');
  style.value = aiDraft.style;
  style.addEventListener('input', () => { aiDraft.style = style.value; });
  aiPanelEl.appendChild(aiField(t('plugin.studio.ai.style', null, 'Style'), style));

  const lyrics = document.createElement('textarea');
  lyrics.className = 'studio-ai-text studio-ai-text--lyrics';
  lyrics.rows = 6;
  lyrics.placeholder = '[Verse]\n…\n[Chorus]\n…';
  lyrics.value = aiDraft.lyrics;
  lyrics.addEventListener('input', () => { aiDraft.lyrics = lyrics.value; });
  aiPanelEl.appendChild(aiField(t('plugin.studio.ai.lyrics', null, 'Lyrics (optional)'), lyrics, t('plugin.studio.ai.lyricsHint', null, 'Mark sections with [Verse], [Chorus], [Bridge]… Leave empty for an instrumental feel.')));

  const row = h('div', 'studio-ai-row');
  const title = uiInput();
  title.type = 'text';
  title.placeholder = t('plugin.studio.ai.titlePlaceholder', null, 'Title');
  title.value = aiDraft.title;
  title.addEventListener('input', () => { aiDraft.title = title.value; });
  row.appendChild(aiField(t('plugin.studio.ai.songTitle', null, 'Title'), title));
  const dur = uiInput();
  dur.type = 'number';
  dur.min = 30;
  dur.max = 300;
  dur.step = 10;
  dur.value = String(aiDraft.duration);
  dur.addEventListener('change', () => { aiDraft.duration = clamp(parseInt(dur.value, 10) || 90, 30, 300); dur.value = String(aiDraft.duration); });
  row.appendChild(aiField(t('plugin.studio.ai.duration', null, 'Length (s)'), dur));
  aiPanelEl.appendChild(row);

  const match = h('label', 'studio-ai-check');
  const cb = document.createElement('input');
  cb.type = 'checkbox';
  cb.checked = aiDraft.match;
  cb.addEventListener('change', () => { aiDraft.match = cb.checked; });
  match.append(cb, h('span', null, t('plugin.studio.ai.matchTempo', null, 'Set the project tempo to the song’s tempo')));
  aiPanelEl.appendChild(match);

  if (aiNotice) {
    const n = h('div', `studio-ai-notice studio-ai-notice--${aiNotice.kind}`);
    n.appendChild(h('span', null, aiNotice.text));
    if (aiNotice.kind === 'download') {
      n.appendChild(button({ variant: 'primary', label: t('plugin.studio.ai.downloadModels', { size: aiNotice.size }, `Download (${aiNotice.size})`), onClick: () => void startModelDownload(aiNotice.quality) }));
    }
    aiPanelEl.appendChild(n);
  }

  const go = button({ variant: 'primary', label: t('plugin.studio.ai.generate', null, 'Generate'), onClick: () => void startSongJob() });
  go.classList.add('studio-ai-go');
  aiPanelEl.appendChild(go);

  if (aiJobs.length) {
    const list = h('div', 'studio-ai-jobs');
    for (const j of aiJobs.slice().reverse()) {
      const r = h('div', `studio-ai-job studio-ai-job--${j.status}`);
      r.appendChild(h('span', 'studio-ai-job-title', j.title));
      const mins = Math.floor((j.elapsed || 0) / 60);
      const secs = String((j.elapsed || 0) % 60).padStart(2, '0');
      const label = j.status === 'running' ? t('plugin.studio.ai.running', { time: `${mins}:${secs}` }, `Composing… ${mins}:${secs}`)
        : j.status === 'done' ? (j.added ? t('plugin.studio.ai.added', null, 'Added to the arrangement') : t('plugin.studio.ai.done', null, 'Ready'))
          : (j.error || t('plugin.studio.ai.failed', null, 'Failed'));
      r.appendChild(h('span', 'studio-ai-job-status', label));
      if (j.status === 'done' && j.result?.path && !j.added) {
        r.appendChild(smallBtn('ui/plus', t('plugin.studio.library.add', null, 'Add to the arrangement at the play position'), () => { j.added = true; void addAudioTrack(j.result.path, { name: j.title, matchTempo: j.match }); renderAiPanel(); }));
      }
      list.appendChild(r);
    }
    aiPanelEl.appendChild(list);
  }
}

async function startSongJob() {
  const style = aiDraft.style.trim();
  if (!style) { toast(t('plugin.studio.ai.needStyle', null, 'Describe the style first'), { type: 'error' }); return; }
  aiNotice = null;
  try {
    const res = await apiFetch('/api/songgen/generate', { method: 'POST', body: JSON.stringify({
      style, lyrics: aiDraft.lyrics, title: aiDraft.title.trim() || undefined, duration: aiDraft.duration,
    }) });
    if (res?.success === false) {
      const d = res.data || {};
      if (d.needs_download) aiNotice = { kind: 'download', quality: d.quality, size: d.download_size, text: t('plugin.studio.ai.needsDownload', { size: d.download_size }, `The music model is not downloaded yet (${d.download_size}).`) };
      else if (d.engine_missing) aiNotice = { kind: 'engine', text: t('plugin.studio.ai.engineMissing', null, 'The local song engine is not installed on this computer (see Settings → Music).') };
      else aiNotice = { kind: 'error', text: res.error || t('plugin.studio.ai.failed', null, 'Failed') };
      renderAiPanel();
      return;
    }
    const d = res?.data || res;
    aiJobs.push({ id: d.job_id, title: d.title || aiDraft.title || style, status: 'running', elapsed: 0, match: aiDraft.match });
    startAiPolling();
    renderAiPanel();
    updateAiBadge();
  } catch (e) {
    aiNotice = { kind: 'error', text: e.message || String(e) };
    renderAiPanel();
  }
}
async function startModelDownload(quality) {
  try {
    await apiFetch('/api/songgen/download', { method: 'POST', body: JSON.stringify({ quality: quality || 'q4' }) });
    aiNotice = { kind: 'engine', text: t('plugin.studio.ai.downloading', null, 'Downloading the music model… you can keep working; press Generate again once it finishes.') };
    renderAiPanel();
    const poll = setInterval(async () => {
      try {
        const st = await apiFetch('/api/songgen/status');
        const downloads = st?.data?.downloads || {};
        const active = Object.values(downloads).some((d) => d?.status === 'downloading' || d?.status === 'running');
        if (!active) {
          clearInterval(poll);
          aiNotice = { kind: 'engine', text: t('plugin.studio.ai.downloaded', null, 'Download finished — press Generate.') };
          renderAiPanel();
        }
      } catch (_) { clearInterval(poll); }
    }, 5000);
  } catch (e) {
    aiNotice = { kind: 'error', text: e.message || String(e) };
    renderAiPanel();
  }
}
function startAiPolling() {
  if (aiPollTimer) return;
  aiPollTimer = setInterval(async () => {
    const running = aiJobs.filter((j) => j.status === 'running');
    if (!running.length) { clearInterval(aiPollTimer); aiPollTimer = 0; updateAiBadge(); return; }
    for (const j of running) {
      try {
        const res = await apiFetch(`/api/songgen/jobs/${encodeURIComponent(j.id)}`);
        const d = res?.data || {};
        j.elapsed = d.elapsed_secs || j.elapsed;
        if (d.status === 'done') {
          j.status = 'done';
          j.result = d.result;
          libraryFiles = null;
          if (tileEl && arrangement && d.result?.path) {
            j.added = true;
            await addAudioTrack(d.result.path, { name: j.title, matchTempo: j.match });
          }
          toast(t('plugin.studio.ai.ready', { name: j.title }, `“${j.title}” is ready`), { type: 'success' });
        } else if (d.status === 'failed') {
          j.status = 'failed';
          j.error = d.error || t('plugin.studio.ai.failed', null, 'Failed');
        }
      } catch (e) {
        if (e.status === 404) { j.status = 'failed'; j.error = t('plugin.studio.ai.lost', null, 'The song was lost because the system restarted'); }
      }
    }
    renderAiPanel();
    updateAiBadge();
  }, 3000);
}
let aiBtnEl = null;
function updateAiBadge() {
  if (!aiBtnEl) return;
  const n = aiJobs.filter((j) => j.status === 'running').length;
  aiBtnEl.classList.toggle('studio-transport--on', n > 0);
  aiBtnEl.dataset.badge = n ? String(n) : '';
}

let browserReveal = false;
function renderBrowser() {
  if (!browserListEl || browserEl.classList.contains('hidden')) return;
  renderBrowserList();
  if (browserReveal) {
    browserReveal = false;
    void listReveal(browserListEl.children);
  }
}
function renderBrowserList() {
  /* tab buttons */
  browserEl.querySelectorAll('.studio-browser-tab').forEach((b) => {
    const on = b.dataset.tab === browserTab;
    b.classList.toggle('studio-btn--on', on);
    b.setAttribute('aria-selected', String(on));
  });
  browserListEl.textContent = '';
  const q = browserQuery.trim().toLowerCase();
  const match = (s) => !q || String(s ?? '').toLowerCase().includes(q);
  if (!q) { renderBrowserSection(match, false); return; }
  // A search looks everywhere, grouped by type.
  const saved = browserTab;
  let any = false;
  for (const [tab, label] of BROWSER_TABS) {
    browserTab = tab;
    const header = h('div', 'studio-browser-group', t(label));
    browserListEl.appendChild(header);
    const before = browserListEl.childElementCount;
    renderBrowserSection(match, true);
    if (browserListEl.childElementCount === before) header.remove(); else any = true;
  }
  browserTab = saved;
  if (!any) browserListEl.appendChild(h('div', 'studio-empty', t('plugin.studio.browser.noResults', null, 'Nothing matches your search.')));
}
function renderBrowserSection(match, searching) {

  if (browserTab === 'audio') { renderLibraryList(match, searching); return; }

  if (browserTab === 'patterns') {
    if (!tracks.length) { browserListEl.appendChild(h('div', 'studio-empty', t('plugin.studio.empty.noPatterns', null, 'No saved patterns yet — save one from the Editor toolbar.'))); return; }
    for (const tr of tracks) {
      if (!match(tr.title)) continue;
      browserListEl.appendChild(browserRow({
        label: tr.title,
        sub: `${Math.round(tr.bpm)} BPM`,
        onClick: () => void playSaved(tr.track_id),
        onDblClick: () => addSavedAsLauncherClip(tr.track_id),
        actions: [
          smallBtn('ui/plus', t('plugin.studio.browser.addToTrack', null, 'Add to selected track'), () => void addSavedAsClip(tr.track_id)),
          smallBtn('ui/trash', t('plugin.studio.browser.deletePattern', null, 'Delete pattern'), async () => {
            if (await confirmDialog(t('plugin.studio.dialog.delete', { name: tr.title }, `Delete "${tr.title}"?`))) await deleteTrack(tr.track_id).catch((e) => toast(e.message, { type: 'error' }));
          }),
        ],
      }));
    }
    return;
  }

  if (browserTab === 'arrangements') {
    if (!searching) {
      browserListEl.appendChild(browserRow({
        label: t('plugin.studio.project.new', null, 'New project'),
        lead: 'ui/plus',
        onClick: () => { closeBrowser(); void newProject(); },
      }));
    }
    if (!arrangements.length && !searching) {
      browserListEl.appendChild(h('div', 'studio-empty', t('plugin.studio.project.emptyHelp', null, 'No saved projects yet. Projects are saved in the Studio (this Projects tab) with Save; exported audio goes to Files → Music/Studio.')));
    }
    for (const a of arrangements) {
      if (!match(a.title)) continue;
      const when = a.updated_at ? String(a.updated_at).replace('T', ' ').slice(0, 16) : '';
      const open = arrangement?.id === a.id;
      const row = browserRow({
        label: a.title,
        sub: `${Math.round(a.bpm)} BPM · ${tp('plugin.studio.project.tracks', a.tracks || 0, { count: a.tracks || 0 }, `${a.tracks || 0} tracks`)}${when ? ` · ${when}` : ''}${open ? ` · ${t('plugin.studio.project.open', null, 'open')}` : ''}`,
        lead: 'ui/folder',
        onClick: () => { closeBrowser(); void openProject(a.id); },
        actions: [
          smallBtn('ui/pencil', t('plugin.studio.project.rename', null, 'Rename'), () => {
            const label = row.querySelector('.studio-browser-label');
            if (label) inlineRename(label, a.title, (v) => void renameProject(a, v));
          }),
          smallBtn('ui/copy', t('plugin.studio.project.duplicate', null, 'Duplicate'), () => void duplicateProject(a)),
          smallBtn('ui/trash', t('plugin.studio.browser.deleteArrangement', null, 'Delete arrangement'), async () => {
            if (await confirmDialog(t('plugin.studio.dialog.delete', { name: a.title }, `Delete "${a.title}"?`))) await deleteArrangement(a.id).catch((e) => toast(e.message, { type: 'error' }));
          }),
        ],
      });
      row.classList.toggle('studio-browser-item--current', open);
      browserListEl.appendChild(row);
    }
    return;
  }

  if (browserTab === 'instruments') {
    const v = current?.voices[selectedVoice];
    for (let i = 0; i < KINDS.length; i++) {
      const k = KINDS[i];
      if (!match(KIND_LABELS[k]) && !match(t(KIND_DESC[k]))) continue;
      browserListEl.appendChild(browserRow({
        label: KIND_LABELS[k],
        sub: t(KIND_DESC[k]),
        color: trackColor(i),
        onClick: () => {
          if (browserMode?.type === 'addTrack') {
            if (browserMode.area === 'lch') addLauncherTrackWithKind(k);
            else addArrTrack(k);
            closeBrowser();
          } else if (browserMode?.type === 'addVoice' && current) {
            addVoiceWithKind(k);
            closeBrowser();
          } else if (v) {
            v.kind = k; v.synth = {};
            markDirty();
            renderEditor();
            renderDevices();
          }
        },
        onDblClick: () => {
          if (current && current.voices.length < 12) addVoiceWithKind(k);
        },
      }));
    }
    /* Saved SynthMe instruments */
    const savedSynthme = presets.filter((p) => p.kind === 'synthme');
    if (savedSynthme.length) {
      browserListEl.appendChild(h('div', 'studio-browser-section', t('plugin.studio.browser.synthmeSection', null, 'SynthMe instruments')));
      for (const p of savedSynthme) {
        if (!match(p.name)) continue;
        browserListEl.appendChild(browserRow({
          label: p.name,
          sub: t('plugin.studio.browser.customInstrument', null, 'custom instrument'),
          color: trackColor(KINDS.indexOf('synthme')),
          onClick: () => {
            if (browserMode?.type === 'addTrack') {
              const tr = { id: `t${Date.now()}`, name: p.name, color: arrangement.tracks.length % TRACK_COLORS.length, mute: false, level: 0.6, pan: 0, automation: { lanes: [] } };
              arrangement.tracks.push(tr);
              arrangement.clips.push({ track: tr.id, start: 0, pattern: synthmePresetPattern(p) });
              markDirty(); renderArranger(); closeBrowser();
            } else if (browserMode?.type === 'addVoice' && current) {
              current.voices.push(synthmePresetVoice(p));
              selectedVoice = current.voices.length - 1;
              markDirty(); renderEditor(); renderDevices(); closeBrowser();
            } else if (v) {
              v.kind = 'synthme';
              v.synth = {};
              applyPreset(v, p);
              renderEditor(); renderDevices();
            }
          },
        }));
      }
    }
    /* Saved Grid patches */
    const savedGrid = presets.filter((p) => p.kind === 'grid');
    if (savedGrid.length) {
      browserListEl.appendChild(h('div', 'studio-browser-section', t('plugin.studio.browser.gridSection', null, 'WaveMe patches')));
      for (const p of savedGrid) {
        if (!match(p.name)) continue;
        browserListEl.appendChild(browserRow({
          label: p.name,
          sub: t('plugin.studio.browser.modularPatch', null, 'modular patch'),
          color: trackColor(KINDS.indexOf('grid')),
          onClick: () => {
            const pr = p.params || {};
            if (browserMode?.type === 'addTrack') {
              const tr = { id: `t${Date.now()}`, name: p.name, color: arrangement.tracks.length % TRACK_COLORS.length, mute: false, level: 0.6, pan: 0, automation: { lanes: [] } };
              arrangement.tracks.push(tr);
              const pat = kitPattern(16);
              pat.title = p.name || 'Grid';
              pat.voices = [gridPresetVoice(p)];
              arrangement.clips.push({ track: tr.id, start: 0, pattern: pat });
              markDirty(); renderArranger(); closeBrowser();
            } else if (browserMode?.type === 'addVoice' && current) {
              current.voices.push(gridPresetVoice(p));
              selectedVoice = current.voices.length - 1;
              markDirty(); renderEditor(); renderDevices(); closeBrowser();
            } else if (v) {
              v.kind = 'grid';
              v.grid = (pr.grid) ? { modules: (pr.grid.modules || []).map((m) => ({ id: m.id, kind: m.kind, params: { ...(m.params || {}) } })), cables: (pr.grid.cables || []).map((c) => ({ from: [c.from[0], c.from[1]], to: [c.to[0], c.to[1]] })) } : null;
              v.synth = {};
              renderEditor(); renderDevices();
            }
          },
        }));
      }
    }
    return;
  }

  if (browserTab === 'effects') {
    const v = current?.voices[selectedVoice];
    for (const k of EFFECT_KINDS) {
      if (!match(EFFECTS[k].label)) continue;
      browserListEl.appendChild(browserRow({
        label: EFFECTS[k].label,
        sub: t('plugin.studio.browser.audioEffect', null, 'audio effect'),
        onClick: () => {
          if (!v) { toast(t('plugin.studio.toast.selectClipFirst', null, 'Select a clip first'), { type: 'error' }); return; }
          if (v.fx.length >= 8) { toast(t('plugin.studio.toast.maxFx8', null, 'Max 8 effects per voice'), { type: 'error' }); return; }
          v.fx.push({ kind: k, params: {}, bypass: false });
          markDirty();
          if (detailPage === 'devices') renderDevices();
        },
      }));
    }
    return;
  }

  if (browserTab === 'presets') {
    const v = current?.voices[selectedVoice];
    if (!v) { browserListEl.appendChild(h('div', 'studio-empty', t('plugin.studio.empty.selectClipFirst', null, 'Select a clip first.'))); return; }
    const factory = FACTORY_PRESETS[v.kind] || [];
    for (const p of factory) {
      if (!match(p.name)) continue;
      browserListEl.appendChild(browserRow({ label: p.name, sub: t('plugin.studio.browser.factoryPreset', null, 'factory'), color: trackColor(selectedVoice), onClick: () => applyPreset(v, p) }));
    }
    for (const p of presets) {
      if (p.kind !== v.kind || !match(p.name)) continue;
      browserListEl.appendChild(browserRow({
        label: p.name,
        sub: t('plugin.studio.browser.savedPreset', null, 'saved'),
        onClick: () => applyPreset(v, p),
        actions: [smallBtn('ui/trash', t('plugin.studio.browser.deletePreset', null, 'Delete preset'), async () => {
          await api(`/api/studio/presets/${p.id}`, { method: 'DELETE' }).catch((e) => toast(e.message, { type: 'error' }));
          await refreshPresets();
          renderBrowser();
        })],
      }));
    }
    if (!factory.length && !presets.some((p) => p.kind === v.kind)) {
      browserListEl.appendChild(h('div', 'studio-empty', t('plugin.studio.empty.noPresets', { instrument: KIND_LABELS[v.kind] }, `No presets for ${KIND_LABELS[v.kind]} — save one from the device header.`)));
    }
  }
}

async function fetchTrackConfig(id) {
  const data = await api(`/api/studio/${id}`);
  return data?.config || null;
}
async function addSavedAsClip(trackId) {
  const cfg = await fetchTrackConfig(trackId).catch(() => null);
  if (!cfg) { toast(t('plugin.studio.toast.patternNotFound', null, 'Pattern not found'), { type: 'error' }); return; }
  if (panels.launcher && !panels.arranger) {
    addConfigAsLauncherClip(cfg);
    return;
  }
  const tr = arrangement.tracks[0];
  if (!tr) { toast(t('plugin.studio.toast.noArrangementTrack', null, 'No arrangement track'), { type: 'error' }); return; }
  const dur = Math.max(1, Math.round((cfg.steps || 16) / 4));
  const start = arrangement.clips.filter((c) => c.track === tr.id).length * 4;
  const maxStart = Math.max(0, Math.round(arrangement.length_beats) - dur);
  arrangement.clips.push({ track: tr.id, start: Math.min(start, maxStart), pattern: normalizePattern(cfg) });
  markDirty();
  renderArranger();
}
async function addSavedAsLauncherClip(trackId) {
  const cfg = await fetchTrackConfig(trackId).catch(() => null);
  if (!cfg) { toast(t('plugin.studio.toast.patternNotFound', null, 'Pattern not found'), { type: 'error' }); return; }
  addConfigAsLauncherClip(cfg);
}
/* ── AI track → Arranger (not the launcher) ─────────────────── */

function trackNameForCfg(cfg) {
  const title = (cfg.title || '').trim();
  if (title && title.toLowerCase() !== 'untitled') return title;
  const kinds = (cfg.voices || []).map((v) => v.kind).filter(Boolean);
  if (!kinds.length) return 'Track';
  const drumKinds = new Set(['kick', 'snare', 'hat', 'clap', 'tom', 'perc', 'drumkit']);
  if (kinds.every((k) => drumKinds.has(k))) return 'Drums';
  if (kinds.length === 1) return KIND_LABELS[kinds[0]] || kinds[0];
  return kinds.map((k) => KIND_LABELS[k] || k).join(' + ');
}

/** Add an AI-composed track as a NEW track + clip in the Arranger timeline. */
function addConfigAsArrangementTrack(cfg) {
  const i = arrangement.tracks.length;
  const tr = {
    id: `t${Date.now()}`,
    name: trackNameForCfg(cfg),
    color: i % TRACK_COLORS.length,
    mute: false,
    level: 0.8,
    pan: 0,
    automation: { lanes: [] },
  };
  arrangement.tracks.push(tr);
  arrangement.clips.push({ track: tr.id, start: 0, pattern: normalizePattern(cfg) });
  markDirty();
  if (!panels.arranger) { panels.arranger = true; panels.launcher = false; }
  sel = { area: 'arr', clipIndex: arrangement.clips.length - 1 };
  syncSelection();
  syncPanels();
}

/* Drop the AI's just-created track into the ARRANGER (and audition it).
   Safe to call before the tile is mounted — it parks the id until mount. */
async function consumePendingAiTrack() {
  const id = pendingAiTrackId;
  if (!id) return;
  if (!arrangement) { pendingAiTrackId = id; return; }  // not mounted yet
  pendingAiTrackId = null;
  if (lastConsumedAiTrackId === id) return;             // already handled
  lastConsumedAiTrackId = id;
  const cfg = await fetchTrackConfig(id).catch(() => null);
  if (!cfg) return;
  addConfigAsArrangementTrack(cfg);
  const blob = await apiFetch(`/api/studio/${id}/audio`, { responseType: 'blob' }).catch(() => null);
  if (blob) {
    const buf = await blob.arrayBuffer();
    const ctx = audioCtxOr();
    if (ctx.state === 'suspended') await ctx.resume();
    const decoded = await ctx.decodeAudioData(buf);
    const src = ctx.createBufferSource();
    src.buffer = decoded;
    src.connect(masterOut());
    src.start();
  }
}
function addConfigAsLauncherClip(cfg) {
  const tr = launcher.tracks[0];
  if (!tr) return;
  let slot = 0;
  while (launcher.clips[`${tr.id}:${slot}`]) slot++;
  if (slot >= launcher.scenes) launcher.scenes = slot + 1;
  launcher.clips[`${tr.id}:${slot}`] = { pattern: normalizePattern(cfg), title: cfg.title || tr.name };
  renderLauncher();
  selectLauncherClip(tr.id, slot);
  if (!panels.launcher) { panels.launcher = true; syncPanels(); }
}

/* ── wav export ─────────────────────────────────────────────── */

/* Export: master the arrangement on the server into Files → Music/Studio. */
async function exportWav() {
  if (busy) return;
  if (!panels.arranger && current) { await downloadWav(); return; }
  if (!arrangement?.clips?.length) {
    toast(t('plugin.studio.toast.needTrackAndClip', null, 'Add a track and a clip to the Arranger first (or use the Launcher)'), { type: 'error' });
    return;
  }
  busy = true;
  setStatus('Rendering…');
  try {
    const data = await api('/api/studio/export', { method: 'POST', body: JSON.stringify({ arrangement: arrangementPayload(), name: arrangement.title }) });
    const lufs = Number.isFinite(data?.lufs) ? data.lufs.toFixed(1) : '—';
    toast(t('plugin.studio.toast.exported', { path: data?.path, lufs }, `Exported to ${data?.path} · ${lufs} LUFS`), { type: 'success' });
    libraryFiles = null;
    setStatus(dirty ? 'dirty' : 'saved');
  } catch (e) {
    console.error('studio: export failed', e);
    toast(e.message || t('plugin.studio.toast.saveFailed', null, 'Save failed'), { type: 'error' });
    setStatus('error');
  } finally {
    busy = false;
  }
}
/* Render and hand the WAV to the browser / Files picker (arrangement or the
   pattern open in the editor). */
async function downloadWav() {
  if (busy) return;
  busy = true;
  setStatus('Rendering…');
  try {
    let blob;
    let name;
    if (panels.arranger || !current) {
      blob = await apiFetch('/api/studio/arrangement/render', { method: 'POST', body: JSON.stringify(arrangementRenderPayload()), responseType: 'blob' });
      name = arrangement.title || 'arrangement';
    } else {
      blob = await apiFetch('/api/studio/preview', { method: 'POST', body: JSON.stringify(serializePattern(current)), responseType: 'blob' });
      name = current.title || 'clip';
    }
    const filename = `${String(name).replace(/[^\w\- ]+/g, '').trim() || 'studio'}.wav`;
    await saveOrDownload(blob, { name: filename, dir: 'Music', app: 'Studio' });
    setStatus(dirty ? 'dirty' : 'saved');
  } catch (e) {
    console.error('studio: download failed', e);
    toast(e.message || t('plugin.studio.toast.saveFailed', null, 'Save failed'), { type: 'error' });
    setStatus('error');
  } finally {
    busy = false;
  }
}

/* Duplicate the selected arrangement clip right after itself. */
function duplicateSelectedClip() {
  const c = selectedArrClip();
  if (!c) return;
  const copy = JSON.parse(JSON.stringify(serializeClip(c)));
  const clip = normalizeArrangement({ tracks: [], clips: [copy] }).clips[0];
  clip.start = c.start + clipDurBeats(c);
  arrangement.clips.push(clip);
  fitArrangementLength();
  selectArrClip(arrangement.clips.length - 1);
  markDirty();
}
function selectedTrack() {
  const c = selectedArrClip();
  return c ? arrangement.tracks.find((x) => x.id === c.track) || null : null;
}

/* ── keyboard shortcuts (Ableton/Bitwig-style) ─────────────── */

function isTypingTarget(t) {
  return !!t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable);
}
function saveProjectNow() {
  void saveArrangement()
    .then(() => { setStatus('saved'); toast(t('plugin.studio.toast.projectSaved', null, 'Project saved'), { type: 'success' }); })
    .catch((e) => toast(e.message, { type: 'error' }));
}
function onStudioKeydown(e) {
  if (!tileEl || !tileEl.isConnected) return;
  /* Shortcuts belong to the Studio window: skip when typing anywhere. */
  if (isTypingTarget(e.target)) return;
  const ae = document.activeElement;
  if (ae && ae !== document.body && !tileEl.contains(ae)) return;
  const meta = e.metaKey || e.ctrlKey;
  if (e.code === 'Space' && !meta) {
    e.preventDefault();
    if (arrPlaying) stopPlayback();
    else if (anyLauncherPlaying()) stopAllLauncher();
    else if (panels.launcher && !panels.arranger) launchScene(0);
    else void renderArrangementAndPlay();
  } else if (meta && (e.key === 's' || e.key === 'S')) {
    e.preventDefault();
    saveProjectNow();
  } else if (meta && e.shiftKey && (e.key === 'e' || e.key === 'E')) {
    e.preventDefault();
    void exportWav();
  } else if (meta && (e.key === 'e' || e.key === 'E')) {
    e.preventDefault();
    splitSelectedAtPlayhead();
  } else if (meta && (e.key === 'd' || e.key === 'D')) {
    e.preventDefault();
    duplicateSelectedClip();
  } else if (!meta && e.key === 'Enter') {
    e.preventDefault();
    const was = arrPlaying;
    if (was) stopPlayback();
    arrStartBeat = 0;
    renderArranger();
    clearArrPlayhead();
    if (was) void renderArrangementAndPlay(0);
  } else if (!meta && !e.altKey && (e.key === 'z' || e.key === 'Z')) {
    e.preventDefault();
    zoomToFit();
  } else if (!meta && !e.altKey && (e.key === 'l' || e.key === 'L')) {
    e.preventDefault();
    loopBtn?.click();
  } else if (!meta && !e.altKey && (e.key === 'm' || e.key === 'M') && selectedTrack()) {
    e.preventDefault();
    const tr = selectedTrack();
    tr.mute = !tr.mute;
    markDirty();
    renderArranger();
    if (detailPage === 'mixer') renderMixer();
  } else if (!meta && !e.altKey && (e.key === 's' || e.key === 'S') && selectedTrack()) {
    e.preventDefault();
    toggleSolo(selectedTrack().id);
    renderArranger();
    if (detailPage === 'mixer') renderMixer();
  } else if ((e.key === 'Delete' || e.key === 'Backspace') && sel) {
    e.preventDefault();
    if (sel.area === 'arr') {
      removeArrClip(sel.clipIndex);
    } else if (sel.area === 'lch') {
      const key = `${sel.trackId}:${sel.scene}`;
      if (launcher.clips[key]) {
        if (trackLoops[sel.trackId]?.slotKey === key) stopLauncherTrack(sel.trackId, false);
        delete launcher.clips[key];
        sel = null;
        syncSelection();
        renderLauncher();
      }
    }
  }
}

/* ── panel toggling ─────────────────────────────────────────── */

function syncPanels() {
  const builder = panels.synthme || panels.grid;
  arrWrapEl?.classList.toggle('hidden', builder || !panels.arranger);
  lchWrapEl?.classList.toggle('hidden', builder || !panels.launcher);
  synthmeWrapEl?.classList.toggle('hidden', !panels.synthme);
  gridWrapEl?.classList.toggle('hidden', !panels.grid);
  viewSeg?.setValue(currentView(), { silent: true });
  if (titleInput) titleInput.value = arrangement.title;
  if (bpmInput) bpmInput.value = String(Math.round(arrangement.bpm));
  renderArranger();
  renderLauncher();
  if (detailPage === 'mixer') renderMixer();
}
/** The visible body view, as the header's segmented control names it. */
function currentView() {
  if (panels.synthme) return 'synthme';
  if (panels.grid) return 'grid';
  if (panels.arranger && panels.launcher) return 'split';
  return panels.launcher ? 'launcher' : 'arranger';
}
/** Show one view from the segmented control. */
function setView(view) {
  panels.synthme = view === 'synthme';
  panels.grid = view === 'grid';
  panels.arranger = view === 'arranger' || view === 'split';
  panels.launcher = view === 'launcher' || view === 'split';
  syncPanels();
  if (panels.synthme) renderSynthme();
  if (panels.grid) renderGrid();
}
function togglePanel(which) {
  if (which === 'synthme' || which === 'grid') {
    const open = !panels[which];
    panels.synthme = false;
    panels.grid = false;
    if (open) {
      panels[which] = true;
      panels.arranger = false;
      panels.launcher = false;
    } else {
      panels.arranger = true;
    }
  } else {
    if (panels.synthme || panels.grid) {
      panels.synthme = false;
      panels.grid = false;
      panels.arranger = which === 'arranger';
      panels.launcher = which === 'launcher';
    } else if (which === 'arranger') {
      if (panels.arranger && !panels.launcher) return;
      panels.arranger = !panels.arranger;
    } else {
      if (panels.launcher && !panels.arranger) return;
      panels.launcher = !panels.launcher;
    }
  }
  syncPanels();
  if (panels.synthme) renderSynthme();
  if (panels.grid) renderGrid();
}

/* ── mount / unmount ────────────────────────────────────────── */

export function mountStudioTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile studio-tile';
  tileEl.dataset.plugin = STUDIO_PLUGIN;

  /* ── header / transport ── */
  barEl = toolbar([], { className: 'studio-bar' });
  barEl.dataset.windowBar = '';

  // The views — Arranger, Launcher, both side by side, SynthMe, WaveMe — are
  // one segmented control (icons; the names are the accessible labels).
  const VIEWS = [
    ['arranger', 'ui/arranger', 'plugin.studio.toolbar.arrangerLabel', 'Arranger timeline', 'plugin.studio.toolbar.arrangerLabel', 'Arranger timeline'],
    ['launcher', 'ui/launcher', 'plugin.studio.toolbar.launcherLabel', 'Clip Launcher', 'plugin.studio.toolbar.launcherLabel', 'Clip Launcher'],
    ['split', 'ui/layers', 'plugin.studio.toolbar.splitLabel', 'Arranger and Launcher', 'plugin.studio.toolbar.splitHint', 'Arranger and Clip Launcher side by side'],
    ['synthme', 'ui/synthme', 'plugin.studio.toolbar.synthmeLabel', 'SynthMe instrument builder', 'plugin.studio.toolbar.synthmeHint', 'SynthMe — build and save custom instruments'],
    ['grid', 'ui/grid', 'plugin.studio.toolbar.gridLabel', 'WaveMe modular patch editor', 'plugin.studio.toolbar.gridHint', 'WaveMe — modular patch editor'],
  ];
  viewSeg = segmented({
    label: t('plugin.studio.toolbar.views', null, 'Views'),
    options: VIEWS.map(([value, , key, fallback]) => ({ value, label: t(key, null, fallback) })),
    value: currentView(),
    onChange: setView,
  });
  viewSeg.classList.add('studio-seg', 'studio-views');
  // Narrow windows and phones fold the view switch whole into More.
  viewSeg.dataset.foldable = '';
  viewSeg.querySelectorAll('button').forEach((b, i) => {
    const [, iconName, key, fallback, hintKey, hint] = VIEWS[i];
    b.replaceChildren(icon(iconName, { size: 15 }));
    b.setAttribute('aria-label', t(key, null, fallback));
    b.title = t(hintKey, null, hint);
  });

  const stopBtn = button({ variant: 'ghost', icon: 'ui/stop', label: '', onClick: () => { stopPlayback(); stopAllLauncher(); } });
  stopBtn.classList.add('studio-transport');
  stopBtn.title = t('plugin.studio.toolbar.stop', null, 'Stop');
  stopBtn.setAttribute('aria-label', t('plugin.studio.toolbar.stop', null, 'Stop'));

  playBtn = button({ variant: 'ghost', icon: 'ui/play', label: '', onClick: () => {
    if (arrPlaying) { stopPlayback(); return; }
    if (panels.arranger) void renderArrangementAndPlay();
    else launchScene(0);
  } });
  playBtn.classList.add('studio-transport', 'studio-transport--play');
  // Play (which also stops) stays on a compact bar; the rest may fold.
  playBtn.dataset.keep = '';
  playBtn.title = t('plugin.studio.toolbar.playHint', null, 'Play (Arranger) / Launch scene 1 (Launcher)');
  playBtn.setAttribute('aria-label', t('plugin.studio.toolbar.play', null, 'Play'));

  loopBtn = button({ variant: 'ghost', icon: 'ui/loop', label: '', onClick: () => {
    arrLoop = !arrLoop;
    if (arrPlaying) { const beat = currentBeat(); stopSources(); if (mixManifest) beginTransport(mixManifest, beat); }
    updateTransport();
    renderArranger();
  } });
  loopBtn.classList.add('studio-transport');
  loopBtn.title = t('plugin.studio.toolbar.loop', null, 'Loop arrangement');
  loopBtn.setAttribute('aria-label', t('plugin.studio.toolbar.loop', null, 'Loop arrangement'));

  metroBtn = button({ variant: 'ghost', icon: 'ui/metronome', label: '', onClick: () => {
    metroOn = !metroOn;
    if (!metroOn) metroStop();
    else if (arrPlaying) metroStart(arrPlayStart, arrangement.bpm, arrStartBeat);
    else if (clockRunning) metroStart(clockT0, launcher.bpm);
    updateTransport();
  } });
  metroBtn.classList.add('studio-transport');
  metroBtn.title = t('plugin.studio.toolbar.metronome', null, 'Metronome');
  metroBtn.setAttribute('aria-label', t('plugin.studio.toolbar.metronome', null, 'Metronome'));

  timeEl = h('span', 'studio-time');
  beatDotEl = h('span', 'studio-beat');
  beatDotEl.title = t('plugin.studio.toolbar.beatIndicator', null, 'Beat indicator');

  bpmInput = uiInput();
  bpmInput.type = 'number';
  bpmInput.classList.add(...('studio-bpm').split(/\s+/));
  bpmInput.min = 40;
  bpmInput.max = 240;
  bpmInput.value = '120';
  bpmInput.title = 'BPM';
  withHint(bpmInput, t('plugin.studio.hint.tempo', null, 'Tempo'));
  bpmInput.addEventListener('change', () => {
    const b = Math.min(240, Math.max(40, parseFloat(bpmInput.value) || 120));
    bpmInput.value = String(b);
    // One project tempo: the Arranger and the Launcher must never drift apart.
    if (arrangement) arrangement.bpm = b;
    if (launcher) launcher.bpm = b;
    if (clockRunning && metroOn) { metroStop(); metroStart(clockT0, b); }
    markDirty();
    renderArranger();
  });
  const bpmLabel = h('span', 'studio-bpm-label', 'BPM');
  // The tempo and its unit fold together, as "120 BPM", in a narrow window.
  const bpmWrap = h('span', 'studio-bpm-wrap');
  bpmWrap.dataset.foldable = '';
  bpmWrap.append(bpmInput, bpmLabel);

  titleInput = uiInput();
  titleInput.type = 'text';
  titleInput.classList.add(...('studio-title').split(/\s+/));
  titleInput.placeholder = t('plugin.studio.toolbar.titlePlaceholder', null, 'Untitled');
  titleInput.maxLength = 120;
  titleInput.value = 'Untitled Arrangement';
  titleInput.title = t('plugin.studio.toolbar.projectTitle', null, 'Project title');
  titleInput.addEventListener('change', () => {
    if (panels.arranger) arrangement.title = titleInput.value.trim() || 'Untitled';
    else launcher.title = titleInput.value.trim() || 'Untitled';
    markDirty();
  });

  const exportBtn = button({ variant: 'ghost', icon: 'ui/download', label: '', onClick: () => void exportWav() });
  exportBtn.classList.add('studio-transport');
  exportBtn.title = t('plugin.studio.toolbar.exportMaster2', null, 'Export the master to Music/Studio (Ctrl+Shift+E)');
  exportBtn.setAttribute('aria-label', t('plugin.studio.toolbar.exportLabel', null, 'Export WAV to Music'));

  aiBtnEl = button({ variant: 'ghost', icon: 'ui/magic-wand', label: '', onClick: () => {
    if (aiPanelEl?.classList.contains('hidden')) openAiPanel(); else closeAiPanel();
  } });
  aiBtnEl.classList.add('studio-transport', 'studio-ai-btn');
  aiBtnEl.title = t('plugin.studio.toolbar.ai', null, 'AI song — generate a full song with the local music model');
  aiBtnEl.setAttribute('aria-label', t('plugin.studio.toolbar.aiLabel', null, 'AI song'));

  const mixBtn = button({ variant: 'ghost', icon: 'ui/channels', label: '', onClick: () => { detailOpen = true; detailPage = 'mixer'; renderDetail(); } });
  mixBtn.classList.add('studio-transport');
  mixBtn.title = t('plugin.studio.toolbar.mixer', null, 'Mixer');
  mixBtn.setAttribute('aria-label', t('plugin.studio.toolbar.mixer', null, 'Mixer'));

  const newBtn = button({ variant: 'ghost', icon: 'ui/file', label: '', onClick: () => void newProject() });
  newBtn.classList.add('studio-transport');
  newBtn.title = t('plugin.studio.project.new', null, 'New project');
  newBtn.setAttribute('aria-label', t('plugin.studio.project.new', null, 'New project'));
  const projBtn = button({ variant: 'ghost', icon: 'ui/folder-open', label: '', onClick: () => { void refreshArrangements().then(renderBrowser).catch(() => {}); openBrowser('arrangements'); } });
  projBtn.classList.add('studio-transport');
  projBtn.title = t('plugin.studio.project.openHint', null, 'Open a project (Browser → Projects)');
  projBtn.setAttribute('aria-label', t('plugin.studio.project.openLabel', null, 'Open project'));
  const saveAsBtn = button({ variant: 'ghost', icon: 'ui/copy', label: '', onClick: () => void saveProjectAs().catch((e) => toast(e.message, { type: 'error' })) });
  saveAsBtn.classList.add('studio-transport');
  saveAsBtn.title = t('plugin.studio.project.saveAs', null, 'Save as a new project');
  saveAsBtn.setAttribute('aria-label', t('plugin.studio.project.saveAs', null, 'Save as a new project'));
  const saveBtn = button({ variant: 'ghost', icon: 'ui/save', label: '', onClick: () => saveProjectNow() });
  saveBtn.classList.add('studio-transport');
  saveBtn.title = t('plugin.studio.toolbar.saveProject', null, 'Save project');
  saveBtn.setAttribute('aria-label', t('plugin.studio.toolbar.saveProject', null, 'Save project'));

  // The left-hand list opener (patterns / instruments / effects / presets),
  // shaped like Word/Calc's documents menu (icon + chevron).
  browserBtn = controlButton();
  browserBtn.type = 'button';
  browserBtn.className = 'studio-menu-btn';
  browserBtn.setAttribute('aria-haspopup', 'true');
  browserBtn.setAttribute('aria-expanded', 'false');
  browserBtn.setAttribute('aria-label', t('plugin.studio.toolbar.browser', null, 'Browser'));
  browserBtn.title = t('plugin.studio.toolbar.browserHint', null, 'Browser (patterns, instruments, effects, presets)');
  const browserIcon = h('span', 'studio-menu-btn-icon');
  browserBtn.appendChild(browserIcon);
  void setIcon(browserIcon, 'ui/search', { size: 15 });
  const browserChevron = h('span', 'studio-menu-btn-chevron');
  browserBtn.appendChild(browserChevron);
  void setIcon(browserChevron, 'ui/chevron-down', { size: 12 });
  browserBtn.addEventListener('click', toggleBrowser);

  statusEl = h('span', 'studio-status');
  saveDotEl = h('span', 'studio-save-dot');
  saveDotEl.setAttribute('aria-hidden', 'true');
  saveDotEl.title = t('plugin.studio.status.saved', null, 'Saved');

  // Single top bar (Word/Calc convention): left menu button, title (flex: 1),
  // then transport/view toggles, then file actions (export → save), then the
  // save dot. Every control keeps the shared 32px metrics.
  barEl.append(browserBtn, titleInput, viewSeg,
    h('span', 'studio-bar-sep'), stopBtn, playBtn, loopBtn, metroBtn,
    beatDotEl, timeEl, bpmWrap, h('span', 'studio-bar-sep'), aiBtnEl, mixBtn, exportBtn, h('span', 'studio-bar-sep'), newBtn, projBtn, saveBtn, saveAsBtn, statusEl, saveDotEl);
  tileEl.appendChild(barEl);

  /* ── body: arranger + launcher side by side ── */
  bodyEl = h('div', 'studio-body');

  arrWrapEl = h('div', 'studio-panel studio-panel--arr');
  const arrScroll = h('div', 'studio-arr-scroll');
  arrScrollEl = arrScroll;
  arrScroll.addEventListener('wheel', onArrWheel, { passive: false });
  arrScroll.addEventListener('scroll', onArrScroll, { passive: true });
  wireArrPinch(arrScroll);
  arrGridEl = h('div', 'studio-arr-grid');
  arrScroll.appendChild(arrGridEl);
  arrWrapEl.appendChild(arrScroll);
  bodyEl.appendChild(arrWrapEl);

  lchWrapEl = h('div', 'studio-panel studio-panel--lch hidden');
  const lchScroll = h('div', 'studio-arr-scroll');
  lchGridEl = h('div', 'studio-arr-grid');
  lchScroll.appendChild(lchGridEl);
  lchWrapEl.appendChild(lchScroll);
  bodyEl.appendChild(lchWrapEl);

  /* SynthMe builder — a full panel in the body (replaces arranger/launcher). */
  synthmeWrapEl = h('div', 'studio-panel studio-panel--synthme hidden');
  bodyEl.appendChild(synthmeWrapEl);

  /* The Grid — modular patch editor (full panel). */
  gridWrapEl = h('div', 'studio-panel studio-panel--synthme hidden');
  bodyEl.appendChild(gridWrapEl);

  tileEl.appendChild(bodyEl);

  /* ── detail panel ── */
  detailEl = h('div', 'studio-detail');
  const rail = h('div', 'studio-detail-rail');
  // Editor / Devices / Mixer: one segmented control with mono labels. A click
  // on the current page re-opens a collapsed panel.
  pageSeg = segmented({
    label: t('plugin.studio.panel.pages', null, 'Detail panel'),
    options: [['editor', 'plugin.studio.panel.editor'], ['devices', 'plugin.studio.panel.devices'], ['mixer', 'plugin.studio.panel.mixer']]
      .map(([value, key]) => ({ value, label: t(key) })),
    value: detailPage,
    onChange: (page) => { detailPage = page; detailOpen = true; renderDetail(); },
  });
  pageSeg.classList.add('studio-seg', 'studio-detail-pages');
  pageSeg.addEventListener('click', () => { if (!detailOpen) { detailOpen = true; renderDetail(); } });
  rail.appendChild(pageSeg);
  collapseBtn = iconCtl('ui/chevron-down', 'studio-head-btn studio-detail-collapse', t('plugin.studio.panel.collapse', null, 'Show/hide detail panel'));
  collapseBtn.addEventListener('click', () => { detailOpen = !detailOpen; renderDetail(); });
  rail.appendChild(collapseBtn);
  detailEl.appendChild(rail);

  detailBodyEl = h('div', 'studio-detail-body');
  const editorWrap = h('div', 'studio-editor-page');
  editorPageEl = editorWrap;
  edToolbarEl = toolbar([], { className: 'studio-editor-toolbar' });
  gridEl = h('div', 'studio-grid');
  pianoEl = h('div', 'studio-pianoroll-inline hidden');
  editorWrap.append(edToolbarEl, gridEl, pianoEl);
  devicesEl = h('div', 'studio-devices-page hidden');
  mixerEl = h('div', 'studio-mixer-page hidden');
  detailBodyEl.append(editorWrap, devicesEl, mixerEl);
  detailEl.appendChild(detailBodyEl);
  tileEl.appendChild(detailEl);

  /* ── footer: a slim, static hint + parameter readout line ── */
  const footer = h('div', 'studio-footer');
  footerHintEl = h('span', 'studio-footer-hint');
  footerParamEl = h('span', 'studio-footer-param');
  footer.append(footerHintEl, footerParamEl);
  tileEl.appendChild(footer);
  tileEl.addEventListener('contextmenu', () => { ctxClip = null; }, true);
  tileEl.addEventListener('mouseover', (e) => {
    const t = e.target.closest?.('[data-hint]');
    if (t) setReadout(t.dataset.hint);
  });
  tileEl.addEventListener('mouseout', (e) => {
    if (e.target.closest?.('[data-hint]')) setReadout('');
  });

  /* ── pop-up browser ── */
  browserEl = h('div', 'studio-browser hidden');
  const bHead = h('div', 'studio-browser-head');
  bHead.appendChild(eyebrow(t('plugin.studio.toolbar.browser', null, 'Browser')));
  const bClose = iconCtl('ui/close');
  bClose.type = 'button';
  bClose.title = t('plugin.studio.common.close', null, 'Close');
  bClose.addEventListener('click', closeBrowser);
  bHead.appendChild(bClose);
  browserEl.appendChild(bHead);
  const search = searchBar({ placeholder: t('plugin.studio.browser.search', null, 'Search…'), onInput: (v) => { browserQuery = v; renderBrowser(); } });
  browserEl.appendChild(search);
  const tabsRow = h('div', 'studio-browser-tabs');
  tabsRow.setAttribute('role', 'tablist');
  tabsRow.setAttribute('aria-label', t('plugin.studio.browser.sectionsLabel', null, 'Browser sections'));
  for (const [tab, label] of BROWSER_TABS) {
    const b = h('button', 'studio-btn studio-browser-tab', t(label));
    b.type = 'button';
    b.dataset.tab = tab;
    b.setAttribute('role', 'tab');
    b.setAttribute('aria-selected', String(tab === browserTab));
    b.addEventListener('click', () => { if (browserTab !== tab) browserReveal = true; browserTab = tab; renderBrowser(); });
    tabsRow.appendChild(b);
  }
  browserEl.appendChild(tabsRow);
  browserListEl = h('div', 'studio-browser-list');
  browserEl.appendChild(browserListEl);
  tileEl.appendChild(browserEl);

  /* ── AI song panel ── */
  aiPanelEl = h('div', 'studio-browser studio-ai hidden');
  aiPanelEl.setAttribute('role', 'dialog');
  aiPanelEl.setAttribute('aria-label', t('plugin.studio.ai.title', null, 'AI song'));
  tileEl.appendChild(aiPanelEl);

  tileEl.addEventListener('keydown', (e) => { if (e.key === 'Escape') { closeBrowser(); closeAiPanel(); } });
  tileEl.addEventListener('pointerdown', (e) => {
    if (!browserEl.classList.contains('hidden') && !browserEl.contains(e.target) && e.target !== browserBtn && !browserBtn.contains(e.target)) closeBrowser();
  });

  /* ── initial state ── */
  arrangement = blankArrangement();
  launcher = blankLauncher();
  sel = { area: 'arr', clipIndex: 0 };
  syncSelection();
  syncPanels();
  renderDetail();
  updateTransport();
  setStatus('saved');
  void refreshTracks().then(renderBrowser).catch(() => {});
  void refreshArrangements().then(renderBrowser).catch(() => {});
  void refreshPresets().then(renderBrowser).catch(() => {});
  void loadCatalog();
  requestAnimationFrame(() => { if (tileEl?.isConnected) { readCanvasColors(); redrawClipPreviews(); } });
  void consumePendingAiTrack().catch(() => {});
  window.addEventListener('keydown', onStudioKeydown);
  window.addEventListener('trackpad:pinch', onShellPinch);
  window.visualViewport?.addEventListener('resize', onViewportResize);
  window.addEventListener('theme:change', onStudioThemeChange);
  window.addEventListener('appearance:change', onStudioThemeChange);

  updateStudioGlow();
  return tileEl;
}

export function unmountStudioTile() {
  tileEl?.querySelectorAll('.ui-dial').forEach(control => control.destroy());
  window.removeEventListener('keydown', onStudioKeydown);
  window.removeEventListener('trackpad:pinch', onShellPinch);
  window.visualViewport?.removeEventListener('resize', onViewportResize);
  arrTouches.clear();
  pinchBase = null;
  window.removeEventListener('theme:change', onStudioThemeChange);
  window.removeEventListener('appearance:change', onStudioThemeChange);
  canvasInkRead = false;
  stopPlayback();
  stopAllLauncher();
  metroStop();
  if (mixRefreshTimer) { clearTimeout(mixRefreshTimer); mixRefreshTimer = 0; }
  stopMeters();
  if (clockUiTimer) { clearInterval(clockUiTimer); clockUiTimer = 0; }
  if (beatFlashTimer) { clearTimeout(beatFlashTimer); beatFlashTimer = 0; }
  if (tileEl) { tileEl.remove(); tileEl = null; }
  barEl = timeEl = titleInput = bpmInput = statusEl = saveDotEl = playBtn = loopBtn = metroBtn = null;
  viewSeg?.destroy?.();
  viewSeg = null;
  bodyEl = arrWrapEl = arrGridEl = arrPlayheadEl = arrScrollEl = null;
  if (arrExtendTimer) { clearTimeout(arrExtendTimer); arrExtendTimer = 0; }
  lchWrapEl = lchGridEl = detailEl = detailBodyEl = gridEl = pianoEl = devicesEl = mixerEl = null;
  synthmeWrapEl = null;
  synthmeDraft = null;
  gridWrapEl = gridCanvasEl = gridSvgEl = null;
  gridSel = null;
  gridDraft = null;
  browserEl = browserListEl = footerHintEl = footerParamEl = edToolbarEl = editorPageEl = null;
  browserBtn = null;
  if (autosaveTimer) { clearTimeout(autosaveTimer); autosaveTimer = 0; }
  aiPanelEl = null;
  aiBtnEl = null;
  stopLibraryPreview();
  beatDotEl = null;
  meterViews = [];
  channels = new Map();
  masterNodes = null;
  stemBuffers = new Map();
  mixManifest = null;
  mixManifestSig = '';
  mixContentSig = '';
  mixdownBuffer = null;
  aiJobs = [];
  if (aiPollTimer) { clearInterval(aiPollTimer); aiPollTimer = 0; }
  autoKnobs = [];
  auditionSource = null;
  lastClockBeat = -1;
  arrStartBeat = 0;
  arrPlayOffset = 0;
  pageSeg?.destroy?.();
  pageSeg = null;
  collapseBtn = null;
  soloTracks = new Set();
  trackAudio = {};
  current = null;
  sel = null;
}

export function getStudioTileElement() {
  return tileEl;
}

/* ── agent integration ──────────────────────────────────────── */

let wired = false;
export function wireStudioEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
}
function onAgentActions(e) {
  const actions = e.detail || [];
  const studio = actions.filter((a) => /^studio_/.test(a?.action || ''));
  if (!studio.length) return;
  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: STUDIO_PLUGIN } }));

  const created = studio.find((a) => a.action === 'studio_create' && a.result === 'ok');
  const rendered = studio.find((a) => a.action === 'studio_render' && a.result === 'ok');
  // Every tool that changes (or measures) an arrangement reports its id; the
  // last one wins and becomes / refreshes the open project.
  const ARR_TOOLS = ['studio_arrangement_save', 'studio_import_audio', 'studio_arrangement_edit', 'studio_export', 'studio_mix_analyze'];
  const savedArr = [...studio].reverse().find((a) => ARR_TOOLS.includes(a.action) && a.result === 'ok' && a.data?.id);
  const changedArr = savedArr && savedArr.action !== 'studio_export' && savedArr.action !== 'studio_mix_analyze';
  const deleted = studio.some((a) => a.action === 'studio_delete' && a.result === 'ok');
  const touchedId = (created || rendered)?.data?.track_id;

  void (async () => {
    if (savedArr) {
      const id = savedArr.data.id;
      libraryFiles = null;
      await refreshArrangements().catch(() => {});
      const isOpen = arrangement?.id === id;
      if (!changedArr && isOpen) { renderBrowser(); return; }   // nothing changed
      if (dirty && arrangement?.clips?.length && (isOpen || arrangement?.id)) {
        // Never drop the user's unsaved edits silently.
        const ok = await confirmDialog(t('plugin.studio.dialog.agentReload', { name: savedArr.data.title || '' },
          'The assistant changed an arrangement. Load it and discard your unsaved changes?'));
        if (!ok) { renderBrowser(); return; }
      }
      const wasPlaying = arrPlaying;
      if (wasPlaying) stopPlayback();
      await loadArrangement(id).catch(() => {});
      if (wasPlaying) void renderArrangementAndPlay();
      renderBrowser();
      return;
    }
    if (touchedId) pendingAiTrackId = touchedId;
    await refreshTracks().catch(() => {});
    renderBrowser();
    if (deleted) {
      renderBrowser();
    } else if (touchedId) {
      await consumePendingAiTrack().catch(() => {});
    }
  })();
}

/** Entries core splices into this window's right-click menu (PLUGINS.md §19).
 *  Core supplies the surrounding separators + window management. */
export function studioContextMenu() {
  const ready = !!arrangement;
  const browserOpen = !!browserEl && !browserEl.classList.contains('hidden');
  return [
    { type: 'item', label: t('plugin.studio.ctx.play', null, 'Play'), icon: 'ui/play', disabled: !ready, onClick: () => {
      if (panels.arranger) void renderArrangementAndPlay();
      else launchScene(0);
    } },
    { type: 'item', label: t('plugin.studio.ctx.stop', null, 'Stop'), icon: 'ui/stop', onClick: () => { stopPlayback(); stopAllLauncher(); } },
    ...(ctxClip && arrangement?.clips[ctxClip.index] ? (() => {
      const cc = ctxClip;
      return [
        { type: 'separator' },
        { type: 'item', label: t('plugin.studio.ctx.splitHere', null, 'Split clip here'), icon: 'ui/pencil', onClick: () => { splitClipAt(cc.index, cc.beat); } },
        { type: 'item', label: t('plugin.studio.ctx.duplicateClip', null, 'Duplicate clip'), icon: 'ui/copy', onClick: () => { selectArrClip(cc.index); duplicateSelectedClip(); } },
        { type: 'item', label: t('plugin.studio.ctx.deleteClip', null, 'Delete clip'), icon: 'ui/trash', onClick: () => removeArrClip(cc.index) },
      ];
    })() : []),
    ...(sel?.area === 'arr' && !ctxClip ? [{ type: 'item', label: t('plugin.studio.ctx.splitPlayhead', null, 'Split clip at playhead'), icon: 'ui/pencil', onClick: () => splitSelectedAtPlayhead() }] : []),
    { type: 'separator' },
    { type: 'item', label: t('plugin.studio.ctx.saveProject', null, 'Save project'), icon: 'ui/save', disabled: !ready, onClick: () => saveProjectNow() },
    { type: 'item', label: t(browserOpen ? 'plugin.studio.ctx.closeBrowser' : 'plugin.studio.ctx.openBrowser', null, browserOpen ? 'Close browser' : 'Open browser'), icon: 'ui/search', checked: browserOpen, onClick: toggleBrowser },
  ];
}

export default {
  name: STUDIO_PLUGIN,
  icon: 'ui/play',
  mount: mountStudioTile,
  unmount: unmountStudioTile,
  getElement: getStudioTileElement,
  wireEvents: wireStudioEvents,
  contextMenu: studioContextMenu,
};

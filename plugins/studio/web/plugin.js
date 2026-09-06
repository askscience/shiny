/**
 * studio.js — the Studio plugin's window (trem-powered DAW).
 *
 * Bitwig Studio-style layout:
 *   • Header — transport (stop / play / loop / metronome), position + BPM,
 *     project title, save, WAV export, browser toggle, status.
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
 * All audio renders through the plugin's REST API (trem engine); playback
 * is WebAudio. Launcher clips loop and launch quantized to the next bar.
 */

import { toast, icon, setIcon, button, searchBar } from '/ui/index.js';
import { apiFetch } from '/js/api.js';

export const STUDIO_PLUGIN = 'studio';

const KINDS = ['kick', 'snare', 'hat', 'clap', 'tom', 'perc', 'bass', 'pluck', 'lead', 'pad', 'sub', 'organ', 'ep', 'bell', 'strings', 'brass', 'synthme', 'grid', 'drumkit'];
const KIND_LABELS = { kick: 'Kick', snare: 'Snare', hat: 'Hat', clap: 'Clap', tom: 'Tom', perc: 'Perc', bass: 'Bass', pluck: 'Pluck', lead: 'Lead', pad: 'Pad', sub: 'Sub', organ: 'Organ', ep: 'E-Piano', bell: 'Bell', strings: 'Strings', brass: 'Brass', synthme: 'SynthMe', grid: 'WaveMe', drumkit: 'Drum Machine' };
const KIND_INITIALS = { kick: 'K', snare: 'S', hat: 'H', clap: 'C', tom: 'T', perc: 'P', bass: 'B', pluck: 'P', lead: 'L', pad: 'P', sub: 'S', organ: 'O', ep: 'EP', bell: 'B', strings: 'St', brass: 'Br', synthme: 'SY', grid: 'WM', drumkit: 'DM' };
const TUNINGS = [['edo12', '12-TET'], ['edo19', '19-TET'], ['ji7', 'Just 7']];
const WAVES = [['sine', 'Sine'], ['triangle', 'Triangle'], ['saw', 'Saw'], ['square', 'Square']];
const TRACK_COLORS = ['#ff5d5d', '#ffb454', '#ffe156', '#8dff9e', '#57d9ff', '#7aa2ff', '#c792ff', '#ff8fd8'];
const MELODIC = new Set(['bass', 'pluck', 'lead', 'pad', 'sub', 'organ', 'ep', 'bell', 'strings', 'brass', 'synthme', 'grid']);

const DEFAULT_LEVEL = { kick: 0.9, snare: 0.75, hat: 0.45, clap: 0.7, tom: 0.7, perc: 0.4, bass: 0.7, pluck: 0.5, lead: 0.55, pad: 0.5, sub: 0.6, organ: 0.5, ep: 0.5, bell: 0.5, strings: 0.6, brass: 0.6, synthme: 0.6, grid: 0.6, drumkit: 0.85 };
const DEFAULT_PAN = { hat: 0.35, lead: -0.25, snare: 0.05, kick: -0.05, bass: 0, pluck: 0, organ: 0.1, ep: -0.1, strings: 0.15, brass: 0.1 };

const PPB = 56;        // px per beat (arranger)
const HEAD_W = 168;    // track header column width
const RULER_H = 26;    // ruler row height
const TRACK_H = 52;    // track lane height
const AUTO_H = 34;     // automation lane height

/* Synth parameter catalog — mirrors trem::dsp nodes. */
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
  kick: 'Bass drum', snare: 'Snare', hat: 'Hi-hat', clap: 'Clap', tom: 'Tom', perc: 'Percussion',
  bass: 'Bass synth', pluck: 'Pluck', lead: 'Lead synth', pad: 'Pad', sub: 'Sub bass',
  organ: 'Tonewheel organ', ep: 'Electric piano', bell: 'Metallic bell', strings: 'Detuned strings', brass: 'Buzzy brass',
  synthme: 'Custom synth built in SynthMe',
  grid: 'Modular patch (WaveMe)',
  drumkit: '16-pad drum machine',
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

function h(tag, cls, text) {
  const el = document.createElement(tag);
  if (cls) el.className = cls;
  if (text != null) el.textContent = text;
  return el;
}
function fmt(n) {
  if (!Number.isFinite(n)) return '0';
  if (Number.isInteger(n)) return String(n);
  return String(Math.round(n * 1000) / 1000);
}

/* ── euclidean / rhythm helpers (mirror trem::euclidean) ────── */

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

const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
// trem's degree 0 resolves to the reference pitch (A4 = 440 Hz = MIDI 69).
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
  return { kind, rhythm, degree: 0, octave: 0, wave: 'sine', notes: [], level: null, pan: null, synth: {}, fx: [], macros: [], pads: [], midi: [], grid: null, ...opts };
}
function defaultKit() {
  return [
    voice('kick', 'e4,0'),
    voice('hat', 'e8,2'),
    voice('snare', 'e4,8'),
  ];
}
function kitPattern(steps = 16) {
  return { title: 'Kit', bpm: 120, steps, tuning: 'edo12', voices: defaultKit(), fx: {} };
}
function bassPattern(steps = 16) {
  return { title: 'Bass', bpm: 120, steps, tuning: 'edo12', voices: [voice('bass', 'x...x...x...x...', { octave: 2, wave: 'triangle' })], fx: {} };
}
function leadPattern(steps = 16) {
  return { title: 'Lead', bpm: 120, steps, tuning: 'edo12', voices: [voice('lead', 'x.x.x.x.x.x.x.x.', { degree: 4, octave: 3, wave: 'saw' })], fx: {} };
}
function padPattern(steps = 16) {
  return { title: 'Pad', bpm: 120, steps, tuning: 'edo12', voices: [voice('pad', 'x...x...x...x...', { degree: 0, octave: 3 })], fx: {} };
}
function simplePattern(kind, steps = 16) {
  if (kind === 'drumkit') {
    const notes = [0, 4, 8, 12].map((s) => ({ step: s, length: 1, degree: 0, octave: 0 }));
    return { title: KIND_LABELS[kind], bpm: 120, steps, tuning: 'edo12', voices: [voice('drumkit', '', { pads: defaultPads(), notes })], fx: {} };
  }
  const oct = (kind === 'bass' || kind === 'sub') ? 2 : 3;
  return { title: KIND_LABELS[kind], bpm: 120, steps, tuning: 'edo12', voices: [voice(kind, 'x.x.x.x.x.x.x.x.', { degree: 0, octave: oct })], fx: {} };
}

function blankArrangement() {
  return {
    id: null,
    title: 'Untitled Arrangement',
    bpm: 120,
    length_beats: 32,
    master: 0.9,
    tracks: [
      { id: 't0', name: 'Drums', color: 0, mute: false, level: 0.85, pan: 0, automation: { lanes: [] } },
      { id: 't1', name: 'Bass', color: 3, mute: false, level: 0.7, pan: -0.1, automation: { lanes: [] } },
      { id: 't2', name: 'Lead', color: 4, mute: false, level: 0.55, pan: 0.2, automation: { lanes: [] } },
    ],
    clips: [
      { track: 't0', start: 0, pattern: kitPattern(16) },
      { track: 't1', start: 0, pattern: bassPattern(16) },
      { track: 't2', start: 0, pattern: leadPattern(16) },
    ],
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
    notes: Array.isArray(v?.notes) ? v.notes.map((n) => ({ step: n.step, length: n.length || 1, degree: n.degree, octave: n.octave })) : [],
    level: typeof v?.level === 'number' ? v.level : null,
    pan: typeof v?.pan === 'number' ? v.pan : null,
    synth: (v?.synth && typeof v.synth === 'object') ? { ...v.synth } : {},
    fx: Array.isArray(v?.fx) ? v.fx.map((f) => ({
      kind: EFFECT_KINDS.includes(f?.kind) ? f.kind : 'distortion',
      params: (f?.params && typeof f.params === 'object') ? { ...f.params } : {},
      bypass: !!f?.bypass,
    })) : [],
    macros: normalizeMacros(v?.macros),
    pads: Array.isArray(v?.pads) ? v.pads.map((p) => ({ name: p.name || 'Pad', kind: KINDS.includes(p.kind) ? p.kind : 'kick' })) : [],
    midi: normalizeMidiFx(v?.midi),
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
    tuning: p.tuning,
    voices: p.voices.map((v) => ({
      kind: v.kind, rhythm: v.rhythm, degree: v.degree, octave: v.octave,
      wave: v.wave, notes: v.notes, level: v.level, pan: v.pan, synth: v.synth,
      fx: v.fx.map((f) => ({ kind: f.kind, params: f.params, bypass: f.bypass })),
      macros: v.macros,
      pads: v.pads,
      midi: v.midi,
      grid: v.grid,
    })),
    fx: p.fx || {},
  };
}
function arrangementPayload() {
  return {
    title: arrangement.title,
    bpm: arrangement.bpm,
    length_beats: arrangement.length_beats,
    master: arrangement.master ?? 0.9,
    tracks: arrangement.tracks.map((t) => ({ id: t.id, name: t.name, color: t.color, mute: t.mute, level: t.level, pan: t.pan, automation: t.automation || { lanes: [] } })),
    clips: arrangement.clips.map((c) => ({ track: c.track, start: c.start, pattern: c.pattern })),
  };
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
let arrSource = null;            // arrangement buffer source
let arrPlaying = false;
let arrPlayStart = 0;
let arrPlayDur = 0;
let arrLoop = false;
let rafId = 0;
let trackLoops = {};             // launcher trackId -> { source, slotKey, when, state }
let trackAudio = {};             // launcher trackId -> { gain, panner }
let clockRunning = false;        // launcher master clock
let clockT0 = 0;
let prevSource = null;           // editor preview
let prevPlaying = false;
let prevStart = 0;
let prevDur = 0;
let auditionSource = null;       // looping SynthMe / WaveMe preview
let pendingAiTrackId = null;     // AI-created track to drop in once mounted
let lastConsumedAiTrackId = null; // de-dupe re-delivered agent:actions
let metroOn = false;
let metroTimer = 0;
let clockUiTimer = 0;
let masterGain = null;          // stereo passthrough bus (all audible sources)
let analyser = null;            // master tap → oscilloscope + spectrum + master meter
let scopeTraceEl = null;        // oscilloscope canvas
let scopeSpecEl = null;         // spectrum canvas
let scopeTimeData = null;
let scopeFreqData = null;
let scopePeaks = null;
let scopeRaf = 0;
let scopeMeter = null;          // footer master meter { fill, peakEl, level, pk }
let mixerMeters = [];           // mixer strip meters (rebuilt each renderMixer)
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
let arrToggleBtn = null;
let lchToggleBtn = null;
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
let pageBtns = {};
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

/* ── audio plumbing ─────────────────────────────────────────── */

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

/* Master output bus — every audible node routes through this stereo
   passthrough gain. A parallel AnalyserNode taps the summed signal for the
   oscilloscope / spectrum / master meter without downmixing the audio to mono. */
function masterOut() {
  const ctx = audioCtxOr();
  if (!masterGain) {
    masterGain = ctx.createGain();
    masterGain.gain.value = 1;
    masterGain.connect(ctx.destination);
    analyser = ctx.createAnalyser();
    analyser.fftSize = 2048;
    analyser.smoothingTimeConstant = 0.85;
    analyser.minDecibels = -95;
    analyser.maxDecibels = -10;
    masterGain.connect(analyser);   // tap only — analyser output is intentionally unused
  }
  return masterGain;
}
/* RMS level of a time-domain buffer, scaled for a musical meter. */
function levelFromTime(buf) {
  if (!buf) return 0;
  let sum = 0;
  for (let i = 0; i < buf.length; i++) { const v = (buf[i] - 128) / 128; sum += v * v; }
  return Math.min(1, Math.sqrt(sum / buf.length) * 3.2);
}
function analyserLevel(an) {
  if (!an) return 0;
  const n = an.fftSize || 1024;
  if (!an._lvlBuf || an._lvlBuf.length !== n) an._lvlBuf = new Uint8Array(n);
  an.getByteTimeDomainData(an._lvlBuf);
  return levelFromTime(an._lvlBuf);
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
function metroStart(startTime, bpm) {
  metroStop();
  if (!metroOn) return;
  const beat = 60 / bpm;
  let nextBeat = startTime;
  let beatIdx = 0;
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

/* ── arrangement playback ───────────────────────────────────── */

function clearArrPlayhead() {
  if (arrPlayheadEl) arrPlayheadEl.style.opacity = '0';
  if (timeEl) timeEl.textContent = '';
}
function stopPlayback() {
  if (rafId) { cancelAnimationFrame(rafId); rafId = 0; }
  try { arrSource?.stop(); } catch (_) { /* noop */ }
  arrSource = null;
  arrPlaying = false;
  stopAudition();
  metroStop();
  clearArrPlayhead();
  clearPrevPlayhead();
  updateTransport();
}

async function renderArrangementAndPlay() {
  if (busy || !arrangement) return;
  busy = true;
  setStatus('Rendering…');
  try {
    const blob = await apiFetch('/api/studio/arrangement/render', {
      method: 'POST', body: JSON.stringify(arrangementRenderPayload()), responseType: 'blob',
    });
    const buf = await blob.arrayBuffer();
    const ctx = audioCtxOr();
    if (ctx.state === 'suspended') await ctx.resume();
    const decoded = await ctx.decodeAudioData(buf);
    stopPlayback();
    const src = ctx.createBufferSource();
    src.buffer = decoded;
    src.loop = arrLoop;
    src.connect(masterOut());
    src.onended = () => { if (!arrLoop) { arrPlaying = false; clearArrPlayhead(); metroStop(); updateTransport(); } };
    const startAt = ctx.currentTime + 0.05;
    src.start(startAt);
    arrSource = src;
    arrPlaying = true;
    arrPlayStart = startAt;
    arrPlayDur = decoded.duration;
    metroStart(startAt, arrangement.bpm);
    setStatus('Playing');
    const totalBeats = arrangement.length_beats;
    const tick = () => {
      if (!arrPlaying || !audioCtx) return;
      const raw = audioCtx.currentTime - arrPlayStart;
      if (raw < 0) { rafId = requestAnimationFrame(tick); return; }
      const elapsed = arrLoop ? (raw % arrPlayDur) : raw;
      if (!arrLoop && raw >= arrPlayDur) { stopPlayback(); return; }
      const beats = (elapsed / arrPlayDur) * totalBeats;
      if (arrPlayheadEl) { arrPlayheadEl.style.opacity = '1'; arrPlayheadEl.style.transform = `translateX(${HEAD_W + beats * PPB}px)`; }
      if (timeEl) timeEl.textContent = `${Math.floor(beats / 4) + 1}.${Math.floor(beats % 4) + 1} · ${fmtTime(elapsed)}`;
      updateAutoKnobs(beats);
      rafId = requestAnimationFrame(tick);
    };
    rafId = requestAnimationFrame(tick);
  } catch (e) {
    console.error('studio: arrangement render/play failed', e);
    toast(e.message || 'Render failed', { type: 'error' });
    setStatus('error');
  } finally {
    busy = false;
    updateTransport();
  }
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
    const { src, decoded } = await previewPattern(serializePattern(current));
    clearPrevPlayhead();
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
    toast(e.message || 'Preview failed', { type: 'error' });
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
  } catch (e) {
    toast(e.message || 'Play failed', { type: 'error' });
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
    const cfg = { ...serializePattern(normalizePattern(clip.pattern)), title: clip.title || 'Clip', bpm: launcher.bpm };
    const ta = ensureTrackAudio(tr);
    const { src } = await previewPattern(cfg, { loop: true, bus: ta.panner || ta.gain });
    const ctx = audioCtxOr();
    if (!clockRunning) {
      clockRunning = true;
      clockT0 = ctx.currentTime + 0.08;
      metroStart(clockT0, launcher.bpm);
      startClockUi();
    }
    const when = nextBarTime();
    src.start(when);
    trackLoops[tr.id] = { source: src, slotKey: key, when, state: 'queued' };
    setStatus('Queued');
  } catch (e) {
    console.error('studio: launch clip failed', e);
    toast(e.message || 'Launch failed', { type: 'error' });
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

async function saveArrangement() {
  const payload = arrangementPayload();
  let data;
  if (arrangement.id) {
    data = await api(`/api/studio/arrangement/${arrangement.id}`, { method: 'PUT', body: JSON.stringify(payload) });
  } else {
    data = await api('/api/studio/arrangement', { method: 'POST', body: JSON.stringify(payload) });
  }
  arrangement.id = data?.id;
  dirty = false;
  await refreshArrangements();
  renderBrowser();
  return data;
}
async function loadArrangement(id) {
  const data = await api(`/api/studio/arrangement/${id}`);
  arrangement = {
    id: data.id,
    title: data.title || 'Untitled',
    bpm: data.bpm ?? 120,
    length_beats: data.length_beats ?? 32,
    master: data.master ?? 0.9,
    tracks: (data.tracks || []).map((t) => ({ id: t.id, name: t.name, color: t.color ?? 0, mute: !!t.mute, level: t.level ?? 0.8, pan: t.pan ?? 0, automation: normalizeAutomation(t.automation) })),
    clips: (data.clips || []).map((c) => ({ track: c.track, start: c.start ?? 0, pattern: normalizePattern(c.pattern || {}) })),
  };
  sel = arrangement.clips.length ? { area: 'arr', clipIndex: 0 } : null;
  syncSelection();
  dirty = false;
  if (titleInput && panels.arranger) titleInput.value = arrangement.title;
  renderArranger();
  renderBrowser();
  setStatus('saved');
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

/* Save the pattern being edited into the pattern library. */
async function savePatternToLibrary() {
  if (!current) { toast('Select a clip first', { type: 'error' }); return; }
  const title = prompt('Pattern name:', current.title || 'Pattern');
  if (!title) return;
  current.title = title;
  const data = await api('/api/studio', { method: 'POST', body: JSON.stringify(serializePattern(current)) });
  await refreshTracks();
  renderBrowser();
  toast(`Saved pattern "${data?.title || title}"`, { type: 'success' });
}
async function savePreset(v) {
  const name = prompt('Preset name:', `${KIND_LABELS[v.kind]} ${new Date().toLocaleTimeString()}`);
  if (!name) return;
  const params = {
    wave: v.wave,
    synth: { ...v.synth },
    fx: v.fx.map((f) => ({ kind: f.kind, params: f.params, bypass: f.bypass })),
    level: v.level,
    pan: v.pan,
  };
  await api('/api/studio/presets', { method: 'POST', body: JSON.stringify({ kind: v.kind, name, params }) });
  await refreshPresets();
  renderDevices();
  renderBrowser();
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
  const busy = s === 'Rendering…' || s === 'Playing' || s === 'Queued';
  const isDirty = dirty && s !== 'saved';
  if (saveDotEl) {
    saveDotEl.classList.toggle('is-active', isDirty);
    saveDotEl.title = isDirty ? 'Unsaved changes' : 'Saved';
  }
  if (!statusEl) return;
  const showText = busy || s === 'error';
  statusEl.textContent = showText ? s : '';
  statusEl.hidden = !showText;
  statusEl.dataset.state = s === 'error' ? 'error' : (busy ? 'busy' : (isDirty ? 'dirty' : 'ok'));
}
function markDirty() {
  dirty = true;
  setStatus('dirty');
}
function updateTransport() {
  if (playBtn) playBtn.classList.toggle('studio-transport--on', arrPlaying);
  if (loopBtn) loopBtn.classList.toggle('studio-btn--on', arrLoop);
  if (metroBtn) metroBtn.classList.toggle('studio-btn--on', metroOn);
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

/* ── live oscilloscope / spectrum / meters (Bitwig-style) ───── */

const SCOPE_TRACE = '#5ad1a8';
const SCOPE_GRID = 'rgba(255,255,255,0.07)';

function flashBeat(accent) {
  if (!beatDotEl) return;
  beatDotEl.classList.add('studio-beat--hit');
  if (accent) beatDotEl.style.boxShadow = '0 0 10px 2px var(--accent)';
  if (beatFlashTimer) clearTimeout(beatFlashTimer);
  beatFlashTimer = setTimeout(() => {
    beatDotEl.classList.remove('studio-beat--hit');
    beatDotEl.style.boxShadow = '';
  }, 140);
}

function sizeScope() {
  if (!scopeTraceEl || !scopeSpecEl) return;
  const dpr = window.devicePixelRatio || 1;
  for (const cv of [scopeTraceEl, scopeSpecEl]) {
    const w = Math.max(2, Math.round(cv.clientWidth * dpr));
    const h = Math.max(2, Math.round(cv.clientHeight * dpr));
    if (cv.width !== w || cv.height !== h) { cv.width = w; cv.height = h; }
  }
}
function startScope() {
  if (scopeRaf) return;
  const tick = () => {
    scopeRaf = requestAnimationFrame(tick);
    drawScopeFrame();
  };
  scopeRaf = requestAnimationFrame(tick);
}
function stopScope() {
  if (scopeRaf) { cancelAnimationFrame(scopeRaf); scopeRaf = 0; }
}

function drawScopeFrame() {
  sizeScope();
  const live = !!analyser;
  let masterLevel = 0;
  if (live) {
    if (!scopeTimeData || scopeTimeData.length !== analyser.fftSize) scopeTimeData = new Uint8Array(analyser.fftSize);
    if (!scopeFreqData || scopeFreqData.length !== analyser.frequencyBinCount) scopeFreqData = new Uint8Array(analyser.frequencyBinCount);
    analyser.getByteTimeDomainData(scopeTimeData);
    analyser.getByteFrequencyData(scopeFreqData);
    masterLevel = levelFromTime(scopeTimeData);
  }
  drawScopeTrace(live ? scopeTimeData : null);
  drawScopeSpec(live ? scopeFreqData : null);
  updateMeters(masterLevel);
}

function drawScopeTrace(data) {
  const cv = scopeTraceEl; if (!cv) return;
  const g = cv.getContext('2d');
  const w = cv.width, h = cv.height;
  g.clearRect(0, 0, w, h);
  g.strokeStyle = SCOPE_GRID; g.lineWidth = 1;
  g.beginPath();
  g.moveTo(0, h / 2); g.lineTo(w, h / 2);
  g.moveTo(0, h / 4); g.lineTo(w, h / 4);
  g.moveTo(0, h * 3 / 4); g.lineTo(w, h * 3 / 4);
  g.stroke();
  g.beginPath();
  if (data) {
    for (let i = 0; i < data.length; i++) {
      const x = (i / (data.length - 1)) * w;
      const y = h / 2 - ((data[i] - 128) / 128) * (h / 2 - 1);
      if (i) g.lineTo(x, y); else g.moveTo(x, y);
    }
  } else {
    /* idle shimmer — a gentle near-flat line so the scope never looks frozen */
    const t = performance.now() / 1000;
    for (let i = 0; i <= 64; i++) {
      const x = (i / 64) * w;
      const y = h / 2 + Math.sin(t * 3 + i * 0.35) * 0.6;
      if (i) g.lineTo(x, y); else g.moveTo(x, y);
    }
  }
  g.strokeStyle = SCOPE_TRACE;
  g.lineWidth = Math.max(1, h * 0.06);
  g.lineJoin = 'round';
  g.shadowColor = SCOPE_TRACE;
  g.shadowBlur = 4;
  g.stroke();
  g.shadowBlur = 0;
}

function drawScopeSpec(data) {
  const cv = scopeSpecEl; if (!cv) return;
  const g = cv.getContext('2d');
  const w = cv.width, h = cv.height;
  g.clearRect(0, 0, w, h);
  const bars = 48;
  if (!scopePeaks || scopePeaks.length !== bars) scopePeaks = new Float32Array(bars);
  const bw = w / bars;
  for (let i = 0; i < bars; i++) {
    let v = 0;
    if (data) {
      const idx = Math.min(data.length - 1, Math.floor(Math.pow(i / bars, 1.6) * (data.length * 0.55)));
      v = data[idx] / 255;
    }
    const bh = Math.max(0, v * h);
    scopePeaks[i] = Math.max(scopePeaks[i] * 0.92, bh);
    const hue = 150 + (i / bars) * 120;
    g.fillStyle = `hsl(${hue} 70% 60%)`;
    g.fillRect(Math.round(i * bw) + 1, h - bh, Math.max(1, bw - 2), bh);
    const pk = scopePeaks[i];
    g.fillStyle = 'rgba(255,255,255,0.7)';
    g.fillRect(Math.round(i * bw) + 1, h - pk, Math.max(1, bw - 2), 1);
  }
}

function paintMeter(m, target) {
  if (!m) return;
  m.level += (target - m.level) * 0.5;
  m.pk = Math.max(m.pk * 0.93, m.level);
  m.fill.style.transform = `scaleX(${m.level.toFixed(4)})`;
  m.peakEl.style.left = `${(m.pk * 100).toFixed(2)}%`;
  m.peakEl.style.opacity = m.pk > 0.02 ? '1' : '0';
  m.fill.classList.toggle('studio-meter-fill--clip', m.level > 0.97);
}
function updateMeters(masterLevel) {
  if (scopeMeter) paintMeter(scopeMeter, masterLevel);
  for (const m of mixerMeters) paintMeter(m, m.getAnalyser ? analyserLevel(m.getAnalyser()) : 0);
}
/* Build a mixer meter; registers itself for the render loop. */
function meterNode(getAnalyser) {
  const wrap = h('div', 'studio-meter');
  const fill = h('span', 'studio-meter-fill');
  const peakEl = h('span', 'studio-meter-peak');
  wrap.append(fill, peakEl);
  mixerMeters.push({ fill, peakEl, getAnalyser, level: 0, pk: 0 });
  return wrap;
}

/* ── native-ish controls ────────────────────────────────────── */

function nativeSelect(options, value, onchange, hint) {
  const selEl = document.createElement('select');
  selEl.className = 'studio-select';
  for (const [val, label] of options) {
    const opt = document.createElement('option');
    opt.value = val;
    opt.textContent = label;
    selEl.appendChild(opt);
  }
  selEl.value = value;
  selEl.addEventListener('change', () => onchange(selEl.value));
  if (hint) withHint(selEl, hint);
  return selEl;
}
function numberField(placeholder, value, onchange, hint) {
  const input = document.createElement('input');
  input.type = 'number';
  input.className = 'studio-number';
  input.placeholder = placeholder;
  input.value = String(value);
  input.addEventListener('change', () => { onchange(parseInt(input.value, 10) || 0); });
  if (hint) withHint(input, hint);
  return input;
}

/* Arc knob: 270° ring from the neutral "0" to the value (bipolar-aware). */
function knob(label, min, max, step, value, onchange, resetTo) {
  const k = h('div', 'studio-knob');
  k.dataset.param = label;
  const lab = h('span', 'studio-knob-label', label);
  const dial = h('div', 'studio-knob-dial');
  dial.title = `${label} — drag up/down, double-click to reset`;
  const ptr = h('span', 'studio-knob-pointer');
  dial.appendChild(ptr);
  const val = h('span', 'studio-knob-value');
  let cur = value;
  const set = (v) => {
    cur = Math.min(max, Math.max(min, v));
    const span = max - min || 1;
    const norm = (cur - min) / span;
    const zeroNorm = Math.min(1, Math.max(0, (0 - min) / span));
    const a0 = -135 + zeroNorm * 270;   // angle of the "0" (neutral) position
    const a1 = -135 + norm * 270;       // angle of the current value
    dial.style.setProperty('--arc-start', `${Math.min(a0, a1).toFixed(2)}deg`);
    dial.style.setProperty('--arc-sweep', `${Math.abs(a1 - a0).toFixed(2)}deg`);
    ptr.style.transform = `rotate(${a1}deg)`;
    val.textContent = fmt(cur);
    k.dataset.value = fmt(cur);
  };
  set(value);
  k._dial = dial;   // expose for automation-follow visual updates
  k._set = set;
  let dragging = false;
  let startY = 0;
  let startV = 0;
  dial.addEventListener('pointerdown', (e) => {
    dragging = true;
    startY = e.clientY;
    startV = cur;
    dial.setPointerCapture?.(e.pointerId);
    e.preventDefault();
  });
  const move = (e) => {
    if (!dragging) return;
    const dy = startY - e.clientY;
    let v = startV + (dy / 150) * (max - min);
    if (step > 0) v = Math.round(v / step) * step;
    v = Math.min(max, Math.max(min, v));
    set(v);
    onchange(v);
    setReadout(`${label} — ${fmt(cur)}`);
  };
  const up = () => { dragging = false; };
  dial.addEventListener('pointermove', move);
  dial.addEventListener('pointerup', up);
  dial.addEventListener('pointercancel', up);
  dial.addEventListener('dblclick', () => { if (resetTo != null) { set(resetTo); onchange(resetTo); } });
  k.addEventListener('mouseenter', () => setReadout(`${label} — ${fmt(cur)}`));
  k.append(lab, dial, val);
  return k;
}

/* Thin vertical fader (mixer strips). */
function fader(label, value, onchange) {
  const wrap = h('div', 'studio-mixer-fader');
  wrap.dataset.param = label;
  const inp = document.createElement('input');
  inp.type = 'range';
  inp.className = 'studio-fader-input';
  inp.min = '0';
  inp.max = '2';
  inp.step = '0.01';
  inp.value = String(value);
  inp.setAttribute('orient', 'vertical');
  const val = h('span', 'studio-mixer-fader-val', fmt(value));
  inp.addEventListener('input', () => {
    const n = parseFloat(inp.value);
    val.textContent = fmt(n);
    wrap.dataset.value = fmt(n);
    setReadout(`${label} — ${fmt(n)}`);
    onchange(n);
  });
  wrap.append(inp, val);
  wrap.addEventListener('mouseenter', () => setReadout(`${label} — ${val.textContent}`));
  return wrap;
}

/* ── arranger (timeline) ────────────────────────────────────── */

function clipDurBeats(c) {
  return Math.max(1, Math.round((c.pattern?.steps || 16) / 4));
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
    { path: 'track.level', label: 'Track Level', min: 0, max: 2 },
    { path: 'track.pan', label: 'Track Pan', min: -1, max: 1 },
  ];
  const rep = arrangement.clips.find((c) => c.track === tr.id && (c.pattern?.voices || []).length);
  if (rep) {
    rep.pattern.voices.forEach((v, vi) => {
      const kl = KIND_LABELS[v.kind] || v.kind;
      for (const p of (SYNTH[v.kind] || [])) out.push({ path: `voice.${vi}.${p.key}`, label: `${kl} ${p.label}`, min: p.min, max: p.max });
      out.push({ path: `voice.${vi}.level`, label: `${kl} Level`, min: 0, max: 2 });
      out.push({ path: `voice.${vi}.pan`, label: `${kl} Pan`, min: -1, max: 1 });
      v.fx.forEach((f, fi) => {
        for (const p of (EFFECTS[f.kind]?.params || [])) out.push({ path: `voice.${vi}.fx.${fi}.${p.key}`, label: `${kl} FX${fi + 1} ${p.label}`, min: p.min, max: p.max });
      });
    });
  }
  for (const p of FX) out.push({ path: `master.${p.key}`, label: `Master ${p.label}`, min: p.min, max: p.max });
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
    pt.beat = clamp(Math.round((x / PPB) * 4) / 4, 0, arrangement.length_beats);
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
  }, 'Automation parameter'));

  if (isFirst) {
    const add = h('button', 'studio-head-btn', '+');
    add.type = 'button';
    add.title = 'Add another automation lane';
    add.addEventListener('click', () => { lanes.push({ param: 'track.level', points: [] }); markDirty(); renderArranger(); });
    hd.appendChild(add);
  }
  if (lane) {
    const clear = h('button', 'studio-head-btn', '×');
    clear.type = 'button';
    clear.title = 'Clear breakpoints';
    clear.addEventListener('click', () => { lane.points = []; markDirty(); renderArranger(); });
    hd.appendChild(clear);
    const rm = h('button', 'studio-head-btn', '⨯');
    rm.type = 'button';
    rm.title = 'Remove lane';
    rm.addEventListener('click', () => { lanes.splice(lanes.indexOf(lane), 1); markDirty(); renderArranger(); });
    hd.appendChild(rm);
  }
  arrGridEl.appendChild(hd);

  const laneEl = h('div', 'studio-auto-lane');
  laneEl.style.gridColumn = `2 / ${beats + 2}`;
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
      const beat = clamp(Math.round((x / PPB) * 4) / 4, 0, arrangement.length_beats);
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
  if (!tr) { toast('No arrangement track to automate', { type: 'error' }); return; }
  tr.autoOpen = true;
  autoLane(tr, path);
  markDirty();
  renderArranger();
  toast(`Automating ${label} on “${tr.name}” — click the lane to add points`, { type: 'success' });
}

/* A device knob with an ⌁ button that connects it to an automation lane. */
function automatableKnob(label, path, min, max, step, value, onchange, resetTo) {
  const wrap = h('div', 'studio-knob-wrap');
  const k = knob(label, min, max, step, value, onchange, resetTo);
  wrap.append(k);
  const auto = h('button', 'studio-knob-auto', '⌁');
  auto.type = 'button';
  auto.title = `Automate ${label}`;
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

function renderArranger() {
  if (!arrGridEl || !arrangement) return;
  arrGridEl.textContent = '';
  const beats = Math.max(4, Math.round(arrangement.length_beats));
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
  arrGridEl.style.gridTemplateColumns = `${HEAD_W}px repeat(${beats}, ${PPB}px)`;
  arrGridEl.style.gridTemplateRows = rowHeights.map((h) => `${h}px`).join(' ');

  arrGridEl.appendChild(h('div', 'studio-arr-corner'));

  /* ruler — bar numbers + loop region */
  for (let b = 0; b < beats; b++) {
    const cell = h('div', 'studio-arr-ruler-cell');
    if (b % 4 === 0) { cell.classList.add('studio-arr-ruler-bar'); cell.textContent = String(b / 4 + 1); }
    if (arrLoop) cell.classList.add('studio-arr-ruler-cell--loop');
    cell.style.gridColumn = String(b + 2);
    cell.style.gridRow = '1';
    arrGridEl.appendChild(cell);
  }

  /* track headers */
  for (let t = 0; t < n; t++) {
    const tr = arrangement.tracks[t];
    const head = h('div', 'studio-arr-head');
    head.style.gridColumn = '1';
    head.style.gridRow = String(rowFor[t].track);
    head.style.setProperty('--track', trackColor(tr.color));
    if (sel?.area === 'arr' && arrTrackIndex(arrangement.clips[sel.clipIndex]?.track) === t) head.classList.add('studio-arr-head--active');
    head.appendChild(h('span', 'studio-track-stripe'));

    const name = h('span', 'studio-arr-name', tr.name);
    name.title = 'Double-click to rename';
    name.addEventListener('dblclick', (e) => { e.stopPropagation(); inlineRename(name, tr.name, (v) => { tr.name = v; markDirty(); renderArranger(); }); });
    head.appendChild(name);

    const btns = h('span', 'studio-arr-head-btns');
    const mute = h('button', 'studio-ms', 'M');
    mute.type = 'button';
    mute.title = 'Mute';
    mute.classList.toggle('studio-ms--on', !!tr.mute);
    mute.addEventListener('click', (e) => { e.stopPropagation(); tr.mute = !tr.mute; markDirty(); renderArranger(); });
    const solo = h('button', 'studio-ms studio-ms--solo', 'S');
    solo.type = 'button';
    solo.title = 'Solo';
    solo.classList.toggle('studio-ms--on', soloTracks.has(tr.id));
    solo.addEventListener('click', (e) => { e.stopPropagation(); toggleSolo(tr.id); renderArranger(); });
    const add = h('button', 'studio-head-btn', '+');
    add.type = 'button';
    add.title = 'Add clip';
    add.addEventListener('click', (e) => { e.stopPropagation(); addArrClip(t); });
    const auto = h('button', 'studio-head-btn', 'A');
    auto.type = 'button';
    auto.title = 'Show/hide automation lane';
    auto.classList.toggle('studio-btn--on', !!tr.autoOpen);
    auto.addEventListener('click', (e) => { e.stopPropagation(); tr.autoOpen = !tr.autoOpen; renderArranger(); });
    const del = h('button', 'studio-head-btn', '×');
    del.type = 'button';
    del.title = 'Remove track';
    del.addEventListener('click', (e) => { e.stopPropagation(); removeArrTrack(t); });
    btns.append(mute, solo, auto, add, del);
    head.appendChild(btns);
    arrGridEl.appendChild(head);

    /* lane background */
    const lane = h('div', 'studio-arr-lane');
    lane.style.gridColumn = `2 / ${beats + 2}`;
    lane.style.gridRow = String(rowFor[t].track);
    lane.style.setProperty('--track', trackColor(tr.color));
    if (!arrTrackAudible(tr)) lane.classList.add('studio-arr-lane--muted');
    lane.addEventListener('click', () => { sel = null; current = null; renderDetail(); });
    arrGridEl.appendChild(lane);

    /* automation lanes (one per lane) */
    if (rowFor[t].autoRows.length) renderAutomationSection(tr, beats, rowFor[t].autoRows);
  }

  /* add track row */
  const addTrack = h('button', 'studio-arr-addtrack', '+ Add track');
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
    const start = Math.max(0, Math.round(clip.start));
    const dur = clipDurBeats(clip);
    const el = h('div', 'studio-arr-clip');
    el.style.gridColumn = `${start + 2} / ${start + dur + 2}`;
    el.style.gridRow = String(rowFor[tIndex].track);
    el.style.setProperty('--track', trackColor(tr.color));
    el.dataset.clipIndex = i;
    el.title = `${clip.pattern?.title || tr.name} — drag to move, double-click to edit`;
    if (sel?.area === 'arr' && sel.clipIndex === i) el.classList.add('studio-arr-clip--sel');
    const melodic = patternIsMelodic(clip.pattern);
    const cv = h('canvas', melodic ? 'studio-arr-clip-midi' : 'studio-arr-clip-wave');
    el.appendChild(cv);
    el.appendChild(h('span', 'studio-arr-clip-name', clip.pattern?.title || tr.name));
    const send = h('button', 'studio-arr-clip-x', '▦');
    send.type = 'button';
    send.style.right = '22px';
    send.title = 'Send to Launcher';
    send.addEventListener('pointerdown', (e) => e.stopPropagation());
    send.addEventListener('click', (e) => { e.stopPropagation(); sendArrangementToLauncher(i); });
    el.appendChild(send);
    const rm = h('button', 'studio-arr-clip-x', '×');
    rm.type = 'button';
    rm.title = 'Remove clip';
    rm.addEventListener('pointerdown', (e) => e.stopPropagation());
    rm.addEventListener('click', (e) => { e.stopPropagation(); removeArrClip(i); });
    el.appendChild(rm);
    el.addEventListener('click', () => selectArrClip(i));
    el.addEventListener('dblclick', () => selectArrClip(i, { openEditor: true }));
    el.addEventListener('pointerdown', (e) => startClipDrag(e, i));
    arrGridEl.appendChild(el);
    if (melodic) requestAnimationFrame(() => drawClipMidi(i));
    else if (clip.peaks) requestAnimationFrame(() => drawClipWave(i));
    else void loadWave(i);
  }

  /* playhead */
  arrPlayheadEl = h('div', 'studio-arr-playhead');
  arrPlayheadEl.style.opacity = '0';
  arrGridEl.appendChild(arrPlayheadEl);
}

function inlineRename(span, value, done) {
  const input = document.createElement('input');
  input.type = 'text';
  input.className = 'studio-rename';
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
}

function addArrTrack(kind) {
  const i = arrangement.tracks.length;
  const label = kind ? KIND_LABELS[kind] : `Track ${i + 1}`;
  const tr = { id: `t${Date.now()}`, name: label, color: i % TRACK_COLORS.length, mute: false, level: 0.8, pan: 0 };
  arrangement.tracks.push(tr);
  const pattern = kind ? simplePattern(kind, 16) : (i % 3 === 0 ? kitPattern(16) : (i % 3 === 1 ? bassPattern(16) : leadPattern(16)));
  arrangement.clips.push({ track: tr.id, start: 0, pattern });
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
function addArrClip(t) {
  const tr = arrangement.tracks[t];
  if (!tr) return;
  const start = arrangement.clips.filter((c) => c.track === tr.id).length * 4;
  const pattern = t % 3 === 0 ? kitPattern(16) : (t % 3 === 1 ? bassPattern(16) : leadPattern(16));
  arrangement.clips.push({ track: tr.id, start: Math.min(start, Math.round(arrangement.length_beats) - 4), pattern });
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

function startClipDrag(e, clipIndex) {
  if (e.button !== 0) return;
  e.preventDefault();
  e.stopPropagation();
  const clip = arrangement.clips[clipIndex];
  const el = e.currentTarget;
  const startX = e.clientX;
  const startBeat = Math.round(clip.start);
  const dur = clipDurBeats(clip);
  const maxBeat = Math.max(0, Math.round(arrangement.length_beats) - dur);
  let moved = false;
  const move = (ev) => {
    const delta = Math.round((ev.clientX - startX) / PPB);
    if (delta !== 0) moved = true;
    clip.start = Math.min(maxBeat, Math.max(0, startBeat + delta));
    el.style.gridColumn = `${clip.start + 2} / ${clip.start + dur + 2}`;
  };
  const up = () => {
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    if (moved) { markDirty(); renderArranger(); }
  };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
}

/* clip previews — MIDI piano roll for melodic patterns, waveform for drums */
function drawClipMidi(i) {
  const clip = arrangement.clips[i];
  if (!clip) return;
  const cv = arrGridEl?.querySelector(`.studio-arr-clip[data-clip-index="${i}"] .studio-arr-clip-midi`);
  if (!cv) return;
  const notes = clipMelodicNotes(clip.pattern);
  const dpr = window.devicePixelRatio || 1;
  const w = cv.clientWidth || 60;
  const hgt = cv.clientHeight || 24;
  cv.width = Math.max(1, Math.round(w * dpr));
  cv.height = Math.max(1, Math.round(hgt * dpr));
  const ctx = cv.getContext('2d');
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, w, hgt);
  if (!notes.length) return;
  const steps = Math.max(1, clip.pattern?.steps || 16);
  const degLo = Math.min(...notes.map((n) => n.degree));
  const degHi = Math.max(...notes.map((n) => n.degree));
  const lo = degLo - 1;
  const span = Math.max(1, (degHi + 1) - lo);
  const stepW = w / steps;
  const pad = 2;
  const innerH = hgt - pad * 2;
  /* faint semitone lanes so it reads as a piano roll */
  ctx.strokeStyle = 'rgba(0,0,0,0.12)';
  ctx.lineWidth = 1;
  for (let d = degLo; d <= degHi + 1; d++) {
    const y = pad + ((degHi + 1 - d) / span) * innerH;
    ctx.beginPath();
    ctx.moveTo(0, Math.round(y) + 0.5);
    ctx.lineTo(w, Math.round(y) + 0.5);
    ctx.stroke();
  }
  for (const n of notes) {
    const x = n.step * stepW;
    const len = Math.max(1, n.length || 1);
    const wid = Math.max(1.5, len * stepW - 1);
    const y = pad + ((degHi + 1 - n.degree) / span) * innerH;
    const nh = Math.max(2, innerH / span - 1);
    ctx.fillStyle = 'rgba(11,11,12,0.62)';
    ctx.fillRect(x, y, wid, nh);
    ctx.fillStyle = 'rgba(255,255,255,0.20)';
    ctx.fillRect(x, y, wid, 1);
  }
}

/* clip waveform previews */
function drawClipWave(i) {
  const clip = arrangement.clips[i];
  if (!clip?.peaks) return;
  const cv = arrGridEl?.querySelector(`.studio-arr-clip[data-clip-index="${i}"] .studio-arr-clip-wave`);
  if (!cv) return;
  const dpr = window.devicePixelRatio || 1;
  const w = cv.clientWidth || 100;
  const hgt = cv.clientHeight || 24;
  cv.width = Math.max(1, Math.round(w * dpr));
  cv.height = Math.max(1, Math.round(hgt * dpr));
  const ctx = cv.getContext('2d');
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, w, hgt);
  ctx.fillStyle = 'rgba(0,0,0,0.4)';
  const peaks = clip.peaks;
  const nPk = peaks.length;
  const bw = w / nPk;
  const mid = hgt / 2;
  for (let k = 0; k < nPk; k++) {
    const mn = peaks[k][0];
    const mx = peaks[k][1];
    const y1 = mid - mx * mid;
    const y2 = mid - mn * mid;
    ctx.fillRect(k * bw, y1, Math.max(1, bw - 0.5), Math.max(1, y2 - y1));
  }
}
async function loadWave(i) {
  const clip = arrangement.clips[i];
  if (!clip || clip.peaks || clip.waveLoading) return;
  clip.waveLoading = true;
  try {
    const data = await api('/api/studio/waveform', { method: 'POST', body: JSON.stringify(serializePattern(normalizePattern(clip.pattern))) });
    clip.peaks = data?.peaks || [];
    clip.waveLoading = false;
    requestAnimationFrame(() => drawClipWave(i));
  } catch (_) {
    clip.waveLoading = false;
  }
}

/* Redraw the selected arranger clip's preview after an edit (cheap; MIDI is
   synchronous, waveform reuses cached peaks). */
function refreshSelectedClipPreview() {
  if (sel?.area !== 'arr' || !arrGridEl) return;
  const i = sel.clipIndex;
  const clip = arrangement.clips[i];
  if (!clip) return;
  if (patternIsMelodic(clip.pattern)) drawClipMidi(i);
  else if (clip.peaks) drawClipWave(i);
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
    head.title = `Launch scene ${c + 1}`;
    head.addEventListener('click', () => launchScene(c));
    lchGridEl.appendChild(head);
  }
  const addScene = h('button', 'studio-launch-scene studio-launch-scene--add', '+ scene');
  addScene.type = 'button';
  addScene.style.gridColumn = String(s + 2);
  addScene.style.gridRow = '1';
  addScene.addEventListener('click', () => { launcher.scenes += 1; renderLauncher(); });
  lchGridEl.appendChild(addScene);

  /* track headers */
  for (let t = 0; t < n; t++) {
    const tr = launcher.tracks[t];
    const head = h('div', 'studio-arr-head');
    head.style.gridColumn = '1';
    head.style.gridRow = String(t + 2);
    head.style.setProperty('--track', trackColor(tr.color));
    head.appendChild(h('span', 'studio-track-stripe'));
    const name = h('span', 'studio-arr-name', tr.name);
    name.title = 'Double-click to rename';
    name.addEventListener('dblclick', (e) => { e.stopPropagation(); inlineRename(name, tr.name, (v) => { tr.name = v; renderLauncher(); }); });
    head.appendChild(name);
    const btns = h('span', 'studio-arr-head-btns');
    const mute = h('button', 'studio-ms', 'M');
    mute.type = 'button';
    mute.title = 'Mute';
    mute.classList.toggle('studio-ms--on', !!tr.mute);
    mute.addEventListener('click', (e) => { e.stopPropagation(); tr.mute = !tr.mute; ensureTrackAudio(tr); renderLauncher(); });
    const solo = h('button', 'studio-ms studio-ms--solo', 'S');
    solo.type = 'button';
    solo.title = 'Solo';
    solo.classList.toggle('studio-ms--on', soloTracks.has(tr.id));
    solo.addEventListener('click', (e) => { e.stopPropagation(); toggleSolo(tr.id); renderLauncher(); });
    const stopBtn = h('button', 'studio-head-btn', '■');
    stopBtn.type = 'button';
    stopBtn.title = 'Stop clips on this track';
    stopBtn.addEventListener('click', (e) => { e.stopPropagation(); stopLauncherTrack(tr.id, true); });
    const del = h('button', 'studio-head-btn', '×');
    del.type = 'button';
    del.title = 'Remove track';
    del.addEventListener('click', (e) => { e.stopPropagation(); removeLauncherTrack(t); });
    btns.append(mute, solo, stopBtn, del);
    head.appendChild(btns);
    lchGridEl.appendChild(head);
  }
  const addTrack = h('button', 'studio-arr-addtrack', '+ Add track');
  addTrack.type = 'button';
  addTrack.style.gridColumn = '1';
  addTrack.style.gridRow = String(n + 2);
  addTrack.addEventListener('click', () => openBrowser('instruments', 'addTrack'));
  lchGridEl.appendChild(addTrack);

  /* clip slots */
  for (let t = 0; t < n; t++) {
    const tr = launcher.tracks[t];
    for (let c = 0; c < s; c++) {
      const key = `${tr.id}:${c}`;
      const clip = launcher.clips[key];
      const cell = h('div', 'studio-launch-slot');
      cell.style.gridColumn = String(c + 2);
      cell.style.gridRow = String(t + 2);
      cell.style.setProperty('--track', trackColor(tr.color));
      if (clip) {
        cell.classList.add('studio-launch-clip');
        const lp = trackLoops[tr.id];
        const isThis = lp && lp.slotKey === key;
        if (isThis && lp.state === 'playing') cell.classList.add('studio-launch-clip--playing');
        if (isThis && lp.state === 'queued') cell.classList.add('studio-launch-clip--queued');
        if (sel?.area === 'lch' && sel.trackId === tr.id && sel.scene === c) cell.classList.add('studio-launch-clip--sel');
        const playIc = h('span', 'studio-launch-play');
        playIc.appendChild(icon('ui/play', { size: 10 }));
        cell.appendChild(playIc);
        cell.appendChild(h('span', 'studio-arr-clip-name', clip.title || clip.pattern?.title || tr.name));
        const send = h('button', 'studio-arr-clip-x', '↗');
        send.type = 'button';
        send.title = 'Send to Arrangement';
        send.addEventListener('click', (e) => { e.stopPropagation(); sendLauncherToArrangement(tr.id, c); });
        cell.appendChild(send);
        const rm = h('button', 'studio-arr-clip-x', '×');
        rm.type = 'button';
        rm.style.right = '22px';
        rm.title = 'Remove clip';
        rm.addEventListener('click', (e) => {
          e.stopPropagation();
          if (trackLoops[tr.id]?.slotKey === key) stopLauncherTrack(tr.id, false);
          delete launcher.clips[key];
          if (sel?.area === 'lch' && sel.trackId === tr.id && sel.scene === c) { sel = null; syncSelection(); }
          renderLauncher();
        });
        cell.appendChild(rm);
        cell.addEventListener('click', () => { void launchClip(t, c); });
        cell.addEventListener('dblclick', () => selectLauncherClip(tr.id, c, { openEditor: true }));
      } else {
        cell.classList.add('studio-launch-empty');
        cell.title = 'Add clip';
        cell.appendChild(h('span', null, '+'));
        cell.addEventListener('click', () => addLauncherClip(t, c));
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
  toast('Sent clip to Arrangement', { type: 'success' });
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
  toast('Sent clip to Launcher', { type: 'success' });
}

/* ── detail panel (Editor / Devices / Mixer) ────────────────── */

function renderDetail() {
  if (!detailEl) return;
  // Collapse only the body — the rail (page buttons + toggle) stays visible so
  // the panel can always be re-expanded.
  if (detailBodyEl) detailBodyEl.classList.toggle('hidden', !detailOpen);
  if (collapseBtn) collapseBtn.textContent = detailOpen ? '▾' : '▴';
  for (const [page, btn] of Object.entries(pageBtns)) btn.classList.toggle('studio-btn--on', detailPage === page);
  if (!detailOpen) return;
  if (editorPageEl) editorPageEl.classList.toggle('hidden', detailPage !== 'editor');
  if (devicesEl) devicesEl.classList.toggle('hidden', detailPage !== 'devices');
  if (mixerEl) mixerEl.classList.toggle('hidden', detailPage !== 'mixer');
  if (detailPage === 'editor') renderEditor();
  else if (detailPage === 'devices') renderDevices();
  else renderMixer();
  if (detailPage === 'editor') {
    const kind = current?.voices?.[selectedVoice]?.kind;
    setHint(!current ? 'Select a clip in the Arranger or Launcher to edit it'
      : MELODIC.has(kind) ? 'Click the grid to add notes · drag to move · drag the right edge to resize'
        : kind === 'drumkit' ? 'Click a pad row to place hits · Fill = euclidean rhythm'
          : 'Click cells to toggle hits · Fill = euclidean rhythm');
  } else {
    setHint(detailPage === 'devices'
      ? 'Drag knobs up/down · double-click a knob resets it'
      : 'Click a strip to select its track · M/S mute & solo');
  }
}

/* ── editor page ────────────────────────────────────────────── */

let edToolbarEl = null;

function renderEditor() {
  if (!gridEl) return;
  renderEditorToolbar();
  if (!current) {
    gridEl.textContent = '';
    gridEl.appendChild(h('div', 'studio-empty', 'Select a clip in the Arranger or Launcher to edit it.'));
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

  const name = document.createElement('input');
  name.type = 'text';
  name.className = 'studio-title studio-title--clip';
  name.value = current.title || 'Clip';
  name.maxLength = 120;
  name.title = 'Clip name';
  name.addEventListener('change', () => { current.title = name.value.trim() || 'Clip'; markDirty(); renderArranger(); renderLauncher(); });
  edToolbarEl.appendChild(name);

  edToolbarEl.appendChild(nativeSelect(['8', '16', '32', '64'].map((s) => [s, `${s} steps`]), String(current.steps), (val) => {
    current.steps = parseInt(val, 10);
    for (const v of current.voices) v.notes = (v.notes || []).filter((nt) => nt.step < current.steps);
    markDirty();
    renderEditor();
    renderArranger();
    renderLauncher();
  }, 'Pattern length'));

  edToolbarEl.appendChild(nativeSelect(TUNINGS, current.tuning, (val) => { current.tuning = val; markDirty(); renderEditor(); }, 'Tuning'));

  const v = current.voices[selectedVoice];
  if (v) {
    /* Euclidean fill — the studio's signature rhythm tool */
    const hitsSel = nativeSelect(Array.from({ length: current.steps + 1 }, (_, i) => [String(i), String(i)]), '4', () => {}, 'Euclidean hits');
    hitsSel.classList.add('studio-select--narrow');
    const rotField = numberField('rot', 0, () => {}, 'Rotation');
    const fill = h('button', 'studio-btn', 'Fill');
    fill.type = 'button';
    fill.title = 'Fill the selected voice with a Euclidean rhythm';
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
    const clear = h('button', 'studio-btn', 'Clear');
    clear.type = 'button';
    clear.title = 'Clear the selected voice';
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
  prev.title = 'Preview clip';
  prev.appendChild(icon(prevPlaying ? 'ui/stop' : 'ui/play', { size: 12 }));
  prev.addEventListener('click', () => void previewClipInEditor());
  edToolbarEl.appendChild(prev);

  const savePat = h('button', 'studio-btn', 'Save pattern');
  savePat.type = 'button';
  savePat.title = 'Save clip to the pattern library';
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
    head.title = `${KIND_LABELS[current.voices[v].kind]} — click to edit`;
    head.addEventListener('click', () => {
      selectedVoice = v;
      renderEditor();
      renderDevices();
    });
    const dot = h('span', 'studio-track-dot');
    head.appendChild(dot);
    head.appendChild(h('span', 'studio-track-name', KIND_INITIALS[current.voices[v].kind] || '?'));
    if (n > 1) {
      const rm = h('span', 'studio-track-x', '×');
      rm.title = 'Remove voice';
      rm.addEventListener('click', (e) => { e.stopPropagation(); removeVoice(v); });
      head.appendChild(rm);
    }
    gridEl.appendChild(head);
  }
  const add = h('button', 'studio-track-add', '+ voice');
  add.type = 'button';
  add.style.gridColumn = '1';
  add.style.gridRow = String(n + 2);
  add.title = 'Add voice';
  add.addEventListener('click', () => {
    if (current.voices.length >= 12) { toast('Max 12 voices', { type: 'error' }); return; }
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
    else vc.notes.push({ step: s, length: 1, degree: vc.degree, octave: vc.octave });
    recomputeRhythmFromNotes(vc);
  } else {
    const cells = rhythmToCells(vc.rhythm, current.steps);
    cells[s] = !cells[s];
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
  else v.notes.push({ step, length: 1, degree: pad, octave: 0 });
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
    bar.addEventListener('pointerdown', (e) => startNoteDrag(e, i, v));
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
      nt.degree = Math.max(PR_DEG_LO, Math.min(PR_DEG_HI, startDeg - Math.round(dy / PR_ROW)));
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
  out.push({ path: 'level', label: 'Level', min: 0, max: 2, step: 0.01, def: (DEFAULT_LEVEL[v.kind] ?? 0.5),
    get: () => (v.level != null ? v.level : (DEFAULT_LEVEL[v.kind] ?? 0.5)), set: (n) => { v.level = n; } });
  out.push({ path: 'pan', label: 'Pan', min: -1, max: 1, step: 0.01, def: 0,
    get: () => (v.pan != null ? v.pan : 0), set: (n) => { v.pan = n; } });
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
  markDirty();
  renderDevices();
}

function renderDevices() {
  if (!devicesEl || detailPage !== 'devices') return;
  devicesEl.textContent = '';
  autoKnobs = [];   // rebuild the automation-follow registry with fresh DOM
  if (!current) {
    devicesEl.appendChild(h('div', 'studio-empty', 'Select a clip to see its devices.'));
    return;
  }
  const v = current.voices[selectedVoice];
  if (!v) return;

  /* voice picker strip */
  const strip = h('div', 'studio-device-voices');
  current.voices.forEach((vc, i) => {
    const b = h('button', 'studio-device-voice', KIND_INITIALS[vc.kind] || '?');
    b.type = 'button';
    b.style.setProperty('--track', trackColor(i));
    b.title = KIND_LABELS[vc.kind];
    b.classList.toggle('studio-device-voice--on', i === selectedVoice);
    b.addEventListener('click', () => { selectedVoice = i; renderDevices(); renderEditor(); });
    strip.appendChild(b);
  });
  devicesEl.appendChild(strip);

  const chain = h('div', 'studio-chain');
  chain.style.setProperty('--track', trackColor(selectedVoice));

  /* instrument device */
  const inst = h('div', 'studio-device');
  inst.style.setProperty('--track', trackColor(selectedVoice));
  const instHead = h('div', 'studio-device-head');
  instHead.appendChild(h('span', 'studio-device-name', KIND_LABELS[v.kind]));
  const instBtns = h('span', 'studio-device-head-btns');
  const presetBtn = h('button', 'studio-head-btn', '▾');
  presetBtn.type = 'button';
  presetBtn.title = 'Presets';
  presetBtn.addEventListener('click', () => openBrowser('presets'));
  instBtns.appendChild(presetBtn);
  const sp = h('button', 'studio-head-btn', '✦');
  sp.type = 'button';
  sp.title = 'Save preset';
  sp.addEventListener('click', () => void savePreset(v).catch((e) => toast(e.message, { type: 'error' })));
  instBtns.appendChild(sp);
  instHead.appendChild(instBtns);
  inst.appendChild(instHead);
  const instBody = h('div', 'studio-device-body');
  instBody.appendChild(nativeSelect(KINDS.map((k) => [k, KIND_LABELS[k]]), v.kind, (val) => {
    v.kind = val; v.synth = {};
    markDirty();
    renderEditor();
    renderDevices();
  }, 'Instrument'));
  if (v.kind === 'bass') instBody.appendChild(nativeSelect(WAVES, v.wave, (val) => { v.wave = val; markDirty(); }, 'Wave'));
  if (MELODIC.has(v.kind)) {
    instBody.appendChild(numberField('deg', v.degree, (n) => { v.degree = n; markDirty(); renderEditor(); }, 'Scale degree'));
    instBody.appendChild(numberField('oct', v.octave, (n) => { v.octave = n; markDirty(); }, 'Octave'));
  }
  if (v.kind === 'drumkit') {
    renderPadGrid(v, instBody);
  } else {
    const groups = {};
    for (const p of (SYNTH[v.kind] || [])) { const g = PARAM_GROUP[p.key] || 'Other'; (groups[g] = groups[g] || []).push(p); }
    for (const g of GROUP_ORDER) {
      const params = groups[g];
      if (!params?.length) continue;
      instBody.appendChild(h('span', 'studio-rack-label', g));
      for (const p of params) {
        const value = v.synth[p.key] != null ? v.synth[p.key] : p.def;
        if (v.kind === 'synthme' && (p.key === 'o1w' || p.key === 'o2w')) {
          instBody.appendChild(nativeSelect([[0, 'Sine'], [1, 'Triangle'], [2, 'Saw'], [3, 'Square']].map(([w, l]) => [String(w), l]), String(Math.round(value)), (val) => { v.synth[p.key] = parseFloat(val); markDirty(); }, p.label));
        } else if (v.kind === 'synthme' && p.key === 'ftype') {
          instBody.appendChild(nativeSelect([['0', 'Lowpass'], ['1', 'Highpass'], ['2', 'Bandpass']], String(Math.round(value)), (val) => { v.synth[p.key] = parseFloat(val); markDirty(); }, p.label));
        } else {
          instBody.appendChild(automatableKnob(p.label, `voice.${selectedVoice}.${p.key}`, p.min, p.max, p.step, value, (n) => { v.synth[p.key] = n; markDirty(); }, p.def));
        }
      }
    }
  }
  inst.appendChild(instBody);
  chain.appendChild(inst);

  /* fx devices */
  v.fx.forEach((f, i) => {
    const d = h('div', 'studio-device');
    const head = h('div', 'studio-device-head');
    head.appendChild(h('span', 'studio-device-name', EFFECTS[f.kind]?.label || f.kind));
    const btns = h('span', 'studio-device-head-btns');
    const bypass = h('button', 'studio-device-bypass', f.bypass ? 'OFF' : 'ON');
    bypass.type = 'button';
    bypass.title = 'Bypass';
    bypass.classList.toggle('studio-device-bypass--on', !f.bypass);
    bypass.addEventListener('click', () => { f.bypass = !f.bypass; markDirty(); renderDevices(); });
    btns.appendChild(bypass);
    const rm = h('button', 'studio-head-btn', '×');
    rm.type = 'button';
    rm.title = 'Remove';
    rm.addEventListener('click', () => { v.fx.splice(i, 1); markDirty(); renderDevices(); });
    btns.appendChild(rm);
    head.appendChild(btns);
    d.appendChild(head);
    const body = h('div', 'studio-device-body');
    for (const p of (EFFECTS[f.kind]?.params || [])) {
      const value = f.params[p.key] != null ? f.params[p.key] : p.def;
      body.appendChild(automatableKnob(p.label, `voice.${selectedVoice}.fx.${i}.${p.key}`, p.min, p.max, p.step, value, (n) => { f.params[p.key] = n; markDirty(); }, p.def));
    }
    d.appendChild(body);
    chain.appendChild(d);
  });

  /* add fx */
  const add = h('div', 'studio-device studio-device--add');
  const addSel = nativeSelect(EFFECT_KINDS.map((k) => [k, EFFECTS[k].label]), 'distortion', () => {});
  const addBtn = h('button', 'studio-btn', '+ FX');
  addBtn.type = 'button';
  addBtn.addEventListener('click', () => {
    if (v.fx.length >= 8) { toast('Max 8 effects per voice', { type: 'error' }); return; }
    v.fx.push({ kind: addSel.value, params: {}, bypass: false });
    markDirty();
    renderDevices();
  });
  add.append(addSel, addBtn);
  chain.appendChild(add);

  /* output device */
  const out = h('div', 'studio-device');
  const outHead = h('div', 'studio-device-head');
  outHead.appendChild(h('span', 'studio-device-name', 'Output'));
  out.appendChild(outHead);
  const outBody = h('div', 'studio-device-body');
  outBody.appendChild(automatableKnob('Level', `voice.${selectedVoice}.level`, 0, 2, 0.01, v.level != null ? v.level : (DEFAULT_LEVEL[v.kind] ?? 0.5), (n) => { v.level = n; markDirty(); }, DEFAULT_LEVEL[v.kind] ?? 0.5));
  outBody.appendChild(automatableKnob('Pan', `voice.${selectedVoice}.pan`, -1, 1, 0.01, v.pan != null ? v.pan : (DEFAULT_PAN[v.kind] ?? 0), (n) => { v.pan = n; markDirty(); }, DEFAULT_PAN[v.kind] ?? 0));
  out.appendChild(outBody);
  chain.appendChild(out);

  /* master fx (pattern-level delay/reverb) */
  const master = h('div', 'studio-device studio-device--master');
  const mHead = h('div', 'studio-device-head');
  mHead.appendChild(h('span', 'studio-device-name', 'Master FX'));
  master.appendChild(mHead);
  const mBody = h('div', 'studio-device-body');
  for (const p of FX) {
    const value = current.fx[p.key] != null ? current.fx[p.key] : p.def;
    mBody.appendChild(automatableKnob(p.label, `master.${p.key}`, p.min, p.max, p.step, value, (n) => { current.fx[p.key] = n; markDirty(); }, p.def));
  }
  master.appendChild(mBody);
  chain.appendChild(master);

  devicesEl.appendChild(chain);

  /* macro rack — each macro drives a list of parameters with per-mapping depth */
  const macros = h('div', 'studio-rack');
  macros.appendChild(h('span', 'studio-rack-label', 'Macros'));
  const flat = flattenVoiceParams(v);
  for (let i = 0; i < 8; i++) {
    const mac = v.macros[i] || (v.macros[i] = { value: 0.5, entries: [] });
    const wrap = h('div', 'studio-macro');
    const head = h('div', 'studio-macro-head');
    head.appendChild(h('span', 'studio-macro-name', `M${i + 1}`));
    const k = knob(`M${i + 1}`, 0, 1, 0.01, macroValue(mac), (nv) => applyMacro(mac, flat, nv), 0.5);
    head.appendChild(k);
    wrap.appendChild(head);
    const list = h('div', 'studio-macro-entries');
    mac.entries.forEach((e, ei) => {
      const row = h('div', 'studio-macro-entry');
      row.appendChild(nativeSelect([['', '—'], ...flat.map((f) => [f.path, f.label])], e.path, (path) => {
        const t = flat.find((f) => f.path === path);
        e.path = path;
        e.amount = 1;
        e.base = t ? t.get() : 0;
        markDirty();
        renderDevices();
      }, 'Destination'));
      row.appendChild(knob('Amt', -1, 1, 0.01, e.amount != null ? e.amount : 1, (n) => { e.amount = n; markDirty(); }, 1));
      const rm = h('button', 'studio-head-btn', '×');
      rm.type = 'button';
      rm.title = 'Remove mapping';
      rm.addEventListener('click', () => { mac.entries.splice(ei, 1); markDirty(); renderDevices(); });
      row.appendChild(rm);
      list.appendChild(row);
    });
    const addEntry = h('button', 'studio-btn', '+');
    addEntry.type = 'button';
    addEntry.title = 'Add a parameter to this macro';
    addEntry.addEventListener('click', () => {
      const first = flat[0];
      mac.entries.push({ path: first ? first.path : '', amount: 1, base: first ? first.get() : 0 });
      markDirty();
      renderDevices();
    });
    list.appendChild(addEntry);
    wrap.appendChild(list);
    macros.appendChild(wrap);
  }
  devicesEl.appendChild(macros);
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
    b.addEventListener('click', () => { selectedPad = i; renderDevices(); });
    grid.appendChild(b);
  }
  container.appendChild(grid);
  const pad = pads[selectedPad] || { name: 'Pad', kind: 'kick' };
  const ed = h('div', 'studio-pad-editor');
  ed.appendChild(h('span', 'studio-rack-label', `Pad ${selectedPad + 1}`));
  const nameInput = document.createElement('input');
  nameInput.type = 'text';
  nameInput.className = 'studio-number';
  nameInput.value = pad.name;
  nameInput.addEventListener('change', () => { pad.name = nameInput.value || 'Pad'; markDirty(); renderDevices(); });
  ed.appendChild(nameInput);
  ed.appendChild(nativeSelect(KINDS.filter((k) => k !== 'drumkit').map((k) => [k, KIND_LABELS[k]]), pad.kind, (val) => { pad.kind = val; markDirty(); renderDevices(); }));
  container.appendChild(ed);
}

/* ── mixer page ─────────────────────────────────────────────── */

function renderMixer() {
  if (!mixerEl || detailPage !== 'mixer') return;
  mixerEl.textContent = '';
  mixerMeters = [];   // strips are rebuilt; re-register their meters
  /* One strip per track of each visible panel + MAIN. */
  const groups = [];
  if (panels.arranger) groups.push({ title: 'Arranger', list: arrangement.tracks, onChange: renderArranger });
  if (panels.launcher) groups.push({ title: 'Launcher', list: launcher.tracks, onChange: renderLauncher });
  if (!groups.length) groups.push({ title: 'Arranger', list: arrangement.tracks, onChange: renderArranger });

  for (const g of groups) {
    const sec = h('div', 'studio-mixer-group');
    sec.appendChild(h('span', 'studio-rack-label', g.title));
    const row = h('div', 'studio-mixer-row');
    g.list.forEach((tr) => {
      const strip = h('div', 'studio-mixer-strip');
      strip.style.setProperty('--track', trackColor(tr.color));
      strip.appendChild(h('span', 'studio-track-stripe'));
      const name = h('span', 'studio-mixer-name', tr.name);
      name.title = 'Double-click to rename';
      name.addEventListener('dblclick', () => inlineRename(name, tr.name, (v) => { tr.name = v; markDirty(); renderMixer(); g.onChange(); }));
      strip.appendChild(name);
      const ms = h('span', 'studio-mixer-ms');
      const m = h('button', 'studio-ms', 'M');
      m.type = 'button';
      m.classList.toggle('studio-ms--on', !!tr.mute);
      m.addEventListener('click', () => { tr.mute = !tr.mute; ensureTrackAudio(tr); markDirty(); renderMixer(); g.onChange(); });
      const s = h('button', 'studio-ms studio-ms--solo', 'S');
      s.type = 'button';
      s.classList.toggle('studio-ms--on', soloTracks.has(tr.id));
      s.addEventListener('click', () => { toggleSolo(tr.id); renderMixer(); renderArranger(); renderLauncher(); });
      ms.append(m, s);
      strip.appendChild(ms);
      strip.appendChild(knob('Pan', -1, 1, 0.01, tr.pan ?? 0, (n) => { tr.pan = n; ensureTrackAudio(tr); markDirty(); }, 0));
      strip.appendChild(fader(`${tr.name} Level`, tr.level ?? 0.8, (n) => { tr.level = n; ensureTrackAudio(tr); markDirty(); }));
      /* Launcher tracks carry a live analyser tap; arrangement audio is a
         single offline render, so those strips meter only via MAIN. */
      strip.appendChild(meterNode(g.title === 'Launcher' ? () => trackAudio[tr.id]?.analyser : () => null));
      row.appendChild(strip);
    });
    sec.appendChild(row);
    mixerEl.appendChild(sec);
  }

  const masterSec = h('div', 'studio-mixer-group');
  masterSec.appendChild(h('span', 'studio-rack-label', 'Main'));
  const master = h('div', 'studio-mixer-strip studio-mixer-strip--master');
  master.appendChild(h('span', 'studio-mixer-name', 'MAIN'));
  master.appendChild(fader('Main gain', arrangement.master ?? 0.9, (n) => { arrangement.master = n; markDirty(); }));
  master.appendChild(meterNode(() => analyser));
  masterSec.appendChild(master);
  mixerEl.appendChild(masterSec);
}

/* ── pop-up browser ─────────────────────────────────────────── */

let browserQuery = '';
const BROWSER_TABS = [['patterns', 'Patterns'], ['arrangements', 'Arrangements'], ['instruments', 'Instruments'], ['effects', 'Effects'], ['presets', 'Presets']];

function openBrowser(tab, mode = null) {
  browserTab = tab || browserTab;
  browserMode = mode ? { type: mode, area: panels.launcher && !panels.arranger ? 'lch' : 'arr' } : null;
  browserEl?.classList.remove('hidden');
  renderBrowser();
  browserEl?.querySelector('input')?.focus();
}
function closeBrowser() {
  browserEl?.classList.add('hidden');
  browserMode = null;
}
function toggleBrowser() {
  if (browserEl?.classList.contains('hidden')) openBrowser(browserTab);
  else closeBrowser();
}

/* ── SynthMe — build & save custom instruments ───────────────── */

const SYNTHME_WAVES = [[0, 'Sine'], [1, 'Triangle'], [2, 'Saw'], [3, 'Square']];
const SYNTHME_FILTERS = [[0, 'Lowpass'], [1, 'Highpass'], [2, 'Bandpass']];

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
  head.appendChild(h('span', 'studio-synthme-title', 'SynthMe — instrument creator'));
  const name = document.createElement('input');
  name.type = 'text';
  name.className = 'studio-title studio-title--clip';
  name.value = d.name;
  name.maxLength = 80;
  name.placeholder = 'Instrument name';
  name.addEventListener('change', () => { d.name = name.value.trim() || 'My Synth'; });
  head.appendChild(name);
  const preview = h('button', 'studio-btn studio-btn--play', '');
  preview.type = 'button';
  preview.title = auditionSource ? 'Stop preview' : 'Preview this instrument';
  preview.appendChild(icon(auditionSource ? 'ui/stop' : 'ui/play', { size: 12 }));
  preview.addEventListener('click', () => void previewSynthmeDraft());
  const reset = h('button', 'studio-btn', 'Reset');
  reset.type = 'button';
  reset.title = 'Reset the instrument to defaults';
  reset.addEventListener('click', () => { synthmeDraft = defaultSynthmeDraft(); renderSynthme(); });
  const close = h('button', 'studio-head-btn', '×');
  close.type = 'button';
  close.title = 'Back to Arranger/Launcher';
  close.addEventListener('click', () => togglePanel('synthme'));
  head.append(preview, reset, close);
  synthmeWrapEl.appendChild(head);

  const body = h('div', 'studio-synthme-body');

  /* oscillators + noise */
  body.appendChild(h('span', 'studio-rack-label', 'Oscillators & Noise'));
  const osc = h('div', 'studio-synthme-row');
  osc.appendChild(nativeSelect(SYNTHME_WAVES.map(([w, l]) => [String(w), l]), String(d.synth.o1w), (val) => { d.synth.o1w = parseInt(val, 10); }, 'Osc 1 wave'));
  osc.appendChild(nativeSelect(SYNTHME_WAVES.map(([w, l]) => [String(w), l]), String(d.synth.o2w), (val) => { d.synth.o2w = parseInt(val, 10); }, 'Osc 2 wave'));
  osc.appendChild(knob('Detune', -24, 24, 0.1, d.synth.detune, (n) => { d.synth.detune = n; }, 0.1));
  osc.appendChild(knob('Mix', 0, 1, 0.05, d.synth.mix, (n) => { d.synth.mix = n; }, 0.5));
  osc.appendChild(knob('Noise', 0, 1, 0.05, d.synth.noise, (n) => { d.synth.noise = n; }, 0));
  body.appendChild(osc);

  /* filter + drive */
  body.appendChild(h('span', 'studio-rack-label', 'Filter & Drive'));
  const filt = h('div', 'studio-synthme-row');
  filt.appendChild(nativeSelect(SYNTHME_FILTERS.map(([w, l]) => [String(w), l]), String(d.synth.ftype), (val) => { d.synth.ftype = parseInt(val, 10); }, 'Filter type'));
  filt.appendChild(knob('Cutoff', 20, 20000, 10, d.synth.cutoff, (n) => { d.synth.cutoff = n; }, 2000));
  filt.appendChild(knob('Res', 0.1, 20, 0.1, d.synth.res, (n) => { d.synth.res = n; }, 1));
  filt.appendChild(knob('Drive', 0.25, 24, 0.05, d.synth.drive, (n) => { d.synth.drive = n; }, 2));
  body.appendChild(filt);

  /* envelope */
  body.appendChild(h('span', 'studio-rack-label', 'Envelope'));
  const env = h('div', 'studio-synthme-row');
  env.appendChild(knob('Attack', 0.001, 5, 0.005, d.synth.attack, (n) => { d.synth.attack = n; }, 0.005));
  env.appendChild(knob('Decay', 0.001, 5, 0.01, d.synth.decay, (n) => { d.synth.decay = n; }, 0.2));
  env.appendChild(knob('Sustain', 0, 1, 0.05, d.synth.sustain, (n) => { d.synth.sustain = n; }, 0.6));
  env.appendChild(knob('Release', 0.001, 5, 0.01, d.synth.release, (n) => { d.synth.release = n; }, 0.3));
  body.appendChild(env);

  /* MIDI effects */
  body.appendChild(h('span', 'studio-rack-label', 'MIDI Effects'));
  const midiWrap = h('div', 'studio-synthme-fx');
  d.midi.forEach((m, i) => {
    const row = h('div', 'studio-synthme-fx-row');
    const sel = nativeSelect(MIDI_FX_KINDS.map((k) => [k, MIDI_FX[k].label]), m.kind, (val) => { m.kind = val; m.params = {}; renderSynthme(); }, 'MIDI effect');
    row.appendChild(sel);
    const rm = h('button', 'studio-head-btn', '×');
    rm.type = 'button';
    rm.title = 'Remove MIDI effect';
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
  const midiAddBtn = h('button', 'studio-btn', '+ MIDI FX');
  midiAddBtn.type = 'button';
  midiAddBtn.addEventListener('click', () => {
    if (d.midi.length >= 6) { toast('Max 6 MIDI effects', { type: 'error' }); return; }
    d.midi.push({ kind: midiAddSel.value, params: {} });
    renderSynthme();
  });
  midiAddRow.append(midiAddSel, midiAddBtn);
  midiWrap.appendChild(midiAddRow);
  body.appendChild(midiWrap);

  /* effects */
  body.appendChild(h('span', 'studio-rack-label', 'Effects'));
  const fxWrap = h('div', 'studio-synthme-fx');
  d.fx.forEach((f, i) => {
    const row = h('div', 'studio-synthme-fx-row');
    row.appendChild(h('span', 'studio-device-name', EFFECTS[f.kind]?.label || f.kind));
    const rm = h('button', 'studio-head-btn', '×');
    rm.type = 'button';
    rm.title = 'Remove effect';
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
  const addBtn = h('button', 'studio-btn', '+ FX');
  addBtn.type = 'button';
  addBtn.addEventListener('click', () => {
    if (d.fx.length >= 6) { toast('Max 6 effects', { type: 'error' }); return; }
    d.fx.push({ kind: addSel.value, params: {}, bypass: false });
    renderSynthme();
  });
  addRow.append(addSel, addBtn);
  fxWrap.appendChild(addRow);
  body.appendChild(fxWrap);

  synthmeWrapEl.appendChild(body);

  /* footer actions */
  const foot = h('div', 'studio-synthme-foot');
  const applyBtn = h('button', 'studio-btn', 'Apply to voice');
  applyBtn.type = 'button';
  applyBtn.title = 'Use this instrument on the selected voice';
  applyBtn.addEventListener('click', () => applySynthmeDraft());
  const saveBtn = h('button', 'studio-btn', 'Save instrument');
  saveBtn.type = 'button';
  saveBtn.title = 'Save as a reusable instrument';
  saveBtn.addEventListener('click', () => void saveSynthmeInstrument());
  foot.append(applyBtn, saveBtn);
  synthmeWrapEl.appendChild(foot);
}
function applySynthmeDraft() {
  const v = current?.voices[selectedVoice];
  if (!v) { toast('Select a clip first', { type: 'error' }); return; }
  v.kind = 'synthme';
  v.synth = { ...synthmeDraft.synth };
  v.fx = synthmeDraft.fx.map((f) => ({ kind: f.kind, params: { ...f.params }, bypass: !!f.bypass }));
  v.midi = synthmeDraft.midi.map((m) => ({ kind: m.kind, params: { ...m.params } }));
  markDirty();
  renderEditor();
  renderDevices();
  toast('SynthMe instrument applied', { type: 'success' });
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
    toast(e.message || 'Preview failed', { type: 'error' });
  }
}
async function saveSynthmeInstrument() {
  const d = synthmeDraft;
  if (!d.name.trim()) { toast('Name the instrument first', { type: 'error' }); return; }
  await api('/api/studio/presets', {
    method: 'POST',
    body: JSON.stringify({ kind: 'synthme', name: d.name.trim(), params: { synth: d.synth, midi: d.midi.map((m) => ({ kind: m.kind, params: m.params })), fx: d.fx.map((f) => ({ kind: f.kind, params: f.params, bypass: f.bypass })) } }),
  });
  await refreshPresets();
  renderBrowser();
  toast(`Saved instrument “${d.name.trim()}”`, { type: 'success' });
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
const GRID_MODULE_LABELS = { osc: 'Oscillator', noise: 'Noise', filter: 'Filter', drive: 'Drive', gain: 'Gain', mixer: 'Mixer', env: 'Envelope', lfo: 'LFO', out: 'Audio Out' };

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
    case 'osc': return [{ key: 'wave', label: 'Wave', min: 0, max: 3, step: 1, def: 2 }, { key: 'detune', label: 'Detune', min: -24, max: 24, step: 0.1, def: 0 }];
    case 'filter': return [{ key: 'type', label: 'Type', min: 0, max: 2, step: 1, def: 0 }, { key: 'cutoff', label: 'Cutoff', min: 20, max: 20000, step: 10, def: 2000 }, { key: 'res', label: 'Res', min: 0.1, max: 20, step: 0.1, def: 1 }];
    case 'drive': return [{ key: 'drive', label: 'Drive', min: 0.25, max: 24, step: 0.05, def: 2 }];
    case 'gain': return [{ key: 'level', label: 'Level', min: 0, max: 2, step: 0.01, def: 0.8 }];
    case 'mixer': return [{ key: 'balance', label: 'Balance', min: 0, max: 1, step: 0.05, def: 0.5 }];
    case 'env': return [{ key: 'attack', label: 'A', min: 0.001, max: 5, step: 0.005, def: 0.005 }, { key: 'decay', label: 'D', min: 0.001, max: 5, step: 0.01, def: 0.2 }, { key: 'sustain', label: 'S', min: 0, max: 1, step: 0.05, def: 0.6 }, { key: 'release', label: 'R', min: 0.001, max: 5, step: 0.01, def: 0.3 }];
    case 'lfo': return [{ key: 'rate', label: 'Rate', min: 0.05, max: 20, step: 0.01, def: 1 }, { key: 'depth', label: 'Depth', min: 0, max: 1, step: 0.01, def: 0.5 }, { key: 'wave', label: 'Wave', min: 0, max: 2, step: 1, def: 0 }];
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
function gridCableAt(portId) {
  return gridDraft.cables.find((c) => c.from[0] + ':' + c.from[1] === portId || c.to[0] + ':' + c.to[1] === portId);
}
function renderGrid() {
  if (!gridWrapEl) return;
  if (!gridDraft) gridDraft = gridDefaultDraft();
  gridWrapEl.textContent = '';
  const d = gridDraft;

  const head = h('div', 'studio-synthme-head');
  head.appendChild(h('span', 'studio-synthme-title', 'WaveMe'));
  const name = document.createElement('input');
  name.type = 'text';
  name.className = 'studio-title studio-title--clip';
  name.value = d.name;
  name.placeholder = 'Patch name';
  name.addEventListener('change', () => { d.name = name.value.trim() || 'My Grid'; });
  head.appendChild(name);
  const preview = h('button', 'studio-btn studio-btn--play', '');
  preview.type = 'button';
  preview.title = auditionSource ? 'Stop preview' : 'Preview patch';
  preview.appendChild(icon(auditionSource ? 'ui/stop' : 'ui/play', { size: 12 }));
  preview.addEventListener('click', () => void previewGridDraft());
  const apply = h('button', 'studio-btn', 'Apply to voice');
  apply.type = 'button';
  apply.addEventListener('click', () => applyGridDraft());
  const save = h('button', 'studio-btn', 'Save');
  save.type = 'button';
  save.addEventListener('click', () => void saveGridInstrument());
  const close = h('button', 'studio-head-btn', '×');
  close.type = 'button';
  close.title = 'Back to Arranger/Launcher';
  close.addEventListener('click', () => togglePanel('grid'));
  head.append(preview, apply, save, close);
  gridWrapEl.appendChild(head);

  /* palette */
  const palette = h('div', 'studio-grid-palette');
  for (const k of ['osc', 'noise', 'filter', 'drive', 'gain', 'mixer', 'env', 'lfo', 'out']) {
    const b = h('button', 'studio-btn studio-grid-palette-item', GRID_MODULE_LABELS[k]);
    b.type = 'button';
    b.title = `Add ${GRID_MODULE_LABELS[k]}`;
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

  const head = h('div', 'studio-grid-module-head', GRID_MODULE_LABELS[m.kind]);
  if (ports.precord) head.appendChild(h('span', 'studio-grid-precord', ports.precord === 'pitch' ? '♪' : '▮'));
  el.appendChild(head);

  /* inputs on the left, outputs on the right */
  ports.inputs.forEach((p, i) => el.appendChild(renderGridPort(m, p, true, i)));
  ports.outputs.forEach((p, i) => el.appendChild(renderGridPort(m, p, false, i)));

  /* params — always visible on the module face */
  const body = h('div', 'studio-grid-module-params');
  for (const p of gridModuleParams(m.kind)) {
    const value = m.params[p.key] != null ? m.params[p.key] : p.def;
    if ((m.kind === 'osc' && p.key === 'wave') || (m.kind === 'filter' && p.key === 'type') || (m.kind === 'lfo' && p.key === 'wave')) {
      const opts = p.key === 'wave' ? [[0, 'Sine'], [1, 'Tri'], [2, 'Saw'], [3, 'Sq']].map(([v, l]) => [String(v), l]) : [[0, 'LP'], [1, 'HP'], [2, 'BP']].map(([v, l]) => [String(v), l]);
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
    if (target && target.dataset.mid !== mid) {
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
  if (!v) { toast('Select a clip first', { type: 'error' }); return; }
  v.kind = 'grid';
  v.grid = gridDraftToPatch(gridDraft);
  v.synth = {};
  markDirty();
  renderEditor();
  renderDevices();
  toast('Grid patch applied', { type: 'success' });
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
  } catch (e) { toast(e.message || 'Preview failed', { type: 'error' }); }
}
function gridDraftToPatch(d) {
  return {
    modules: d.modules.map((m) => ({ id: m.id, kind: m.kind, params: { ...m.params } })),
    cables: d.cables.map((c) => ({ from: [c.from[0], c.from[1]], to: [c.to[0], c.to[1]] })),
  };
}
async function saveGridInstrument() {
  const d = gridDraft;
  if (!d.name.trim()) { toast('Name the patch first', { type: 'error' }); return; }
  await api('/api/studio/presets', {
    method: 'POST',
    body: JSON.stringify({ kind: 'grid', name: d.name.trim(), params: { grid: gridDraftToPatch(d) } }),
  });
  await refreshPresets();
  renderBrowser();
  toast(`Saved Grid patch “${d.name.trim()}”`, { type: 'success' });
}

function browserRow({ label, sub, color, onClick, onDblClick, actions = [] }) {
  const r = h('div', 'studio-browser-item');
  r.title = label;
  if (color) {
    const d = h('span', 'studio-track-dot');
    d.style.setProperty('--track', color);
    r.appendChild(d);
  }
  r.appendChild(h('span', 'studio-browser-label', label));
  if (sub) r.appendChild(h('span', 'studio-browser-sub', sub));
  for (const a of actions) r.appendChild(a);
  if (onClick) r.addEventListener('click', onClick);
  if (onDblClick) r.addEventListener('dblclick', onDblClick);
  return r;
}
function smallBtn(text, title, onClick) {
  const b = h('button', 'studio-head-btn', text);
  b.type = 'button';
  b.title = title;
  b.addEventListener('click', (e) => { e.stopPropagation(); onClick(e); });
  return b;
}

function renderBrowser() {
  if (!browserListEl || browserEl.classList.contains('hidden')) return;
  /* tab buttons */
  browserEl.querySelectorAll('.studio-browser-tab').forEach((b) => {
    b.classList.toggle('studio-btn--on', b.dataset.tab === browserTab);
  });
  browserListEl.textContent = '';
  const q = browserQuery.trim().toLowerCase();
  const match = (s) => !q || String(s).toLowerCase().includes(q);

  if (browserTab === 'patterns') {
    if (!tracks.length) { browserListEl.appendChild(h('div', 'studio-empty', 'No saved patterns yet — save one from the Editor toolbar.')); return; }
    for (const t of tracks) {
      if (!match(t.title)) continue;
      browserListEl.appendChild(browserRow({
        label: t.title,
        sub: `${Math.round(t.bpm)} BPM`,
        onClick: () => void playSaved(t.track_id),
        onDblClick: () => addSavedAsLauncherClip(t.track_id),
        actions: [
          smallBtn('+', 'Add to selected track', () => void addSavedAsClip(t.track_id)),
          smallBtn('×', 'Delete pattern', async () => {
            if (confirm(`Delete "${t.title}"?`)) await deleteTrack(t.track_id).catch((e) => toast(e.message, { type: 'error' }));
          }),
        ],
      }));
    }
    return;
  }

  if (browserTab === 'arrangements') {
    browserListEl.appendChild(browserRow({
      label: '+ New arrangement',
      onClick: () => {
        arrangement = blankArrangement();
        sel = arrangement.clips.length ? { area: 'arr', clipIndex: 0 } : null;
        syncSelection();
        dirty = false;
        if (titleInput) titleInput.value = arrangement.title;
        renderArranger();
      },
    }));
    if (!arrangements.length) browserListEl.appendChild(h('div', 'studio-empty', 'None saved.'));
    for (const a of arrangements) {
      if (!match(a.title)) continue;
      browserListEl.appendChild(browserRow({
        label: a.title,
        sub: `${Math.round(a.bpm)} BPM`,
        onClick: () => void loadArrangement(a.id).catch((e) => toast(e.message, { type: 'error' })),
        actions: [smallBtn('×', 'Delete arrangement', async () => {
          if (confirm(`Delete "${a.title}"?`)) await deleteArrangement(a.id).catch((e) => toast(e.message, { type: 'error' }));
        })],
      }));
    }
    return;
  }

  if (browserTab === 'instruments') {
    const v = current?.voices[selectedVoice];
    for (let i = 0; i < KINDS.length; i++) {
      const k = KINDS[i];
      if (!match(KIND_LABELS[k]) && !match(KIND_DESC[k])) continue;
      browserListEl.appendChild(browserRow({
        label: KIND_LABELS[k],
        sub: KIND_DESC[k],
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
      browserListEl.appendChild(h('div', 'studio-browser-section', 'SynthMe instruments'));
      for (const p of savedSynthme) {
        if (!match(p.name)) continue;
        browserListEl.appendChild(browserRow({
          label: p.name,
          sub: 'custom instrument',
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
      browserListEl.appendChild(h('div', 'studio-browser-section', 'WaveMe patches'));
      for (const p of savedGrid) {
        if (!match(p.name)) continue;
        browserListEl.appendChild(browserRow({
          label: p.name,
          sub: 'modular patch',
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
        onClick: () => {
          if (!v) { toast('Select a clip first', { type: 'error' }); return; }
          if (v.fx.length >= 8) { toast('Max 8 effects per voice', { type: 'error' }); return; }
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
    if (!v) { browserListEl.appendChild(h('div', 'studio-empty', 'Select a clip first.')); return; }
    const factory = FACTORY_PRESETS[v.kind] || [];
    for (const p of factory) {
      if (!match(p.name)) continue;
      browserListEl.appendChild(browserRow({ label: `✦ ${p.name}`, color: trackColor(selectedVoice), onClick: () => applyPreset(v, p) }));
    }
    for (const p of presets) {
      if (p.kind !== v.kind || !match(p.name)) continue;
      browserListEl.appendChild(browserRow({
        label: p.name,
        onClick: () => applyPreset(v, p),
        actions: [smallBtn('×', 'Delete preset', async () => {
          await api(`/api/studio/presets/${p.id}`, { method: 'DELETE' }).catch((e) => toast(e.message, { type: 'error' }));
          await refreshPresets();
          renderBrowser();
        })],
      }));
    }
    if (!factory.length && !presets.some((p) => p.kind === v.kind)) {
      browserListEl.appendChild(h('div', 'studio-empty', `No presets for ${KIND_LABELS[v.kind]} — save one from the device header.`));
    }
  }
}

async function fetchTrackConfig(id) {
  const data = await api(`/api/studio/${id}`);
  return data?.config || null;
}
async function addSavedAsClip(trackId) {
  const cfg = await fetchTrackConfig(trackId).catch(() => null);
  if (!cfg) { toast('Pattern not found', { type: 'error' }); return; }
  if (panels.launcher && !panels.arranger) {
    addConfigAsLauncherClip(cfg);
    return;
  }
  const tr = arrangement.tracks[0];
  if (!tr) { toast('No arrangement track', { type: 'error' }); return; }
  const dur = Math.max(1, Math.round((cfg.steps || 16) / 4));
  const start = arrangement.clips.filter((c) => c.track === tr.id).length * 4;
  const maxStart = Math.max(0, Math.round(arrangement.length_beats) - dur);
  arrangement.clips.push({ track: tr.id, start: Math.min(start, maxStart), pattern: normalizePattern(cfg) });
  markDirty();
  renderArranger();
}
async function addSavedAsLauncherClip(trackId) {
  const cfg = await fetchTrackConfig(trackId).catch(() => null);
  if (!cfg) { toast('Pattern not found', { type: 'error' }); return; }
  addConfigAsLauncherClip(cfg);
}
/* Drop the AI's just-created track into the launcher (and audition it).
   Safe to call before the tile is mounted — it parks the id until mount. */
async function consumePendingAiTrack() {
  const id = pendingAiTrackId;
  if (!id) return;
  if (!launcher) { pendingAiTrackId = id; return; }   // not mounted yet
  pendingAiTrackId = null;
  if (lastConsumedAiTrackId === id) return;           // already handled
  lastConsumedAiTrackId = id;
  const cfg = await fetchTrackConfig(id).catch(() => null);
  if (!cfg) return;
  addConfigAsLauncherClip(cfg);
  if (!panels.launcher) { panels.launcher = true; syncPanels(); }
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

async function exportWav() {
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
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = `${String(name).replace(/[^\w\- ]+/g, '').trim() || 'studio'}.wav`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 5000);
    setStatus(dirty ? 'dirty' : 'saved');
    toast('Exported WAV', { type: 'success' });
  } catch (e) {
    console.error('studio: export failed', e);
    toast(e.message || 'Export failed', { type: 'error' });
    setStatus('error');
  } finally {
    busy = false;
  }
}

/* ── panel toggling ─────────────────────────────────────────── */

function syncPanels() {
  const builder = panels.synthme || panels.grid;
  arrWrapEl?.classList.toggle('hidden', builder || !panels.arranger);
  lchWrapEl?.classList.toggle('hidden', builder || !panels.launcher);
  synthmeWrapEl?.classList.toggle('hidden', !panels.synthme);
  gridWrapEl?.classList.toggle('hidden', !panels.grid);
  arrToggleBtn?.classList.toggle('studio-transport--on', !builder && panels.arranger);
  lchToggleBtn?.classList.toggle('studio-transport--on', !builder && panels.launcher);
  if (titleInput) titleInput.value = arrangement.title;
  if (bpmInput) bpmInput.value = String(Math.round(arrangement.bpm));
  renderArranger();
  renderLauncher();
  if (detailPage === 'mixer') renderMixer();
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
  barEl = h('div', 'studio-bar');

  arrToggleBtn = h('button', 'studio-transport studio-transport--on');
  arrToggleBtn.type = 'button';
  arrToggleBtn.title = 'Show/hide the Arranger (timeline)';
  arrToggleBtn.appendChild(icon('ui/arranger', { size: 16 }));
  arrToggleBtn.addEventListener('click', () => togglePanel('arranger'));
  lchToggleBtn = h('button', 'studio-transport');
  lchToggleBtn.type = 'button';
  lchToggleBtn.title = 'Show/hide the Clip Launcher';
  lchToggleBtn.appendChild(icon('ui/launcher', { size: 16 }));
  lchToggleBtn.addEventListener('click', () => togglePanel('launcher'));

  const synthmeBtn = button({ variant: 'ghost', icon: 'ui/synthme', label: '', onClick: () => togglePanel('synthme') });
  synthmeBtn.classList.add('studio-transport');
  synthmeBtn.title = 'SynthMe — build and save custom instruments';

  const gridBtn = button({ variant: 'ghost', icon: 'ui/grid', label: '', onClick: () => togglePanel('grid') });
  gridBtn.classList.add('studio-transport');
  gridBtn.title = 'WaveMe — modular patch editor';

  const stopBtn = button({ variant: 'ghost', icon: 'ui/stop', label: '', onClick: () => { stopPlayback(); stopAllLauncher(); } });
  stopBtn.classList.add('studio-transport');
  stopBtn.title = 'Stop';

  playBtn = button({ variant: 'ghost', icon: 'ui/play', label: '', onClick: () => {
    if (arrPlaying) { stopPlayback(); return; }
    if (panels.arranger) void renderArrangementAndPlay();
    else launchScene(0);
  } });
  playBtn.classList.add('studio-transport', 'studio-transport--play');
  playBtn.title = 'Play (Arranger) / Launch scene 1 (Launcher)';

  loopBtn = button({ variant: 'ghost', icon: 'ui/loop', label: '', onClick: () => {
    arrLoop = !arrLoop;
    if (arrSource) arrSource.loop = arrLoop;
    updateTransport();
    renderArranger();
  } });
  loopBtn.classList.add('studio-transport');
  loopBtn.title = 'Loop arrangement';

  metroBtn = button({ variant: 'ghost', icon: 'ui/metronome', label: '', onClick: () => {
    metroOn = !metroOn;
    if (!metroOn) metroStop();
    else if (arrPlaying) metroStart(arrPlayStart, arrangement.bpm);
    else if (clockRunning) metroStart(clockT0, launcher.bpm);
    updateTransport();
  } });
  metroBtn.classList.add('studio-transport');
  metroBtn.title = 'Metronome';

  timeEl = h('span', 'studio-time');
  beatDotEl = h('span', 'studio-beat');
  beatDotEl.title = 'Beat indicator';

  bpmInput = document.createElement('input');
  bpmInput.type = 'number';
  bpmInput.className = 'studio-bpm';
  bpmInput.min = 40;
  bpmInput.max = 240;
  bpmInput.value = '120';
  bpmInput.title = 'BPM';
  withHint(bpmInput, 'Tempo');
  bpmInput.addEventListener('change', () => {
    const b = Math.min(240, Math.max(40, parseFloat(bpmInput.value) || 120));
    bpmInput.value = String(b);
    if (panels.arranger) arrangement.bpm = b;
    else {
      launcher.bpm = b;
      if (clockRunning && metroOn) { metroStop(); metroStart(clockT0, b); }
    }
    markDirty();
  });
  const bpmLabel = h('span', 'studio-bpm-label', 'BPM');

  titleInput = document.createElement('input');
  titleInput.type = 'text';
  titleInput.className = 'studio-title';
  titleInput.placeholder = 'Untitled';
  titleInput.maxLength = 120;
  titleInput.value = 'Untitled Arrangement';
  titleInput.title = 'Project title';
  titleInput.addEventListener('change', () => {
    if (panels.arranger) arrangement.title = titleInput.value.trim() || 'Untitled';
    else launcher.title = titleInput.value.trim() || 'Untitled';
    markDirty();
  });

  const saveBtn = button({ variant: 'ghost', icon: 'ui/save', label: '', onClick: () => {
    void saveArrangement()
      .then(() => { setStatus('saved'); toast('Project saved', { type: 'success' }); })
      .catch((e) => toast(e.message, { type: 'error' }));
  } });
  saveBtn.classList.add('studio-transport');
  saveBtn.title = 'Save project';

  const exportBtn = button({ variant: 'ghost', icon: 'ui/download', label: '', onClick: () => void exportWav() });
  exportBtn.classList.add('studio-transport');
  exportBtn.title = 'Export WAV';

  const browserBtn = button({ variant: 'ghost', icon: 'ui/search', label: '', onClick: toggleBrowser });
  browserBtn.classList.add('studio-transport');
  browserBtn.title = 'Browser (patterns, instruments, effects, presets)';

  statusEl = h('span', 'studio-status');
  saveDotEl = h('span', 'studio-save-dot');
  saveDotEl.setAttribute('aria-hidden', 'true');
  saveDotEl.title = 'Saved';

  barEl.append(arrToggleBtn, lchToggleBtn, synthmeBtn, gridBtn, h('span', 'studio-bar-sep'), stopBtn, playBtn, loopBtn, metroBtn,
    beatDotEl, timeEl, bpmInput, bpmLabel, titleInput, saveBtn, exportBtn, browserBtn, statusEl, saveDotEl);
  tileEl.appendChild(barEl);

  /* ── body: arranger + launcher side by side ── */
  bodyEl = h('div', 'studio-body');

  arrWrapEl = h('div', 'studio-panel studio-panel--arr');
  const arrScroll = h('div', 'studio-arr-scroll');
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
  for (const [page, label] of [['editor', 'Editor'], ['devices', 'Devices'], ['mixer', 'Mixer']]) {
    const b = h('button', 'studio-btn studio-detail-page', label);
    b.type = 'button';
    b.classList.toggle('studio-btn--on', detailPage === page);
    b.addEventListener('click', () => { detailPage = page; detailOpen = true; renderDetail(); });
    pageBtns[page] = b;
    rail.appendChild(b);
  }
  collapseBtn = h('button', 'studio-btn studio-detail-collapse', '▾');
  collapseBtn.type = 'button';
  collapseBtn.title = 'Show/hide detail panel';
  collapseBtn.addEventListener('click', () => { detailOpen = !detailOpen; renderDetail(); });
  rail.appendChild(collapseBtn);
  detailEl.appendChild(rail);

  detailBodyEl = h('div', 'studio-detail-body');
  const editorWrap = h('div', 'studio-editor-page');
  editorPageEl = editorWrap;
  edToolbarEl = h('div', 'studio-editor-toolbar');
  gridEl = h('div', 'studio-grid');
  pianoEl = h('div', 'studio-pianoroll-inline hidden');
  editorWrap.append(edToolbarEl, gridEl, pianoEl);
  devicesEl = h('div', 'studio-devices-page hidden');
  mixerEl = h('div', 'studio-mixer-page hidden');
  detailBodyEl.append(editorWrap, devicesEl, mixerEl);
  detailEl.appendChild(detailBodyEl);
  tileEl.appendChild(detailEl);

  /* ── footer (hints + live oscilloscope/spectrum/meter) ── */
  const footer = h('div', 'studio-footer');
  footerHintEl = h('span', 'studio-footer-hint');
  const scopeWrap = h('div', 'studio-scope');
  scopeWrap.title = 'Live output — oscilloscope + spectrum analyser';
  scopeTraceEl = document.createElement('canvas');
  scopeTraceEl.className = 'studio-scope-trace';
  scopeSpecEl = document.createElement('canvas');
  scopeSpecEl.className = 'studio-scope-spec';
  scopeWrap.append(scopeTraceEl, scopeSpecEl);
  const scopeMeterWrap = h('div', 'studio-meter studio-meter--scope');
  const scopeMeterFill = h('span', 'studio-meter-fill');
  const scopeMeterPeak = h('span', 'studio-meter-peak');
  scopeMeterWrap.append(scopeMeterFill, scopeMeterPeak);
  scopeWrap.appendChild(scopeMeterWrap);
  scopeMeter = { fill: scopeMeterFill, peakEl: scopeMeterPeak, level: 0, pk: 0 };
  footerParamEl = h('span', 'studio-footer-param');
  footer.append(footerHintEl, scopeWrap, footerParamEl);
  tileEl.appendChild(footer);
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
  bHead.appendChild(h('span', 'studio-browser-title', 'Browser'));
  const bClose = h('button', 'studio-head-btn', '×');
  bClose.type = 'button';
  bClose.title = 'Close';
  bClose.addEventListener('click', closeBrowser);
  bHead.appendChild(bClose);
  browserEl.appendChild(bHead);
  const search = searchBar({ placeholder: 'Search…', onInput: (v) => { browserQuery = v; renderBrowser(); } });
  browserEl.appendChild(search);
  const tabsRow = h('div', 'studio-browser-tabs');
  for (const [tab, label] of BROWSER_TABS) {
    const b = h('button', 'studio-btn studio-browser-tab', label);
    b.type = 'button';
    b.dataset.tab = tab;
    b.addEventListener('click', () => { browserTab = tab; renderBrowser(); });
    tabsRow.appendChild(b);
  }
  browserEl.appendChild(tabsRow);
  browserListEl = h('div', 'studio-browser-list');
  browserEl.appendChild(browserListEl);
  tileEl.appendChild(browserEl);

  tileEl.addEventListener('keydown', (e) => { if (e.key === 'Escape') closeBrowser(); });
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
  startScope();
  void consumePendingAiTrack().catch(() => {});

  return tileEl;
}

export function unmountStudioTile() {
  stopPlayback();
  stopAllLauncher();
  metroStop();
  stopScope();
  if (clockUiTimer) { clearInterval(clockUiTimer); clockUiTimer = 0; }
  if (beatFlashTimer) { clearTimeout(beatFlashTimer); beatFlashTimer = 0; }
  if (tileEl) { tileEl.remove(); tileEl = null; }
  barEl = timeEl = titleInput = bpmInput = statusEl = saveDotEl = playBtn = loopBtn = metroBtn = null;
  arrToggleBtn = lchToggleBtn = bodyEl = arrWrapEl = arrGridEl = arrPlayheadEl = null;
  lchWrapEl = lchGridEl = detailEl = detailBodyEl = gridEl = pianoEl = devicesEl = mixerEl = null;
  synthmeWrapEl = null;
  synthmeDraft = null;
  gridWrapEl = gridCanvasEl = gridSvgEl = null;
  gridSel = null;
  gridDraft = null;
  browserEl = browserListEl = footerHintEl = footerParamEl = edToolbarEl = editorPageEl = null;
  scopeTraceEl = scopeSpecEl = beatDotEl = null;
  scopeTimeData = scopeFreqData = scopePeaks = null;
  scopeMeter = null;
  mixerMeters = [];
  autoKnobs = [];
  auditionSource = null;
  lastClockBeat = -1;
  pageBtns = {};
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
  const deleted = studio.some((a) => a.action === 'studio_delete' && a.result === 'ok');
  const touchedId = (created || rendered)?.data?.track_id;
  if (touchedId) pendingAiTrackId = touchedId;
  void (async () => {
    await refreshTracks().catch(() => {});
    renderBrowser();
    if (deleted) {
      renderBrowser();
    } else if (touchedId) {
      await consumePendingAiTrack().catch(() => {});
    }
  })();
}

export default {
  name: STUDIO_PLUGIN,
  icon: 'ui/play',
  mount: mountStudioTile,
  unmount: unmountStudioTile,
  getElement: getStudioTileElement,
  wireEvents: wireStudioEvents,
};

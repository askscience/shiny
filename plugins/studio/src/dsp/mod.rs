//! The Studio DSP core — a self-contained, allocation-free audio engine.
//!
//! Everything here is written for *offline* rendering (a whole arrangement in
//! one pass) while staying block-based so the same code can drive realtime
//! preview later. Design rules:
//!
//! * **No per-sample dynamic dispatch.** Instruments, effects and grid modules
//!   are plain structs with `match` on a small enum where needed. This keeps
//!   the inner loops tight and branch-predictable.
//! * **f64 state, f32 signal.** Phase accumulators, envelopes and filter state
//!   are `f64` (no drift over a 10-minute render); buffers are `f32` (half the
//!   memory traffic).
//! * **Deterministic.** Every random source is a seeded xorshift so a config
//!   always renders bit-identically.
//! * **Alias-conscious.** Oscillators are polyBLEP band-limited; nonlinear
//!   stages (distortion, drums, limiter) run through a 2× oversampled path.

pub mod delay;
pub mod drums;
pub mod env;
pub mod filter;
pub mod fx;
pub mod limiter;
pub mod loudness;
pub mod lfo;
pub mod noise;
pub mod osc;
pub mod reverb;
pub mod synth;
pub mod util;

/// The engine renders at 44.1 kHz (CD rate) — the WAV contract the DB and the
/// browser already speak. Nonlinear stages oversample internally.
pub const SR: f64 = 44100.0;

/// Processing block size. 64 frames ≈ 1.45 ms at 44.1 kHz: small enough that
/// control-rate modulation is smooth, large enough to amortise loop overhead.
pub const BLOCK: usize = 64;

/// Default polyphony ceilings. Drums are one-shot so a single voice per pad is
/// enough; melodic instruments get real polyphony (chords!).
pub const POLY_DEFAULT: usize = 12;
pub const POLY_DRUM: usize = 3;

//! Loudness analysis (ITU-R BS.1770-4 / EBU R128).
//!
//! The engine uses this to *measure* a finished render and, when the caller
//! asks for a target, to trim it into a professional loudness window. This is
//! what stops AI-composed tracks from landing 8 dB quieter than everything
//! else in the user's library.

use super::util::gain_to_db;

/// One RBJ biquad, direct form 1 — analysis only, so clarity beats speed.
#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    fn high_shelf(f0: f64, gain_db: f64, q: f64, sr: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * f0 / sr;
        let (sn, cs) = (w0.sin(), w0.cos());
        let beta = sn * (a * a + 1.0).sqrt() / q;
        // Alpha for a shelf uses the standard RBJ formulation.
        let alpha = sn / (2.0 * q);
        let _ = beta;
        let b0 = a * ((a + 1.0) + (a - 1.0) * cs + 2.0 * a.sqrt() * alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cs);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cs - 2.0 * a.sqrt() * alpha);
        let a0 = (a + 1.0) - (a - 1.0) * cs + 2.0 * a.sqrt() * alpha;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cs);
        let a2 = (a + 1.0) - (a - 1.0) * cs - 2.0 * a.sqrt() * alpha;
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, ..Default::default() }
    }

    fn high_pass(f0: f64, q: f64, sr: f64) -> Self {
        let w0 = 2.0 * std::f64::consts::PI * f0 / sr;
        let (sn, cs) = (w0.sin(), w0.cos());
        let alpha = sn / (2.0 * q);
        let b0 = (1.0 + cs) / 2.0;
        let b1 = -(1.0 + cs);
        let b2 = (1.0 + cs) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cs;
        let a2 = 1.0 - alpha;
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, ..Default::default() }
    }

    #[inline]
    fn tick(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Measured properties of a render.
#[derive(Debug, Clone, Copy)]
pub struct LoudnessReport {
    /// Integrated loudness in LUFS (BS.1770-4 gated).
    pub integrated_lufs: f64,
    /// Loudness range (LRA) in LU — 10th–95th percentile spread of gated blocks.
    pub range_lu: f64,
    /// Sample peak, linear.
    pub peak: f32,
    /// RMS level in dBFS.
    pub rms_db: f64,
    /// Crest factor in dB (peak − RMS).
    pub crest_db: f64,
    /// Fraction of samples at or above 0.999 (clipping detector).
    pub clipped: f64,
}

/// Measure a stereo (or mono) render.
pub fn measure(channels: &[Vec<f32>], sample_rate: f64) -> LoudnessReport {
    let n = channels.first().map(|c| c.len()).unwrap_or(0);
    let mut peak = 0.0f32;
    let mut sum_sq = 0.0f64;
    let mut count = 0usize;
    let mut clipped = 0usize;
    let mut kf: Vec<(Biquad, Biquad)> = (0..channels.len().max(1))
        .map(|_| {
            (
                Biquad::high_shelf(1681.974_450_955_533, 3.999_843_853_973_347, 0.707_175_236_955_419_6, sample_rate),
                Biquad::high_pass(38.135_470_876_024_44, 0.500_327_037_323_877_3, sample_rate),
            )
        })
        .collect();

    // Per-400 ms block mean square, accumulated on the K-weighted signal.
    let block = (0.4 * sample_rate).round() as usize;
    let step = (0.1 * sample_rate).round() as usize;
    let mut block_loudness: Vec<f64> = Vec::new();

    if n == 0 || block == 0 {
        return LoudnessReport { integrated_lufs: -70.0, range_lu: 0.0, peak: 0.0, rms_db: -120.0, crest_db: 0.0, clipped: 0.0 };
    }

    // K-weight each channel into a scratch buffer.
    let mut weighted: Vec<Vec<f64>> = vec![Vec::with_capacity(n); channels.len().max(1)];
    for (ci, ch) in channels.iter().enumerate() {
        let (s1, s2) = &mut kf[ci];
        for &s in ch {
            let x = s as f64;
            weighted[ci].push(s2.tick(s1.tick(x)));
            let a = s.abs();
            if a > peak {
                peak = a;
            }
            sum_sq += x * x;
            count += 1;
            if a >= 0.999 {
                clipped += 1;
            }
        }
    }

    let mut pos = 0usize;
    while pos + block <= n {
        let mut sum = 0.0f64;
        for ch in &weighted {
            let mut ms = 0.0f64;
            for &s in &ch[pos..pos + block] {
                ms += s * s;
            }
            sum += ms / block as f64;
        }
        let l = -0.691 + 10.0 * sum.max(1e-12).log10();
        block_loudness.push(l);
        pos += step;
    }

    // Absolute gate at -70 LUFS.
    let above_abs: Vec<f64> = block_loudness.iter().copied().filter(|l| *l > -70.0).collect();
    let integrated = if above_abs.is_empty() {
        -70.0
    } else {
        // Relative gate: 10 LU below the mean of the absolute-gated blocks.
        let mean_pow: f64 = above_abs.iter().map(|l| 10f64.powf(l / 10.0)).sum::<f64>() / above_abs.len() as f64;
        let rel = -0.691 + 10.0 * mean_pow.max(1e-12).log10() - 10.0;
        let kept: Vec<f64> = above_abs.iter().copied().filter(|l| *l > rel).collect();
        if kept.is_empty() {
            -70.0
        } else {
            let p: f64 = kept.iter().map(|l| 10f64.powf(l / 10.0)).sum::<f64>() / kept.len() as f64;
            -0.691 + 10.0 * p.max(1e-12).log10()
        }
    };

    // Loudness range: spread between the 10th and 95th percentile blocks.
    let mut sorted = block_loudness.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let range = if sorted.len() >= 2 {
        let lo = sorted[(sorted.len() as f64 * 0.10) as usize];
        let hi = sorted[(sorted.len() as f64 * 0.95).min((sorted.len() - 1) as f64) as usize];
        (hi - lo).max(0.0)
    } else {
        0.0
    };

    let rms = (sum_sq / count.max(1) as f64).sqrt();
    LoudnessReport {
        integrated_lufs: integrated,
        range_lu: range,
        peak,
        rms_db: gain_to_db(rms as f32) as f64,
        crest_db: gain_to_db(peak.max(1e-9)) as f64 - gain_to_db(rms as f32) as f64,
        clipped: clipped as f64 / count.max(1) as f64,
    }
}

/// Gain (linear) needed to move `current_lufs` to `target_lufs`.
pub fn normalization_gain(current_lufs: f64, target_lufs: f64) -> f64 {
    10f64.powf((target_lufs - current_lufs) / 20.0)
}

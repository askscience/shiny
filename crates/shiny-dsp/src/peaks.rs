//! Min/max peak envelopes for waveform drawing.
//!
//! One pass over the samples, one bucket per output column. Buckets are
//! equal slices of the input (`b·n/buckets`), so every sample lands in
//! exactly one bucket and the envelope never skips a transient.

/// Min/max over `buckets` equal slices, every channel folded together.
/// `buckets` is clamped to 16..=4096.
pub fn peaks<C: AsRef<[f32]>>(channels: &[C], buckets: usize) -> Vec<(f32, f32)> {
    let n = buckets.clamp(16, 4096);
    let frames = channels.first().map(|c| c.as_ref().len()).unwrap_or(0);
    let mut out = Vec::with_capacity(n);
    for b in 0..n {
        let s = b * frames / n;
        let e = ((b + 1) * frames / n).max(s + 1).min(frames);
        let (mut mn, mut mx) = (0.0f32, 0.0f32);
        for ch in channels {
            let ch = ch.as_ref();
            for &x in &ch[s.min(ch.len())..e.min(ch.len())] {
                mn = mn.min(x);
                mx = mx.max(x);
            }
        }
        out.push((mn, mx));
    }
    out
}

/// Streaming peaks for long media: feed interleaved `i16` samples of any
/// length in chunks, get one `(min, max)` pair per `per_bucket` frames.
/// Used to build a whole recording's envelope without holding it in memory.
pub struct PeakStream {
    channels: usize,
    per_bucket: usize,
    filled: usize,
    mn: i16,
    mx: i16,
    out: Vec<(i16, i16)>,
}

impl PeakStream {
    pub fn new(channels: usize, per_bucket: usize) -> Self {
        Self {
            channels: channels.max(1),
            per_bucket: per_bucket.max(1),
            filled: 0,
            mn: 0,
            mx: 0,
            out: Vec::new(),
        }
    }

    pub fn push(&mut self, interleaved: &[i16]) {
        for frame in interleaved.chunks_exact(self.channels) {
            for &x in frame {
                self.mn = self.mn.min(x);
                self.mx = self.mx.max(x);
            }
            self.filled += 1;
            if self.filled == self.per_bucket {
                self.out.push((self.mn, self.mx));
                (self.filled, self.mn, self.mx) = (0, 0, 0);
            }
        }
    }

    /// The envelope, including a final partial bucket.
    pub fn finish(mut self) -> Vec<(i16, i16)> {
        if self.filled > 0 {
            self.out.push((self.mn, self.mx));
        }
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sample_lands_in_a_bucket() {
        let mut x = vec![0.0f32; 1000];
        x[999] = 0.9;
        x[0] = -0.5;
        let p = peaks(&[&x[..]], 16);
        assert_eq!(p.len(), 16);
        assert_eq!(p[0].0, -0.5);
        assert_eq!(p[15].1, 0.9);
    }

    #[test]
    fn stream_matches_buckets() {
        let mut s = PeakStream::new(2, 3);
        s.push(&[1, -1, 2, -2, 3, -3, 4, 0]);
        s.push(&[-7, 7]);
        assert_eq!(s.finish(), vec![(-3, 3), (-7, 7)]);
    }
}

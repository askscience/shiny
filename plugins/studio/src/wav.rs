//! Minimal WAV encoder: planar f32 buffers → interleaved PCM RIFF/WAVE.
//!
//! The engine renders into planar channel buffers (`Vec<Vec<f32>>`,
//! channel-major); this tiny dependency-free encoder writes them as 16-bit
//! PCM by default, or 24-bit PCM on request (`wav_bits`). 16-bit stays the
//! default so existing rows and the browser contract are unchanged.

/// Encode planar samples (one `Vec<f32>` per channel) as a 16-bit PCM WAV file.
pub fn encode_wav(samples: &[Vec<f32>], sample_rate: u32) -> Vec<u8> {
    encode_wav_bits(samples, sample_rate, 16)
}

/// Encode planar samples as a WAV file with the given bit depth (16 or 24).
///
/// One input channel encodes as a mono file; two channels are interleaved
/// L/R. Samples are clamped to `[-1.0, 1.0]`; the 16-bit path adds TPDF
/// dither (±1 LSB triangular) so quiet tails fade into a smooth noise floor
/// at −96 dBFS instead of turning into gritty truncation distortion.
pub fn encode_wav_bits(samples: &[Vec<f32>], sample_rate: u32, bits: u16) -> Vec<u8> {
    let bits: u16 = match bits {
        24 => 24,
        _ => 16,
    };
    let bytes_per_sample = (bits / 8) as usize;
    let channels = if samples.len() >= 2 { 2 } else { 1 };
    let frames = samples.iter().map(|c| c.len()).max().unwrap_or(0);

    let data_len = frames * channels * bytes_per_sample;
    let mut out = Vec::with_capacity(44 + data_len);

    // RIFF header
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");

    // fmt chunk (PCM)
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&(channels as u16).to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(
        &((sample_rate as usize * channels * bytes_per_sample) as u32).to_le_bytes(),
    ); // byte rate
    out.extend_from_slice(&((channels * bytes_per_sample) as u16).to_le_bytes()); // block align
    out.extend_from_slice(&bits.to_le_bytes()); // bits per sample

    // data chunk
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());

    let l = samples.first().map(|c| c.as_slice()).unwrap_or(&[]);
    let r = if channels == 2 {
        samples.get(1).map(|c| c.as_slice()).unwrap_or(&[])
    } else {
        l
    };

    let mut dither = Dither::new(0x0D17_4E55);
    for i in 0..frames {
        let li = l.get(i).copied().unwrap_or(0.0);
        write_sample(&mut out, li, bits, &mut dither);
        // Only stereo interleaves a second sample — a mono file must write
        // exactly one sample per frame or the data chunk outgrows its header.
        if channels == 2 {
            let ri = r.get(i).copied().unwrap_or(0.0);
            write_sample(&mut out, ri, bits, &mut dither);
        }
    }

    out
}

/// Deterministic TPDF dither source (sum of two uniform variables).
struct Dither(u64);

impl Dither {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    #[inline]
    fn uniform(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / 9_007_199_254_740_992.0
    }

    /// Triangular noise in (−1, 1) LSB.
    #[inline]
    fn tpdf(&mut self) -> f64 {
        self.uniform() - self.uniform()
    }
}

#[inline]
fn write_sample(out: &mut Vec<u8>, s: f32, bits: u16, dither: &mut Dither) {
    let s = if s.is_finite() { s.clamp(-1.0, 1.0) as f64 } else { 0.0 };
    if bits == 24 {
        let v = (s * 8_388_607.0).round() as i32;
        out.extend_from_slice(&v.to_le_bytes()[0..3]);
    } else {
        // Digital silence stays digital silence (no dither on exact zeros).
        let v = if s == 0.0 { 0.0 } else { (s * 32_767.0 + dither.tpdf()).round() };
        out.extend_from_slice(&(v.clamp(-32_768.0, 32_767.0) as i16).to_le_bytes());
    }
}

/// A decoded WAV file: planar `f32` channels at the file's own rate.
#[derive(Debug, Clone)]
pub struct Decoded {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl Decoded {
    pub fn frames(&self) -> usize {
        self.channels.first().map(|c| c.len()).unwrap_or(0)
    }
}

/// Decode a RIFF/WAVE file: PCM 8/16/24/32-bit, IEEE float 32/64-bit, and
/// `WAVE_FORMAT_EXTENSIBLE` wrappers of those. Any channel count is read;
/// callers usually keep the first two.
pub fn decode_wav(bytes: &[u8]) -> Result<Decoded, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file (only .wav audio can be imported)".into());
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut data: Option<&[u8]> = None;
    let mut i = 12usize;
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let size = u32_at(i + 4) as usize;
        let body_start = i + 8;
        let body_end = body_start.saturating_add(size).min(bytes.len());
        if id == b"fmt " && body_end >= body_start + 16 {
            let mut tag = u16_at(body_start);
            let ch = u16_at(body_start + 2);
            let sr = u32_at(body_start + 4);
            let bits = u16_at(body_start + 14);
            if tag == 0xFFFE && body_end >= body_start + 26 {
                // Extensible: the real format tag is the sub-format GUID's
                // first two bytes.
                tag = u16_at(body_start + 24);
            }
            fmt = Some((tag, ch, sr, bits));
        } else if id == b"data" {
            data = Some(&bytes[body_start..body_end]);
        }
        i = body_start.saturating_add(size + (size & 1));
    }
    let (tag, ch, sr, bits) = fmt.ok_or("WAV file has no fmt chunk")?;
    let data = data.ok_or("WAV file has no data chunk")?;
    if ch == 0 || sr == 0 {
        return Err("WAV file has no channels".into());
    }
    let ch = ch as usize;
    let bps = (bits as usize).div_ceil(8);
    if bps == 0 {
        return Err("WAV file has an invalid bit depth".into());
    }
    let frames = data.len() / (bps * ch);
    let mut out: Vec<Vec<f32>> = (0..ch).map(|_| Vec::with_capacity(frames)).collect();
    for f in 0..frames {
        for (c, chan) in out.iter_mut().enumerate() {
            let o = (f * ch + c) * bps;
            let b = &data[o..o + bps];
            let v = match (tag, bits) {
                (1, 8) => (b[0] as f32 - 128.0) / 128.0,
                (1, 16) => i16::from_le_bytes([b[0], b[1]]) as f32 / 32_768.0,
                (1, 24) => (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
                (1, 32) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
                (3, 32) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                (3, 64) => f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f32,
                _ => return Err(format!("unsupported WAV encoding (format {tag}, {bits}-bit)")),
            };
            chan.push(if v.is_finite() { v } else { 0.0 });
        }
    }
    Ok(Decoded { sample_rate: sr, channels: out })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_valid_header() {
        let mono = vec![vec![0.0f32, 0.5, -0.5]];
        let bytes = encode_wav(&mono, 44100);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        // 1 channel, 44100 Hz, 16-bit
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 1);
        assert_eq!(u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]), 44100);
        assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 16);
        assert_eq!(&bytes[36..40], b"data");
        // Mono: exactly one i16 per frame — written bytes must match the
        // header's data length (they used to be double).
        let data_len = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as usize;
        assert_eq!(data_len, 3 * 2);
        assert_eq!(bytes.len(), 44 + data_len);
    }

    #[test]
    fn stereo_data_length() {
        let stereo = vec![vec![0.5f32; 100], vec![-0.5f32; 100]];
        let bytes = encode_wav(&stereo, 48000);
        let data_len = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as usize;
        assert_eq!(data_len, 100 * 2 * 2);
        assert_eq!(bytes.len(), 44 + data_len);
    }

    #[test]
    fn round_trips_through_the_decoder() {
        let stereo = vec![vec![0.25f32, -0.5, 0.0], vec![0.0f32, 0.75, -1.0]];
        for bits in [16u16, 24] {
            let bytes = encode_wav_bits(&stereo, 48_000, bits);
            let d = decode_wav(&bytes).expect("decodes");
            assert_eq!(d.sample_rate, 48_000);
            assert_eq!(d.channels.len(), 2);
            for c in 0..2 {
                for i in 0..3 {
                    assert!((d.channels[c][i] - stereo[c][i]).abs() < 2e-4, "bits {bits}");
                }
            }
        }
        // Exact zeros are never dithered.
        let silent = encode_wav_bits(&[vec![0.0f32; 64]], 44_100, 16);
        assert!(silent[44..].iter().all(|b| *b == 0));
    }

    #[test]
    fn writes_valid_24bit_header() {
        let mono = vec![vec![0.0f32, 0.5, -0.5]];
        let bytes = encode_wav_bits(&mono, 48000, 24);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        // 1 channel, 48000 Hz, 24-bit
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 1);
        assert_eq!(
            u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
            48000
        );
        assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 24);
        // block align = channels * 3 bytes
        assert_eq!(u16::from_le_bytes([bytes[32], bytes[33]]), 3);
        let data_len = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as usize;
        assert_eq!(data_len, 3 * 3);
        assert_eq!(bytes.len(), 44 + data_len);
    }
}

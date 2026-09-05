//! Minimal WAV encoder: planar f32 buffers → interleaved 16-bit PCM RIFF/WAVE.
//!
//! trem-mio (the trem crate's WAV/FLAC I/O) is not published on crates.io, so
//! the plugin owns a tiny, dependency-free encoder. We render stereo from
//! trem's planar output (`Vec<Vec<f32>>`, channel-major).

/// Encode planar samples (one `Vec<f32>` per channel) as a 16-bit PCM WAV file.
///
/// Mono input is duplicated to both channels; 2 channels are interleaved
/// L/R as expected. Samples are soft-clipped to `[-1.0, 1.0]` before quantizing.
pub fn encode_wav(samples: &[Vec<f32>], sample_rate: u32) -> Vec<u8> {
    let channels = if samples.len() >= 2 { 2 } else { 1 };
    let frames = samples.iter().map(|c| c.len()).max().unwrap_or(0);

    let data_len = frames * channels * 2; // 16-bit = 2 bytes per sample
    let mut out = Vec::with_capacity(44 + data_len);

    // RIFF header
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");

    // fmt chunk (16-byte PCM)
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&(channels as u16).to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&((sample_rate * channels as u32 * 2) as u32).to_le_bytes()); // byte rate
    out.extend_from_slice(&((channels * 2) as u16).to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    // data chunk
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());

    let l = samples.first().map(|c| c.as_slice()).unwrap_or(&[]);
    let r = if channels == 2 {
        samples.get(1).map(|c| c.as_slice()).unwrap_or(&[])
    } else {
        l
    };

    for i in 0..frames {
        let li = l.get(i).copied().unwrap_or(0.0).clamp(-1.0, 1.0);
        let ri = r.get(i).copied().unwrap_or(0.0).clamp(-1.0, 1.0);
        out.extend_from_slice(&((li * 32767.0) as i16).to_le_bytes());
        out.extend_from_slice(&((ri * 32767.0) as i16).to_le_bytes());
    }

    out
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
    }

    #[test]
    fn stereo_data_length() {
        let stereo = vec![vec![0.5f32; 100], vec![-0.5f32; 100]];
        let bytes = encode_wav(&stereo, 48000);
        let data_len = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as usize;
        assert_eq!(data_len, 100 * 2 * 2);
        assert_eq!(bytes.len(), 44 + data_len);
    }
}

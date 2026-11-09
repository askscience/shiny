//! A minimal Matroska writer for raw NV12 frames with real timestamps.
//!
//! ffmpeg's rawvideo pipe input has no timestamps, so every tick must send a
//! frame and the encoder pays for each copy of an unchanged screen (x264
//! spends tens of ms analysing even a static 1080p frame). This stream
//! carries a timestamp per frame, so the engine sends a frame only when the
//! picture changed and the encoder idles while the screen does.
//!
//! Layout: EBML header, a live Segment of unknown size with Info and one
//! video track (`V_UNCOMPRESSED`, colour space `NV12`), then one Cluster per
//! frame holding its timestamp and a single keyframe SimpleBlock. The frame
//! bytes are written straight from the caller's buffer, never copied.

use std::io::{self, Write};

/// Timestamps are in microseconds.
const TIMESTAMP_SCALE_NS: u64 = 1_000;

fn vint_size(n: u64) -> [u8; 8] {
    // Always the 8-byte form: simple, and the size of every element is known
    // before its body is written.
    let mut b = n.to_be_bytes();
    b[0] = 0x01;
    b
}

fn uint(id: &[u8], v: u64, out: &mut Vec<u8>) {
    let bytes = v.to_be_bytes();
    let skip = bytes.iter().take(7).take_while(|&&b| b == 0).count();
    element(id, &bytes[skip..], out);
}

fn element(id: &[u8], body: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(id);
    out.extend_from_slice(&vint_size(body.len() as u64));
    out.extend_from_slice(body);
}

fn master(id: &[u8], f: impl FnOnce(&mut Vec<u8>), out: &mut Vec<u8>) {
    let mut body = Vec::new();
    f(&mut body);
    element(id, &body, out);
}

/// The stream header: EBML header, open Segment, Info and Tracks.
pub fn header(width: usize, height: usize) -> Vec<u8> {
    let mut out = Vec::new();
    master(
        &[0x1A, 0x45, 0xDF, 0xA3],
        |b| {
            uint(&[0x42, 0x86], 1, b); // EBMLVersion
            uint(&[0x42, 0xF7], 1, b); // EBMLReadVersion
            uint(&[0x42, 0xF2], 4, b); // EBMLMaxIDLength
            uint(&[0x42, 0xF3], 8, b); // EBMLMaxSizeLength
            element(&[0x42, 0x82], b"matroska", b); // DocType
            uint(&[0x42, 0x87], 4, b); // DocTypeVersion
            uint(&[0x42, 0x85], 2, b); // DocTypeReadVersion
        },
        &mut out,
    );
    // Segment of unknown size (live stream).
    out.extend_from_slice(&[
        0x18, 0x53, 0x80, 0x67, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    ]);
    master(
        &[0x15, 0x49, 0xA9, 0x66],
        |b| {
            uint(&[0x2A, 0xD7, 0xB1], TIMESTAMP_SCALE_NS, b);
            element(&[0x4D, 0x80], b"liteaura-capture", b); // MuxingApp
            element(&[0x57, 0x41], b"liteaura-capture", b); // WritingApp
        },
        &mut out,
    );
    master(
        &[0x16, 0x54, 0xAE, 0x6B],
        |b| {
            master(
                &[0xAE],
                |t| {
                    uint(&[0xD7], 1, t); // TrackNumber
                    uint(&[0x73, 0xC5], 1, t); // TrackUID
                    uint(&[0x83], 1, t); // TrackType: video
                    uint(&[0x9C], 0, t); // FlagLacing
                    element(&[0x86], b"V_UNCOMPRESSED", t);
                    master(
                        &[0xE0],
                        |v| {
                            uint(&[0xB0], width as u64, v);
                            uint(&[0xBA], height as u64, v);
                            element(&[0x2E, 0xB5, 0x24], b"NV12", v); // ColourSpace
                            master(
                                &[0x55, 0xB0],
                                |c| {
                                    uint(&[0x55, 0xB1], 1, c); // MatrixCoefficients: BT.709
                                    uint(&[0x55, 0xB9], 1, c); // Range: broadcast
                                    uint(&[0x55, 0xBA], 1, c); // TransferCharacteristics
                                    uint(&[0x55, 0xBB], 1, c); // Primaries
                                },
                                v,
                            );
                        },
                        t,
                    );
                },
                b,
            );
        },
        &mut out,
    );
    out
}

/// Bytes that precede a frame's pixels: a Cluster with its Timestamp and the
/// head of one SimpleBlock.
pub fn frame_head(pts_us: u64, len: usize) -> Vec<u8> {
    let mut ts = Vec::with_capacity(16);
    uint(&[0xE7], pts_us, &mut ts);
    let block = 4 + len as u64; // track vint, i16 relative time, flags
    let cluster = ts.len() as u64 + 1 + 8 + block;
    let mut out = Vec::with_capacity(48);
    out.extend_from_slice(&[0x1F, 0x43, 0xB6, 0x75]);
    out.extend_from_slice(&vint_size(cluster));
    out.extend_from_slice(&ts);
    out.push(0xA3);
    out.extend_from_slice(&vint_size(block));
    out.extend_from_slice(&[0x81, 0x00, 0x00, 0x80]); // track 1, +0, keyframe
    out
}

/// Write one frame (head, then the pixels in place).
pub fn write_frame(w: &mut impl Write, pts_us: u64, frame: &[u8]) -> io::Result<()> {
    w.write_all(&frame_head(pts_us, frame.len()))?;
    w.write_all(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_consistent() {
        let h = frame_head(1_234_567, 100);
        // Cluster ID, 8-byte size, then the rest is exactly that size minus the pixels.
        let size = u64::from_be_bytes([0, h[5], h[6], h[7], h[8], h[9], h[10], h[11]]);
        assert_eq!(size as usize, h.len() - 12 + 100);
        assert_eq!(&h[h.len() - 4..], &[0x81, 0, 0, 0x80]);
    }

    #[test]
    fn header_names_the_pixel_format() {
        let h = header(1920, 1080);
        assert!(h.windows(4).any(|w| w == b"NV12"));
        assert!(h.windows(14).any(|w| w == b"V_UNCOMPRESSED"));
    }
}

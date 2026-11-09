//! JPEG previews straight from NV12 (the camera bubble on screen and the
//! Capture window's preview). Small images only: the frame is scaled first,
//! then expanded to full-range 4:4:4 through a lookup table.

use crate::nv12::Nv12;
use crate::scale::Scaler;

struct Lut {
    y: [u8; 256],
    c: [u8; 256],
}

fn lut() -> &'static Lut {
    static L: std::sync::OnceLock<Lut> = std::sync::OnceLock::new();
    L.get_or_init(|| {
        let mut y = [0u8; 256];
        let mut c = [0u8; 256];
        for i in 0..256 {
            y[i] = ((i as f32 - 16.0) * 255.0 / 219.0)
                .round()
                .clamp(0.0, 255.0) as u8;
            c[i] = ((i as f32 - 128.0) * 255.0 / 224.0 + 128.0)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
        Lut { y, c }
    })
}

/// Encode `f` (limited-range NV12) as a JFIF JPEG.
pub fn encode(f: &Nv12, quality: u8) -> Option<Vec<u8>> {
    let l = lut();
    let (w, h) = (f.w, f.h);
    let (yp, uvp) = f.planes();
    let mut px = vec![0u8; w * h * 3];
    for r in 0..h {
        let crow = &uvp[(r / 2) * w..][..w];
        let yrow = &yp[r * w..][..w];
        let out = &mut px[r * w * 3..][..w * 3];
        for x in 0..w {
            let ci = x & !1;
            out[x * 3] = l.y[yrow[x] as usize];
            out[x * 3 + 1] = l.c[crow[ci] as usize];
            out[x * 3 + 2] = l.c[crow[ci + 1] as usize];
        }
    }
    let mut buf = Vec::with_capacity(w * h / 4);
    let mut enc = jpeg_encoder::Encoder::new(&mut buf, quality);
    enc.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
    enc.encode(&px, w as u16, h as u16, jpeg_encoder::ColorType::Ycbcr)
        .ok()?;
    Some(buf)
}

/// A cached downscaler + encoder for previews no wider than `max_w`.
pub struct Preview {
    max_w: usize,
    scaler: Option<Scaler>,
    small: Nv12,
}

impl Preview {
    pub fn new(max_w: usize) -> Self {
        Self {
            max_w,
            scaler: None,
            small: Nv12::new(2, 2),
        }
    }

    pub fn jpeg(&mut self, f: &Nv12, quality: u8) -> Option<Vec<u8>> {
        if f.w <= self.max_w {
            return encode(f, quality);
        }
        let dw = self.max_w & !1;
        let dh = ((f.h * dw / f.w) & !1).max(2);
        if (self.small.w, self.small.h) != (dw, dh) {
            self.small = Nv12::new(dw, dh);
            self.scaler = None;
        }
        let s = self
            .scaler
            .get_or_insert_with(|| Scaler::new(f.w, f.h, dw, dh));
        if !s.matches(f, &self.small) {
            *s = Scaler::new(f.w, f.h, dw, dh);
        }
        s.run(f, &mut self.small);
        encode(&self.small, quality)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_a_valid_jpeg() {
        let mut f = Nv12::new(64, 48);
        f.data.fill(128);
        let j = Preview::new(32).jpeg(&f, 80).unwrap();
        assert_eq!(&j[..2], &[0xFF, 0xD8]);
        assert_eq!(&j[j.len() - 2..], &[0xFF, 0xD9]);
    }
}

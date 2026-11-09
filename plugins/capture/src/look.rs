//! The source as it looks right now, before anything records: one grab of
//! the screen (or of a window's or region's rectangle), box-scaled to a small
//! RGB picture and encoded as JPEG. The Capture window polls it twice a second
//! so the preview is never empty while choosing what to record.

use crate::x11::Screen;
use crate::{Rect, Result};

/// A JPEG of `want` (screen pixels; the whole screen when `None`), no wider
/// than `max_w` and no taller than `max_h`.
pub fn jpeg(want: Option<Rect>, max_w: usize, max_h: usize, quality: u8) -> Result<Vec<u8>> {
    let mut screen = Screen::open(None, false)?;
    let bounds = screen.bounds();
    let r = match want {
        Some(w) => w.intersect(&bounds),
        None => bounds,
    };
    if r.is_empty() {
        return Err("the source is off the screen".into());
    }
    let px = screen.grab(r)?;
    let (sw, sh) = (r.w as usize, r.h as usize);
    let (dw, dh) = fit_size(sw, sh, max_w, max_h);
    let rgb = box_scale_bgrx(px, sw, sh, dw, dh);
    let mut buf = Vec::with_capacity(dw * dh / 4);
    let mut enc = jpeg_encoder::Encoder::new(&mut buf, quality);
    enc.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
    enc.encode(&rgb, dw as u16, dh as u16, jpeg_encoder::ColorType::Rgb)?;
    Ok(buf)
}

/// The largest size inside `max_w`×`max_h` with the source's aspect (never
/// larger than the source).
pub fn fit_size(sw: usize, sh: usize, max_w: usize, max_h: usize) -> (usize, usize) {
    let s = (max_w as f64 / sw as f64)
        .min(max_h as f64 / sh as f64)
        .min(1.0);
    (
        ((sw as f64 * s).round() as usize).max(1),
        ((sh as f64 * s).round() as usize).max(1),
    )
}

/// Average every source pixel that falls in each destination pixel (packed
/// BGRX in, packed RGB out): sharp text stays readable when shrunk.
pub fn box_scale_bgrx(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let mut out = vec![0u8; dw * dh * 3];
    for dy in 0..dh {
        let y0 = dy * sh / dh;
        let y1 = ((dy + 1) * sh / dh).max(y0 + 1).min(sh);
        for dx in 0..dw {
            let x0 = dx * sw / dw;
            let x1 = ((dx + 1) * sw / dw).max(x0 + 1).min(sw);
            let (mut b, mut g, mut r) = (0u32, 0u32, 0u32);
            for y in y0..y1 {
                let row = &src[(y * sw + x0) * 4..(y * sw + x1) * 4];
                for p in row.as_chunks::<4>().0 {
                    b += p[0] as u32;
                    g += p[1] as u32;
                    r += p[2] as u32;
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let o = &mut out[(dy * dw + dx) * 3..][..3];
            o[0] = (r / n) as u8;
            o[1] = (g / n) as u8;
            o[2] = (b / n) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_inside_the_box_keeping_aspect() {
        assert_eq!(fit_size(1920, 1080, 640, 640), (640, 360));
        assert_eq!(fit_size(1080, 1920, 640, 640), (360, 640));
        assert_eq!(fit_size(300, 200, 640, 640), (300, 200));
    }

    #[test]
    fn box_scale_averages_and_swaps_to_rgb() {
        // 2×1 BGRX: pure blue then pure red, shrunk to one pixel.
        let src = [255, 0, 0, 0, 0, 0, 255, 0];
        assert_eq!(box_scale_bgrx(&src, 2, 1, 1, 1), vec![127, 0, 127]);
        let one = [10, 20, 30, 0];
        assert_eq!(box_scale_bgrx(&one, 1, 1, 1, 1), vec![30, 20, 10]);
    }
}

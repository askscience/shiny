//! Layer compositor — flattens an ordered stack of pixel layers and folders
//! into one RGBA image, applying per-layer opacity, blend mode and optional
//! grayscale mask. The maths follow the W3C/PDF compositing model: blend
//! modes operate on straight (non-premultiplied) channels, then the result is
//! source-over composited with the layer alpha.
//!
//! This is the single rendering engine behind both the agent tool
//! (`image_render`, `image_flatten`) and the window's live preview, so the AI
//! and the human always see the same pixels.

use crate::blend::BlendMode;

/// A pixel layer placed at (`x`, `y`) on the document canvas.
pub struct Raster<'a> {
    /// Local raw RGBA (`width` × `height` × 4) pixels.
    pub pixels: &'a [u8],
    pub w: u32,
    pub h: u32,
    pub x: i32,
    pub y: i32,
    pub opacity: f32,
    pub blend: BlendMode,
    pub visible: bool,
    /// Optional local raw 8-bit grayscale coverage (white reveals, black
    /// hides); sampled nearest-neighbour to the layer rectangle.
    pub mask: Option<&'a [u8]>,
    pub mask_w: u32,
    pub mask_h: u32,
    pub mask_enabled: bool,
}

/// A folder: its children composite into their own buffer first, then that
/// buffer is composited as one source with the folder's opacity/blend.
pub struct Group<'a> {
    pub opacity: f32,
    pub blend: BlendMode,
    pub visible: bool,
    pub children: Vec<Node<'a>>,
}

pub enum Node<'a> {
    Layer(Raster<'a>),
    Group(Group<'a>),
}

#[inline]
fn clamp01(v: f32) -> f32 {
    if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

/// A premultiplied linear f32 working canvas.
struct Canvas {
    w: u32,
    h: u32,
    px: Vec<f32>,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Self {
        Canvas {
            w,
            h,
            px: vec![0.0; (w as usize) * (h as usize) * 4],
        }
    }

    /// Draw a pixel layer onto the canvas.
    fn draw_raster(&mut self, r: &Raster<'_>) {
        if !r.visible || r.opacity <= 0.0 || r.w == 0 || r.h == 0 {
            return;
        }
        let src_len = r.pixels.len();
        for sy in 0..r.h {
            let dy = r.y + sy as i32;
            if dy < 0 || dy >= self.h as i32 {
                continue;
            }
            for sx in 0..r.w {
                let dx = r.x + sx as i32;
                if dx < 0 || dx >= self.w as i32 {
                    continue;
                }
                let si = ((sy as usize) * (r.w as usize) + sx as usize) * 4;
                if si + 3 >= src_len {
                    continue;
                }
                let a = r.pixels[si + 3];
                if a == 0 {
                    continue;
                }
                let mask_cov = mask_coverage(r, sx, sy);
                if mask_cov <= 0.0 {
                    continue;
                }
                let sa = (a as f32 / 255.0) * clamp01(r.opacity) * mask_cov;
                let src = [
                    r.pixels[si] as f32 / 255.0,
                    r.pixels[si + 1] as f32 / 255.0,
                    r.pixels[si + 2] as f32 / 255.0,
                ];
                let base = ((dy as usize) * (self.w as usize) + dx as usize) * 4;
                self.composite(base, src, sa, r.blend);
            }
        }
    }

    /// Source-over one straight pixel with the standard W3C formula, writing
    /// premultiplied output into the canvas.
    #[inline]
    fn composite(&mut self, base: usize, cs: [f32; 3], sa: f32, mode: BlendMode) {
        if sa <= 0.0 {
            return;
        }
        let ab = self.px[base + 3];
        let inv_s = 1.0 - sa;
        for c in 0..3 {
            let cb_pre = self.px[base + c];
            let cb = if ab > 0.0 { cb_pre / ab } else { 0.0 };
            let b = if ab > 0.0 { mode.channel(cb, cs[c]) } else { cs[c] };
            // Co = (1-As)*Cb_pre + As*((1-Ab)*Cs + Ab*B)
            self.px[base + c] = inv_s * cb_pre + sa * ((1.0 - ab) * cs[c] + ab * b);
        }
        self.px[base + 3] = sa + ab * inv_s;
    }

    /// Composite another (premultiplied) canvas as one source.
    fn draw_canvas_as_source(&mut self, other: &Canvas, opacity: f32, mode: BlendMode) {
        if opacity <= 0.0 {
            return;
        }
        for y in 0..self.h.min(other.h) {
            for x in 0..self.w.min(other.w) {
                let i = ((y as usize) * (self.w as usize) + x as usize) * 4;
                let oi = ((y as usize) * (other.w as usize) + x as usize) * 4;
                let oa = other.px[oi + 3];
                if oa <= 0.0 {
                    continue;
                }
                let sa = oa * clamp01(opacity);
                let cs = [
                    other.px[oi] / oa,
                    other.px[oi + 1] / oa,
                    other.px[oi + 2] / oa,
                ];
                self.composite(i, cs, sa, mode);
            }
        }
    }

    fn to_rgba(&self) -> Vec<u8> {
        let mut out = vec![0u8; (self.w as usize) * (self.h as usize) * 4];
        for i in 0..(self.w as usize) * (self.h as usize) {
            let b = i * 4;
            let a = self.px[b + 3];
            if a > 0.0 {
                for c in 0..3 {
                    let v = (self.px[b + c] / a).clamp(0.0, 1.0);
                    out[b + c] = (v * 255.0).round() as u8;
                }
                out[b + 3] = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
        out
    }
}

/// Nearest-neighbour sample of a layer's mask at a local pixel.
#[inline]
fn mask_coverage(r: &Raster<'_>, sx: u32, sy: u32) -> f32 {
    let Some(mask) = r.mask else { return 1.0 };
    if !r.mask_enabled || r.mask_w == 0 || r.mask_h == 0 {
        return 1.0;
    }
    let mx = ((sx as u64 * r.mask_w as u64) / r.w.max(1) as u64) as usize;
    let my = ((sy as u64 * r.mask_h as u64) / r.h.max(1) as u64) as usize;
    let idx = my * r.mask_w as usize + mx;
    match mask.get(idx) {
        Some(v) => *v as f32 / 255.0,
        None => 1.0,
    }
}

/// Composite an ordered (bottom-to-top) node list into a straight RGBA buffer.
pub fn composite(w: u32, h: u32, nodes: &[Node<'_>]) -> Vec<u8> {
    let mut canvas = Canvas::new(w.max(1), h.max(1));
    draw_nodes(&mut canvas, nodes);
    canvas.to_rgba()
}

fn draw_nodes(canvas: &mut Canvas, nodes: &[Node<'_>]) {
    for node in nodes {
        match node {
            Node::Layer(r) => canvas.draw_raster(r),
            Node::Group(g) => {
                if !g.visible || g.opacity <= 0.0 || g.children.is_empty() {
                    continue;
                }
                let mut sub = Canvas::new(canvas.w, canvas.h);
                draw_nodes(&mut sub, &g.children);
                canvas.draw_canvas_as_source(&sub, g.opacity, g.blend);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer<'a>(
        pixels: &'a [u8],
        w: u32,
        h: u32,
    ) -> Raster<'a> {
        Raster {
            pixels,
            w,
            h,
            x: 0,
            y: 0,
            opacity: 1.0,
            blend: BlendMode::Normal,
            visible: true,
            mask: None,
            mask_w: 0,
            mask_h: 0,
            mask_enabled: true,
        }
    }

    #[test]
    fn single_opaque_layer_is_identity() {
        let px = vec![10, 20, 30, 255, 40, 50, 60, 255];
        let out = composite(2, 1, &[Node::Layer(layer(&px, 2, 1))]);
        assert_eq!(out, px);
    }

    #[test]
    fn half_opacity_over_transparent() {
        let px = vec![200, 100, 50, 255];
        let mut l = layer(&px, 1, 1);
        l.opacity = 0.5;
        let out = composite(1, 1, &[Node::Layer(l)]);
        assert_eq!(out[0], 200);
        assert_eq!(out[1], 100);
        assert_eq!(out[2], 50);
        assert!((out[3] as i32 - 128).abs() <= 1, "alpha was {}", out[3]);
    }

    #[test]
    fn invisible_layer_is_skipped() {
        let px = vec![1, 2, 3, 255];
        let mut l = layer(&px, 1, 1);
        l.visible = false;
        let out = composite(1, 1, &[Node::Layer(l)]);
        assert_eq!(out, vec![0, 0, 0, 0]);
    }

    #[test]
    fn multiply_darkens() {
        let bottom = vec![255, 255, 255, 255];
        let top = vec![128, 128, 128, 255];
        let mut t = layer(&top, 1, 1);
        t.blend = BlendMode::Multiply;
        let out = composite(1, 1, &[Node::Layer(layer(&bottom, 1, 1)), Node::Layer(t)]);
        assert!((out[0] as i32 - 128).abs() <= 1, "got {}", out[0]);
        assert_eq!(out[3], 255);
    }

    #[test]
    fn offset_layer_lands_at_position() {
        let top = vec![9, 9, 9, 255];
        let mut t = layer(&top, 1, 1);
        t.x = 1;
        t.y = 1;
        let out = composite(2, 2, &[Node::Layer(t)]);
        let idx = (1 * 2 + 1) * 4;
        assert_eq!(&out[idx..idx + 4], &[9, 9, 9, 255]);
        assert_eq!(&out[0..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn group_opacity_multiplies() {
        let px = vec![255, 0, 0, 255];
        let group = Group {
            opacity: 0.5,
            blend: BlendMode::Normal,
            visible: true,
            children: vec![Node::Layer(layer(&px, 1, 1))],
        };
        let out = composite(1, 1, &[Node::Group(group)]);
        assert_eq!(out[0], 255);
        assert!((out[3] as i32 - 128).abs() <= 1, "alpha was {}", out[3]);
    }

    #[test]
    fn mask_hides_black_pixels() {
        let px = vec![255, 255, 255, 255, 255, 255, 255, 255];
        let mask = vec![0u8, 255u8];
        let mut l = layer(&px, 2, 1);
        l.mask = Some(&mask);
        l.mask_w = 2;
        l.mask_h = 1;
        let out = composite(2, 1, &[Node::Layer(l)]);
        assert_eq!(out[3], 0);
        assert_eq!(out[7], 255);
    }
}

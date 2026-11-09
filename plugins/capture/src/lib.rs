//! Capture: screen recording and live streaming for LiteAura.
//!
//! The engine keeps every pixel in NV12 from the moment it leaves the X server:
//! the screen is converted once (and only where it changed), the webcam already
//! delivers NV12, and the encoder (ffmpeg, a child process) reads NV12 straight
//! from a pipe. See `docs/architecture.md` for the pipeline and the numbers.

pub mod bubble;
pub mod camera;
pub mod clock;
pub mod cursor;
pub mod devices;
pub mod editor;
pub mod encoder;
pub mod jpeg;
pub mod library;
pub mod look;
pub mod mask;
pub mod meter;
pub mod mkv;
pub mod nv12;
pub mod platforms;
pub mod plugin;
pub mod routes;
pub mod scale;
pub mod session;
pub mod stats;
pub mod store;
pub mod tools;
pub mod v4l2;
pub mod x11;

/// Boxed error for the engine; routes map it to `AppError` at the edge.
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

/// An axis-aligned rectangle in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h as i32
    }

    pub fn area(&self) -> u64 {
        self.w as u64 * self.h as u64
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        if x1 <= x0 || y1 <= y0 {
            return Rect::default();
        }
        Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32)
    }

    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = self.right().max(o.right());
        let y1 = self.bottom().max(o.bottom());
        Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32)
    }

    /// Grow outwards so every edge lands on a multiple of `n` (a power of two),
    /// then clip to `bounds`. NV12 chroma covers 2×2 blocks, so partial updates
    /// must start and end on even pixels (4 when the source is downscaled 2×).
    pub fn align_out(&self, n: i32, bounds: &Rect) -> Rect {
        let m = n - 1;
        let x0 = self.x & !m;
        let y0 = self.y & !m;
        let x1 = (self.right() + m) & !m;
        let y1 = (self.bottom() + m) & !m;
        Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32).intersect(bounds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_out_snaps_to_even_and_clips() {
        let b = Rect::new(0, 0, 100, 50);
        assert_eq!(
            Rect::new(3, 5, 4, 4).align_out(2, &b),
            Rect::new(2, 4, 6, 6)
        );
        assert_eq!(
            Rect::new(97, 47, 9, 9).align_out(4, &b),
            Rect::new(96, 44, 4, 6)
        );
    }

    #[test]
    fn union_and_intersect() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.union(&b), Rect::new(0, 0, 15, 15));
        assert_eq!(a.intersect(&b), Rect::new(5, 5, 5, 5));
        assert!(a.intersect(&Rect::new(20, 20, 1, 1)).is_empty());
    }
}

//! X11 screen source: MIT-SHM grabs, XDamage change tracking, XFixes cursor.
//!
//! - **Shared memory**: `ShmGetImage` writes pixels straight into a segment
//!   mapped in this process; nothing crosses the socket but the request.
//! - **Damage**: each frame asks the server which rectangles changed
//!   (`DamageSubtract` into a region, then `FetchRegion`: atomic, so no change
//!   can slip between reading and resetting). An idle screen costs one round
//!   trip and no pixels.
//! - **Cursor**: `XFixesGetCursorImage` returns position, serial and image in
//!   one reply; the image is converted only when the serial changes.

use crate::{Rect, Result};
use x11rb::connection::Connection;
use x11rb::protocol::damage::{self, ConnectionExt as _};
use x11rb::protocol::shm::ConnectionExt as _;
use x11rb::protocol::xfixes::ConnectionExt as _;
use x11rb::protocol::xproto::{ImageFormat, Window};
use x11rb::rust_connection::RustConnection;

/// A System V shared-memory segment attached to this process and the server.
struct Segment {
    id: u32,
    addr: *mut u8,
    size: usize,
}

impl Drop for Segment {
    fn drop(&mut self) {
        // SAFETY: `addr` came from shmat and is detached exactly once.
        unsafe { libc::shmdt(self.addr as *const libc::c_void) };
    }
}

// The segment is only touched by the thread that owns the Screen.
unsafe impl Send for Segment {}

/// What changed since the last call.
#[derive(Debug, Clone)]
pub enum Changes {
    /// Nothing; the previous frame is still exact.
    None,
    /// These rectangles (screen coordinates) changed.
    Rects(Vec<Rect>),
    /// Damage tracking is unavailable: treat everything as changed.
    All,
}

pub struct CursorState {
    pub x: i32,
    pub y: i32,
    pub serial: u32,
    pub w: usize,
    pub h: usize,
    pub xhot: i32,
    pub yhot: i32,
    /// Only filled when `serial` differs from the one passed in.
    pub argb: Option<Vec<u32>>,
}

pub struct Screen {
    conn: RustConnection,
    root: Window,
    pub width: u32,
    pub height: u32,
    seg: Segment,
    damage: Option<(damage::Damage, u32)>,
}

impl Screen {
    /// Connect to `$DISPLAY` and prepare a segment big enough for `max` pixels
    /// (the whole screen when `None`).
    pub fn open(max: Option<Rect>, track_damage: bool) -> Result<Self> {
        let (conn, n) = x11rb::connect(None)?;
        let screen = &conn.setup().roots[n];
        let (root, width, height, depth) = (
            screen.root,
            screen.width_in_pixels as u32,
            screen.height_in_pixels as u32,
            screen.root_depth,
        );
        let bpp = conn
            .setup()
            .pixmap_formats
            .iter()
            .find(|f| f.depth == depth)
            .map(|f| f.bits_per_pixel)
            .unwrap_or(0);
        if bpp != 32 {
            return Err(format!("unsupported X pixel format: depth {depth}, {bpp} bpp").into());
        }
        conn.shm_query_version()?
            .reply()
            .map_err(|_| "the X server has no MIT-SHM")?;
        let area = max
            .map(|r| r.area())
            .unwrap_or(width as u64 * height as u64);
        let size = (area * 4) as usize;

        // SAFETY: plain System V shm calls; the id is marked for removal right
        // after both sides attached, so it never leaks past the process.
        let seg = unsafe {
            let shmid = libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o600);
            if shmid < 0 {
                return Err("shmget failed".into());
            }
            let addr = libc::shmat(shmid, std::ptr::null(), 0);
            if addr as isize == -1 {
                libc::shmctl(shmid, libc::IPC_RMID, std::ptr::null_mut());
                return Err("shmat failed".into());
            }
            let id = conn.generate_id()?;
            let attached = conn.shm_attach(id, shmid as u32, false).map(|c| c.check());
            libc::shmctl(shmid, libc::IPC_RMID, std::ptr::null_mut());
            attached??;
            Segment {
                id,
                addr: addr as *mut u8,
                size,
            }
        };

        let damage = if track_damage {
            Self::init_damage(&conn, root).ok()
        } else {
            None
        };
        conn.xfixes_query_version(5, 0)?.reply()?;
        Ok(Self {
            conn,
            root,
            width,
            height,
            seg,
            damage,
        })
    }

    fn init_damage(conn: &RustConnection, root: Window) -> Result<(damage::Damage, u32)> {
        conn.xfixes_query_version(5, 0)?.reply()?;
        conn.damage_query_version(1, 1)?.reply()?;
        let d = conn.generate_id()?;
        conn.damage_create(d, root, damage::ReportLevel::NON_EMPTY)?
            .check()?;
        let region = conn.generate_id()?;
        conn.xfixes_create_region(region, &[])?.check()?;
        Ok((d, region))
    }

    pub fn tracks_damage(&self) -> bool {
        self.damage.is_some()
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    /// Changed areas since the previous call. Rectangles are merged into their
    /// bounding box when that is cheaper than grabbing them one by one.
    pub fn changes(&mut self) -> Result<Changes> {
        // Drain DamageNotify events: they only say "something changed".
        while self.conn.poll_for_event()?.is_some() {}
        let Some((d, region)) = self.damage else {
            return Ok(Changes::All);
        };
        self.conn.damage_subtract(d, x11rb::NONE, region)?;
        let reply = self.conn.xfixes_fetch_region(region)?.reply()?;
        let rects: Vec<Rect> = reply
            .rectangles
            .iter()
            .map(|r| Rect::new(r.x as i32, r.y as i32, r.width as u32, r.height as u32))
            .filter(|r| !r.is_empty())
            .collect();
        if rects.is_empty() {
            return Ok(Changes::None);
        }
        Ok(Changes::Rects(merge(rects)))
    }

    /// Grab `r` (screen coordinates, inside the screen) into the segment and
    /// return it as packed BGRX, `r.w * 4` bytes per row.
    pub fn grab(&mut self, r: Rect) -> Result<&[u8]> {
        let n = (r.area() * 4) as usize;
        if n > self.seg.size {
            return Err("grab larger than the shared segment".into());
        }
        self.conn
            .shm_get_image(
                self.root,
                r.x as i16,
                r.y as i16,
                r.w as u16,
                r.h as u16,
                !0,
                ImageFormat::Z_PIXMAP.into(),
                self.seg.id,
                0,
            )?
            .reply()?;
        // SAFETY: the server finished writing `n` bytes (we waited for the
        // reply) and the segment outlives the borrow.
        Ok(unsafe { std::slice::from_raw_parts(self.seg.addr, n) })
    }

    /// Pointer position and, when its serial changed, the cursor image.
    pub fn cursor(&mut self, known_serial: u32) -> Result<CursorState> {
        let c = self.conn.xfixes_get_cursor_image()?.reply()?;
        let changed = c.cursor_serial != known_serial;
        Ok(CursorState {
            x: c.x as i32,
            y: c.y as i32,
            serial: c.cursor_serial,
            w: c.width as usize,
            h: c.height as usize,
            xhot: c.xhot as i32,
            yhot: c.yhot as i32,
            argb: changed.then_some(c.cursor_image),
        })
    }
}

/// Merge damage rectangles: keep them separate only when there are few and
/// they cover well under half of their bounding box (two distant spots).
pub fn merge(rects: Vec<Rect>) -> Vec<Rect> {
    let bbox = rects.iter().fold(Rect::default(), |a, r| a.union(r));
    let sum: u64 = rects.iter().map(Rect::area).sum();
    if rects.len() <= 4 && sum * 2 < bbox.area() {
        rects
    } else {
        vec![bbox]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_distant_spots_apart() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(1000, 1000, 10, 10);
        assert_eq!(merge(vec![a, b]).len(), 2);
        let c = Rect::new(5, 0, 10, 10);
        assert_eq!(merge(vec![a, c]), vec![Rect::new(0, 0, 15, 10)]);
    }
}

//! A minimal V4L2 capture client: memory-mapped buffers, no library.
//!
//! Only what the webcam bubble needs: list devices and their raw modes, open
//! one mode, and hand out frames in place. Frames are read straight from the
//! driver's mmap buffers (no copy) and requeued as soon as the caller is done.
//! NV12 is taken as is; YUYV (the other format every UVC camera offers) is
//! converted to NV12 in one pass.

use crate::nv12::Nv12;
use crate::Result;
use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};

const VIDIOC_QUERYCAP: libc::c_ulong = 0x8068_5600;
const VIDIOC_ENUM_FMT: libc::c_ulong = 0xC040_5602;
const VIDIOC_S_FMT: libc::c_ulong = 0xC0D0_5605;
const VIDIOC_REQBUFS: libc::c_ulong = 0xC014_5608;
const VIDIOC_QUERYBUF: libc::c_ulong = 0xC058_5609;
const VIDIOC_QBUF: libc::c_ulong = 0xC058_560F;
const VIDIOC_DQBUF: libc::c_ulong = 0xC058_5611;
const VIDIOC_STREAMON: libc::c_ulong = 0x4004_5612;
const VIDIOC_STREAMOFF: libc::c_ulong = 0x4004_5613;
const VIDIOC_S_PARM: libc::c_ulong = 0xC0CC_5616;
const VIDIOC_ENUM_FRAMESIZES: libc::c_ulong = 0xC02C_564A;

const BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
const MEMORY_MMAP: u32 = 1;
const CAP_VIDEO_CAPTURE: u32 = 0x1;
const CAP_STREAMING: u32 = 0x0400_0000;
const CAP_DEVICE_CAPS: u32 = 0x8000_0000;
const FRMSIZE_TYPE_DISCRETE: u32 = 1;

pub const FOURCC_NV12: u32 = u32::from_le_bytes(*b"NV12");
pub const FOURCC_YUYV: u32 = u32::from_le_bytes(*b"YUYV");

#[repr(C)]
struct Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

#[repr(C)]
struct FmtDesc {
    index: u32,
    kind: u32,
    flags: u32,
    description: [u8; 32],
    pixelformat: u32,
    mbus_code: u32,
    reserved: [u32; 3],
}

#[repr(C)]
struct FrmSizeEnum {
    index: u32,
    pixel_format: u32,
    kind: u32,
    // Discrete: width, height; stepwise uses all six.
    size: [u32; 6],
    reserved: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PixFormat {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    private: u32,
    flags: u32,
    ycbcr_enc: u32,
    quantization: u32,
    xfer_func: u32,
}

#[repr(C)]
struct Format {
    kind: u32,
    _pad: u32,
    pix: PixFormat,
    _rest: [u8; 200 - std::mem::size_of::<PixFormat>()],
}

#[repr(C)]
struct RequestBuffers {
    count: u32,
    kind: u32,
    memory: u32,
    capabilities: u32,
    flags: u8,
    reserved: [u8; 3],
}

#[repr(C)]
struct Buffer {
    index: u32,
    kind: u32,
    bytesused: u32,
    flags: u32,
    field: u32,
    _pad: u32,
    timestamp: libc::timeval,
    timecode: [u32; 4],
    sequence: u32,
    memory: u32,
    offset: u64,
    length: u32,
    reserved2: u32,
    request_fd: u32,
    _pad2: u32,
}

#[repr(C)]
struct StreamParm {
    kind: u32,
    capability: u32,
    capturemode: u32,
    numerator: u32,
    denominator: u32,
    extendedmode: u32,
    readbuffers: u32,
    reserved: [u32; 4],
    _rest: [u8; 200 - 40],
}

const _: () = {
    assert!(std::mem::size_of::<Capability>() == 104);
    assert!(std::mem::size_of::<FmtDesc>() == 64);
    assert!(std::mem::size_of::<FrmSizeEnum>() == 44);
    assert!(std::mem::size_of::<Format>() == 208);
    assert!(std::mem::size_of::<RequestBuffers>() == 20);
    assert!(std::mem::size_of::<Buffer>() == 88);
    assert!(std::mem::size_of::<StreamParm>() == 204);
};

fn zeroed<T>() -> T {
    // SAFETY: every struct here is plain old data; all-zero is valid.
    unsafe { std::mem::zeroed() }
}

fn xioctl<T>(fd: i32, req: libc::c_ulong, arg: &mut T) -> std::io::Result<()> {
    loop {
        // SAFETY: `arg` is the struct the request expects (sizes checked above).
        let r = unsafe { libc::ioctl(fd, req as _, arg as *mut T) };
        if r == 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        if e.kind() != std::io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

fn open_fd(path: &Path, nonblock: bool) -> Result<OwnedFd> {
    let c = CString::new(path.as_os_str().as_encoded_bytes())?;
    let flags = libc::O_RDWR | libc::O_CLOEXEC | if nonblock { libc::O_NONBLOCK } else { 0 };
    // SAFETY: plain open(2) on a NUL-terminated path.
    let fd = unsafe { libc::open(c.as_ptr(), flags) };
    if fd < 0 {
        return Err(format!("{}: {}", path.display(), std::io::Error::last_os_error()).into());
    }
    // SAFETY: we own the new descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn cstr(b: &[u8]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..n]).into_owned()
}

/// A capture device and the raw modes it offers.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Device {
    pub path: PathBuf,
    pub name: String,
    /// `(width, height)` of each NV12 or YUYV mode, largest first.
    pub modes: Vec<(u32, u32)>,
    /// Whether the modes are NV12 (no conversion) rather than YUYV.
    pub nv12: bool,
}

fn frame_sizes(fd: i32, fourcc: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for index in 0..64 {
        let mut s: FrmSizeEnum = zeroed();
        s.index = index;
        s.pixel_format = fourcc;
        if xioctl(fd, VIDIOC_ENUM_FRAMESIZES, &mut s).is_err() {
            break;
        }
        if s.kind == FRMSIZE_TYPE_DISCRETE {
            out.push((s.size[0], s.size[1]));
        } else {
            // Stepwise/continuous: offer the common sizes inside the range.
            let (minw, maxw, minh, maxh) = (s.size[0], s.size[1], s.size[3], s.size[4]);
            for (w, h) in [(1920, 1080), (1280, 720), (640, 480), (640, 360)] {
                if (minw..=maxw).contains(&w) && (minh..=maxh).contains(&h) {
                    out.push((w, h));
                }
            }
            break;
        }
    }
    out.sort_by_key(|&(w, h)| std::cmp::Reverse(w as u64 * h as u64));
    out.dedup();
    out
}

/// Every video capture device with a raw mode the engine can use.
pub fn devices() -> Vec<Device> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir("/dev")
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("video"))
                })
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    paths.into_iter().filter_map(|p| probe(&p)).collect()
}

fn probe(path: &Path) -> Option<Device> {
    let fd = open_fd(path, true).ok()?;
    let raw = fd.as_raw_fd();
    let mut cap: Capability = zeroed();
    xioctl(raw, VIDIOC_QUERYCAP, &mut cap).ok()?;
    let caps = if cap.capabilities & CAP_DEVICE_CAPS != 0 {
        cap.device_caps
    } else {
        cap.capabilities
    };
    if caps & CAP_VIDEO_CAPTURE == 0 || caps & CAP_STREAMING == 0 {
        return None;
    }
    let mut formats = Vec::new();
    for index in 0..32 {
        let mut d: FmtDesc = zeroed();
        d.index = index;
        d.kind = BUF_TYPE_VIDEO_CAPTURE;
        if xioctl(raw, VIDIOC_ENUM_FMT, &mut d).is_err() {
            break;
        }
        formats.push(d.pixelformat);
    }
    let fourcc = [FOURCC_NV12, FOURCC_YUYV]
        .into_iter()
        .find(|f| formats.contains(f))?;
    let modes = frame_sizes(raw, fourcc);
    if modes.is_empty() {
        return None;
    }
    Some(Device {
        path: path.to_path_buf(),
        name: cstr(&cap.card),
        modes,
        nv12: fourcc == FOURCC_NV12,
    })
}

struct Mapped {
    ptr: *mut u8,
    len: usize,
}

/// An open, streaming camera.
pub struct Camera {
    fd: OwnedFd,
    bufs: Vec<Mapped>,
    pub width: usize,
    pub height: usize,
    pub fourcc: u32,
    stride: usize,
}

// The mappings belong to the camera and are only touched by its owner.
unsafe impl Send for Camera {}

impl Camera {
    /// Open `path` at `(w, h)` in its raw format, about `fps` frames a second.
    pub fn open(path: &Path, w: u32, h: u32, fps: u32) -> Result<Self> {
        let dev = probe(path).ok_or("not a usable camera")?;
        let fourcc = if dev.nv12 { FOURCC_NV12 } else { FOURCC_YUYV };
        let fd = open_fd(path, false)?;
        let raw = fd.as_raw_fd();
        let mut f: Format = zeroed();
        f.kind = BUF_TYPE_VIDEO_CAPTURE;
        f.pix.width = w;
        f.pix.height = h;
        f.pix.pixelformat = fourcc;
        f.pix.field = 1; // V4L2_FIELD_NONE
        xioctl(raw, VIDIOC_S_FMT, &mut f)?;
        if f.pix.pixelformat != fourcc {
            return Err("the camera refused its own raw format".into());
        }
        let mut p: StreamParm = zeroed();
        p.kind = BUF_TYPE_VIDEO_CAPTURE;
        p.numerator = 1;
        p.denominator = fps.max(1);
        let _ = xioctl(raw, VIDIOC_S_PARM, &mut p); // best effort
        let mut req: RequestBuffers = zeroed();
        req.count = 3;
        req.kind = BUF_TYPE_VIDEO_CAPTURE;
        req.memory = MEMORY_MMAP;
        xioctl(raw, VIDIOC_REQBUFS, &mut req)?;
        let mut cam = Self {
            fd,
            bufs: Vec::new(),
            width: f.pix.width as usize,
            height: f.pix.height as usize,
            fourcc,
            stride: f.pix.bytesperline as usize,
        };
        for index in 0..req.count {
            let mut b: Buffer = zeroed();
            b.index = index;
            b.kind = BUF_TYPE_VIDEO_CAPTURE;
            b.memory = MEMORY_MMAP;
            xioctl(raw, VIDIOC_QUERYBUF, &mut b)?;
            // SAFETY: mapping the driver buffer at the offset it gave us.
            let ptr = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    b.length as usize,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    raw,
                    b.offset as libc::off_t,
                )
            };
            if ptr == libc::MAP_FAILED {
                return Err("mmap of a camera buffer failed".into());
            }
            cam.bufs.push(Mapped {
                ptr: ptr as *mut u8,
                len: b.length as usize,
            });
            xioctl(raw, VIDIOC_QBUF, &mut b)?;
        }
        let mut kind = BUF_TYPE_VIDEO_CAPTURE as libc::c_int;
        xioctl(raw, VIDIOC_STREAMON, &mut kind)?;
        Ok(cam)
    }

    /// Wait up to `timeout_ms` for the next frame and convert it into `out`
    /// (resized to the camera's size if needed). `Ok(false)` on timeout.
    pub fn next_into(&mut self, out: &mut Nv12, timeout_ms: i32) -> Result<bool> {
        let raw = self.fd.as_raw_fd();
        let mut pfd = libc::pollfd {
            fd: raw,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one pollfd.
        let n = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if n <= 0 {
            return Ok(false);
        }
        let mut b: Buffer = zeroed();
        b.kind = BUF_TYPE_VIDEO_CAPTURE;
        b.memory = MEMORY_MMAP;
        xioctl(raw, VIDIOC_DQBUF, &mut b)?;
        let m = &self.bufs[b.index as usize];
        // SAFETY: the driver filled `bytesused` bytes of this mapping and does
        // not touch it until we queue it again below.
        let data = unsafe { std::slice::from_raw_parts(m.ptr, (b.bytesused as usize).min(m.len)) };
        if out.w != self.width || out.h != self.height {
            *out = Nv12::new(self.width, self.height);
        }
        let ok = match self.fourcc {
            FOURCC_NV12 => nv12_copy(data, self.stride.max(self.width), out),
            _ => yuyv_to_nv12(data, self.stride.max(self.width * 2), out),
        };
        xioctl(raw, VIDIOC_QBUF, &mut b)?;
        Ok(ok)
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        let mut kind = BUF_TYPE_VIDEO_CAPTURE as libc::c_int;
        let _ = xioctl(self.fd.as_raw_fd(), VIDIOC_STREAMOFF, &mut kind);
        for m in &self.bufs {
            // SAFETY: unmapping what we mapped.
            unsafe { libc::munmap(m.ptr as *mut libc::c_void, m.len) };
        }
    }
}

/// Driver NV12 (rows may be padded to `stride`) into a packed frame.
fn nv12_copy(data: &[u8], stride: usize, out: &mut Nv12) -> bool {
    let (w, h) = (out.w, out.h);
    if data.len() < stride * h * 3 / 2 {
        return false;
    }
    if stride == w {
        out.data.copy_from_slice(&data[..w * h * 3 / 2]);
        return true;
    }
    let (y, uv) = out.planes_mut();
    for r in 0..h {
        y[r * w..][..w].copy_from_slice(&data[r * stride..][..w]);
    }
    let base = stride * h;
    for r in 0..h / 2 {
        uv[r * w..][..w].copy_from_slice(&data[base + r * stride..][..w]);
    }
    true
}

/// Packed YUYV 4:2:2 into NV12 4:2:0: luma as is, chroma averaged over the
/// two rows of each pair (rounded like `pavgb`).
pub fn yuyv_to_nv12(data: &[u8], stride: usize, out: &mut Nv12) -> bool {
    let (w, h) = (out.w, out.h);
    if data.len() < stride * h {
        return false;
    }
    let (y, uv) = out.planes_mut();
    for r in (0..h).step_by(2) {
        let (a, b) = (
            &data[r * stride..][..w * 2],
            &data[(r + 1) * stride..][..w * 2],
        );
        let (ya, yb) = (r * w, (r + 1) * w);
        let c = (r / 2) * w;
        for x in (0..w).step_by(2) {
            let i = x * 2;
            y[ya + x] = a[i];
            y[ya + x + 1] = a[i + 2];
            y[yb + x] = b[i];
            y[yb + x + 1] = b[i + 2];
            uv[c + x] = ((a[i + 1] as u16 + b[i + 1] as u16 + 1) >> 1) as u8;
            uv[c + x + 1] = ((a[i + 3] as u16 + b[i + 3] as u16 + 1) >> 1) as u8;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yuyv_converts() {
        // 2×2: Y 10 20 / 30 40, U 100/120 → 110, V 200/220 → 210.
        let data = [10, 100, 20, 200, 30, 120, 40, 220];
        let mut f = Nv12::new(2, 2);
        assert!(yuyv_to_nv12(&data, 4, &mut f));
        assert_eq!(f.data, vec![10, 20, 30, 40, 110, 210]);
    }

    #[test]
    fn padded_nv12_is_packed() {
        let mut data = vec![0u8; 4 * 2 + 4];
        data[..2].copy_from_slice(&[1, 2]);
        data[4..6].copy_from_slice(&[3, 4]);
        data[8..10].copy_from_slice(&[5, 6]);
        let mut f = Nv12::new(2, 2);
        assert!(nv12_copy(&data, 4, &mut f));
        assert_eq!(f.data, vec![1, 2, 3, 4, 5, 6]);
    }
}

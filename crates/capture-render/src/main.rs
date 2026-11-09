//! `liteaura-capture-render` — render one HTML graphic of a Capture project.
//!
//! ```text
//! liteaura-capture-render --html g.html --width 960 --height 216 --fps 30 --duration 4
//!     → width×height×4 RGBA bytes per frame on stdout (straight alpha)
//! liteaura-capture-render --html g.html --width 960 --height 216 --at 1.5 --png
//!     → one PNG on stdout
//! ```
//!
//! The page runs in Qt WebEngine offscreen with no network (only `data:`,
//! `blob:` and `about:` load), its own throw-away profile, and a virtual
//! clock: before each frame the renderer calls `capture.seek(t)` (injected
//! by Capture), waits for two animation frames and grabs the pixels. The
//! same HTML therefore renders to the same frames every time.

#[cfg(not(no_qt))]
extern "C" {
    fn capture_render_main(argc: i32, argv: *const *const std::ffi::c_char) -> i32;
}

fn main() {
    #[cfg(not(no_qt))]
    {
        let args: Vec<std::ffi::CString> = std::env::args()
            .map(|a| std::ffi::CString::new(a).unwrap_or_default())
            .collect();
        let ptrs: Vec<*const std::ffi::c_char> = args.iter().map(|a| a.as_ptr()).collect();
        // SAFETY: argv outlives the call; the shim only reads it.
        let rc = unsafe { capture_render_main(ptrs.len() as i32, ptrs.as_ptr()) };
        std::process::exit(rc);
    }
    #[cfg(no_qt)]
    {
        eprintln!("liteaura-capture-render was built without Qt WebEngine");
        std::process::exit(2);
    }
}

//! The C ABI declared in `shim/peakd_qt.h`, plus small safe wrappers.
//!
//! Every function here must be called on the Qt main thread. The shell's
//! callbacks (extern "C" fns in `main.rs`) are invoked on that thread too.

use std::ffi::{c_char, c_void, CString};

pub type IpcCallback = extern "C" fn(*mut c_void, *const c_char);
pub type ViewCallback = extern "C" fn(*mut c_void, *const c_char, *const c_char, *const c_char);
pub type PumpCallback = extern "C" fn(*mut c_void);
pub type JsCallback = extern "C" fn(*mut c_void, i32, *const c_char);
/// Returns 1 to block the request, 0 to allow it.
pub type FilterCallback =
    extern "C" fn(*mut c_void, *const c_char, *const c_char, i32, *const c_char) -> i32;
/// Returns a User-Agent to force for this host, or null.
pub type UaCallback = extern "C" fn(*mut c_void, *const c_char) -> *const c_char;
/// One download lifecycle event: `(userdata, id, kind, payload_json)`.
pub type DownloadCallback =
    extern "C" fn(*mut c_void, *const c_char, *const c_char, *const c_char);

extern "C" {
    pub fn peakd_qt_run(
        url: *const c_char,
        data_dir: *const c_char,
        probe: i32,
        ipc: IpcCallback,
        view: ViewCallback,
        pump: PumpCallback,
        js: JsCallback,
        userdata: *mut c_void,
    ) -> i32;
    pub fn peakd_qt_inject_script(name: *const c_char, source: *const c_char);
    pub fn peakd_qt_set_window(title: *const c_char, width: i32, height: i32);
    pub fn peakd_qt_quit();
    pub fn peakd_qt_main_load(url: *const c_char);
    pub fn peakd_qt_main_zoom(factor: f64);
    pub fn peakd_qt_main_run_js(script: *const c_char, callback_id: i32);
    pub fn peakd_qt_screen_dpi() -> f64;
    pub fn peakd_qt_screen_size(width: *mut i32, height: *mut i32);
    pub fn peakd_qt_view_create(
        id: *const c_char,
        url: *const c_char,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        visible: i32,
        incognito: i32,
    );
    pub fn peakd_qt_view_navigate(id: *const c_char, url: *const c_char);
    pub fn peakd_qt_view_bounds(id: *const c_char, x: i32, y: i32, w: i32, h: i32);
    pub fn peakd_qt_view_visible(id: *const c_char, visible: i32);
    pub fn peakd_qt_view_back(id: *const c_char);
    pub fn peakd_qt_view_forward(id: *const c_char);
    pub fn peakd_qt_view_reload(id: *const c_char);
    pub fn peakd_qt_view_focus(id: *const c_char);
    pub fn peakd_qt_view_close(id: *const c_char);
    pub fn peakd_qt_set_filter_cb(cb: FilterCallback, userdata: *mut c_void);
    pub fn peakd_qt_set_ua_cb(cb: UaCallback, userdata: *mut c_void);
    pub fn peakd_qt_set_download_cb(cb: DownloadCallback, userdata: *mut c_void);
    pub fn peakd_qt_set_download_dir(dir: *const c_char);
    pub fn peakd_qt_download_action(id: *const c_char, action: *const c_char);
}

fn cs(value: &str) -> CString {
    CString::new(value).expect("shell strings carry no NUL")
}

pub fn inject_script(name: &str, source: &str) {
    unsafe { peakd_qt_inject_script(cs(name).as_ptr(), cs(source).as_ptr()) }
}

pub fn set_window(title: &str, width: f64, height: f64) {
    unsafe {
        peakd_qt_set_window(
            cs(title).as_ptr(),
            width.round() as i32,
            height.round() as i32,
        )
    }
}

pub fn quit() {
    unsafe { peakd_qt_quit() }
}

pub fn main_load(url: &str) {
    unsafe { peakd_qt_main_load(cs(url).as_ptr()) }
}

pub fn main_zoom(factor: f64) {
    unsafe { peakd_qt_main_zoom(factor) }
}

/// Run JS on the main view; `callback_id` labels the result for [`JsCallback`].
pub fn main_run_js(script: &str, callback_id: i32) {
    unsafe { peakd_qt_main_run_js(cs(script).as_ptr(), callback_id) }
}

pub fn screen_dpi() -> f64 {
    unsafe { peakd_qt_screen_dpi() }
}

pub fn screen_size() -> (i32, i32) {
    let (mut width, mut height) = (0, 0);
    unsafe { peakd_qt_screen_size(&mut width, &mut height) }
    (width, height)
}

pub fn view_create(id: &str, url: &str, x: i32, y: i32, w: i32, h: i32, visible: bool, incognito: bool) {
    unsafe {
        peakd_qt_view_create(
            cs(id).as_ptr(),
            cs(url).as_ptr(),
            x,
            y,
            w,
            h,
            i32::from(visible),
            i32::from(incognito),
        )
    }
}

pub fn view_navigate(id: &str, url: &str) {
    unsafe { peakd_qt_view_navigate(cs(id).as_ptr(), cs(url).as_ptr()) }
}

pub fn view_bounds(id: &str, x: i32, y: i32, w: i32, h: i32) {
    unsafe { peakd_qt_view_bounds(cs(id).as_ptr(), x, y, w, h) }
}

pub fn view_visible(id: &str, visible: bool) {
    unsafe { peakd_qt_view_visible(cs(id).as_ptr(), i32::from(visible)) }
}

pub fn view_back(id: &str) {
    unsafe { peakd_qt_view_back(cs(id).as_ptr()) }
}

pub fn view_forward(id: &str) {
    unsafe { peakd_qt_view_forward(cs(id).as_ptr()) }
}

pub fn view_reload(id: &str) {
    unsafe { peakd_qt_view_reload(cs(id).as_ptr()) }
}

pub fn view_focus(id: &str) {
    unsafe { peakd_qt_view_focus(cs(id).as_ptr()) }
}

pub fn view_close(id: &str) {
    unsafe { peakd_qt_view_close(cs(id).as_ptr()) }
}

pub fn set_filter_cb(cb: FilterCallback) {
    unsafe { peakd_qt_set_filter_cb(cb, std::ptr::null_mut()) }
}

pub fn set_ua_cb(cb: UaCallback) {
    unsafe { peakd_qt_set_ua_cb(cb, std::ptr::null_mut()) }
}

pub fn set_download_cb(cb: DownloadCallback) {
    unsafe { peakd_qt_set_download_cb(cb, std::ptr::null_mut()) }
}

pub fn set_download_dir(dir: &str) {
    unsafe { peakd_qt_set_download_dir(cs(dir).as_ptr()) }
}

pub fn download_action(id: &str, action: &str) {
    unsafe { peakd_qt_download_action(cs(id).as_ptr(), cs(action).as_ptr()) }
}

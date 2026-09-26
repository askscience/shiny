//! Minimal Linux-PAM client over `dlopen`.
//!
//! The helper deliberately does **not** link `libpam` at build time: it must
//! build on any machine (no `libpam0g-dev` headers required) and only needs
//! `libpam.so.0`, which every Debian/Ubuntu installation ships as part of the
//! `libpam0g` package. The C API used here is small and ABI-stable.
//!
//! Only verification is exposed — `pam_authenticate` + `pam_acct_mgmt`. No
//! session, credentials or environment handling is done, so the helper can
//! never leak a shell or a session to a caller.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::OnceLock;

const PAM_SUCCESS: c_int = 0;
const PAM_OPEN_ERR: c_int = 1;
const PAM_SYMBOL_ERR: c_int = 2;
const PAM_SERVICE_ERR: c_int = 3;
const PAM_SYSTEM_ERR: c_int = 4;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;
const PAM_ABORT: c_int = 26;
const PAM_CONV_AGAIN: c_int = 30;
const PAM_INCOMPLETE: c_int = 31;
const PAM_SILENT: c_int = 0x8000;

const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

type ConverseFn = unsafe extern "C" fn(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata_ptr: *mut c_void,
) -> c_int;

#[repr(C)]
struct PamConv {
    conv: Option<ConverseFn>,
    appdata_ptr: *mut c_void,
}

type PamHandle = c_void;

type StartFn = unsafe extern "C" fn(
    service_name: *const c_char,
    user: *const c_char,
    conv: *const PamConv,
    pamh: *mut *mut PamHandle,
) -> c_int;
type AuthFn = unsafe extern "C" fn(pamh: *mut PamHandle, flags: c_int) -> c_int;
type EndFn = unsafe extern "C" fn(pamh: *mut PamHandle, status: c_int) -> c_int;
type StrerrorFn = unsafe extern "C" fn(pamh: *mut PamHandle, errnum: c_int) -> *const c_char;

/// The loaded libpam entry points. `_lib` keeps the handle alive for the
/// process lifetime (we never `dlclose`).
struct PamApi {
    _lib: *mut c_void,
    start: StartFn,
    authenticate: AuthFn,
    acct_mgmt: AuthFn,
    end: EndFn,
    strerror: StrerrorFn,
}

// SAFETY: the function pointers are immutable after load and PAM is re-entrant
// per handle; the helper serialises nothing across them itself.
unsafe impl Send for PamApi {}
unsafe impl Sync for PamApi {}

static API: OnceLock<Result<PamApi, String>> = OnceLock::new();

fn api() -> Result<&'static PamApi, String> {
    API.get_or_init(load).as_ref().map_err(|e| e.clone())
}

fn load() -> Result<PamApi, String> {
    let name = CString::new("libpam.so.0").expect("static string");
    let lib = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if lib.is_null() {
        return Err("could not load libpam.so.0 — is libpam0g installed?".into());
    }

    // SAFETY: each symbol is looked up by name and transmuted to the exact C
    // signature above; a wrong signature would be a programming error, not
    // caller input.
    unsafe fn sym<T: Copy>(lib: *mut c_void, name: &str) -> Result<T, String> {
        let c = CString::new(name).expect("static string");
        let ptr = libc::dlsym(lib, c.as_ptr());
        if ptr.is_null() {
            return Err(format!("libpam is missing symbol `{name}`"));
        }
        Ok(std::mem::transmute_copy::<*mut c_void, T>(&ptr))
    }

    unsafe {
        Ok(PamApi {
            start: sym(lib, "pam_start")?,
            authenticate: sym(lib, "pam_authenticate")?,
            acct_mgmt: sym(lib, "pam_acct_mgmt")?,
            end: sym(lib, "pam_end")?,
            strerror: sym(lib, "pam_strerror")?,
            _lib: lib,
        })
    }
}

/// Credentials handed to PAM through `appdata_ptr`.
struct Creds {
    user: CString,
    password: CString,
}

/// PAM conversation: answer password prompts with the supplied password and
/// username prompts with the supplied username; informational messages get an
/// empty reply. The response array (and its strings) are `malloc`-allocated
/// because PAM frees them itself.
unsafe extern "C" fn converse(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata_ptr: *mut c_void,
) -> c_int {
    if appdata_ptr.is_null() || resp.is_null() || msg.is_null() || num_msg < 0 {
        return PAM_BUF_ERR;
    }
    let creds = &*(appdata_ptr as *const Creds);
    let count = num_msg as usize;

    let responses = libc::calloc(count, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
    if responses.is_null() {
        return PAM_BUF_ERR;
    }

    for i in 0..count {
        let message = *msg.add(i);
        let style = if message.is_null() {
            // Treat a null message as informational.
            3 // PAM_TEXT_INFO
        } else {
            (*message).msg_style
        };
        let answer = match style {
            PAM_PROMPT_ECHO_OFF => Some(creds.password.clone()),
            PAM_PROMPT_ECHO_ON => Some(creds.user.clone()),
            // Informational / error messages take no reply.
            _ => None,
        };
        (*responses.add(i)).resp = match answer {
            Some(text) => libc::strdup(text.as_ptr()),
            None => std::ptr::null_mut(),
        };
        (*responses.add(i)).resp_retcode = 0;
    }

    *resp = responses;
    PAM_SUCCESS
}

/// Outcome of a PAM verification.
pub enum PamError {
    /// PAM could not run at all (library missing, bad service, internal error).
    /// The caller should fall back to local authentication.
    Unavailable(String),
    /// PAM ran and rejected the credentials / account.
    Denied(i32, String),
}

fn cstr(ptr: *const c_char) -> String {
    if ptr.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
    }
}

/// Verify `password` for `user` against the PAM service `service`.
///
/// Runs `pam_authenticate` followed by `pam_acct_mgmt` (so locked/expired
/// accounts are rejected too) and always calls `pam_end`. The password is held
/// only for the duration of the call and never logged.
pub fn verify(service: &str, user: &str, password: &str) -> Result<(), PamError> {
    let api = api().map_err(PamError::Unavailable)?;

    let service_c = CString::new(service)
        .map_err(|_| PamError::Unavailable("invalid PAM service name".into()))?;
    let user_c =
        CString::new(user).map_err(|_| PamError::Unavailable("invalid user name".into()))?;
    let password_c = CString::new(password)
        .map_err(|_| PamError::Unavailable("invalid password".into()))?;

    let mut creds = Creds {
        user: user_c.clone(),
        password: password_c,
    };
    let conv = PamConv {
        conv: Some(converse),
        appdata_ptr: &mut creds as *mut Creds as *mut c_void,
    };

    let mut handle: *mut PamHandle = std::ptr::null_mut();
    let rc = unsafe { (api.start)(service_c.as_ptr(), user_c.as_ptr(), &conv, &mut handle) };
    if rc != PAM_SUCCESS || handle.is_null() {
        return Err(PamError::Unavailable(format!(
            "pam_start failed ({rc}) — is /etc/pam.d/{service} installed?"
        )));
    }

    let auth = unsafe { (api.authenticate)(handle, PAM_SILENT) };
    let status = if auth == PAM_SUCCESS {
        unsafe { (api.acct_mgmt)(handle, PAM_SILENT) }
    } else {
        auth
    };
    let message = cstr(unsafe { (api.strerror)(handle, status) });
    unsafe { (api.end)(handle, status) };

    if status == PAM_SUCCESS {
        Ok(())
    } else if matches!(
        status,
        PAM_OPEN_ERR
            | PAM_SYMBOL_ERR
            | PAM_SERVICE_ERR
            | PAM_SYSTEM_ERR
            | PAM_BUF_ERR
            | PAM_CONV_ERR
            | PAM_ABORT
            | PAM_CONV_AGAIN
            | PAM_INCOMPLETE
    ) {
        // PAM could not run (bad service file, module missing, conversation
        // failure) — this is an availability problem, not a bad password, so
        // the caller may fall back to local authentication.
        Err(PamError::Unavailable(format!("{message} ({status})")))
    } else {
        Err(PamError::Denied(status, message))
    }
}

/// True when libpam could be loaded (used by the `ping` op and startup logs).
pub fn available() -> bool {
    api().is_ok()
}

/// The reason libpam could not be loaded, if any.
pub fn unavailable_reason() -> Option<String> {
    api().err()
}

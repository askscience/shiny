//! Linux OS-user identity.
//!
//! Bridges a Shiny account to a real Linux account so the Files plugin can
//! operate on the user's actual `$HOME` and the web login can (later) verify
//! the real password through PAM. Everything here is read-only NSS lookup via
//! `getpwnam_r`/`getpwuid_r`/`getpwent`; no password material is ever touched.
//!
//! Linux-only by construction: on other platforms (the workspace also builds
//! `peakd` on macOS) every lookup returns `None`, so callers keep the virtual
//! `.shiny/home/<id>` behaviour.

use serde::Serialize;

/// One resolved Linux account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnixUser {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: String,
    pub shell: String,
    /// Full name field from `/etc/passwd` (GECOS), possibly empty.
    pub gecos: String,
}

impl UnixUser {
    /// A regular human account: uid in the user range and a real login shell.
    /// Excludes system accounts (`uid < 1000`) and service shells
    /// (`/usr/sbin/nologin`, `/bin/false`, …) from the login picker.
    pub fn is_human(&self) -> bool {
        self.uid >= 1000 && self.uid < 65534 && is_login_shell(&self.shell)
    }

    /// Display name for the login picker: the GECOS full name when present,
    /// otherwise the login name.
    pub fn display_name(&self) -> String {
        let full = self.gecos.split(',').next().unwrap_or("").trim();
        if full.is_empty() {
            self.name.clone()
        } else {
            full.to_string()
        }
    }
}

fn is_login_shell(shell: &str) -> bool {
    !shell.is_empty()
        && !shell.ends_with("nologin")
        && !shell.ends_with("/false")
        && !shell.ends_with("/sync")
}

/// Resolve one account by login name.
pub fn lookup_name(name: &str) -> Option<UnixUser> {
    imp::lookup_name(name)
}

/// Resolve one account by uid.
pub fn lookup_uid(uid: u32) -> Option<UnixUser> {
    imp::lookup_uid(uid)
}

/// All human accounts, sorted by login name. Used by the login picker.
pub fn list_human_users() -> Vec<UnixUser> {
    let mut users: Vec<UnixUser> = imp::list_all()
        .into_iter()
        .filter(UnixUser::is_human)
        .collect();
    users.sort_by(|a, b| a.name.cmp(&b.name));
    users.dedup_by(|a, b| a.name == b.name);
    users
}

/// True when `name` resolves to a human Linux account. Convenience for
/// validation before binding a Shiny account to it.
pub fn is_human_user(name: &str) -> bool {
    lookup_name(name).map(|u| u.is_human()).unwrap_or(false)
}

// ── Linux implementation ─────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod imp {
    use super::UnixUser;
    use std::ffi::{CStr, CString};
    use std::mem::MaybeUninit;

    /// NSS buffer. Accounts with very long group memberships can exceed this,
    /// in which case the lookup fails closed (returns `None`) rather than
    /// truncating.
    const BUF_LEN: usize = 16 * 1024;

    fn cstr(ptr: *const libc::c_char) -> String {
        if ptr.is_null() {
            return String::new();
        }
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }

    fn from_passwd(pw: &libc::passwd) -> UnixUser {
        UnixUser {
            name: cstr(pw.pw_name),
            uid: pw.pw_uid as u32,
            gid: pw.pw_gid as u32,
            home: cstr(pw.pw_dir),
            shell: cstr(pw.pw_shell),
            gecos: cstr(pw.pw_gecos),
        }
    }

    pub fn lookup_name(name: &str) -> Option<UnixUser> {
        if name.is_empty() || name.contains('\0') {
            return None;
        }
        let cname = CString::new(name).ok()?;
        let mut buf = vec![0 as libc::c_char; BUF_LEN];
        let mut pw: MaybeUninit<libc::passwd> = MaybeUninit::uninit();
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let rc = unsafe {
            libc::getpwnam_r(
                cname.as_ptr(),
                pw.as_mut_ptr(),
                buf.as_mut_ptr(),
                buf.len(),
                &mut result,
            )
        };
        if rc != 0 || result.is_null() {
            return None;
        }
        let pw = unsafe { pw.assume_init() };
        Some(from_passwd(&pw))
    }

    pub fn lookup_uid(uid: u32) -> Option<UnixUser> {
        let mut buf = vec![0 as libc::c_char; BUF_LEN];
        let mut pw: MaybeUninit<libc::passwd> = MaybeUninit::uninit();
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let rc = unsafe {
            libc::getpwuid_r(
                uid as libc::uid_t,
                pw.as_mut_ptr(),
                buf.as_mut_ptr(),
                buf.len(),
                &mut result,
            )
        };
        if rc != 0 || result.is_null() {
            return None;
        }
        let pw = unsafe { pw.assume_init() };
        Some(from_passwd(&pw))
    }

    /// Enumerate the account database. `getpwent` is not thread-safe, so a
    /// global lock serialises it (NSS modules can be slow; the list is tiny
    /// and this is only hit by the login picker).
    pub fn list_all() -> Vec<UnixUser> {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        unsafe {
            libc::setpwent();
            loop {
                let pw = libc::getpwent();
                if pw.is_null() {
                    break;
                }
                out.push(from_passwd(&*pw));
            }
            libc::endpwent();
        }
        out
    }
}

// ── Non-Linux stub ───────────────────────────────────────────────────

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::UnixUser;

    pub fn lookup_name(_name: &str) -> Option<UnixUser> {
        None
    }

    pub fn lookup_uid(_uid: u32) -> Option<UnixUser> {
        None
    }

    pub fn list_all() -> Vec<UnixUser> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_filter_accepts_user_range_with_login_shell() {
        let u = UnixUser {
            name: "eev".into(),
            uid: 1000,
            gid: 1000,
            home: "/home/eev".into(),
            shell: "/bin/bash".into(),
            gecos: "Eev,,,".into(),
        };
        assert!(u.is_human());
        assert_eq!(u.display_name(), "Eev");
    }

    #[test]
    fn human_filter_rejects_system_accounts_and_nologin() {
        let root = UnixUser {
            name: "root".into(),
            uid: 0,
            gid: 0,
            home: "/root".into(),
            shell: "/bin/bash".into(),
            gecos: String::new(),
        };
        assert!(!root.is_human());

        let daemon = UnixUser {
            name: "daemon".into(),
            uid: 1001,
            gid: 1001,
            home: "/".into(),
            shell: "/usr/sbin/nologin".into(),
            gecos: String::new(),
        };
        assert!(!daemon.is_human());
    }

    #[test]
    fn display_name_falls_back_to_login() {
        let u = UnixUser {
            name: "eev".into(),
            uid: 1000,
            gid: 1000,
            home: "/home/eev".into(),
            shell: "/bin/bash".into(),
            gecos: String::new(),
        };
        assert_eq!(u.display_name(), "eev");
    }
}

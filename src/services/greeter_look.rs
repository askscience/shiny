//! The look each account chose (theme, accent, gradient, motion), published
//! for the machine's login screen.
//!
//! The greeter runs as its own system account with its own server and web
//! profile: it cannot read a user's preferences (they live in the user's
//! database and web storage, under a private `~/.local`). So each session
//! server writes the look its user picked in Settings to
//! `<dir>/<login name>.json` — by default `/var/lib/shiny/looks`, a
//! `root:shiny` directory with mode `3775` (group-writable, setgid, sticky:
//! every account in `shiny` can add its own file and only its own) — and the
//! greeter reads it back for the login picker.
//!
//! Only a look is ever written: a theme name, colours, a gradient and a
//! motion style, each validated on the way out and on the way in. The greeter
//! takes a file only when it is a small regular file **owned by the account
//! it is named after**, so no one can dress another account's login.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where looks are kept unless `SHINY_LOOKS_DIR` says otherwise.
pub const DEFAULT_DIR: &str = "/var/lib/shiny/looks";
/// A look is a few hundred bytes; anything bigger is not one.
const MAX_BYTES: u64 = 4096;
const MOTIONS: [&str; 3] = ["kinetic", "fluid", "depth"];

/// The directory looks are published to.
pub fn dir() -> PathBuf {
    std::env::var_os("SHINY_LOOKS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DIR))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default)]
    pub angle: f64,
    pub stops: Vec<String>,
}

/// One account's look.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Look {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<Gradient>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<String>,
    /// When it was published (seconds since the epoch); the login screen
    /// opens on the most recent one. Set by [`read`] from the file's time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<u64>,
}

fn is_hex_colour(s: &str) -> bool {
    let hex = s.strip_prefix('#').unwrap_or("");
    (hex.len() == 6 || hex.len() == 3) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl Look {
    /// The same look with anything that is not a theme name, a colour, a
    /// gradient of 2–8 colours or a known motion style dropped.
    pub fn sanitized(&self) -> Look {
        let gradient = self.gradient.as_ref().and_then(|g| {
            let ok = (2..=8).contains(&g.stops.len())
                && g.stops.iter().all(|s| is_hex_colour(s))
                && g.angle.is_finite();
            ok.then(|| Gradient {
                id: g.id.clone().filter(|id| is_token(id)),
                angle: g.angle.rem_euclid(360.0),
                stops: g.stops.iter().map(|s| s.to_ascii_lowercase()).collect(),
            })
        });
        Look {
            theme: self.theme.clone().filter(|t| is_token(t)),
            accent: self
                .accent
                .clone()
                .filter(|a| is_hex_colour(a))
                .map(|a| a.to_ascii_lowercase()),
            gradient,
            motion: self
                .motion
                .clone()
                .filter(|m| MOTIONS.contains(&m.as_str())),
            updated: None,
        }
    }

    fn is_empty(&self) -> bool {
        self.theme.is_none()
            && self.accent.is_none()
            && self.gradient.is_none()
            && self.motion.is_none()
    }
}

fn file_for(dir: &Path, user: &str) -> Option<PathBuf> {
    // A login name, never a path.
    let ok = !user.is_empty()
        && user.len() <= 64
        && !user.starts_with('.')
        && user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    ok.then(|| dir.join(format!("{user}.json")))
}

/// Publish `user`'s look into `dir`: written to a temporary file in the same
/// directory and renamed over the old one, so the greeter never reads half a
/// file. World-readable (it is only colours). A missing directory is not an
/// error — the machine simply has no greeter to dress.
pub fn publish(dir: &Path, user: &str, look: &Look) -> std::io::Result<bool> {
    let Some(path) = file_for(dir, user) else {
        return Ok(false);
    };
    if !dir.is_dir() {
        return Ok(false);
    }
    let look = look.sanitized();
    if look.is_empty() {
        return Ok(false);
    }
    let body = serde_json::to_vec(&look).map_err(std::io::Error::other)?;
    if std::fs::read(&path).ok().as_deref() == Some(body.as_slice()) {
        return Ok(true);
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{user}.{}.{nonce}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o644);
    let written = options
        .open(&tmp)
        .and_then(|mut file| {
            std::io::Write::write_all(&mut file, &body)?;
            // Explicitly, past the umask: the greeter reads it as another account.
            #[cfg(unix)]
            file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o644))?;
            Ok(())
        })
        .and_then(|()| std::fs::rename(&tmp, &path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written.map(|()| true)
}

/// `user`'s published look, when the file is theirs (owned by `uid`), small
/// and valid.
pub fn read(dir: &Path, user: &str, uid: u32) -> Option<Look> {
    let path = file_for(dir, user)?;
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != uid {
            return None;
        }
    }
    #[cfg(not(unix))]
    let _ = uid;
    let look: Look = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    let mut look = look.sanitized();
    if look.is_empty() {
        return None;
    }
    look.updated = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    Some(look)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid() -> u32 {
        #[cfg(unix)]
        {
            unsafe { libc::getuid() }
        }
        #[cfg(not(unix))]
        {
            0
        }
    }

    fn look() -> Look {
        Look {
            theme: Some("ink".into()),
            accent: Some("#4F7CFF".into()),
            gradient: Some(Gradient {
                id: Some("aurora".into()),
                angle: 495.0,
                stops: vec!["#112233".into(), "#445566".into()],
            }),
            motion: Some("fluid".into()),
            updated: None,
        }
    }

    #[test]
    fn a_published_look_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        assert!(publish(dir.path(), "eev", &look()).unwrap());
        let back = read(dir.path(), "eev", uid()).unwrap();
        assert_eq!(back.theme.as_deref(), Some("ink"));
        assert_eq!(back.accent.as_deref(), Some("#4f7cff"));
        assert_eq!(back.gradient.as_ref().unwrap().angle, 135.0);
        assert_eq!(back.motion.as_deref(), Some("fluid"));
        assert!(back.updated.is_some());
        // Publishing the same look again leaves the file alone.
        assert!(publish(dir.path(), "eev", &look()).unwrap());
    }

    #[test]
    fn a_file_owned_by_someone_else_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        publish(dir.path(), "eev", &look()).unwrap();
        assert!(read(dir.path(), "eev", uid().wrapping_add(1)).is_none());
    }

    #[test]
    fn anything_but_a_look_is_dropped() {
        let hostile = Look {
            theme: Some("../../etc".into()),
            accent: Some("red; background: url(x)".into()),
            gradient: Some(Gradient {
                id: None,
                angle: f64::NAN,
                stops: vec!["#fff".into()],
            }),
            motion: Some("spin".into()),
            updated: None,
        };
        let dir = tempfile::tempdir().unwrap();
        assert!(
            !publish(dir.path(), "eev", &hostile).unwrap(),
            "nothing valid, nothing written"
        );
        assert!(
            !publish(dir.path(), "../eev", &look()).unwrap(),
            "a name, never a path"
        );
        std::fs::write(dir.path().join("big.json"), vec![b' '; 5000]).unwrap();
        assert!(read(dir.path(), "big", uid()).is_none());
    }

    #[test]
    fn no_directory_no_greeter() {
        assert!(!publish(Path::new("/nonexistent/looks"), "eev", &look()).unwrap());
    }
}

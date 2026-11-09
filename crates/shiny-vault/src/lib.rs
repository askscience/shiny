//! Sealing primitives shared by LiteAura plugins.
//!
//! - [`seal`] / [`open`]: AES-256-GCM, stored as base64 `nonce || ciphertext`,
//!   with associated data that binds a value to its owner and slot (a row
//!   copied to another account or id never opens).
//! - [`derive`]: Argon2id key stretching for password-sealed keys (Browser).
//! - [`machine_key`]: a random key kept in a 0600 file next to the plugin's
//!   data, for secrets that must open without a password (Capture's stream
//!   keys): a copied database alone reveals nothing.
//!
//! Linked as an rlib into each plugin; nothing here crosses the plugin ABI.

use std::io::Write;
use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
pub use zeroize::Zeroizing;

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 12;

pub type Key = Zeroizing<[u8; KEY_LEN]>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// `N` bytes from the OS.
pub fn random<const N: usize>() -> Result<[u8; N]> {
    let mut buf = [0u8; N];
    getrandom::getrandom(&mut buf).map_err(|e| Error(format!("no system randomness: {e}")))?;
    Ok(buf)
}

/// Argon2id cost parameters (memory KiB, passes, lanes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfCost {
    pub m: u32,
    pub t: u32,
    pub p: u32,
}

/// Stretch `password` into a key.
pub fn derive(password: &str, salt: &[u8], cost: KdfCost) -> Result<Key> {
    let params = Params::new(cost.m, cost.t, cost.p, Some(KEY_LEN))
        .map_err(|e| Error(format!("vault cost: {e}")))?;
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|e| Error(format!("vault key: {e}")))?;
    Ok(key)
}

/// AES-256-GCM, returned as base64 `nonce || ciphertext`.
pub fn seal(key: &[u8; KEY_LEN], plaintext: &[u8], aad: &str) -> Result<String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| Error("vault key length".into()))?;
    let nonce = random::<NONCE_LEN>()?;
    let ct = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| Error("could not seal".into()))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(B64.encode(out))
}

/// The inverse of [`seal`]; `None` for a wrong key, a tampered value or a
/// value sealed for another slot.
pub fn open(key: &[u8; KEY_LEN], sealed: &str, aad: &str) -> Option<Zeroizing<Vec<u8>>> {
    let raw = B64.decode(sealed).ok()?;
    if raw.len() <= NONCE_LEN {
        return None;
    }
    let (nonce, ct) = raw.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ct,
                aad: aad.as_bytes(),
            },
        )
        .ok()
        .map(Zeroizing::new)
}

/// The key stored at `path`, created (random, mode 0600) on first use.
pub fn machine_key(path: &Path) -> Result<Key> {
    if let Ok(raw) = std::fs::read(path) {
        let raw = Zeroizing::new(raw);
        let bytes: [u8; KEY_LEN] = raw
            .as_slice()
            .try_into()
            .map_err(|_| Error(format!("{} is not a key", path.display())))?;
        return Ok(Zeroizing::new(bytes));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error(format!("key dir: {e}")))?;
    }
    let key = Zeroizing::new(random::<KEY_LEN>()?);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    match opts.open(path) {
        Ok(mut f) => {
            f.write_all(key.as_ref())
                .map_err(|e| Error(format!("key write: {e}")))?;
            Ok(key)
        }
        // Lost a race with another caller: use theirs.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => machine_key(path),
        Err(e) => Err(Error(format!("key create: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_round_trip_and_binding() {
        let key = [7u8; KEY_LEN];
        let s = seal(&key, b"secret", "slot:a").unwrap();
        assert_eq!(open(&key, &s, "slot:a").unwrap().as_slice(), b"secret");
        assert!(open(&key, &s, "slot:b").is_none());
        assert!(open(&[8u8; KEY_LEN], &s, "slot:a").is_none());
    }

    #[test]
    fn machine_key_is_stable_and_private() {
        let dir = std::env::temp_dir().join(format!("shiny-vault-{}", std::process::id()));
        let path = dir.join("k");
        let a = machine_key(&path).unwrap();
        let b = machine_key(&path).unwrap();
        assert_eq!(*a, *b);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn derive_is_deterministic() {
        let c = KdfCost { m: 64, t: 1, p: 1 };
        assert_eq!(
            *derive("pw", b"saltsalt", c).unwrap(),
            *derive("pw", b"saltsalt", c).unwrap()
        );
    }
}

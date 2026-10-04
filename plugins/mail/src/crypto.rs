//! Password encryption at rest.
//!
//! Account passwords are stored as
//! `enc:v1:<hex(nonce || AES-256-GCM ciphertext)>`. The 32-byte key lives in
//! `data/mail.key` (mode 0600), or at `SHINY_MAIL_KEY_FILE` when set; it is
//! generated on first use. Values without the `enc:v1:` prefix (rows written
//! before this landed) are returned as-is by [`decrypt`] and re-encrypted the
//! next time the account is loaded or saved, so existing installs migrate
//! lazily without a schema change.

use std::io::Write;
use std::path::PathBuf;

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};
use shiny_plugin_sdk::errors::AppError;

const PREFIX: &str = "enc:v1:";
const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;

fn key_path() -> PathBuf {
    std::env::var("SHINY_MAIL_KEY_FILE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/mail.key"))
}

fn read_key_file(path: &std::path::Path) -> Result<[u8; KEY_LEN], AppError> {
    let bytes = std::fs::read(path)
        .map_err(|e| AppError::Internal(format!("mail key {}: {e}", path.display())))?;
    if bytes.len() != KEY_LEN {
        return Err(AppError::Internal(format!(
            "mail key {} must be {KEY_LEN} bytes",
            path.display()
        )));
    }
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&bytes);
    Ok(key)
}

fn key_bytes() -> Result<[u8; KEY_LEN], AppError> {
    let path = key_path();
    if path.exists() {
        return read_key_file(&path);
    }
    let mut key = [0u8; KEY_LEN];
    SystemRandom::new()
        .fill(&mut key)
        .map_err(|_| AppError::Internal("mail key generation failed".into()))?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AppError::Internal(format!("mail key dir: {e}")))?;
        }
    }
    // `create_new` makes concurrent first-use races safe: the loser re-reads.
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
            }
            file.write_all(&key)
                .map_err(|e| AppError::Internal(format!("mail key write: {e}")))?;
            Ok(key)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => read_key_file(&path),
        Err(e) => Err(AppError::Internal(format!("mail key create: {e}"))),
    }
}

fn aead() -> Result<LessSafeKey, AppError> {
    let key = key_bytes()?;
    let unbound = UnboundKey::new(&AES_256_GCM, &key)
        .map_err(|_| AppError::Internal("mail key rejected".into()))?;
    Ok(LessSafeKey::new(unbound))
}

/// Whether a stored value is already in the encrypted envelope.
pub fn is_encrypted(stored: &str) -> bool {
    stored.starts_with(PREFIX)
}

/// Encrypt a password for storage. The empty string stays empty (accounts may
/// legitimately have no password yet). Returns an `AppError` when the key is
/// unavailable or unusable — never silently falls back to plaintext.
pub fn encrypt(plain: &str) -> Result<String, AppError> {
    if plain.is_empty() {
        return Ok(String::new());
    }
    let key = aead()?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce_bytes)
        .map_err(|_| AppError::Internal("mail nonce failed".into()))?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let mut in_out = plain.as_bytes().to_vec();
    key.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
        .map_err(|_| AppError::Internal("mail encryption failed".into()))?;
    let mut out = Vec::with_capacity(NONCE_LEN + in_out.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&in_out);
    Ok(format!("{PREFIX}{}", hex::encode(out)))
}

/// Decrypt a stored password. Non-prefixed values are legacy plaintext and are
/// returned unchanged (the caller re-saves them through [`encrypt`]).
pub fn decrypt(stored: &str) -> Result<String, AppError> {
    let Some(hexed) = stored.strip_prefix(PREFIX) else {
        return Ok(stored.to_string());
    };
    let raw = hex::decode(hexed).map_err(|_| AppError::Internal("mail secret is corrupt".into()))?;
    if raw.len() < NONCE_LEN + TAG_LEN {
        return Err(AppError::Internal("mail secret is corrupt".into()));
    }
    let key = aead()?;
    let (nonce_bytes, ciphertext) = raw.split_at(NONCE_LEN);
    let nonce = Nonce::try_assume_unique_for_key(nonce_bytes)
        .map_err(|_| AppError::Internal("mail secret is corrupt".into()))?;
    let mut buf = ciphertext.to_vec();
    let plain = key
        .open_in_place(nonce, Aad::empty(), &mut buf)
        .map_err(|_| AppError::Internal("mail secret could not be decrypted".into()))?;
    String::from_utf8(plain.to_vec())
        .map_err(|_| AppError::Internal("mail secret is not UTF-8".into()))
}

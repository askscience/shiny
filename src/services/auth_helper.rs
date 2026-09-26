//! Client for the privileged `shiny-auth` PAM helper.
//!
//! The web server is unprivileged and cannot read `/etc/shadow`, so it cannot
//! verify a Linux password itself. When `SHINY_AUTH_ENABLED=true` it asks the
//! root-owned `shiny-auth` helper over a local Unix socket instead. The helper
//! is optional: when it is missing or unreachable every call degrades to
//! [`VerifyOutcome::Unavailable`] and the caller falls back to the local
//! Argon2 hash.

use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Result of asking the helper to verify a password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// The Linux password is correct.
    Ok,
    /// The helper ran and rejected the credentials (or the account).
    Denied,
    /// The helper could not be reached or could not run — fall back to local.
    Unavailable,
}

/// Verify `user`'s Linux password through the helper at `sock`.
///
/// Never panics and never propagates the password anywhere but the socket. A
/// hard timeout bounds a hung helper so a login request cannot stall forever.
pub async fn verify(sock: &str, user: &str, password: &str) -> VerifyOutcome {
    let request = serde_json::json!({
        "op": "verify",
        "user": user,
        "password": password,
    })
    .to_string();

    match tokio::time::timeout(Duration::from_secs(15), roundtrip(sock, &request)).await {
        Ok(Ok(response)) => match response.get("ok").and_then(Value::as_bool) {
            Some(true) => VerifyOutcome::Ok,
            Some(false) => match response.get("code").and_then(Value::as_str) {
                // The helper ran but could not use PAM — let the caller fall back.
                Some("unavailable") => VerifyOutcome::Unavailable,
                _ => VerifyOutcome::Denied,
            },
            None => VerifyOutcome::Unavailable,
        },
        Ok(Err(e)) => {
            tracing::debug!("shiny-auth helper unavailable: {e}");
            VerifyOutcome::Unavailable
        }
        Err(_) => {
            tracing::warn!("shiny-auth helper timed out");
            VerifyOutcome::Unavailable
        }
    }
}

/// Is the helper reachable and ready to verify? Used by diagnostics only.
pub async fn ping(sock: &str) -> bool {
    let request = r#"{"op":"ping"}"#.to_string();
    matches!(
        tokio::time::timeout(Duration::from_secs(3), roundtrip(sock, &request)).await,
        Ok(Ok(v)) if v.get("ok").and_then(Value::as_bool) == Some(true)
    )
}

async fn roundtrip(sock: &str, line: &str) -> std::io::Result<Value> {
    let stream = UnixStream::connect(sock).await?;
    let (read_half, mut write_half) = stream.into_split();
    write_half.write_all(line.as_bytes()).await?;
    write_half.write_all(b"\n").await?;
    write_half.flush().await?;

    let mut reader = BufReader::new(read_half);
    let mut buf = String::new();
    reader.read_line(&mut buf).await?;
    serde_json::from_str(buf.trim())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

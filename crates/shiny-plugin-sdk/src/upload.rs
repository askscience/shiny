//! Upload helpers shared by plugins that accept multipart bodies.
//!
//! axum's `DefaultBodyLimit` is applied by *extractors* (`Json`, `Bytes`, …).
//! Plugin route handlers receive a raw `Request` and read the body themselves,
//! so the limit does not apply to them: a plugin that calls
//! `field.bytes().await` buffers the whole field with no ceiling. In a `dlopen`
//! plugin this is host-process memory, so one oversized upload aborts the app
//! for every user. Use [`field_bytes_capped`] instead.

use crate::errors::AppError;

/// Read a multipart field into memory, refusing anything over `max` bytes.
///
/// The check is enforced on the streamed chunks, not on the (forgeable)
/// `Content-Length` header.
pub async fn field_bytes_capped(
    mut field: axum::extract::multipart::Field<'_>,
    max: usize,
) -> Result<Vec<u8>, AppError> {
    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|e| AppError::BadRequest(format!("read error: {e}")))?
    {
        if buf.len() + chunk.len() > max {
            return Err(AppError::BadRequest(format!(
                "upload is too large ({} MiB max)",
                max / (1024 * 1024)
            )));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

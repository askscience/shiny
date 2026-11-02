//! Upload helpers shared by plugins that accept multipart bodies.
//!
//! Two traps for a `dlopen` plugin reading an upload:
//!
//! - **No ceiling.** Plugin route handlers receive a raw `Request`; a plugin
//!   that calls `field.bytes().await` buffers the whole field with no limit.
//!   This is host-process memory, so one oversized upload aborts the app for
//!   every user.
//! - **A hidden 2 MB ceiling.** axum's `Multipart` extractor caps the body at
//!   its 2 MB default unless it finds core's `DefaultBodyLimit` extension — and
//!   the plugin's own copy of axum never sees core's. Every file over 2 MB then
//!   fails with "Error parsing `multipart/form-data` request".
//!
//! Use [`read_upload_file`], which parses the multipart body itself and
//! enforces the plugin's own limit on the streamed chunks.

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

/// Read the field `file` of a `multipart/form-data` request: its bytes and the
/// uploaded file name. Refuses bodies over `max` bytes (checked on the streamed
/// chunks) and requests without that field.
///
/// Prefer this to `axum::extract::Multipart`, which silently limits plugin
/// uploads to 2 MB (see the module docs).
pub async fn read_upload_file(
    req: axum::extract::Request,
    max: usize,
) -> Result<(Vec<u8>, Option<String>), AppError> {
    let boundary = req
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|ct| multer::parse_boundary(ct).ok())
        .ok_or_else(|| AppError::BadRequest("expected a multipart/form-data upload".into()))?;
    let mut multipart = multer::Multipart::new(req.into_body().into_data_stream(), boundary);
    let mut found: Option<(Vec<u8>, Option<String>)> = None;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("upload error: {e}")))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let name = field.file_name().map(str::to_string);
        let mut buf: Vec<u8> = Vec::with_capacity(256 * 1024);
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| AppError::BadRequest(format!("upload error: {e}")))?
        {
            if buf.len() + chunk.len() > max {
                return Err(AppError::BadRequest(format!(
                    "the file is too large ({} MB max)",
                    max / (1024 * 1024)
                )));
            }
            buf.extend_from_slice(&chunk);
        }
        found = Some((buf, name));
    }
    found.ok_or_else(|| AppError::BadRequest("missing 'file' field".into()))
}

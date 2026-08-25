//! Document storage for the word plugin — core-owned table (`documents`),
//! documents stored as real OpenDocument Text (.odt) bytes via the SDK codec.
//! Same pattern as `saved_artifacts`: core owns the schema, the plugin owns
//! the domain logic (its tools write through the same table).

use serde::Serialize;
use sqlx::SqlitePool;

use crate::errors::AppError;
use shiny_plugin_sdk::odt;

#[derive(Debug, Clone, Serialize)]
pub struct DocumentSummary {
    pub id: String,
    pub title: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Document {
    pub id: String,
    pub title: String,
    pub html: String,
    pub updated_at: String,
}

pub async fn list_documents(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<DocumentSummary>, AppError> {
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT id, title, updated_at FROM documents \
         WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 200",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, title, updated_at)| DocumentSummary {
            id,
            title,
            updated_at,
        })
        .collect())
}

pub async fn create_document(
    pool: &SqlitePool,
    user_id: &str,
    title: &str,
    html: &str,
) -> Result<Document, AppError> {
    let id = uuid::Uuid::new_v4().to_string();
    let title = clean_title(title);
    let odt_bytes = odt::html_to_odt(&title, html)?;

    sqlx::query(
        "INSERT INTO documents (id, user_id, title, odt, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))",
    )
    .bind(&id)
    .bind(user_id)
    .bind(&title)
    .bind(&odt_bytes)
    .execute(pool)
    .await?;

    Ok(Document {
        id,
        title,
        html: html.to_string(),
        updated_at: "now".into(),
    })
}

pub async fn load_document_html(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
) -> Result<Document, AppError> {
    let row = sqlx::query_as::<_, (String, Vec<u8>, String)>(
        "SELECT title, odt, updated_at FROM documents WHERE id = ?1 AND user_id = ?2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Document not found".into()))?;

    let (title, odt_bytes, updated_at) = row;
    let html = odt::odt_to_html(&odt_bytes)?;

    Ok(Document {
        id: id.to_string(),
        title,
        html,
        updated_at,
    })
}

/// Load the raw .odt bytes (used by export and by the plugin's AI tools).
pub async fn load_document_odt(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
) -> Result<(String, Vec<u8>), AppError> {
    let row = sqlx::query_as::<_, (String, Vec<u8>)>(
        "SELECT title, odt FROM documents WHERE id = ?1 AND user_id = ?2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Document not found".into()))?;
    Ok(row)
}

pub async fn save_document_html(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
    title: &str,
    html: &str,
) -> Result<(), AppError> {
    let title = clean_title(title);
    let odt_bytes = odt::html_to_odt(&title, html)?;
    let result = sqlx::query(
        "UPDATE documents SET title = ?1, odt = ?2, updated_at = datetime('now') \
         WHERE id = ?3 AND user_id = ?4",
    )
    .bind(&title)
    .bind(&odt_bytes)
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Document not found".into()));
    }
    Ok(())
}

/// Insert a raw .odt file (import path). Returns the stored title.
pub async fn import_document_odt(
    pool: &SqlitePool,
    user_id: &str,
    title: &str,
    odt_bytes: &[u8],
) -> Result<String, AppError> {
    // Validate before storing so garbage never lands in the table.
    let _ = odt::odt_to_html(odt_bytes)?;
    let id = uuid::Uuid::new_v4().to_string();
    let title = clean_title(title);

    sqlx::query(
        "INSERT INTO documents (id, user_id, title, odt, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))",
    )
    .bind(&id)
    .bind(user_id)
    .bind(&title)
    .bind(odt_bytes)
    .execute(pool)
    .await?;

    Ok(title)
}

pub async fn delete_document(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
) -> Result<bool, AppError> {
    let result = sqlx::query("DELETE FROM documents WHERE id = ?1 AND user_id = ?2")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub fn clean_title(title: &str) -> String {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        "Untitled".into()
    } else {
        trimmed.chars().take(120).collect()
    }
}

/// Sanitize a title for use in a Content-Disposition filename.
pub fn filename_for_title(title: &str) -> String {
    let clean: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let clean = clean.trim().trim_matches('.').to_string();
    let clean = if clean.is_empty() { "document".to_string() } else { clean };
    format!("{}.odt", clean)
}
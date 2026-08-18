//! Saved-artifact persistence for plugin tools (`update_artifact`).
//! Same `saved_artifacts` table core uses; the plugin reads/merges/writes
//! payloads directly.

use sqlx::SqlitePool;

use shiny_plugin_sdk::artifacts::Artifact;
use shiny_plugin_sdk::errors::AppError;

#[derive(Debug, sqlx::FromRow)]
struct SavedArtifactRow {
    payload_json: String,
}

pub async fn load_artifact(
    pool: &SqlitePool,
    traveler_id: &str,
    id: &str,
) -> Result<Artifact, AppError> {
    let row = sqlx::query_as::<_, SavedArtifactRow>(
        "SELECT payload_json FROM saved_artifacts WHERE id = ?1 AND traveler_id = ?2",
    )
    .bind(id)
    .bind(traveler_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Artifact not found".into()))?;

    serde_json::from_str(&row.payload_json)
        .map_err(|e| AppError::Internal(format!("Invalid artifact JSON: {}", e)))
}

pub async fn save_artifact(
    pool: &SqlitePool,
    traveler_id: &str,
    artifact: &Artifact,
) -> Result<Artifact, AppError> {
    let payload = serde_json::to_string(artifact)
        .map_err(|e| AppError::Internal(format!("Failed to serialize artifact: {}", e)))?;

    sqlx::query(
        "INSERT INTO saved_artifacts (id, traveler_id, trip_id, artifact_type, title, payload_json, created_at, updated_at) \
         VALUES (?1, ?2, NULL, ?3, ?4, ?5, datetime('now'), datetime('now')) \
         ON CONFLICT(id) DO UPDATE SET \
           artifact_type = excluded.artifact_type, \
           title = excluded.title, \
           payload_json = excluded.payload_json, \
           updated_at = datetime('now')",
    )
    .bind(&artifact.id)
    .bind(traveler_id)
    .bind(&artifact.artifact_type)
    .bind(&artifact.title)
    .bind(&payload)
    .execute(pool)
    .await?;

    Ok(artifact.clone())
}

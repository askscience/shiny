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
    plugin: &str,
) -> Result<Artifact, AppError> {
    // Preserve core's plugin attribution across updates: keep the stored
    // `plugin` key when present, self-tag otherwise.
    let existing: Option<String> = sqlx::query_scalar::<_, Option<String>>(
        "SELECT json_extract(payload_json, '$.plugin') FROM saved_artifacts \
         WHERE id = ?1 AND traveler_id = ?2",
    )
    .bind(&artifact.id)
    .bind(traveler_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    let owner = existing.unwrap_or_else(|| plugin.to_string());

    let mut payload = serde_json::to_value(artifact)
        .map_err(|e| AppError::Internal(format!("Failed to serialize artifact: {}", e)))?;
    if let serde_json::Value::Object(map) = &mut payload {
        map.insert("plugin".into(), serde_json::Value::String(owner));
    }
    let payload = serde_json::to_string(&payload)
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

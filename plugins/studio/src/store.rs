//! Shared SQLite data layer for the plugin-owned `studio_tracks` table.
//!
//! Both the agent tools and the REST routes read/write through these helpers so
//! the AI and the Studio window always observe the same rows.

use serde_json::{json, Value};
use sqlx::SqlitePool;

/// Row shape: id, title, bpm, steps, tuning, duration_ms, wav, config_json, updated_at.
pub type Row = (
    String,
    String,
    f64,
    i64,
    String,
    i64,
    Option<Vec<u8>>,
    String,
    String,
);

/// Summarize a stored config into (voice count, kinds).
fn config_summary(config_json: &str) -> (usize, Vec<String>) {
    let parsed: Value = serde_json::from_str(config_json).unwrap_or(Value::Null);
    let mut kinds = Vec::new();
    if let Some(arr) = parsed.get("voices").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(k) = v.get("kind").and_then(|k| k.as_str()) {
                kinds.push(k.to_string());
            }
        }
    }
    (kinds.len(), kinds)
}

/// Compact metadata for list/get payloads.
pub fn meta_json(r: &Row) -> Value {
    let (voices, kinds) = config_summary(&r.7);
    json!({
        "track_id": r.0,
        "title": r.1,
        "bpm": r.2,
        "steps": r.3,
        "tuning": r.4,
        "duration_ms": r.5,
        "has_audio": r.6.is_some(),
        "voices": voices,
        "kinds": kinds,
        "updated_at": r.8,
    })
}

/// Metadata plus the full `config` object (for reloading a track into the grid).
pub fn full_json(r: &Row) -> Value {
    let mut v = meta_json(r);
    if let Some(obj) = v.as_object_mut() {
        let cfg: Value = serde_json::from_str(&r.7).unwrap_or_else(|_| json!({}));
        obj.insert("config".into(), cfg);
    }
    v
}

/// List the caller's tracks, newest first.
pub async fn list(pool: &SqlitePool, user_id: &str) -> sqlx::Result<Vec<Row>> {
    sqlx::query_as::<_, Row>(
        "SELECT id, title, bpm, steps, tuning, duration_ms, wav, config_json, updated_at \
         FROM studio_tracks WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 100",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Fetch one track by id (scoped to the user).
pub async fn get(pool: &SqlitePool, user_id: &str, id: &str) -> sqlx::Result<Option<Row>> {
    sqlx::query_as::<_, Row>(
        "SELECT id, title, bpm, steps, tuning, duration_ms, wav, config_json, updated_at \
         FROM studio_tracks WHERE id = ?1 AND user_id = ?2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

/// Resolve an `id_or_title` param to a real id (accepts UUID or exact title).
pub async fn resolve_id(
    pool: &SqlitePool,
    user_id: &str,
    id_or_title: Option<String>,
) -> sqlx::Result<Option<String>> {
    let Some(v) = id_or_title.filter(|s| !s.trim().is_empty()) else {
        return Ok(None);
    };
    let v = v.trim();

    let by_id: Option<String> =
        sqlx::query_scalar("SELECT id FROM studio_tracks WHERE id = ?1 AND user_id = ?2")
            .bind(v)
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    if by_id.is_some() {
        return Ok(by_id);
    }

    sqlx::query_scalar(
        "SELECT id FROM studio_tracks WHERE lower(title) = lower(?1) AND user_id = ?2 \
         ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(v)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

/// Insert a freshly rendered track.
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &SqlitePool,
    id: &str,
    user_id: &str,
    cfg_json: &str,
    title: &str,
    bpm: f64,
    steps: i64,
    tuning: &str,
    duration_ms: i64,
    wav: &[u8],
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO studio_tracks \
         (id, user_id, title, bpm, steps, tuning, config_json, duration_ms, wav) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(id)
    .bind(user_id)
    .bind(title)
    .bind(bpm)
    .bind(steps)
    .bind(tuning)
    .bind(cfg_json)
    .bind(duration_ms)
    .bind(wav)
    .execute(pool)
    .await?;
    Ok(())
}

/// Update a track's editable fields without re-rendering. Returns true when a row changed.
#[allow(clippy::too_many_arguments)]
pub async fn update_config(
    pool: &SqlitePool,
    id: &str,
    user_id: &str,
    cfg_json: &str,
    title: &str,
    bpm: f64,
    steps: i64,
    tuning: &str,
) -> sqlx::Result<bool> {
    let res = sqlx::query(
        "UPDATE studio_tracks SET title = ?3, bpm = ?4, steps = ?5, tuning = ?6, \
         config_json = ?7, updated_at = datetime('now') WHERE id = ?1 AND user_id = ?2",
    )
    .bind(id)
    .bind(user_id)
    .bind(title)
    .bind(bpm)
    .bind(steps)
    .bind(tuning)
    .bind(cfg_json)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Replace a track's rendered audio. Returns true when a row changed.
pub async fn update_render(
    pool: &SqlitePool,
    id: &str,
    user_id: &str,
    duration_ms: i64,
    wav: &[u8],
) -> sqlx::Result<bool> {
    let res = sqlx::query(
        "UPDATE studio_tracks SET wav = ?3, duration_ms = ?4, updated_at = datetime('now') \
         WHERE id = ?1 AND user_id = ?2",
    )
    .bind(id)
    .bind(user_id)
    .bind(wav)
    .bind(duration_ms)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Delete a track. Returns its title when a row was removed.
pub async fn delete(pool: &SqlitePool, id: &str, user_id: &str) -> sqlx::Result<Option<String>> {
    let title: Option<String> =
        sqlx::query_scalar("SELECT title FROM studio_tracks WHERE id = ?1 AND user_id = ?2")
            .bind(id)
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    let res = sqlx::query("DELETE FROM studio_tracks WHERE id = ?1 AND user_id = ?2")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    if res.rows_affected() > 0 {
        Ok(title)
    } else {
        Ok(None)
    }
}

// ─────────────────────────────────────────────────────────────
// Arrangements (multi-clip timeline layouts)
// ─────────────────────────────────────────────────────────────

/// Arrangement row: id, title, bpm, length_beats, master, config_json, updated_at.
pub type ArrRow = (String, String, f64, f64, f64, String, String);

fn arr_cfg(r: &ArrRow) -> Value {
    serde_json::from_str(&r.5).unwrap_or(json!({}))
}

/// Compact metadata for the arrangement list.
pub fn arr_meta_json(r: &ArrRow) -> Value {
    let cfg = arr_cfg(r);
    let tracks = cfg.get("tracks").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    let clips = cfg.get("clips").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    json!({
        "id": r.0, "title": r.1, "bpm": r.2, "length_beats": r.3, "master": r.4,
        "tracks": tracks, "clips": clips, "updated_at": r.6,
    })
}

/// Full arrangement (flattened) for loading.
pub fn arr_full_json(r: &ArrRow) -> Value {
    let cfg = arr_cfg(r);
    json!({
        "id": r.0, "title": r.1, "bpm": r.2, "length_beats": r.3, "master": r.4,
        "tracks": cfg.get("tracks").cloned().unwrap_or(json!([])),
        "clips": cfg.get("clips").cloned().unwrap_or(json!([])),
        "updated_at": r.6,
    })
}

pub async fn list_arrangements(pool: &SqlitePool, user_id: &str) -> sqlx::Result<Vec<ArrRow>> {
    sqlx::query_as::<_, ArrRow>(
        "SELECT id, title, bpm, length_beats, master, config_json, updated_at \
         FROM studio_arrangements WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 100",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn get_arrangement(pool: &SqlitePool, user_id: &str, id: &str) -> sqlx::Result<Option<ArrRow>> {
    sqlx::query_as::<_, ArrRow>(
        "SELECT id, title, bpm, length_beats, master, config_json, updated_at \
         FROM studio_arrangements WHERE id = ?1 AND user_id = ?2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_arrangement(
    pool: &SqlitePool,
    id: &str,
    user_id: &str,
    title: &str,
    bpm: f64,
    length_beats: f64,
    master: f64,
    cfg_json: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO studio_arrangements (id, user_id, title, bpm, length_beats, master, config_json) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(id)
    .bind(user_id)
    .bind(title)
    .bind(bpm)
    .bind(length_beats)
    .bind(master)
    .bind(cfg_json)
    .execute(pool)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn update_arrangement(
    pool: &SqlitePool,
    id: &str,
    user_id: &str,
    title: &str,
    bpm: f64,
    length_beats: f64,
    master: f64,
    cfg_json: &str,
) -> sqlx::Result<bool> {
    let res = sqlx::query(
        "UPDATE studio_arrangements SET title = ?3, bpm = ?4, length_beats = ?5, master = ?6, \
         config_json = ?7, updated_at = datetime('now') WHERE id = ?1 AND user_id = ?2",
    )
    .bind(id)
    .bind(user_id)
    .bind(title)
    .bind(bpm)
    .bind(length_beats)
    .bind(master)
    .bind(cfg_json)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

// ─────────────────────────────────────────────────────────────
// Presets (named parameter snapshots per instrument kind)
// ─────────────────────────────────────────────────────────────

/// Preset row: id, kind, name, params_json.
pub type PresetRow = (String, String, String, String);

pub fn preset_json(r: &PresetRow) -> Value {
    let params: Value = serde_json::from_str(&r.3).unwrap_or(json!({}));
    json!({ "id": r.0, "kind": r.1, "name": r.2, "params": params })
}

pub async fn list_presets(pool: &SqlitePool, user_id: &str) -> sqlx::Result<Vec<PresetRow>> {
    sqlx::query_as::<_, PresetRow>(
        "SELECT id, kind, name, params_json FROM studio_presets WHERE user_id = ?1 \
         ORDER BY kind, name LIMIT 500",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn insert_preset(
    pool: &SqlitePool,
    id: &str,
    user_id: &str,
    kind: &str,
    name: &str,
    params_json: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO studio_presets (id, user_id, kind, name, params_json) VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(id)
    .bind(user_id)
    .bind(kind)
    .bind(name)
    .bind(params_json)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_preset(pool: &SqlitePool, id: &str, user_id: &str) -> sqlx::Result<Option<String>> {
    let name: Option<String> =
        sqlx::query_scalar("SELECT name FROM studio_presets WHERE id = ?1 AND user_id = ?2")
            .bind(id)
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    let res = sqlx::query("DELETE FROM studio_presets WHERE id = ?1 AND user_id = ?2")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    if res.rows_affected() > 0 {
        Ok(name)
    } else {
        Ok(None)
    }
}

pub async fn delete_arrangement(pool: &SqlitePool, id: &str, user_id: &str) -> sqlx::Result<Option<String>> {
    let title: Option<String> =
        sqlx::query_scalar("SELECT title FROM studio_arrangements WHERE id = ?1 AND user_id = ?2")
            .bind(id)
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    let res = sqlx::query("DELETE FROM studio_arrangements WHERE id = ?1 AND user_id = ?2")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    if res.rows_affected() > 0 {
        Ok(title)
    } else {
        Ok(None)
    }
}

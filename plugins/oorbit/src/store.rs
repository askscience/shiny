//! Saved planets: one JSON document per user in `oorbit_planets`. Every
//! read and action loads the planet, advances it to now, changes it and
//! writes it back, under one lock so two requests never interleave.

use std::sync::{Mutex, MutexGuard, OnceLock};

use oorbit_sim::{Catalog, Planet};
use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::util::as_text;

/// The game's catalog, parsed once.
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(Catalog::builtin)
}

/// Serializes every load-change-save of a planet.
pub fn lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn load(ctx: &PluginCtx, user: &str) -> Result<Option<Planet>, AppError> {
    let rows = ctx.db().query(
        "SELECT state FROM oorbit_planets WHERE user_id = ?1",
        &[Value::text(user)],
    )?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let planet: Planet = serde_json::from_str(&as_text(&row[0]))
        .map_err(|e| AppError::Internal(format!("saved planet is unreadable: {e}")))?;
    Ok(Some(planet))
}

pub fn save(ctx: &PluginCtx, user: &str, planet: &Planet) -> Result<(), AppError> {
    let state = serde_json::to_string(planet)
        .map_err(|e| AppError::Internal(format!("planet cannot be saved: {e}")))?;
    ctx.db().execute(
        "INSERT INTO oorbit_planets (user_id, state) VALUES (?1, ?2) \
         ON CONFLICT(user_id) DO UPDATE SET state = excluded.state, updated_at = datetime('now')",
        &[Value::text(user), Value::text(state)],
    )?;
    Ok(())
}

pub fn delete(ctx: &PluginCtx, user: &str) -> Result<(), AppError> {
    ctx.db().execute(
        "DELETE FROM oorbit_planets WHERE user_id = ?1",
        &[Value::text(user)],
    )?;
    Ok(())
}

/// A seed for a new planet: different per user and per founding time.
pub fn new_seed(user: &str, now_ms: i64) -> u64 {
    // FNV-1a over the user id, mixed with the time.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in user.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash ^ (now_ms as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

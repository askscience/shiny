//! Per-account preference persistence. An update is one atomic operation even
//! when it contains multiple keys; wire values retain their legacy strings.
use crate::errors::AppError;
use serde_json::{Map, Value};
use sqlx::SqlitePool;

pub struct PreferenceStore<'a> {
    pool: &'a SqlitePool,
}
impl<'a> PreferenceStore<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }
    pub async fn list(&self, user_id: &str) -> Result<Map<String, Value>, AppError> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT key, value FROM user_preferences WHERE user_id = ?")
                .bind(user_id)
                .fetch_all(self.pool)
                .await?;
        Ok(rows
            .into_iter()
            .map(|(key, value)| (key, Value::String(value)))
            .collect())
    }
    pub async fn put(&self, user_id: &str, values: &Value) -> Result<(), AppError> {
        let values = values
            .as_object()
            .ok_or_else(|| AppError::BadRequest("expected a JSON object".into()))?;
        let mut tx = self.pool.begin().await?;
        for (key, value) in values {
            let raw = match value {
                Value::String(value) => value.clone(),
                other => other.to_string(),
            };
            sqlx::query(
                "INSERT INTO user_preferences (user_id, key, value, updated_at)
                VALUES (?, ?, ?, datetime('now')) ON CONFLICT(user_id, key)
                DO UPDATE SET value = excluded.value, updated_at = datetime('now')",
            )
            .bind(user_id)
            .bind(key)
            .bind(raw)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

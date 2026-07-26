//! Per-plugin migration runner. The installer passes the plugin's
//! `migrations/` directory and the shared SQLite pool; this helper applies
//! any `.sql` files that haven't been recorded in `plugin_schema_versions`.

use std::path::Path;
use sqlx::SqlitePool;
use crate::errors::AppError;

pub async fn ensure_meta_table(pool: &SqlitePool) -> Result<(), AppError> {
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS plugin_schema_versions (\
            plugin TEXT NOT NULL,\
            file TEXT NOT NULL,\
            applied_at TEXT NOT NULL DEFAULT (datetime('now')),\
            PRIMARY KEY (plugin, file)\
        );",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Run any `migrations/*.sql` in `dir` that haven't been recorded for `plugin`.
pub async fn run_plugin_migrations(pool: &SqlitePool, plugin: &str, dir: &Path) -> Result<Vec<String>, AppError> {
    ensure_meta_table(pool).await?;

    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut files: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .filter(|n| n.ends_with(".sql"))
        .collect();
    files.sort();

    let mut applied = Vec::new();
    for file in files {
        let already: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM plugin_schema_versions WHERE plugin = ?1 AND file = ?2",
        )
        .bind(plugin)
        .bind(&file)
        .fetch_one(pool)
        .await?;
        if already > 0 {
            continue;
        }

        let path = dir.join(&file);
        let sql = std::fs::read_to_string(&path)?;
        sqlx::raw_sql(&sql).execute(pool).await?;

        sqlx::query("INSERT INTO plugin_schema_versions (plugin, file) VALUES (?1, ?2)")
            .bind(plugin)
            .bind(&file)
            .execute(pool)
            .await?;

        applied.push(file);
    }
    Ok(applied)
}
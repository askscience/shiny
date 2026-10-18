use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Connection;
use sqlx::{SqliteConnection, SqlitePool};
use std::str::FromStr;
use std::time::Duration;

use crate::errors::AppError;

/// Build the connect options for a `DATABASE_URL` (or bare path), creating the
/// file and any parent directory when missing.
fn connect_options(database_url: &str) -> Result<SqliteConnectOptions, sqlx::Error> {
    let opts = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .busy_timeout(Duration::from_secs(5));
    let path = opts.get_filename();
    if path != std::path::Path::new(":memory:") {
        if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(opts)
}

/// One connection, used to run migrations *before* the pool is created. sqlx
/// caches prepared statements per connection; if the schema is altered after a
/// connection has prepared `SELECT * FROM …`, the cached column metadata goes
/// stale and the next row read panics with an index out of bounds. Running the
/// migrations on a throwaway connection and then opening the pool means every
/// pooled connection only ever sees the final schema.
pub async fn connect(database_url: &str) -> Result<SqliteConnection, sqlx::Error> {
    sqlx::Connection::connect_with(&connect_options(database_url)?).await
}

pub async fn init_pool(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let opts = connect_options(database_url)?;

    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
}

/// Ordered, append-only migration registry. Existing databases are adopted
/// using column probes in the same transaction that records their versions.
const MIGRATIONS: &[(i64, &str, &str)] = &[
    (
        1,
        "001_init.sql",
        include_str!("../../migrations/001_init.sql"),
    ),
    (
        2,
        "002_artifacts.sql",
        include_str!("../../migrations/002_artifacts.sql"),
    ),
    (
        3,
        "003_username_avatar.sql",
        include_str!("../../migrations/003_username_avatar.sql"),
    ),
    (
        4,
        "004_admin.sql",
        include_str!("../../migrations/004_admin.sql"),
    ),
    (
        5,
        "005_user_plugin_states.sql",
        include_str!("../../migrations/005_user_plugin_states.sql"),
    ),
    (
        6,
        "006_user_preferences.sql",
        include_str!("../../migrations/006_user_preferences.sql"),
    ),
    (
        7,
        "007_chat_conversations.sql",
        include_str!("../../migrations/007_chat_conversations.sql"),
    ),
    (
        8,
        "008_chat_indexes.sql",
        include_str!("../../migrations/008_chat_indexes.sql"),
    ),
    (
        9,
        "009_linux_identity.sql",
        include_str!("../../migrations/009_linux_identity.sql"),
    ),
];

pub async fn run_migrations(conn: &mut SqliteConnection) -> Result<(), AppError> {
    // Reserve the write lock before checking versions: another process cannot
    // race the schema check or the migration completion record.
    let mut tx = conn.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS core_schema_versions (
        version INTEGER PRIMARY KEY, file TEXT NOT NULL, checksum TEXT NOT NULL,
        applied_at TEXT NOT NULL DEFAULT (datetime('now'))
    )",
    )
    .execute(&mut *tx)
    .await?;
    for &(version, file, sql) in MIGRATIONS {
        let checksum = hex::encode(Sha256::digest(sql.as_bytes()));
        let existing: Option<(String, String)> =
            sqlx::query_as("SELECT file, checksum FROM core_schema_versions WHERE version = ?")
                .bind(version)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some((stored_file, stored_checksum)) = existing {
            if stored_file != file || stored_checksum != checksum {
                return Err(AppError::Internal(format!(
                    "Applied core migration changed: {file}"
                )));
            }
            continue;
        }
        apply_legacy_migration(&mut tx, version, sql).await?;
        sqlx::query("INSERT INTO core_schema_versions (version, file, checksum) VALUES (?, ?, ?)")
            .bind(version)
            .bind(file)
            .bind(checksum)
            .execute(&mut *tx)
            .await?;
    }
    // Preserve the previous startup behavior even when migrations are recorded.
    sqlx::query(
        "UPDATE travelers SET is_admin = 1 WHERE id =
        (SELECT id FROM travelers ORDER BY created_at ASC LIMIT 1)",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::info!("Database migrations verified");
    Ok(())
}

async fn has_column(
    conn: &mut SqliteConnection,
    table: &str,
    column: &str,
) -> Result<bool, AppError> {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pragma_table_info(?) WHERE name = ?")
        .bind(table)
        .bind(column)
        .fetch_one(conn)
        .await?;
    Ok(count != 0)
}

async fn apply_legacy_migration(
    conn: &mut SqliteConnection,
    version: i64,
    sql: &str,
) -> Result<(), AppError> {
    match version {
        3 | 9 => {
            if version == 3 {
                // A complete legacy migration must be adopted without running
                // its data backfill again. A user may have intentionally kept
                // a nullable username after that migration originally ran.
                let indexed: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_travelers_username'"
                ).fetch_one(&mut *conn).await?;
                if indexed > 0
                    && has_column(conn, "travelers", "username").await?
                    && has_column(conn, "travelers", "avatar").await?
                {
                    return Ok(());
                }
            }
            // Old releases applied these without a transaction. Recover partial
            // application by guarding each ALTER, then run their idempotent tail.
            for statement in sql.split(';') {
                let Some(alter) = statement.find("ALTER TABLE ") else {
                    if !statement.trim().is_empty() {
                        sqlx::raw_sql(statement).execute(&mut *conn).await?;
                    }
                    continue;
                };
                let words: Vec<_> = statement[alter..].split_whitespace().collect();
                let (table, column) = (words[2], words[5]);
                if !has_column(conn, table, column).await? {
                    sqlx::raw_sql(statement).execute(&mut *conn).await?;
                }
            }
        }
        4 => {
            if !has_column(conn, "travelers", "is_admin").await? {
                sqlx::raw_sql(sql).execute(&mut *conn).await?;
            }
        }
        7 => {
            sqlx::raw_sql(sql).execute(&mut *conn).await?;
            for column in ["conversation_id", "thinking"] {
                if !has_column(conn, "chat_messages", column).await? {
                    sqlx::raw_sql(&format!(
                        "ALTER TABLE chat_messages ADD COLUMN {column} TEXT"
                    ))
                    .execute(&mut *conn)
                    .await?;
                }
            }
        }
        _ => {
            sqlx::raw_sql(sql).execute(&mut *conn).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod path_tests {
    use super::*;
    #[test]
    fn bare_paths_and_sqlite_urls_keep_query_options_and_create_only_their_parent() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("nested/state.db");
        for url in [
            format!("{}?mode=rwc", path.display()),
            format!("sqlite://{}?mode=rwc", path.display()),
        ] {
            let options = connect_options(&url).unwrap();
            assert_eq!(options.get_filename(), &path);
        }
        assert!(path.parent().unwrap().is_dir());
        assert!(!path.exists());
    }
}

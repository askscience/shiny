use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{SqliteConnection, SqlitePool};
use std::str::FromStr;
use std::path::Path;

use crate::errors::AppError;

/// Build the connect options for a `DATABASE_URL` (or bare path), creating the
/// file and any parent directory when missing.
fn connect_options(database_url: &str) -> Result<SqliteConnectOptions, sqlx::Error> {
    std::fs::create_dir_all("data").ok();
    let opts = if database_url.starts_with("sqlite://") {
        SqliteConnectOptions::from_str(database_url)?.create_if_missing(true)
    } else if let Some(path) = database_url.strip_prefix("sqlite:") {
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent).ok();
        }
        SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
    } else {
        SqliteConnectOptions::from_str(database_url)?.create_if_missing(true)
    };
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

pub async fn run_migrations(conn: &mut SqliteConnection) -> Result<(), AppError> {
    let migration = include_str!("../../migrations/001_init.sql");
    sqlx::raw_sql(migration).execute(&mut *conn).await?;
    let migration2 = include_str!("../../migrations/002_artifacts.sql");
    sqlx::raw_sql(migration2).execute(&mut *conn).await?;

    let has_username: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('travelers') WHERE name = 'username'",
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(AppError::Database)?;

    if has_username == 0 {
        let migration3 = include_str!("../../migrations/003_username_avatar.sql");
        sqlx::raw_sql(migration3).execute(&mut *conn).await?;
    }

    let has_is_admin: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('travelers') WHERE name = 'is_admin'",
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(AppError::Database)?;

    if has_is_admin == 0 {
        let migration4 = include_str!("../../migrations/004_admin.sql");
        sqlx::raw_sql(migration4).execute(&mut *conn).await?;
    }
    // Re-apply the first-user promotion (idempotent).
    sqlx::raw_sql(
        "UPDATE travelers SET is_admin = 1 WHERE id = \
         (SELECT id FROM travelers ORDER BY created_at ASC LIMIT 1)",
    )
    .execute(&mut *conn)
    .await?;

    let migration5 = include_str!("../../migrations/005_user_plugin_states.sql");
    sqlx::raw_sql(migration5).execute(&mut *conn).await?;

    let migration6 = include_str!("../../migrations/006_user_preferences.sql");
    sqlx::raw_sql(migration6).execute(&mut *conn).await?;

    // Chat conversations (007): create the table, and add the
    // `conversation_id` column to chat_messages only when it is missing (the
    // raw ALTER is not idempotent, so guard it with pragma_table_info).
    sqlx::raw_sql(include_str!("../../migrations/007_chat_conversations.sql"))
        .execute(&mut *conn)
        .await?;
    let has_conversation_id: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('chat_messages') WHERE name = 'conversation_id'",
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(AppError::Database)?;
    if has_conversation_id == 0 {
        sqlx::raw_sql("ALTER TABLE chat_messages ADD COLUMN conversation_id TEXT")
            .execute(&mut *conn)
            .await?;
    }

    let migration8 = include_str!("../../migrations/008_chat_indexes.sql");
    sqlx::raw_sql(migration8).execute(&mut *conn).await?;

    // Linux-user identity (009): add the `unix_*` columns only when the first
    // one is missing — the raw ALTERs are not idempotent.
    let has_unix_user: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('travelers') WHERE name = 'unix_user'",
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(AppError::Database)?;
    if has_unix_user == 0 {
        let migration9 = include_str!("../../migrations/009_linux_identity.sql");
        sqlx::raw_sql(migration9).execute(&mut *conn).await?;
    }

    tracing::info!("Database migrations applied");
    Ok(())
}

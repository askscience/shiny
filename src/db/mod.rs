use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;
use std::path::Path;

use crate::errors::AppError;

pub async fn init_pool(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
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

    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
}

pub async fn run_migrations(pool: &SqlitePool) -> Result<(), AppError> {
    let migration = include_str!("../../migrations/001_init.sql");
    sqlx::raw_sql(migration).execute(pool).await?;
    let migration2 = include_str!("../../migrations/002_artifacts.sql");
    sqlx::raw_sql(migration2).execute(pool).await?;

    let has_username: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('travelers') WHERE name = 'username'",
    )
    .fetch_one(pool)
    .await
    .map_err(AppError::Database)?;

    if has_username == 0 {
        let migration3 = include_str!("../../migrations/003_username_avatar.sql");
        sqlx::raw_sql(migration3).execute(pool).await?;
    }

    let has_is_admin: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('travelers') WHERE name = 'is_admin'",
    )
    .fetch_one(pool)
    .await
    .map_err(AppError::Database)?;

    if has_is_admin == 0 {
        let migration4 = include_str!("../../migrations/004_admin.sql");
        sqlx::raw_sql(migration4).execute(pool).await?;
    }
    // Re-apply the first-user promotion (idempotent).
    sqlx::raw_sql(
        "UPDATE travelers SET is_admin = 1 WHERE id = \
         (SELECT id FROM travelers ORDER BY created_at ASC LIMIT 1)",
    )
    .execute(pool)
    .await?;

    let migration5 = include_str!("../../migrations/005_user_plugin_states.sql");
    sqlx::raw_sql(migration5).execute(pool).await?;

    let migration6 = include_str!("../../migrations/006_user_preferences.sql");
    sqlx::raw_sql(migration6).execute(pool).await?;

    // Chat conversations (007): create the table, and add the
    // `conversation_id` column to chat_messages only when it is missing (the
    // raw ALTER is not idempotent, so guard it with pragma_table_info).
    sqlx::raw_sql(include_str!("../../migrations/007_chat_conversations.sql"))
        .execute(pool)
        .await?;
    let has_conversation_id: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('chat_messages') WHERE name = 'conversation_id'",
    )
    .fetch_one(pool)
    .await
    .map_err(AppError::Database)?;
    if has_conversation_id == 0 {
        sqlx::raw_sql("ALTER TABLE chat_messages ADD COLUMN conversation_id TEXT")
            .execute(pool)
            .await?;
    }

    tracing::info!("Database migrations applied");
    Ok(())
}

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
    tracing::info!("Database migrations applied");
    Ok(())
}

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;
use crate::errors::AppError;

pub async fn init_pool(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    SqlitePoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await
}

pub async fn run_migrations(pool: &SqlitePool) -> Result<(), AppError> {
    let migration = include_str!("../../migrations/001_init.sql");
    sqlx::raw_sql(migration).execute(pool).await?;
    tracing::info!("Database migrations applied");
    Ok(())
}

//! Recovery checks always use scratch databases, never the kiosk's user data.
mod support;
use shiny::db;
use shiny_plugin_sdk::migrations::run_plugin_migrations;

#[tokio::test]
async fn fresh_schema_is_recorded_and_repeated_startup_preserves_data() {
    let app = support::TestApp::new().await;
    sqlx::query("INSERT INTO travelers (id, name, email, password_hash) VALUES ('u', 'User', 'u@test', '!')")
        .execute(&app.state.pool).await.unwrap();
    sqlx::query("INSERT INTO user_preferences (user_id, key, value) VALUES ('u', 'desktop.layout', '{\"mode\":\"windows\"}')")
        .execute(&app.state.pool).await.unwrap();
    let mut conn = db::connect(&app.state.config.database_url).await.unwrap();
    db::run_migrations(&mut conn).await.unwrap();
    db::run_migrations(&mut conn).await.unwrap();
    let versions: i64 = sqlx::query_scalar("SELECT count(*) FROM core_schema_versions")
        .fetch_one(&app.state.pool)
        .await
        .unwrap();
    assert_eq!(versions, 12);
    let value: String =
        sqlx::query_scalar("SELECT value FROM user_preferences WHERE user_id = 'u'")
            .fetch_one(&app.state.pool)
            .await
            .unwrap();
    assert_eq!(value, "{\"mode\":\"windows\"}");
}

#[tokio::test]
async fn adopts_legacy_and_partially_applied_columns_without_losing_identity() {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", root.path().join("legacy.db").display());
    let mut conn = db::connect(&url).await.unwrap();
    sqlx::raw_sql(include_str!("../migrations/001_init.sql"))
        .execute(&mut conn)
        .await
        .unwrap();
    // Reproduce an old interrupted migration: only its first ALTER persisted.
    sqlx::raw_sql("ALTER TABLE travelers ADD COLUMN username TEXT;
        ALTER TABLE travelers ADD COLUMN unix_user TEXT;
        INSERT INTO travelers (id, name, email, password_hash, auth_token) VALUES ('legacy', 'Legacy', 'legacy@test', 'hash', 'token');")
        .execute(&mut conn).await.unwrap();
    db::run_migrations(&mut conn).await.unwrap();
    let row: (String, String, String) = sqlx::query_as(
        "SELECT username, password_hash, auth_token FROM travelers WHERE id = 'legacy'",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_eq!(row, ("legacy".into(), "hash".into(), "token".into()));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_schema_versions")
            .fetch_one(&mut conn)
            .await
            .unwrap(),
        12
    );
}

#[tokio::test]
async fn refuses_modified_migration_records() {
    let app = support::TestApp::new().await;
    sqlx::query("UPDATE core_schema_versions SET checksum = 'changed' WHERE version = 1")
        .execute(&app.state.pool)
        .await
        .unwrap();
    let mut conn = db::connect(&app.state.config.database_url).await.unwrap();
    assert!(db::run_migrations(&mut conn)
        .await
        .unwrap_err()
        .to_string()
        .contains("Applied core migration changed"));
}

#[tokio::test]
async fn core_bootstrap_failure_rolls_back_schema_and_ledger() {
    let root = tempfile::tempdir().unwrap();
    let mut conn = db::connect(&format!(
        "sqlite://{}",
        root.path().join("broken.db").display()
    ))
    .await
    .unwrap();
    // A conflicting legacy object makes CREATE INDEX fail after earlier DDL.
    sqlx::query("CREATE TABLE idx_trips_traveler (id TEXT)")
        .execute(&mut conn)
        .await
        .unwrap();
    assert!(db::run_migrations(&mut conn).await.is_err());
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_master WHERE name IN ('travelers', 'core_schema_versions')",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_eq!(count, 0);
    sqlx::query("DROP TABLE idx_trips_traveler")
        .execute(&mut conn)
        .await
        .unwrap();
    db::run_migrations(&mut conn).await.unwrap();
}

#[tokio::test]
async fn failed_plugin_migration_rolls_back_ddl_and_can_be_retried() {
    let app = support::TestApp::new().await;
    let dir = app.root.path().join("migration-fixture");
    std::fs::create_dir(&dir).unwrap();
    let file = dir.join("001_test.sql");
    std::fs::write(
        &file,
        "CREATE TABLE probe (id INTEGER); INSERT INTO missing_table VALUES (1);",
    )
    .unwrap();
    assert!(run_plugin_migrations(&app.state.pool, "probe", &dir)
        .await
        .is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE name = 'probe'")
        .fetch_one(&app.state.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    std::fs::write(
        &file,
        "CREATE TABLE probe (id INTEGER); INSERT INTO probe VALUES (1);",
    )
    .unwrap();
    assert_eq!(
        run_plugin_migrations(&app.state.pool, "probe", &dir)
            .await
            .unwrap(),
        ["001_test.sql"]
    );
    assert!(run_plugin_migrations(&app.state.pool, "probe", &dir)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn concurrent_plugin_migrations_apply_once() {
    let app = support::TestApp::new().await;
    let dir = app.root.path().join("concurrent-migration");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(
        dir.join("001_test.sql"),
        "CREATE TABLE once_only (id INTEGER); INSERT INTO once_only VALUES (1);",
    )
    .unwrap();
    let (a, b) = tokio::join!(
        run_plugin_migrations(&app.state.pool, "once", &dir),
        run_plugin_migrations(&app.state.pool, "once", &dir)
    );
    assert_eq!(a.unwrap().len() + b.unwrap().len(), 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM once_only")
            .fetch_one(&app.state.pool)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn plugin_completion_record_failure_also_rolls_back_the_schema() {
    let app = support::TestApp::new().await;
    shiny_plugin_sdk::migrations::ensure_meta_table(&app.state.pool)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TRIGGER reject_record BEFORE INSERT ON plugin_schema_versions BEGIN SELECT RAISE(ABORT, 'fixture'); END;")
        .execute(&app.state.pool).await.unwrap();
    let dir = app.root.path().join("record-failure");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(
        dir.join("001_test.sql"),
        "CREATE TABLE unrecorded (id INTEGER);",
    )
    .unwrap();
    assert!(run_plugin_migrations(&app.state.pool, "record", &dir)
        .await
        .is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM sqlite_master WHERE name = 'unrecorded'"
        )
        .fetch_one(&app.state.pool)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn preference_update_is_atomic_when_a_later_key_fails() {
    let app = support::TestApp::new().await;
    sqlx::raw_sql("INSERT INTO travelers (id, name, email, password_hash) VALUES ('u', 'User', 'u@test', '!');
        CREATE TRIGGER reject_pref BEFORE INSERT ON user_preferences WHEN NEW.key = 'z' BEGIN SELECT RAISE(ABORT, 'fixture'); END;")
        .execute(&app.state.pool).await.unwrap();
    let store = shiny::services::preferences::PreferenceStore::new(&app.state.pool);
    assert!(store
        .put("u", &serde_json::json!({"a":"first", "z":"fail"}))
        .await
        .is_err());
    assert!(store.list("u").await.unwrap().is_empty());
}

#[tokio::test]
async fn complete_legacy_migration_is_adopted_without_rewriting_profile_values() {
    let app = support::TestApp::new().await;
    sqlx::raw_sql(
        "INSERT INTO travelers (id, name, email, password_hash, username, avatar)
        VALUES ('legacy', 'User', 'legacy@test', 'hash', NULL, 'saved-avatar');
        DROP TABLE core_schema_versions;",
    )
    .execute(&app.state.pool)
    .await
    .unwrap();
    let mut conn = db::connect(&app.state.config.database_url).await.unwrap();
    db::run_migrations(&mut conn).await.unwrap();
    let profile: (Option<String>, String) =
        sqlx::query_as("SELECT username, avatar FROM travelers WHERE id = 'legacy'")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!(profile, (None, "saved-avatar".into()));
}

//! A per-user server creates the account of the Linux user it runs as on its
//! first start, so the kiosk's boot token has an account to sign into and the
//! login screen's password is the only one asked.
mod support;

#[tokio::test]
async fn the_server_creates_its_own_linux_account_once() {
    let app = support::TestApp::new().await;
    let mut state = app.state.clone();
    let me = shiny::services::unix_user::lookup_uid(unsafe { libc::getuid() });
    let Some(me) = me.filter(|u| u.is_human()) else {
        eprintln!("skipped: the test does not run as a human account");
        return;
    };

    // Not a per-user server (no Linux users): nothing to do.
    assert_eq!(
        shiny::api::auth::provision_session_account(&state).await,
        None
    );

    // The login screen's own server never creates accounts.
    state.config.linux_users = true;
    state.config.greeter_mode = true;
    assert_eq!(
        shiny::api::auth::provision_session_account(&state).await,
        None
    );

    state.config.greeter_mode = false;
    let id = shiny::api::auth::provision_session_account(&state)
        .await
        .expect("the account is created on the first start");
    let unix_user: String = sqlx::query_scalar("SELECT unix_user FROM travelers WHERE id = ?1")
        .bind(&id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(unix_user, me.name);

    // The next start finds it.
    assert_eq!(
        shiny::api::auth::provision_session_account(&state).await,
        None
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM travelers")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

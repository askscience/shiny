use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::{RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::{RegistryBuilder, Tool},
};

shiny_plugin_sdk::ctx_plugin!(MailPlugin);

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str = "an email assistant; read, manage and send the user's mail";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec::get("/api/mail/status", "mail_status"),
        RouteSpec::get("/api/mail/accounts", "mail_accounts_list"),
        RouteSpec::post("/api/mail/accounts", "mail_accounts_create"),
        RouteSpec::post("/api/mail/accounts/test", "mail_accounts_test"),
        RouteSpec::put("/api/mail/accounts/:id", "mail_accounts_update"),
        RouteSpec::delete("/api/mail/accounts/:id", "mail_accounts_delete"),
        RouteSpec::get("/api/mail/folders", "mail_folders"),
        RouteSpec::get("/api/mail/list", "mail_list"),
        RouteSpec::get("/api/mail/search", "mail_search"),
        RouteSpec::post("/api/mail/sync", "mail_sync"),
        RouteSpec::get("/api/mail/message", "mail_message"),
        RouteSpec::post("/api/mail/send", "mail_send"),
        RouteSpec::post("/api/mail/flag", "mail_flag"),
        RouteSpec::post("/api/mail/delete", "mail_delete"),
    ]
}

#[async_trait]
impl Plugin for MailPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("mail")
                .description("Mail client — IMAP inbox + SMTP compose, backed by io-email")
                .summary("Mail client: configure mail accounts, read the inbox and send email")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/mail.md"))
            .context_line(
                "Mail: enabled — read and send email from the user's configured mail accounts.",
            );
        builder.routes(route_specs());
        builder.tools([
            Arc::new(crate::tools::MailStatus) as Arc<dyn Tool>,
            Arc::new(crate::tools::MailList) as Arc<dyn Tool>,
            Arc::new(crate::tools::MailRead) as Arc<dyn Tool>,
            Arc::new(crate::tools::MailSearch) as Arc<dyn Tool>,
            Arc::new(crate::tools::MailSync) as Arc<dyn Tool>,
            Arc::new(crate::tools::MailSend) as Arc<dyn Tool>,]);
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }

    /// One-shot at load: encrypt every account password still stored in
    /// plaintext (rows written before encryption-at-rest landed). Runs for all
    /// users; failures are logged, never fatal.
    async fn on_load(&self, ctx: Arc<PluginCtx>) {
        use shiny_plugin_sdk::db::Value;
        let db = ctx.db();
        let rows = match db.query(
            "SELECT id, password FROM mail_accounts \
             WHERE password IS NOT NULL AND password != '' AND password NOT LIKE 'enc:v1:%'",
            &[],
        ) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("mail: plaintext-password scan failed: {e}");
                return;
            }
        };
        let mut migrated = 0usize;
        for row in rows {
            let id = match row.first() {
                Some(Value::Text(s)) => s.clone(),
                _ => continue,
            };
            let plain = match row.get(1) {
                Some(Value::Text(s)) => s.clone(),
                _ => continue,
            };
            match crate::crypto::encrypt(&plain) {
                Ok(enc) => {
                    if let Err(e) = db.execute(
                        "UPDATE mail_accounts SET password = ?1 WHERE id = ?2",
                        &[Value::text(&enc), Value::text(&id)],
                    ) {
                        tracing::warn!("mail: could not encrypt account {id}: {e}");
                    } else {
                        migrated += 1;
                    }
                }
                Err(e) => tracing::warn!("mail: encryption failed for account {id}: {e}"),
            }
        }
        if migrated > 0 {
            tracing::info!("mail: encrypted {migrated} stored account password(s)");
        }
    }
}


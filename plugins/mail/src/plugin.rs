use std::sync::{Arc, OnceLock};
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    routes::{HttpMethod, RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::{RegistryBuilder, Tool},
};

pub struct MailPlugin {
    ctx: OnceLock<Arc<PluginCtx>>,
}

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str = "an email assistant; read, manage and send the user's mail";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec { method: HttpMethod::Get, path: "/api/mail/status".into(), auth: "auth".into(), handler_tag: "mail_status".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/mail/accounts".into(), auth: "auth".into(), handler_tag: "mail_accounts_list".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/mail/accounts".into(), auth: "auth".into(), handler_tag: "mail_accounts_create".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/mail/accounts/test".into(), auth: "auth".into(), handler_tag: "mail_accounts_test".into() },
        RouteSpec { method: HttpMethod::Put, path: "/api/mail/accounts/:id".into(), auth: "auth".into(), handler_tag: "mail_accounts_update".into() },
        RouteSpec { method: HttpMethod::Delete, path: "/api/mail/accounts/:id".into(), auth: "auth".into(), handler_tag: "mail_accounts_delete".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/mail/folders".into(), auth: "auth".into(), handler_tag: "mail_folders".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/mail/list".into(), auth: "auth".into(), handler_tag: "mail_list".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/mail/message".into(), auth: "auth".into(), handler_tag: "mail_message".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/mail/send".into(), auth: "auth".into(), handler_tag: "mail_send".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/mail/flag".into(), auth: "auth".into(), handler_tag: "mail_flag".into() },
    ]
}

#[async_trait]
impl Plugin for MailPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| Manifest {
            name: "mail".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some(
                "Mail client — IMAP inbox + SMTP compose, backed by io-email".into(),
            ),
            author: Some("shiny".into()),
            summary: Some("Mail client: configure mail accounts, read the inbox and send email".into()),
            migrations_dir: "migrations".into(),
            skills_dir: "skills".into(),
            web_dir: "web".into(),
            signature: None,
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        let _ = self.ctx.set(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/mail.md"))
            .context_line(
                "Mail: enabled — read and send email from the user's configured mail accounts.",
            );
        for spec in route_specs() {
            builder.route(spec);
        }
        for tool in [
            Arc::new(crate::tools::MailStatus) as Arc<dyn Tool>,
            Arc::new(crate::tools::MailList) as Arc<dyn Tool>,
            Arc::new(crate::tools::MailRead) as Arc<dyn Tool>,
            Arc::new(crate::tools::MailSend) as Arc<dyn Tool>,
        ] {
            builder.tool_arc(shiny_plugin_sdk::tools::bridged(tool));
        }
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        let ctx = self.ctx.get()?;
        crate::routes::handle(ctx, tag)
    }
}

/// The C entry symbol the loader transmutes and calls.
#[no_mangle]
pub extern "C" fn shiny_plugin_entry() -> *mut dyn Plugin {
    Box::into_raw(Box::new(MailPlugin { ctx: OnceLock::new() }))
}

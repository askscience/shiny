//! The `browser` plugin: an in-app browser window powered by the shared
//! `shiny-filter` engine.
//!
//! Why this plugin exists next to the `peakd` shell: a native webview
//! cannot be embedded in an HTML page, and — measured, not assumed — macOS
//! gives a plain `WKWebView` no way to filter its own requests (wry's
//! `ProxyConfig` produced zero proxied requests in testing). Inside the Shiny
//! desktop the window *is* HTML, so the proxy can be the window's **origin**,
//! which makes filtering here complete rather than best-effort: there is no
//! request the engine does not see.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    routes::{HttpMethod, RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

pub struct BrowserPlugin {
    /// The ctx handed to `register`; routes resolve through it.
    pub ctx: OnceLock<Arc<PluginCtx>>,
}

/// The plugin's name, in one place: the manifest, the tile's `data-plugin`
/// attribute, the route namespace and the artifacts' owner all have to agree.
pub const PLUGIN_NAME: &str = "browser";

/// Persona fragment the agent sees while this plugin is active.
pub const PERSONA: &str = "a web navigator; open pages and search the web in the browser window";

pub const SKILLS: &str = include_str!("../skills/browser.md");

#[async_trait]
impl Plugin for BrowserPlugin {
    fn manifest(&self) -> &Manifest {
        static M: OnceLock<Manifest> = OnceLock::new();
        M.get_or_init(|| Manifest {
            name: PLUGIN_NAME.into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some(
                "Browse the web inside Shiny — a filtered window powered by the shared \
                 shiny-filter adblocking engine, with a news home ranked from your searches"
                    .into(),
            ),
            author: Some("shiny".into()),
            summary: Some(
                "Browser: adblock-filtered web browsing in its own window, with related-news cards \
                 chosen from what you search for"
                    .into(),
            ),
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
            .skills(SKILLS)
            .context_line(
                "Browser: enabled — `browser_open` / `browser_search` open pages in the Browser \
                 window and `browser_read` returns a page's text. The window's home surface shows \
                 related-news cards ranked from the user's recent searches.",
            );

        for spec in [
            (HttpMethod::Get, "/api/browser/state", "browser_state"),
            (HttpMethod::Get, "/api/browser/sessions", "browser_sessions"),
            (HttpMethod::Post, "/api/browser/session", "browser_session_create"),
            (HttpMethod::Post, "/api/browser/session/close", "browser_session_close"),
            (HttpMethod::Post, "/api/browser/navigate", "browser_navigate"),
            (HttpMethod::Get, "/api/browser/metrics", "browser_metrics"),
            (HttpMethod::Post, "/api/browser/filter/toggle", "browser_filter_toggle"),
            (HttpMethod::Get, "/api/browser/history", "browser_history"),
            (HttpMethod::Get, "/api/browser/news", "browser_news"),
            (HttpMethod::Post, "/api/browser/news/click", "browser_news_click"),
            (HttpMethod::Post, "/api/browser/preview", "browser_preview"),
        ] {
            builder.route(RouteSpec {
                method: spec.0,
                path: spec.1.into(),
                auth: "auth".into(),
                handler_tag: spec.2.into(),
            });
        }

        // A URL the window should be showing right now, for the UI to poll.
        for tool in [
            Arc::new(crate::tools::BrowserOpen) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::BrowserSearch),
            Arc::new(crate::tools::BrowserRead),
        ] {
            builder.tool_arc(shiny_plugin_sdk::tools::bridged(tool));
        }
    }

    /// Bring up the filter proxy as soon as the plugin loads, so the window is
    /// ready the moment the user opens it.
    ///
    /// **This must run on the plugin's own runtime.** A plugin cdylib links
    /// its own copy of Tokio, so driving plugin futures from the host's
    /// runtime panics with *"there is no reactor running, must be called from
    /// the context of a Tokio 1.x runtime"* — and because the panic unwinds
    /// across the `dlopen` boundary the process then aborts. That is precisely
    /// the failure mode `PLUGINS.md` §15 warns about, and it took down the
    /// server during the first install of this plugin. `rt::bridge` sends the
    /// work to the plugin-owned runtime instead; the host just awaits a
    /// channel.
    async fn on_load(&self, ctx: Arc<PluginCtx>) {
        // Repair a `peakd_history` created before the profile's `query` column
        // existed. A migration cannot do this idempotently (SQLite has no
        // conditional DDL), and the code that needs the column cannot function
        // without it — see `history::ensure_schema`.
        crate::history::ensure_schema(&ctx);

        let started = shiny_plugin_sdk::rt::bridge(async move {
            let cache_dir = shiny_filter::engine::default_cache_dir(std::path::Path::new("data"));
            match crate::proxy::ensure_started(cache_dir).await {
                Ok(handle) => Ok((handle.base().to_string(), handle.filter.rule_count())),
                Err(err) => Err(err),
            }
        })
        .await;

        match started {
            Ok(Ok((base, rules))) => {
                tracing::info!("browser: filter proxy on {base} ({rules} rules)")
            }
            Ok(Err(err)) => tracing::warn!("browser: filter proxy unavailable: {err}"),
            Err(err) => tracing::warn!("browser: filter proxy startup failed: {err}"),
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
    Box::into_raw(Box::new(BrowserPlugin { ctx: OnceLock::new() }))
}

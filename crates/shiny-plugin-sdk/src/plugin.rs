//! The `Plugin` trait.
//!
//! Plugins are compiled as `cdylib` and loaded with `libloading`. The export
//! symbol is `shiny_plugin_entry`; it returns a heap-allocated `*mut dyn Plugin`
//! that the loader takes ownership of.

use std::sync::Arc;
use async_trait::async_trait;

use crate::manifest::Manifest;
use crate::tools::RegistryBuilder;
use crate::services::PluginCtx;

pub const PLUGIN_ENTRY_SYMBOL: &str = "shiny_plugin_entry";

/// The trait every plugin implements.
#[async_trait]
pub trait Plugin: Send + Sync {
    fn manifest(&self) -> &Manifest;

    /// Populate `builder` with tools, routes, crons, skills markdown, and
    /// persona fragment. The installer calls this once after `dlopen`.
    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>);

    /// Called once after registration — gives the plugin a chance to start
    /// background services (gpsd, cron loops, etc.) and stash any state.
    /// Default impl is a no-op.
    async fn on_load(&self, _ctx: Arc<PluginCtx>) {}

    /// Called on plugin uninstall — gives the plugin a chance to flush
    /// background state. Default no-op.
    async fn on_unload(&self, _ctx: Arc<PluginCtx>) {}

    /// Optional hook fired when a new user registers in core identity.
    /// Plugins can use this to provision a profile row. Default no-op.
    async fn on_user_registered(&self, _ctx: Arc<PluginCtx>, _user_id: &str) {}

    /// Resolve a `handler_tag` (declared via `builder.route(..)` in `register`)
    /// to its handler. Called by the installer when building the live router.
    /// Default: no routes. Override to serve `RouteSpec`s.
    fn route_handler(&self, _tag: &str) -> Option<crate::routes::RouteHandler> {
        None
    }
}

/// C entry symbol shape. The loader transmutes the loaded symbol pointer to
/// this function type and calls it.
pub type PluginEntry = unsafe extern "C" fn() -> *mut dyn Plugin;
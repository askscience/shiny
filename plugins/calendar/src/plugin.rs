use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::{RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

shiny_plugin_sdk::ctx_plugin!(CalendarPlugin);

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str =
    "a calendar assistant; schedule, list and organize the user's events";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec::get("/api/calendar/events", "event_list"),
        RouteSpec::post("/api/calendar/events", "event_create"),
        RouteSpec::put("/api/calendar/events/:id", "event_update"),
        RouteSpec::delete("/api/calendar/events/:id", "event_delete"),
    ]
}

#[async_trait]
impl Plugin for CalendarPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("calendar")
                .description("Calendar — schedule, list and organize events in the Calendar window")
                .summary("Calendar: schedule and organize events")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/calendar.md"))
            .context_line("Calendar: enabled — the Calendar window shows the user's schedule by month.");
        builder.routes(route_specs());
        builder.tools([
            Arc::new(crate::tools::CalendarCreate) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::CalendarList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::CalendarGet) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::CalendarUpdate) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::CalendarDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,]);
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}


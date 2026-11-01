//! Studio plugin entry: manifest, tool/route registration, and the C entry point.

use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::{RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

shiny_plugin_sdk::ctx_plugin!(StudioPlugin);

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str =
    "a music producer AI; compose patterns and arrangements (synths, drums, modular Grid), place AI-generated songs on audio tracks, balance and master the mix, and export it";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec::get("/api/studio", "studio_list"),
        RouteSpec::post("/api/studio", "studio_create"),
        RouteSpec::get("/api/studio/:id", "studio_get"),
        RouteSpec::get("/api/studio/:id/audio", "studio_audio"),
        RouteSpec::put("/api/studio/:id", "studio_update"),
        RouteSpec::post("/api/studio/:id/render", "studio_render"),
        RouteSpec::post("/api/studio/arrangement/render", "studio_arrangement_render"),
        RouteSpec::post("/api/studio/preview", "studio_preview"),
        RouteSpec::post("/api/studio/waveform", "studio_waveform"),
        RouteSpec::get("/api/studio/catalog", "studio_catalog"),
        RouteSpec::post("/api/studio/analyze", "studio_analyze"),
        RouteSpec::get("/api/studio/arrangement", "studio_arrangement_list"),
        RouteSpec::post("/api/studio/arrangement", "studio_arrangement_save"),
        RouteSpec::get("/api/studio/arrangement/:id", "studio_arrangement_get"),
        RouteSpec::put("/api/studio/arrangement/:id", "studio_arrangement_update"),
        RouteSpec::delete("/api/studio/arrangement/:id", "studio_arrangement_delete"),
        RouteSpec::get("/api/studio/presets", "studio_preset_list"),
        RouteSpec::post("/api/studio/presets", "studio_preset_save"),
        RouteSpec::delete("/api/studio/presets/:id", "studio_preset_delete"),
        RouteSpec::post("/api/studio/mix/stems", "studio_mix_stems"),
        RouteSpec::get("/api/studio/mix/stem/:key", "studio_mix_stem"),
        RouteSpec::post("/api/studio/mix/analyze", "studio_mix_analyze"),
        RouteSpec::post("/api/studio/export", "studio_export"),
        RouteSpec::get("/api/studio/audio/library", "studio_audio_library"),
        RouteSpec::get("/api/studio/audio/info", "studio_audio_info"),
        RouteSpec::get("/api/studio/audio/file", "studio_audio_file"),
        RouteSpec::delete("/api/studio/:id", "studio_delete"),
    ]
}

#[async_trait]
impl Plugin for StudioPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("studio")
                .description("Music studio — a DAW-grade step sequencer, polyphonic synth, drum machine and modular Grid: compose patterns and arrangements and render them to audio in the Studio window")
                .summary("Music studio: synth + drums + Grid, offline-rendered by a self-contained DSP engine")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        // The sampler reads user-supplied SoundFont banks from
        // `<plugins_dir>/studio/soundfonts`; remember the path for the engine.
        crate::dsp::sampler::set_dir(
            std::path::PathBuf::from(&ctx.config.plugins_dir)
                .join("studio")
                .join("soundfonts"),
        );
        self.install_ctx(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/studio.md"))
            .context_line("Studio: enabled — a DAW: compose patterns/arrangements, put generate_song results on audio tracks (studio_import_audio), mix (sends, inserts, master), measure (studio_mix_analyze) and export (studio_export).");
        builder.routes(route_specs());
        builder.tools([
            Arc::new(crate::tools::StudioList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioCreate) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioGet) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioRender) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioUpdate) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioPresetList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioPresetSave) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioPresetDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioArrangementList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioArrangementSave) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioArrangementGet) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioArrangementDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioCatalog) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioAnalyze) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioImportAudio) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioArrangementEdit) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioMixAnalyze) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioExport) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioAudioInfo) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
        ]);
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}


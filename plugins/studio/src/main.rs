//! The studio plugin as its own process (`runtime = "process"`).

fn main() {
    shiny_plugin_sdk::process::run(shiny_studio_plugin::plugin::StudioPlugin::new());
}

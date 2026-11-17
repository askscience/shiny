//! The capture plugin as its own process (`runtime = "process"`).

fn main() {
    shiny_plugin_sdk::process::run(shiny_capture_plugin::plugin::CapturePlugin::new());
}

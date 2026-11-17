//! The radio plugin as its own process (`runtime = "process"`).

fn main() {
    shiny_plugin_sdk::process::run(shiny_radio_plugin::RadioPlugin::new());
}

//! The happywalk plugin as its own process (`runtime = "process"`).

fn main() {
    shiny_plugin_sdk::process::run(shiny_happywalk_plugin::HappyWalkPlugin::new());
}

//! The oorbit plugin as its own process (`runtime = "process"`).

fn main() {
    shiny_plugin_sdk::process::run(shiny_oorbit_plugin::OorbitPlugin::new());
}

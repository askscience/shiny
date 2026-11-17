//! The browser plugin as its own process (`runtime = "process"`).

fn main() {
    shiny_plugin_sdk::process::run(shiny_browser_plugin::BrowserPlugin::new());
}

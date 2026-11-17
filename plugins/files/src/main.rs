//! The files plugin as its own process (`runtime = "process"`).

fn main() {
    shiny_plugin_sdk::process::run(shiny_files_plugin::FilesPlugin::new());
}

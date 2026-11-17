//! The hello plugin as its own process (`runtime = "process"`).

fn main() {
    shiny_plugin_sdk::process::run_with(
        shiny_hello_plugin::HelloPlugin,
        shiny_plugin_sdk::process::Options {
            // No background work: the server may stop us when idle.
            idle_stop: Some(std::time::Duration::from_secs(600)),
        },
    );
}

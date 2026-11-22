//! @@TITLE@@ as its own process (`runtime = "process"` in plugin.toml).
//! The server starts this executable and talks to it over a private socket;
//! the SDK does all of that. Nothing to change here.

fn main() {
    shiny_plugin_sdk::process::run_with(
        @@CRATE@@::@@TYPE@@::new(),
        shiny_plugin_sdk::process::Options {
            // No background work: the server may stop the process when idle
            // and start it again on the next request. Use `None` if `on_load`
            // ever starts something that must keep running.
            idle_stop: Some(std::time::Duration::from_secs(600)),
        },
    );
}

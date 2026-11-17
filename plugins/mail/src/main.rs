//! The mail plugin as its own process (`runtime = "process"`).

fn main() {
    shiny_plugin_sdk::process::run(shiny_mail_plugin::MailPlugin::new());
}

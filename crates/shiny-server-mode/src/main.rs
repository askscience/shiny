//! Shiny **server mode** window.
//!
//! When the user turns on Server mode, the kiosk closes and this small,
//! peakd-themed window takes the seat. It shows the Iroh link (hidden behind a
//! Reveal button, with Copy and a QR code), the live connection status, and the
//! controls: Allow Terminal, Autostart, and Stop.
//!
//! It talks to the local Shiny server with the loopback-only session token from
//! `$XDG_RUNTIME_DIR/shiny-session-token`. Linux-only.

#[cfg(target_os = "linux")]
fn main() {
    linux::run();
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("shiny-server-mode is Linux-only");
}

#[cfg(target_os = "linux")]
mod linux {
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::rc::Rc;
    use std::cell::RefCell;
    use std::time::Duration;

    use gtk::prelude::*;
    use serde_json::Value;

    const APP_ID: &str = "computer.shiny.ServerMode";

    /// HTTP base for the local server.
    fn base_url() -> String {
        let port = port_from_env_file()
            .or_else(|| std::env::var("SHINY_PORT").ok().and_then(|p| p.trim().parse().ok()))
            .unwrap_or(8080);
        format!("http://127.0.0.1:{port}")
    }

    fn port_from_env_file() -> Option<u16> {
        let home = std::env::var_os("HOME")?;
        let path = PathBuf::from(home).join(".config/shiny/env");
        let text = std::fs::read_to_string(path).ok()?;
        text.lines().find_map(|line| {
            line.trim()
                .strip_prefix("SERVER_PORT=")
                .and_then(|v| v.trim().parse().ok())
        })
    }

    fn session_token() -> Option<String> {
        let path = std::env::var("SHINY_SESSION_TOKEN_FILE")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("XDG_RUNTIME_DIR")
                    .map(|d| PathBuf::from(d).join("shiny-session-token"))
            })?;
        std::fs::read_to_string(path).ok().map(|t| t.trim().to_string())
    }

    fn client() -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("http client")
    }

    pub fn run() {
        let app = gtk::Application::builder().application_id(APP_ID).build();
        app.connect_activate(build_ui);
        app.run();
    }

    fn build_ui(app: &gtk::Application) {
        // Match the greeter/kiosk: the Noir GTK theme.
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_theme_name(Some("Shiny"));
        }

        let base = base_url();
        let token = session_token().unwrap_or_default();
        let client = client();

        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Shiny — Server mode")
            .default_width(560)
            .default_height(620)
            .build();

        let root = gtk::Box::new(gtk::Orientation::Vertical, 14);
        root.set_margin_top(24);
        root.set_margin_bottom(24);
        root.set_margin_start(24);
        root.set_margin_end(24);

        let title = gtk::Label::new(Some("Server mode"));
        title.set_halign(gtk::Align::Start);
        title.set_markup("<span size='x-large' weight='bold'>Server mode</span>");
        root.pack_start(&title, false, false, 0);

        let status = gtk::Label::new(Some("Starting…"));
        status.set_halign(gtk::Align::Start);
        status.set_line_wrap(true);
        root.pack_start(&status, false, false, 0);

        let link_title = gtk::Label::new(Some("Connection link"));
        link_title.set_halign(gtk::Align::Start);
        root.pack_start(&link_title, false, false, 4);

        let ticket = gtk::Entry::new();
        ticket.set_editable(false);
        ticket.set_visible(false);
        ticket.set_width_chars(40);
        root.pack_start(&ticket, false, false, 0);

        let qr = gtk::Image::new();
        qr.set_visible(false);
        qr.set_halign(gtk::Align::Start);
        root.pack_start(&qr, false, false, 0);

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let reveal = gtk::Button::with_label("Reveal link");
        let copy = gtk::Button::with_label("Copy");
        let rotate = gtk::Button::with_label("Rotate key");
        let pair = gtk::Button::with_label("Pair device");
        let forget = gtk::Button::with_label("Forget devices");
        buttons.pack_start(&reveal, false, false, 0);
        buttons.pack_start(&copy, false, false, 0);
        buttons.pack_start(&rotate, false, false, 0);
        buttons.pack_start(&pair, false, false, 0);
        buttons.pack_start(&forget, false, false, 0);
        root.pack_start(&buttons, false, false, 0);

        let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
        root.pack_start(&sep, false, false, 6);

        // Allow Terminal
        let term_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let term_label = gtk::Label::new(Some("Allow Terminal from remote clients"));
        term_label.set_halign(gtk::Align::Start);
        term_label.set_hexpand(true);
        let term_switch = gtk::Switch::new();
        term_row.pack_start(&term_label, true, true, 0);
        term_row.pack_end(&term_switch, false, false, 0);
        root.pack_start(&term_row, false, false, 0);
        let term_hint = gtk::Label::new(Some(
            "The Terminal is a real shell on this machine. Off by default.",
        ));
        term_hint.set_halign(gtk::Align::Start);
        term_hint.set_line_wrap(true);
        term_hint.set_markup("<small>Off by default — enable only for devices you trust.</small>");
        root.pack_start(&term_hint, false, false, 0);

        // Autostart
        let auto_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let auto_label = gtk::Label::new(Some("Autostart server mode on login"));
        auto_label.set_halign(gtk::Align::Start);
        auto_label.set_hexpand(true);
        let auto_switch = gtk::Switch::new();
        auto_row.pack_start(&auto_label, true, true, 0);
        auto_row.pack_end(&auto_switch, false, false, 0);
        root.pack_start(&auto_row, false, false, 0);

        root.pack_start(&gtk::Separator::new(gtk::Orientation::Horizontal), false, false, 6);

        let stop = gtk::Button::with_label("Stop server");
        stop.set_hexpand(false);
        root.pack_start(&stop, false, false, 0);

        window.add(&root);

        // Keep the machine awake while serving.
        let inhibitor = start_inhibitor();
        let inhibitor = Rc::new(RefCell::new(inhibitor));

        // ── state + actions ──────────────────────────────────────────────
        let revealed = Rc::new(RefCell::new(false));
        let current_ticket = Rc::new(RefCell::new(String::new()));
        let client_for_status = client.clone();
        let base_for_status = base.clone();
        let token_for_status = token.clone();

        // Fetch + render status.
        {
            let status = status.clone();
            let ticket = ticket.clone();
            let qr = qr.clone();
            let revealed = revealed.clone();
            let current_ticket = current_ticket.clone();

            gtk::glib::timeout_add_seconds_local(2, move || {
                if let Some(value) = fetch_status(
                    &client_for_status,
                    &base_for_status,
                    &token_for_status,
                ) {
                    let enabled = value.get("enabled").and_then(Value::as_bool).unwrap_or(false);
                    let conns = value.get("connections").and_then(Value::as_u64).unwrap_or(0);
                    let bytes = value.get("bytes").and_then(Value::as_u64).unwrap_or(0);
                    let id = value
                        .get("endpoint_id")
                        .and_then(Value::as_str)
                        .map(|s| s[..s.len().min(12)].to_string())
                        .unwrap_or_default();
                    let t = value
                        .get("ticket")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let paired = value.get("paired").and_then(Value::as_u64).unwrap_or(0);
                    let pairing = value.get("pairing").and_then(Value::as_bool).unwrap_or(false);

                    if enabled {
                        let mut line =
                            format!("On — {id}… · {conns} connection(s) · {bytes} bytes");
                        if paired > 0 {
                            line.push_str(&format!(" · {paired} paired"));
                        }
                        if pairing {
                            line.push_str(" · pairing…");
                        }
                        status.set_text(&line);
                        *current_ticket.borrow_mut() = t.clone();
                        if *revealed.borrow() {
                            ticket.set_text(&t);
                            load_qr(&client_for_status, &base_for_status, &token_for_status, &qr);
                        }
                    } else {
                        status.set_text("Off");
                    }

                    // Preferences from the status payload (if the server sent them).
                    // They are also mirrored to the switches locally on change.
                }
                gtk::glib::ControlFlow::Continue
            });
        }

        // Reveal
        {
            let revealed = revealed.clone();
            let ticket = ticket.clone();
            let qr = qr.clone();
            let reveal_btn = reveal.clone();
            let current_ticket = current_ticket.clone();
            let client = client.clone();
            let base = base.clone();
            let token = token.clone();
            reveal.connect_clicked(move |_| {
                let mut r = revealed.borrow_mut();
                *r = !*r;
                if *r {
                    reveal_btn.set_label("Hide link");
                    ticket.set_visible(true);
                    qr.set_visible(true);
                    ticket.set_text(&current_ticket.borrow());
                    load_qr(&client, &base, &token, &qr);
                } else {
                    reveal_btn.set_label("Reveal link");
                    ticket.set_visible(false);
                    qr.set_visible(false);
                }
            });
        }

        // Copy
        {
            let ticket = ticket.clone();
            copy.connect_clicked(move |_| {
                ticket.select_region(0, -1);
                ticket.copy_clipboard();
            });
        }

        // Rotate
        {
            let client = client.clone();
            let base = base.clone();
            let token = token.clone();
            let status = status.clone();
            rotate.connect_clicked(move |_| {
                let _ = client
                    .post(format!("{base}/api/remote/rotate"))
                    .bearer_auth(&token)
                    .send();
                status.set_text("Rotating…");
            });
        }

        // Pair the next device
        {
            let client = client.clone();
            let base = base.clone();
            let token = token.clone();
            let status = status.clone();
            pair.connect_clicked(move |_| {
                let _ = client
                    .post(format!("{base}/api/remote/pair"))
                    .bearer_auth(&token)
                    .send();
                status.set_text("Pairing open for 120s — connect from the new device now");
            });
        }

        // Forget paired devices
        {
            let client = client.clone();
            let base = base.clone();
            let token = token.clone();
            let status = status.clone();
            forget.connect_clicked(move |_| {
                let _ = client
                    .post(format!("{base}/api/remote/unpair"))
                    .bearer_auth(&token)
                    .send();
                status.set_text("Forgot paired devices");
            });
        }

        // Allow Terminal / Autostart → preferences.
        {
            let client = client.clone();
            let base = base.clone();
            let token = token.clone();
            term_switch.connect_state_set(move |_, on| {
                let _ = client
                    .put(format!("{base}/api/preferences"))
                    .bearer_auth(&token)
                    .json(&serde_json::json!({ "remote.allow_terminal": if on { "true" } else { "" } }))
                    .send();
                gtk::glib::Propagation::Proceed
            });
        }
        {
            let client = client.clone();
            let base = base.clone();
            let token = token.clone();
            auto_switch.connect_state_set(move |_, on| {
                let _ = client
                    .put(format!("{base}/api/preferences"))
                    .bearer_auth(&token)
                    .json(&serde_json::json!({ "remote.autostart": if on { "true" } else { "" } }))
                    .send();
                gtk::glib::Propagation::Proceed
            });
        }

        // Load the current preference values once.
        seed_switches(&client, &base, &token, &term_switch, &auto_switch);

        // Stop → turn server mode off and exit back to the kiosk.
        {
            let client = client.clone();
            let base = base.clone();
            let token = token.clone();
            let window = window.clone();
            let inhibitor = inhibitor.clone();
            stop.connect_clicked(move |_| {
                let _ = client
                    .post(format!("{base}/api/remote/enable"))
                    .bearer_auth(&token)
                    .json(&serde_json::json!({ "enabled": false }))
                    .send();
                release_inhibitor(&inhibitor);
                window.close();
                std::process::exit(0);
            });
        }

        window.show_all();
        ticket.set_visible(false);
        qr.set_visible(false);
    }

    fn fetch_status(
        client: &reqwest::blocking::Client,
        base: &str,
        token: &str,
    ) -> Option<Value> {
        client
            .get(format!("{base}/api/remote/status"))
            .bearer_auth(token)
            .send()
            .ok()?
            .json::<Value>()
            .ok()
    }

    fn load_qr(client: &reqwest::blocking::Client, base: &str, token: &str, image: &gtk::Image) {
        let bytes = client
            .get(format!("{base}/api/remote/qr"))
            .bearer_auth(token)
            .send()
            .ok()
            .and_then(|r| r.bytes().ok());
        let Some(bytes) = bytes else { return };
        let loader = gtk::gdk_pixbuf::PixbufLoader::new();
        if loader.write(&bytes).is_err() || loader.close().is_err() {
            return;
        }
        if let Some(pixbuf) = loader.pixbuf() {
            image.set_from_pixbuf(Some(&pixbuf));
        }
    }

    fn seed_switches(
        client: &reqwest::blocking::Client,
        base: &str,
        token: &str,
        term: &gtk::Switch,
        auto: &gtk::Switch,
    ) {
        if let Ok(value) = client
            .get(format!("{base}/api/preferences"))
            .bearer_auth(token)
            .send()
            .and_then(|r| r.json::<Value>())
        {
            let data = value.get("data").cloned().unwrap_or(Value::Null);
            let truthy = |key: &str| {
                data.get(key)
                    .and_then(Value::as_str)
                    .map(|v| v == "true" || v == "1")
                    .unwrap_or(false)
            };
            term.set_active(truthy("remote.allow_terminal"));
            auto.set_active(truthy("remote.autostart"));
        }
    }

    /// Ask systemd to keep the machine awake (no idle/lid suspend) while serving.
    fn start_inhibitor() -> Option<Child> {
        Command::new("systemd-inhibit")
            .args([
                "--what=idle:sleep:handle-lid-switch",
                "--who=shiny-server-mode",
                "--why=Serving remote clients",
                "--mode=block",
                "sleep",
                "infinity",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()
    }

    fn release_inhibitor(child: &Rc<RefCell<Option<Child>>>) {
        if let Some(mut child) = child.borrow_mut().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

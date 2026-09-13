//! Launch configuration for the PEAK'D! browser (`peakd`).
//!
//! Everything is overridable so the shell can be pointed at a development
//! server, a benchmark target, or a bare external URL without a rebuild.

use std::env;

/// Default origin of the PEAK'D! app this browser exists to serve.
pub const DEFAULT_APP_ORIGIN: &str = "http://127.0.0.1:8080";

#[derive(Clone, Debug)]
pub struct PeakdConfig {
    /// What the first tab opens. Defaults to the PEAK'D! app origin.
    pub start_url: String,
    /// Origin treated as "the app": the only thing the browser serves in
    /// app mode, and the origin the in-app plugin window is part of.
    pub app_origin: String,
    /// App mode: no address bar, no chrome — a kiosk window for PEAK'D! itself.
    pub app_mode: bool,
    pub window_title: String,
    pub width: f64,
    pub height: f64,
    /// Open with devtools (useful while developing the proxy/rewriter).
    pub devtools: bool,
    /// Upstream proxy to route webview traffic through, e.g.
    /// `127.0.0.1:8899`. Empty = direct (no filtering).
    pub proxy: Option<String>,
    /// Directory for the compiled filter cache (and where lists are fetched
    /// into). Relative paths resolve against the process working directory.
    pub data_dir: String,
    /// Skip downloading filter lists; use only the cache and pinned rules.
    pub offline: bool,
}

impl PeakdConfig {
    pub fn from_env_and_args() -> Self {
        let app_origin =
            env::var("PEAKD_APP_ORIGIN").unwrap_or_else(|_| DEFAULT_APP_ORIGIN.to_string());

        let mut cfg = PeakdConfig {
            start_url: app_origin.clone(),
            app_origin,
            app_mode: false,
            window_title: "PEAK'D!".into(),
            width: 1280.0,
            height: 860.0,
            devtools: env_flag("PEAKD_DEVTOOLS"),
            proxy: env::var("PEAKD_PROXY")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            data_dir: env::var("PEAKD_DATA_DIR")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "data/peakd".to_string()),
            offline: env_flag("PEAKD_OFFLINE"),
        };

        let args: Vec<String> = env::args().skip(1).collect();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--url" | "-u" => {
                    if let Some(v) = args.get(i + 1) {
                        cfg.start_url = normalize_url(v);
                        i += 1;
                    }
                }
                "--app-mode" | "--app" => cfg.app_mode = true,
                "--offline" => cfg.offline = true,
                "--data-dir" => {
                    if let Some(v) = args.get(i + 1) {
                        cfg.data_dir = v.clone();
                        i += 1;
                    }
                }
                "--devtools" => cfg.devtools = true,
                "--proxy" => {
                    if let Some(v) = args.get(i + 1) {
                        cfg.proxy = Some(v.clone());
                        i += 1;
                    }
                }
                "--title" => {
                    if let Some(v) = args.get(i + 1) {
                        cfg.window_title = v.clone();
                        i += 1;
                    }
                }
                "--size" => {
                    if let Some(v) = args.get(i + 1) {
                        if let Some((w, h)) = v.split_once('x') {
                            if let (Ok(w), Ok(h)) = (w.parse(), h.parse()) {
                                cfg.width = w;
                                cfg.height = h;
                            }
                        }
                        i += 1;
                    }
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => {
                    // A bare argument is treated as the URL to open.
                    if !other.starts_with('-') {
                        cfg.start_url = normalize_url(other);
                    }
                }
            }
            i += 1;
        }

        cfg
    }

    /// A `ProxyEndpoint`-shaped `(host, port)` when a proxy is configured.
    pub fn proxy_endpoint(&self) -> Option<(String, String)> {
        let raw = self.proxy.as_ref()?;
        let raw = raw.trim();
        let raw = raw
            .strip_prefix("http://")
            .or_else(|| raw.strip_prefix("https://"))
            .unwrap_or(raw);
        let raw = raw.trim_end_matches('/');
        match raw.rsplit_once(':') {
            Some((host, port)) => Some((host.to_string(), port.to_string())),
            None => Some((raw.to_string(), "80".to_string())),
        }
    }
}

/// Give a bare host the scheme it obviously meant.
///
/// `https://` for the open web, `http://` for loopback: a local dev server has
/// no certificate, and most public sites now answer a plain-HTTP request with
/// a CDN error page rather than a redirect.
pub fn normalize_url(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return DEFAULT_APP_ORIGIN.to_string();
    }
    if trimmed.contains("://") || trimmed.starts_with("about:") {
        return trimmed.to_string();
    }
    let host = trimmed.split(['/', ':']).next().unwrap_or(trimmed);
    let is_local = host == "localhost" || host == "127.0.0.1" || host == "[::1]";
    if is_local {
        format!("http://{trimmed}")
    } else {
        format!("https://{trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_host_defaults_by_locality() {
        assert_eq!(normalize_url("example.com"), "https://example.com");
        assert_eq!(normalize_url("localhost:8080"), "http://localhost:8080");
        assert_eq!(normalize_url("127.0.0.1:8080/x"), "http://127.0.0.1:8080/x");
        assert_eq!(
            normalize_url("http://example.com/a"),
            "http://example.com/a"
        );
        assert_eq!(normalize_url(""), DEFAULT_APP_ORIGIN);
    }
}

fn env_flag(key: &str) -> bool {
    matches!(
        env::var(key).unwrap_or_default().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn print_help() {
    println!(
        "peakd — the PEAK'D! desktop browser

USAGE:
    peakd [URL] [OPTIONS]

OPTIONS:
    -u, --url <URL>      Page to open (default {DEFAULT_APP_ORIGIN})
        --app-mode       Chrome-less kiosk window for the PEAK'D! app
        --devtools       Open with devtools enabled
        --proxy <H:P>    Route webview traffic through a filtering proxy
        --data-dir <DIR> Filter cache location (default data/peakd)
        --offline        Never download filter lists; use the cache only
        --title <TITLE>  Window title
        --size <WxH>     Window size (default 1280x860)
    -h, --help           Show this help

ENVIRONMENT:
    PEAKD_APP_ORIGIN         Origin of the PEAK'D! app (default {DEFAULT_APP_ORIGIN})
    PEAKD_PROXY      Upstream filtering proxy, e.g. 127.0.0.1:8899
    PEAKD_DATA_DIR   Filter cache location
    PEAKD_OFFLINE    Set to 1 to skip downloading filter lists
    PEAKD_DEVTOOLS   Set to 1 to open devtools"
    );
}

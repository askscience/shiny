//! Launch configuration for the PEAK'D! kiosk shell (`peakd`).
//!
//! Everything is overridable so the shell can be pointed at a development
//! server, a benchmark target, or a bare external URL without a rebuild.

use std::env;
use std::path::PathBuf;

/// Default origin of the PEAK'D! app this browser exists to serve.
pub const DEFAULT_APP_ORIGIN: &str = "http://127.0.0.1:8080";

#[derive(Clone, Debug)]
pub struct PeakdConfig {
    /// What the first tab opens. Defaults to the PEAK'D! app origin.
    pub start_url: String,
    /// Origin treated as "the app": the origin the in-app plugin windows are
    /// part of.
    pub app_origin: String,
    /// App mode: no address bar, no chrome — a kiosk window for PEAK'D! itself.
    pub app_mode: bool,
    pub window_title: String,
    pub width: f64,
    pub height: f64,
    /// Open with devtools (Chromium's remote debugging on loopback).
    pub devtools: bool,
    /// Where the QtWebEngine profile lives (cookies, cache). Relative paths
    /// resolve against the process working directory.
    pub data_dir: String,
    /// Where Settings stores the interface-scale choice (`"auto"` or a factor).
    /// The shell reads it at launch and watches it for changes.
    pub display_file: PathBuf,
    /// Where the shell records the scale it applied (read by the server so
    /// Settings can show the resolved value).
    pub display_runtime_file: PathBuf,
    /// Explicit scale override (flag/env). Wins over the choice file at
    /// startup; `None` means "use whatever Settings stored".
    pub scale_override: Option<f64>,
    /// Drag a plugin window during the benchmark instead of asking the user to
    /// move the OS window.
    pub benchmark_drag_tile: bool,
    /// Run the in-app frame benchmark instead of a normal session.
    ///
    /// Off by default and compiled into an `Option`, so a normal launch carries
    /// no measurement code on the hot path.
    pub benchmark: Option<Benchmark>,
    /// Dial a remote Shiny server over Iroh instead of the local app origin.
    /// `--iroh <ticket>` / `PEAKD_IROH`.
    pub iroh: Option<String>,
}

/// Sensible defaults, so `--benchmark-seconds 5` works on its own and in any
/// argument order.
fn default_benchmark() -> Benchmark {
    Benchmark {
        seconds: 10,
        settle_seconds: 12,
        output: None,
    }
}

/// How to run the in-app frame benchmark.
#[derive(Clone, Debug)]
pub struct Benchmark {
    /// Seconds to measure for, once the app has settled.
    pub seconds: u64,
    /// Seconds to wait for the app to boot before measuring. The orb, the tiles
    /// and the map all initialise asynchronously, so measuring immediately would
    /// report the startup burst as if it were steady state.
    pub settle_seconds: u64,
    /// Write the JSON report here as well as printing it.
    pub output: Option<String>,
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
            data_dir: env::var("PEAKD_DATA_DIR")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(default_data_dir),
            display_file: env_path("PEAKD_DISPLAY_FILE")
                .unwrap_or_else(|| xdg_dir("XDG_CONFIG_HOME", ".config").join("display.json")),
            display_runtime_file: env_path("PEAKD_DISPLAY_RUNTIME_FILE")
                .unwrap_or_else(|| xdg_dir("XDG_CACHE_HOME", ".cache").join("display.json")),
            scale_override: env::var("PEAKD_UI_SCALE")
                .ok()
                .and_then(|raw| raw.trim().parse::<f64>().ok())
                .filter(|factor| factor.is_finite() && *factor >= 1.0 && *factor <= 3.0),
            benchmark_drag_tile: false,
            benchmark: None,
            iroh: env::var("PEAKD_IROH")
                .ok()
                .filter(|v| !v.trim().is_empty()),
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
                "--benchmark-drag-tile" => {
                    // Implies --benchmark: there is nothing to drag without it.
                    cfg.benchmark_drag_tile = true;
                    cfg.benchmark.get_or_insert_with(default_benchmark);
                }
                "--benchmark" => {
                    // The benchmark makes the app measure *itself*, in the
                    // window that is actually on screen. That is the whole
                    // point: a separate measurement window never becomes
                    // visible, and the engine throttles `requestAnimationFrame`
                    // to a near stop in a hidden page.
                    cfg.benchmark = Some(default_benchmark());
                }
                "--benchmark-seconds" => {
                    // Consume the value unconditionally. Guarding this on
                    // `cfg.benchmark` being set already meant the value fell
                    // through to the bare-argument branch below and was taken
                    // as the URL to open — which is what actually happened the
                    // first time these flags were used.
                    if let Some(v) = args.get(i + 1) {
                        if let Some(n) = v.parse().ok() {
                            cfg.benchmark.get_or_insert_with(default_benchmark).seconds = n;
                        }
                        i += 1;
                    }
                }
                "--benchmark-settle" => {
                    // See the note on --benchmark-seconds: consume unconditionally.
                    if let Some(v) = args.get(i + 1) {
                        if let Some(n) = v.parse().ok() {
                            cfg.benchmark.get_or_insert_with(default_benchmark).settle_seconds = n;
                        }
                        i += 1;
                    }
                }
                "--benchmark-output" => {
                    // See the note on --benchmark-seconds: consume unconditionally.
                    if let Some(v) = args.get(i + 1) {
                        cfg.benchmark.get_or_insert_with(default_benchmark).output =
                            Some(v.clone());
                        i += 1;
                    }
                }
                "--scale" => {
                    // UI page zoom, e.g. `--scale 1.5`. Overrides the file for
                    // this launch (useful for testing on a specific panel).
                    if let Some(v) = args.get(i + 1) {
                        cfg.scale_override = v
                            .parse::<f64>()
                            .ok()
                            .filter(|factor| factor.is_finite() && *factor >= 1.0 && *factor <= 3.0);
                        i += 1;
                    }
                }
                "--data-dir" => {
                    if let Some(v) = args.get(i + 1) {
                        cfg.data_dir = v.clone();
                        i += 1;
                    }
                }
                "--devtools" => cfg.devtools = true,
                "--iroh" | "--ticket" => {
                    if let Some(v) = args.get(i + 1) {
                        cfg.iroh = Some(v.clone());
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
                    // A bare argument is treated as the URL to open, or as an
                    // Iroh ticket when it carries the `iroh://` scheme.
                    if let Some(ticket) = other.strip_prefix("iroh://") {
                        cfg.iroh = Some(ticket.to_string());
                    } else if !other.starts_with('-') {
                        cfg.start_url = normalize_url(other);
                    }
                }
            }
            i += 1;
        }

        cfg
    }
}

/// Where the QtWebEngine profile lives when `PEAKD_DATA_DIR` is unset.
///
/// Anchored to the executable rather than the working directory, because a
/// macOS `.app` launches with `/` as its cwd — a relative default would try to
/// write at the filesystem root (and fail). Resolving against the binary works
/// the same on Linux.
fn default_data_dir() -> String {
    if let Ok(exe) = std::env::current_exe() {
        // target/<profile>/peakd -> the repo root is two levels up.
        if let Some(root) = exe.parent().and_then(|p| p.parent()).and_then(|p| p.parent()) {
            let candidate = root.join("data").join("peakd");
            // Only adopt it when it is plausibly the project root: a bundled
            // app should still read the repo's cache during development, but a
            // relocated binary must not create directories beside itself.
            if root.join("Cargo.toml").is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    "data/peakd".to_string()
}

/// Give a bare host the scheme it obviously meant.
///
/// `https://` for the open web, `http://` for loopback: a local dev server has
/// no certificate, and most public sites now answer a plain-HTTP request with a
/// CDN error page rather than a redirect.
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

/// An environment variable naming a file, ignoring an empty value.
fn env_path(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// `<xdg>/peakd/`, falling back to `<HOME>/<fallback>/peakd/`.
fn xdg_dir(xdg: &str, fallback: &str) -> PathBuf {
    let root = env::var_os(xdg)
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))
        .unwrap_or_else(|| PathBuf::from(fallback));
    root.join("peakd")
}

fn print_help() {
    println!(
        "peakd — the PEAK'D! kiosk shell on Qt 6 + QtWebEngine

USAGE:
    peakd [URL] [OPTIONS]

OPTIONS:
    -u, --url <URL>      Page to open (default {DEFAULT_APP_ORIGIN})
        --app-mode       Chrome-less kiosk window for the PEAK'D! app
        --iroh <LINK>    Dial a remote Shiny server over Iroh (the shiny-iroh:// link from Remote settings)
        --devtools       Expose Chromium devtools on 127.0.0.1:9222
        --data-dir <DIR> QtWebEngine profile location (default data/peakd)
        --scale <F>      Interface scale (page zoom), 1.0–3.0; overrides Settings
        --benchmark      Measure this app's own frame timing, then exit
        --benchmark-seconds <N>  Measurement window (default 10)
        --benchmark-settle <N>   Seconds to wait for the app to boot (default 12)
        --benchmark-output <P>   Also write the JSON report to P
        --title <TITLE>  Window title
        --size <WxH>     Window size (default 1280x860)
    -h, --help           Show this help

ENVIRONMENT:
    PEAKD_APP_ORIGIN         Origin of the PEAK'D! app (default {DEFAULT_APP_ORIGIN})
    PEAKD_IROH       Iroh ticket to dial instead of the local app origin
    PEAKD_DATA_DIR   QtWebEngine profile location
    PEAKD_DEVTOOLS   Set to 1 to expose devtools
    PEAKD_UI_SCALE   Interface scale override"
    );
}

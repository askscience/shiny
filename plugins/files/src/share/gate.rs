//! The public gate in front of Gokapi.
//!
//! Tailscale Funnel (or the user's reverse proxy) publishes the whole port it
//! points at. Pointed straight at Gokapi, that would expose its admin login,
//! its REST API, the public upload form and the setup pages to the internet.
//! Shiny never uses any of those through the public URL, so a tiny HTTP/1.1
//! filter sits between them. It forwards only what a recipient's browser needs
//! to open a share link ([`is_allowed`]) and answers 404 for the rest.
//!
//! Each client connection carries exactly one request: the gate rewrites the
//! request to `Connection: close`, so Gokapi closes after its response and no
//! second, unchecked request can ride the same connection (keep-alive or
//! smuggling). Recipients' browsers simply open a new connection per request.
//!
//! The gate runs on its own thread and multi-thread runtime: a large download
//! must never queue behind the plugin's serial route worker (`share/mod.rs`).
//! It touches no SQLite, so a second runtime is safe here.

use std::sync::Mutex;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

/// Largest request head accepted (request line + headers).
const MAX_HEAD: usize = 16 * 1024;
/// Time a client gets to send its request head.
const HEAD_TIMEOUT: Duration = Duration::from_secs(15);

/// Exact paths of Gokapi v2.2.4's public download pages and their assets.
const EXACT: &[&str] = &[
    "/d",            // download page (GET) and its password form (POST)
    "/downloadFile", // the download itself
    "/error",        // "file not found / expired" page
    "/main.wasm",    // client-side decryption (end-to-end encrypted files)
    "/robots.txt",
    "/site.webmanifest",
    "/favicon.ico",
    "/favicon-16x16.png",
    "/favicon-32x32.png",
    "/favicon-apple-touch-icon.png",
];

/// Path prefixes of public routes and static asset folders.
const PREFIXES: &[&str] = &[
    "/d/",       // /d/{id}/{filename} → redirect to /d?id=
    "/dh/",      // /dh/{id}/{filename} → direct download with name
    "/h/",       // hotlinks (images)
    "/hotlink/", // hotlinks, legacy form
    "/assets/",  // bootstrap, jquery, icons
    "/css/",
    "/js/",
    "/serviceworker/", // StreamSaver for end-to-end downloads
    "/custom/",        // Shiny's theme (custom.css / public.js)
];

/// Paths that accept a request body.
const POST_PATHS: &[&str] = &["/d", "/downloadFile"];

/// True when a request for `target` with `method` may reach Gokapi.
///
/// Only origin-form targets (`/path?query`) pass. Anything that could be
/// resolved to a different path upstream (`..`, encoded dots or slashes,
/// backslashes) is refused outright instead of being normalised here.
pub fn is_allowed(method: &str, target: &str) -> bool {
    if !target.starts_with('/') || target.starts_with("//") {
        return false;
    }
    let path = target.split(['?', '#']).next().unwrap_or("");
    let lower = path.to_ascii_lowercase();
    if lower.contains("..")
        || lower.contains("%2e")
        || lower.contains("%2f")
        || lower.contains("%5c")
        || lower.contains('\\')
        || lower.contains("%00")
    {
        return false;
    }
    // Gokapi serves `custom/` with a directory-listing file server, and
    // `custom/s/` holds one metadata file per share: a listing would reveal
    // every live link. Only plain file names pass there, never directories.
    if path.starts_with("/custom") && !is_custom_file(path) {
        return false;
    }
    let listed = EXACT.contains(&path) || PREFIXES.iter().any(|p| path.starts_with(p));
    match method {
        "GET" | "HEAD" => listed,
        "POST" => POST_PATHS.contains(&path),
        _ => false,
    }
}

/// `/custom/<file>.<ext>` or `/custom/s/<id>.json`: a file, not a folder.
fn is_custom_file(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/custom/") else {
        return false;
    };
    let ok_name = |n: &str| {
        !n.is_empty()
            && n.contains('.')
            && !n.starts_with('.')
            && n.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    };
    match rest.split_once('/') {
        None => ok_name(rest),
        Some(("s", name)) => ok_name(name) && name.ends_with(".json") && !name.contains('/'),
        Some(_) => false,
    }
}

/// Rewrite a request head for the single forwarded request: keep the request
/// line and headers, drop hop-by-hop connection headers, force `close`.
/// Returns `(method, target, new_head)` or `None` when the head is malformed.
pub fn rewrite_head(head: &str) -> Option<(String, String, String)> {
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split(' ');
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let version = parts.next()?;
    if parts.next().is_some() || !version.starts_with("HTTP/1.") || method.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(head.len() + 32);
    out.push_str(request_line);
    out.push_str("\r\n");
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let name = line
            .split(':')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "connection" | "keep-alive" | "proxy-connection" | "upgrade" | "te"
        ) {
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str("Connection: close\r\n\r\n");
    Some((method, target, out))
}

const NOT_FOUND: &[u8] = b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 10\r\nConnection: close\r\n\r\nNot found\n";
const BAD_REQUEST: &[u8] =
    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const BAD_GATEWAY: &[u8] =
    b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// Find the end of the request head (`\r\n\r\n`), returning its length.
fn head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

async fn handle(mut client: TcpStream, upstream_port: u16) {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let head_len = loop {
        if let Some(n) = head_end(&buf) {
            break n;
        }
        if buf.len() >= MAX_HEAD {
            let _ = client.write_all(BAD_REQUEST).await;
            return;
        }
        match tokio::time::timeout(HEAD_TIMEOUT, client.read(&mut chunk)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => return,
            Ok(Ok(n)) => buf.extend_from_slice(&chunk[..n]),
        }
    };
    let Ok(head) = std::str::from_utf8(&buf[..head_len]) else {
        let _ = client.write_all(BAD_REQUEST).await;
        return;
    };
    let Some((method, target, new_head)) = rewrite_head(head.trim_end_matches("\r\n")) else {
        let _ = client.write_all(BAD_REQUEST).await;
        return;
    };
    if !is_allowed(&method, &target) {
        let _ = client.write_all(NOT_FOUND).await;
        let _ = client.shutdown().await;
        return;
    }
    let Ok(mut upstream) = TcpStream::connect(("127.0.0.1", upstream_port)).await else {
        let _ = client.write_all(BAD_GATEWAY).await;
        return;
    };
    // Head, then whatever body bytes already arrived, then pipe both ways.
    if upstream.write_all(new_head.as_bytes()).await.is_err()
        || upstream.write_all(&buf[head_len..]).await.is_err()
    {
        return;
    }
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
}

/* ── lifecycle ────────────────────────────────────────────── */

static GATE: Mutex<Option<(u16, oneshot::Sender<()>)>> = Mutex::new(None);

/// True when the gate is listening on `port`.
pub fn is_running_on(port: u16) -> bool {
    matches!(&*GATE.lock().unwrap_or_else(|e| e.into_inner()), Some((p, _)) if *p == port)
}

/// Listen on `127.0.0.1:listen_port` and forward allowed requests to
/// `127.0.0.1:upstream_port`. A no-op when already running on that port.
pub fn start(listen_port: u16, upstream_port: u16) -> Result<(), String> {
    let mut guard = GATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((p, _)) = &*guard {
        if *p == listen_port {
            return Ok(());
        }
    }
    if let Some((_, tx)) = guard.take() {
        let _ = tx.send(());
    }
    // Bind here so a taken port is reported to the caller, not lost in a thread.
    let std_listener = std::net::TcpListener::bind(("127.0.0.1", listen_port))
        .map_err(|e| format!("share gate: cannot listen on port {listen_port}: {e}"))?;
    std_listener
        .set_nonblocking(true)
        .map_err(|e| e.to_string())?;
    let (tx, mut rx) = oneshot::channel::<()>();
    std::thread::Builder::new()
        .name("shiny-share-gate".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing_log(&format!("share gate runtime: {e}"));
                    return;
                }
            };
            rt.block_on(async move {
                let Ok(listener) = TcpListener::from_std(std_listener) else {
                    return;
                };
                loop {
                    tokio::select! {
                        _ = &mut rx => break,
                        accepted = listener.accept() => {
                            if let Ok((conn, _)) = accepted {
                                tokio::spawn(handle(conn, upstream_port));
                            }
                        }
                    }
                }
            });
            // Dropping the runtime cancels in-flight transfers.
        })
        .map_err(|e| format!("share gate thread: {e}"))?;
    *guard = Some((listen_port, tx));
    Ok(())
}

/// Stop listening; open transfers are cut.
pub fn stop() {
    if let Some((_, tx)) = GATE.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = tx.send(());
    }
}

fn tracing_log(msg: &str) {
    eprintln!("[files] {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_download_page_assets_and_hotlinks() {
        for t in [
            "/d?id=abc123",
            "/downloadFile?id=abc123",
            "/d/abc123/report.pdf",
            "/dh/abc123/report.pdf",
            "/h/xyz.png",
            "/hotlink/xyz.png",
            "/assets/dist/css/bootstrap.min.css",
            "/js/min/all_public.min.js",
            "/css/min/gokapi.min.5.css",
            "/custom/custom.v1.css",
            "/serviceworker/index.html",
            "/main.wasm?v=1",
            "/error?e2e",
        ] {
            assert!(is_allowed("GET", t), "{t}");
        }
        assert!(is_allowed("POST", "/d?id=abc123"));
        assert!(is_allowed("POST", "/downloadFile?id=abc123"));
    }

    #[test]
    fn blocks_admin_api_upload_and_root() {
        for t in [
            "/",
            "/index",
            "/login",
            "/admin",
            "/api/files/list",
            "/api/",
            "/apiKeys",
            "/publicUpload",
            "/uploadChunk",
            "/e2eSetup",
            "/users",
            "/logs",
            "/forgotpw",
            "/changePassword",
            "/auth/token",
            "/e2e.wasm",
            "/filerequests",
            "/logout",
            "/dx",
            "/downloadFileX",
            "/d2",
        ] {
            assert!(!is_allowed("GET", t), "{t}");
        }
    }

    #[test]
    fn custom_folder_serves_files_never_listings() {
        assert!(is_allowed("GET", "/custom/custom.v3.css"));
        assert!(is_allowed("GET", "/custom/public.v3.js"));
        assert!(is_allowed("GET", "/custom/s/Ab12_x-9.json"));
        for t in [
            "/custom",
            "/custom/",
            "/custom/s",
            "/custom/s/",
            "/custom/s/x",
            "/custom/s/a.js",
            "/custom/s/sub/a.json",
            "/custom/other/a.css",
            "/custom/.hidden.css",
            "/custom/a%20b.css",
            "/customx.css",
        ] {
            assert!(!is_allowed("GET", t), "{t}");
        }
    }

    #[test]
    fn blocks_traversal_and_odd_forms() {
        for t in [
            "/assets/../admin",
            "/assets/%2e%2e/admin",
            "/assets/%2E%2E/admin",
            "/js/..%2fadmin",
            "/css\\..\\admin",
            "//admin",
            "http://host/admin",
            "*",
            "/d%00",
        ] {
            assert!(!is_allowed("GET", t), "{t}");
        }
        // Body-carrying or exotic methods only where expected.
        assert!(!is_allowed("POST", "/assets/x.css"));
        assert!(!is_allowed("POST", "/api/chunk/add"));
        assert!(!is_allowed("PUT", "/d?id=a"));
        assert!(!is_allowed("DELETE", "/downloadFile?id=a"));
        assert!(!is_allowed("CONNECT", "/d"));
    }

    #[test]
    fn rewrite_forces_single_request() {
        let head = "GET /d?id=a HTTP/1.1\r\nHost: h\r\nConnection: keep-alive\r\nKeep-Alive: 5\r\nUpgrade: h2c\r\nAccept: */*";
        let (m, t, out) = rewrite_head(head).unwrap();
        assert_eq!((m.as_str(), t.as_str()), ("GET", "/d?id=a"));
        assert_eq!(
            out,
            "GET /d?id=a HTTP/1.1\r\nHost: h\r\nAccept: */*\r\nConnection: close\r\n\r\n"
        );
        assert!(rewrite_head("GET /d").is_none());
        assert!(rewrite_head("GET /d HTTP/2.0").is_none());
        assert!(rewrite_head("GET /d HTTP/1.1 extra").is_none());
    }

    #[test]
    fn gate_forwards_allowed_and_blocks_others() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            // Fake upstream: echoes back the request head it received.
            let up = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let up_port = up.local_addr().unwrap().port();
            tokio::spawn(async move {
                loop {
                    let (mut s, _) = up.accept().await.unwrap();
                    tokio::spawn(async move {
                        let mut buf = vec![0u8; 4096];
                        let n = s.read(&mut buf).await.unwrap();
                        let body = String::from_utf8_lossy(&buf[..n]).to_string();
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = s.write_all(resp.as_bytes()).await;
                    });
                }
            });
            let gate_port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
            start(gate_port, up_port).unwrap();
            assert!(is_running_on(gate_port));

            async fn ask(port: u16, req: &str) -> String {
                let mut c = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                c.write_all(req.as_bytes()).await.unwrap();
                let mut out = String::new();
                c.read_to_string(&mut out).await.unwrap();
                out
            }
            let ok = ask(gate_port, "GET /d?id=x HTTP/1.1\r\nHost: h\r\nConnection: keep-alive\r\n\r\n").await;
            assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
            assert!(ok.contains("Connection: close\r\n\r\n"), "{ok}");
            assert!(!ok.contains("keep-alive"), "{ok}");

            let blocked = ask(gate_port, "GET /admin HTTP/1.1\r\nHost: h\r\n\r\n").await;
            assert!(blocked.starts_with("HTTP/1.1 404"), "{blocked}");

            stop();
            assert!(!is_running_on(gate_port));
        });
    }
}

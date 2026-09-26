//! Local HTTP-over-Iroh proxy.
//!
//! Listens for plain HTTP on a local address and tunnels each connection to a
//! remote Shiny server over an Iroh peer-to-peer QUIC connection, so an ordinary
//! browser — or `peakd` in Iroh mode — can reach the app. One QUIC bidirectional
//! stream carries one HTTP/1.1 connection.
//!
//! The proxy tags every forwarded request with `x-shiny-remote: 1` (and
//! `Connection: close`, so one request rides each connection) — the signal the
//! server uses to deny host-capability actions to remote clients.

use std::error::Error;
use std::net::SocketAddr;

use iroh::{endpoint::presets, Endpoint, EndpointAddr};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Must match the server's ALPN.
pub const ALPN: &[u8] = b"shiny/http/1";

/// Header injected on every forwarded request.
pub const REMOTE_HEADER: &str = "x-shiny-remote";

/// Parse a connection link (`shiny-iroh://…`) or a legacy ticket.
pub fn parse_ticket(s: &str) -> Result<EndpointAddr, String> {
    shiny_iroh_proto::decode(s)
}

/// Start the proxy on a background runtime and return the bound local address
/// (useful when `listen` uses port 0).
pub fn spawn(ticket: &str, listen: SocketAddr) -> std::io::Result<SocketAddr> {
    let std_listener = std::net::TcpListener::bind(listen)?;
    std_listener.set_nonblocking(true)?;
    let local = std_listener.local_addr()?;
    let ticket = ticket.to_string();

    std::thread::Builder::new()
        .name("iroh-client".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                Ok(runtime) => runtime,
                Err(e) => {
                    eprintln!("shiny-iroh-client: runtime: {e}");
                    return;
                }
            };
            runtime.block_on(async move {
                match TcpListener::from_std(std_listener) {
                    Ok(listener) => {
                        if let Err(e) = serve(listener, &ticket).await {
                            eprintln!("shiny-iroh-client: {e}");
                        }
                    }
                    Err(e) => eprintln!("shiny-iroh-client: listener: {e}"),
                }
            });
        })?;

    Ok(local)
}

/// Accept loop for the local proxy. Runs until the listener errors.
pub async fn serve(
    listener: TcpListener,
    ticket: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let addr = parse_ticket(ticket)?;
    let endpoint = Endpoint::bind(presets::N0).await?;
    tracing::info!("proxy listening; remote endpoint {}", addr.id);

    loop {
        let (tcp, _peer) = listener.accept().await?;
        let endpoint = endpoint.clone();
        let addr = addr.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(tcp, endpoint, addr).await {
                tracing::warn!("connection error: {e}");
            }
        });
    }
}

async fn handle(
    tcp: TcpStream,
    endpoint: Endpoint,
    addr: EndpointAddr,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let conn = endpoint.connect(addr, ALPN).await?;
    let (mut send, mut recv) = conn.open_bi().await?;
    let (mut tcp_r, mut tcp_w) = tcp.into_split();

    let up = async {
        if let Err(e) = forward_request(&mut tcp_r, &mut send).await {
            tracing::debug!("upstream: {e}");
        }
        let _ = send.finish();
    };
    let down = async {
        let _ = tokio::io::copy(&mut recv, &mut tcp_w).await;
        let _ = tcp_w.shutdown().await;
    };
    tokio::join!(up, down);
    Ok(())
}

/// Forward one HTTP request, injecting the remote-client header, then stream the
/// rest of the connection verbatim.
async fn forward_request<R, W>(reader: &mut R, send: &mut W) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let header_end;
    loop {
        let n = reader.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find_headers_end(&buf) {
            header_end = pos;
            break;
        }
        if buf.len() > 128 * 1024 {
            return Ok(());
        }
    }
    send.write_all(&rewrite_headers(&buf[..header_end])).await?;
    send.write_all(&buf[header_end..]).await?;
    tokio::io::copy(reader, send).await?;
    Ok(())
}

/// Index just past the first `\r\n\r\n`.
fn find_headers_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

/// Drop any client-supplied `x-shiny-remote`/`Connection` headers and append our
/// own, so a remote client cannot clear the remote flag.
fn rewrite_headers(head: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(head);
    let mut out = String::new();
    let mut lines = text.split("\r\n");
    if let Some(request_line) = lines.next() {
        out.push_str(request_line);
        out.push_str("\r\n");
    }
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("x-shiny-remote:") || lower.starts_with("connection:") {
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str("x-shiny-remote: 1\r\n");
    out.push_str("Connection: close\r\n");
    out.push_str("\r\n");
    out.into_bytes()
}

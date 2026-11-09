//! Minimal HTTP/1.1 over a loopback TCP socket for the local sidecars
//! (Whisper, Supertonic, Qwen3-TTS), so the editor needs no HTTP client
//! crate and no async runtime: every call blocks its worker thread.

use crate::Result;
use std::io::{Read, Write};
use std::time::Duration;

/// POST `body` and return the reply's body bytes (status 200 only).
pub fn post(url: &str, content_type: &str, body: &[u8], timeout: Duration) -> Result<Vec<u8>> {
    request("POST", url, content_type, body, timeout)
}

/// GET and return the reply's body bytes (status 200 only).
pub fn get(url: &str, timeout: Duration) -> Result<Vec<u8>> {
    request("GET", url, "", &[], timeout)
}

fn request(
    method: &str,
    url: &str,
    content_type: &str,
    body: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("{url}: only local http:// sidecars are supported"))?;
    let (hostport, path) = rest
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((rest, "/".into()));
    let addr = std::net::ToSocketAddrs::to_socket_addrs(hostport)?
        .next()
        .ok_or_else(|| format!("{hostport}: no address"))?;
    let mut s = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(timeout))?;
    let ct = if content_type.is_empty() {
        String::new()
    } else {
        format!("Content-Type: {content_type}\r\n")
    };
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: {hostport}\r\n{ct}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    s.write_all(body)?;
    let mut resp = Vec::new();
    s.read_to_end(&mut resp)?;
    let split = resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| format!("{hostport}: malformed reply"))?;
    let head = String::from_utf8_lossy(&resp[..split]).to_string();
    let payload = &resp[split + 4..];
    let status = head.lines().next().unwrap_or("").to_string();
    let chunked = head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked");
    let body = if chunked {
        dechunk(payload)
    } else {
        payload.to_vec()
    };
    if !(status.starts_with("HTTP/1.1 200") || status.starts_with("HTTP/1.0 200")) {
        let detail = String::from_utf8_lossy(&body[..body.len().min(300)]).to_string();
        return Err(format!("{hostport} answered {status}: {detail}").into());
    }
    Ok(body)
}

/// Undo `Transfer-Encoding: chunked`.
pub fn dechunk(mut rest: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(eol) = rest.windows(2).position(|w| w == b"\r\n") {
        let len = std::str::from_utf8(&rest[..eol])
            .ok()
            .and_then(|l| usize::from_str_radix(l.split(';').next().unwrap_or("").trim(), 16).ok())
            .unwrap_or(0);
        rest = &rest[eol + 2..];
        if len == 0 || rest.len() < len {
            break;
        }
        out.extend_from_slice(&rest[..len]);
        rest = &rest[len..];
        if rest.starts_with(b"\r\n") {
            rest = &rest[2..];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dechunks_binary_bodies() {
        let body = b"4\r\n\x00\x01\r\n\r\n3\r\nabc\r\n0\r\n\r\n";
        assert_eq!(dechunk(body), b"\x00\x01\r\nabc");
    }
}

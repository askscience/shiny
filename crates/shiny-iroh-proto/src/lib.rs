//! The Shiny Iroh **connection link**.
//!
//! A link looks like `shiny-iroh://<base64url(postcard(EndpointAddr))>` — short
//! enough to paste and to encode in a QR code. Both the server (which mints it)
//! and the clients (which dial it) use this crate, so the format cannot drift.
//!
//! The legacy hex-encoded-JSON ticket is still accepted by [`decode`], so old
//! links keep working.

use base64::Engine as _;
use iroh_base::EndpointAddr;

/// URI scheme for a connection link.
pub const SCHEME: &str = "shiny-iroh://";

/// Encode an endpoint address as a compact base64url string (no scheme).
pub fn encode(addr: &EndpointAddr) -> String {
    let bytes = postcard::to_allocvec(addr).unwrap_or_default();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// A full, shareable connection link.
pub fn link(addr: &EndpointAddr) -> String {
    format!("{SCHEME}{}", encode(addr))
}

/// Decode a link (with or without a scheme) back into an endpoint address.
/// Accepts the compact form and the legacy hex/JSON ticket.
pub fn decode(input: &str) -> Result<EndpointAddr, String> {
    let raw = input.trim();
    let token = raw
        .strip_prefix(SCHEME)
        .or_else(|| raw.strip_prefix("shiny://"))
        .or_else(|| raw.strip_prefix("iroh://"))
        .unwrap_or(raw);

    if let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(token) {
        if let Ok(addr) = postcard::from_bytes::<EndpointAddr>(&bytes) {
            return Ok(addr);
        }
    }
    if let Ok(bytes) = hex::decode(token) {
        if let Ok(addr) = serde_json::from_slice::<EndpointAddr>(&bytes) {
            return Ok(addr);
        }
    }
    Err("invalid Shiny Iroh link".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh_base::EndpointId;

    fn sample() -> EndpointAddr {
        let key: EndpointId = "9a596f16918d043be7b44bc80c016df22527338580ce8ecaa55720aad9c20f81"
            .parse()
            .expect("key");
        EndpointAddr::new(key).with_relay_url(
            "https://euc1-1.relay.n0.iroh.link./".parse().expect("relay"),
        )
    }

    #[test]
    fn link_round_trips_and_is_compact() {
        let addr = sample();
        let link = link(&addr);
        assert!(link.starts_with(SCHEME));
        // Far shorter than the old hex(JSON) ticket (~374 chars).
        assert!(link.len() < 180, "link too long: {}", link.len());
        assert_eq!(decode(&link).expect("decode"), addr);
        // Bare token also decodes.
        assert_eq!(decode(link.trim_start_matches(SCHEME)).expect("bare"), addr);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode("not a link").is_err());
    }
}

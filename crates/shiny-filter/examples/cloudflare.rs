//! Manual check: what does the Cloudflare challenge page require?
//!
//! Run with `cargo run -p shiny-filter --example cloudflare`. Network only.

use wreq_util::emulate::{Emulation, Profile};

const EMULATION: Profile = Emulation::Chrome136;
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";

#[tokio::main]
async fn main() {
    let client = wreq::Client::builder()
        .emulation(EMULATION)
        .user_agent(UA)
        .redirect(wreq::redirect::Policy::none())
        .build()
        .expect("client builds");

    let url = "https://nowsecure.nl/";
    let resp = client.get(url).send().await.expect("send");
    println!("status {}", resp.status());
    let interesting: Vec<(String, String)> = resp
        .headers()
        .iter()
        .filter(|(name, _)| {
            let n = name.as_str();
            n.eq_ignore_ascii_case("set-cookie")
                || n.eq_ignore_ascii_case("server")
                || n.eq_ignore_ascii_case("cf-mitigated")
        })
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().unwrap_or("<bin>").to_string(),
            )
        })
        .collect();
    for (name, value) in interesting {
        println!("  {name}: {value}");
    }
    let body = resp.text().await.unwrap_or_default();
    for needle in [
        "Just a moment",
        "challenge-platform",
        "cf_clearance",
        "top.location",
        "self !== top",
        "window.top",
        "frameElement",
        "parent !== window",
    ] {
        println!("  contains {needle:?}: {}", body.contains(needle));
    }
    // Show the first challenge script reference, if any.
    if let Some(idx) = body.find("challenge-platform") {
        let start = idx.saturating_sub(80);
        println!("...{}...", &body[start..(idx + 120).min(body.len())]);
    }
}

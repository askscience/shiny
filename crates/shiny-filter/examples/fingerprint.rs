//! Manual check: does the upstream client actually present a Chrome
//! fingerprint, and do the search engines the old rustls client got 403s from
//! now answer?
//!
//! Run with `cargo run -p shiny-filter --example fingerprint`. Network only; it
//! is not part of the test suite.

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

    let targets = [
        "https://tls.peet.ws/api/all",
        "https://html.duckduckgo.com/html/?q=rust",
        "https://www.mojeek.com/search?q=rust",
        "https://www.ecosia.org/search?q=rust",
        "https://search.brave.com/search?q=rust",
    ];

    for url in targets {
        match client.get(url).send().await {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                println!("{status} {url} ({} bytes)", body.len());
                if url.contains("peet") {
                    println!("{}", &body[..body.len().min(2000)]);
                }
            }
            Err(err) => println!("ERR {url}: {err}"),
        }
    }
}

//! How long catching up takes: a planet left alone for 1, 7 and 30 days.
//!   cargo run --release -p oorbit-sim --example catchup
use std::time::Instant;

use oorbit_sim::{advance, Catalog, Planet};

fn main() {
    let catalog = Catalog::builtin();
    let t0: i64 = 1_794_470_400_000;
    for days in [1i64, 7, 30] {
        let mut planet = Planet::new(1, "temperate", t0, &catalog).unwrap();
        let start = Instant::now();
        advance(&mut planet, &catalog, t0 + days * 86_400_000);
        let advanced = start.elapsed();
        let start = Instant::now();
        let json = serde_json::to_string(&planet).unwrap();
        let _: Planet = serde_json::from_str(&json).unwrap();
        println!(
            "{days:2} days ({} steps): advance {:?}, save+load {:?}, {} bytes",
            days * 144,
            advanced,
            start.elapsed(),
            json.len()
        );
    }
}

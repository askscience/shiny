//! Print a map of the city a seed gives, around the start.
//!
//!     cargo run --example map -- 2026 [reach]
//!
//! One character per lot: `h` homes, `s` shops, `P` park, `Q` square; the
//! district is the case of the border row below it. Then the nearest place
//! of each kind.

use bevy::math::{IVec2, Vec2};
use happywalk_game::gen::{self, District, LotKind, PlaceKind};

fn main() {
    let mut args = std::env::args().skip(1);
    let seed: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(1);
    let reach: i32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(8);
    println!(
        "seed {seed}, lots {}..{} (x to the right, z downwards, start by lot 0,0)",
        -reach, reach
    );
    for z in -reach..=reach {
        let mut lots = String::new();
        let mut districts = String::new();
        for x in -reach..=reach {
            let coord = IVec2::new(x, z);
            lots.push(match gen::lot_kind(seed, coord) {
                LotKind::Homes => 'h',
                LotKind::Shops => 's',
                LotKind::Park => 'P',
                LotKind::Square => 'Q',
            });
            districts.push(match gen::district(seed, coord) {
                District::Centre => 'C',
                District::Residential => 'r',
                District::Green => '.',
            });
        }
        println!("{z:>4}  {lots}   {districts}");
    }
    let start = Vec2::new(gen::PITCH / 2.0, gen::PITCH / 2.0);
    for kind in PlaceKind::ALL {
        match gen::nearest_place(seed, start, kind, reach) {
            Some(place) => println!(
                "{:<16} at ({:.0}, {:.0}), {:.0} m from the start",
                kind.name(),
                place.at.x,
                place.at.y,
                place.at.distance(start)
            ),
            None => println!("{:<16} none within reach", kind.name()),
        }
    }
}

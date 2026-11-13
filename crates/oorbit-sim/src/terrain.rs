//! A planet's land and sea, drawn from its seed and climate: continents of
//! smooth noise, mountains on the highest ground, ice towards the poles,
//! beaches along the sea, forests where it is wet and deserts where it is
//! dry. The climate's biome weights set how much land each biome gets.

use crate::catalog::{Biome, ClimateDef};
use crate::cells::Layout;
use crate::planet::Tile;

/// Height of the sea: ocean cells sit exactly on it.
pub const SEA_LEVEL: f64 = 1.0;

fn hash(seed: u64, x: i64, y: i64, z: i64) -> f64 {
    let mut h = seed
        ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (z as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Smooth value noise in 0..1.
fn noise(seed: u64, p: [f64; 3]) -> f64 {
    let fl = p.map(f64::floor);
    let f = [p[0] - fl[0], p[1] - fl[1], p[2] - fl[2]].map(|t| t * t * (3.0 - 2.0 * t));
    let i = fl.map(|v| v as i64);
    let mut sum = 0.0;
    for corner in 0..8 {
        let d = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
        let mut w = 1.0;
        for k in 0..3 {
            w *= if d[k] == 1 { f[k] } else { 1.0 - f[k] };
        }
        sum += w * hash(seed, i[0] + d[0] as i64, i[1] + d[1] as i64, i[2] + d[2] as i64);
    }
    sum
}

/// Three octaves of noise, 0..1.
fn fbm(seed: u64, p: [f64; 3], scale: f64) -> f64 {
    let mut total = 0.0;
    let mut amp = 0.5;
    let mut freq = scale;
    let mut norm = 0.0;
    for octave in 0..3u64 {
        let q = p.map(|v| v * freq + 17.0);
        total += amp * noise(seed.wrapping_add(octave * 0x51ED), q);
        norm += amp;
        amp *= 0.4;
        freq *= 2.0;
    }
    total / norm
}

/// Cells sorted by `key`, ascending (ties by index, so it is stable).
fn sorted_by(cells: &[usize], key: impl Fn(usize) -> f64) -> Vec<usize> {
    let mut v = cells.to_vec();
    v.sort_by(|&a, &b| key(a).total_cmp(&key(b)).then(a.cmp(&b)));
    v
}

fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

/// The planet's cells and its home: where the first settlement stands.
pub fn generate(seed: u64, climate: &ClimateDef, layout: &Layout) -> (Vec<Tile>, usize) {
    let n = layout.len();
    let elevation: Vec<f64> = (0..n)
        .map(|i| {
            let c = layout.centers[i];
            0.8 * fbm(seed ^ 0xE1E7, c, 1.1) + 0.2 * fbm(seed ^ 0x0E1E, c, 2.4)
        })
        .collect();
    let moisture: Vec<f64> = (0..n)
        .map(|i| fbm(seed ^ 0x3015, layout.centers[i], 1.5))
        .collect();
    let all: Vec<usize> = (0..n).collect();
    let mut biome = vec![Biome::Plains; n];

    // Sea: the lowest ground.
    let sea = ((climate.ocean.clamp(0.1, 0.85)) * n as f64).round() as usize;
    let by_height = sorted_by(&all, |i| elevation[i]);
    for &i in &by_height[..sea] {
        biome[i] = Biome::Ocean;
    }
    // No puddles: seas smaller than a few cells become land.
    let mut seen = vec![false; n];
    for start in 0..n {
        if seen[start] || biome[start] != Biome::Ocean {
            continue;
        }
        let reach = layout.hops(&[start], |j| biome[j] == Biome::Ocean);
        let members: Vec<usize> = (0..n).filter(|&j| reach[j] != usize::MAX).collect();
        for &j in &members {
            seen[j] = true;
        }
        if members.len() < 5 {
            for &j in &members {
                biome[j] = Biome::Plains;
            }
        }
    }
    let land: Vec<usize> = by_height
        .iter()
        .copied()
        .filter(|&i| biome[i] != Biome::Ocean)
        .collect();
    let total_weight: u32 = climate.biomes.values().sum::<u32>().max(1);
    let share = |b: Biome| -> usize {
        let w = climate.biomes.get(&b).copied().unwrap_or(0);
        ((w as f64 / total_weight as f64) * land.len() as f64).round() as usize
    };

    // Mountains on the highest land.
    let mountains = share(Biome::Mountain);
    for &i in land.iter().rev().take(mountains) {
        biome[i] = Biome::Mountain;
    }
    let free = |biome: &[Biome]| -> Vec<usize> {
        land.iter().copied().filter(|&i| biome[i] == Biome::Plains).collect()
    };
    // Ice towards the poles.
    let ice = share(Biome::Ice);
    let candidates = free(&biome);
    let polar = sorted_by(&candidates, |i| {
        -(layout.y(i).abs() + 0.25 * moisture[i])
    });
    for &i in polar.iter().take(ice) {
        biome[i] = Biome::Ice;
    }
    // Beaches: low land by the sea.
    let coast = share(Biome::Coast);
    let shore: Vec<usize> = free(&biome)
        .into_iter()
        .filter(|&i| layout.neighbors[i].iter().any(|&j| biome[j] == Biome::Ocean))
        .collect();
    for &i in sorted_by(&shore, |i| elevation[i]).iter().take(coast) {
        biome[i] = Biome::Coast;
    }
    // Forest where it is wet, desert where it is dry and warm.
    let forest = share(Biome::Forest);
    for &i in sorted_by(&free(&biome), |i| -moisture[i]).iter().take(forest) {
        biome[i] = Biome::Forest;
    }
    let desert = share(Biome::Desert);
    for &i in sorted_by(&free(&biome), |i| moisture[i] + 0.4 * layout.y(i).abs())
        .iter()
        .take(desert)
    {
        biome[i] = Biome::Desert;
    }

    let home = settle(layout, &mut biome);

    // Heights: the sea is flat; land rises with its elevation rank, and
    // mountains stand well above it.
    let mut height = vec![SEA_LEVEL; n];
    let land_count = land.len().max(1) as f64;
    for (rank, &i) in land.iter().enumerate() {
        let r = rank as f64 / land_count;
        height[i] = match biome[i] {
            Biome::Mountain => SEA_LEVEL + 0.035 + 0.025 * r + 0.015 * moisture[i],
            Biome::Coast => SEA_LEVEL + 0.005,
            Biome::Ice => SEA_LEVEL + 0.014 + 0.01 * r,
            Biome::Ocean => SEA_LEVEL,
            _ => SEA_LEVEL + 0.009 + 0.014 * r,
        };
    }
    let tiles = (0..n)
        .map(|i| Tile {
            biome: biome[i],
            building: None,
            height: round4(height[i]),
            stock: 1.0,
        })
        .collect();
    (tiles, home)
}

/// Pick the home cell (plains with forest, hills and sea close by) and make
/// sure the first settlement has what the stone age needs within reach.
fn settle(layout: &Layout, biome: &mut [Biome]) -> usize {
    let land = |b: Biome| b != Biome::Ocean;
    let score = |biome: &[Biome], i: usize| -> f64 {
        if !matches!(biome[i], Biome::Plains | Biome::Coast) {
            return f64::NEG_INFINITY;
        }
        let near = layout.hops(&[i], |_| true);
        let mut kinds = [false; 4];
        let mut land_near = 0;
        for (j, &d) in near.iter().enumerate() {
            if d == 0 || d > 3 {
                continue;
            }
            if land(biome[j]) {
                land_near += 1;
            }
            if d <= 2 {
                match biome[j] {
                    Biome::Forest => kinds[0] = true,
                    Biome::Mountain => kinds[1] = true,
                    Biome::Plains => kinds[2] = true,
                    Biome::Ocean | Biome::Coast => kinds[3] = true,
                    _ => {}
                }
            }
        }
        // Away from the poles, so the camera sees it well.
        kinds.iter().filter(|k| **k).count() as f64 * 10.0 + land_near as f64
            - 20.0 * layout.y(i).abs()
    };
    // Only on the largest continent, so the settlement has room to grow.
    let mut continent = vec![usize::MAX; layout.len()];
    let mut biggest = (0, 0);
    for start in 0..layout.len() {
        if continent[start] != usize::MAX || !land(biome[start]) {
            continue;
        }
        let reach = layout.hops(&[start], |j| land(biome[j]));
        let members: Vec<usize> = (0..layout.len()).filter(|&j| reach[j] != usize::MAX).collect();
        for &j in &members {
            continent[j] = start;
        }
        if members.len() > biggest.1 {
            biggest = (start, members.len());
        }
    }
    let home = (0..layout.len())
        .filter(|&i| continent[i] == biggest.0)
        .max_by(|&a, &b| score(biome, a).total_cmp(&score(biome, b)).then(b.cmp(&a)))
        .unwrap_or(0);
    if !land(biome[home]) {
        biome[home] = Biome::Plains;
    }
    let near = layout.hops(&[home], |_| true);
    for want in [Biome::Forest, Biome::Mountain, Biome::Plains] {
        let has = near
            .iter()
            .enumerate()
            .any(|(j, &d)| j != home && d <= 2 && biome[j] == want);
        if has {
            continue;
        }
        // Turn the nearest ordinary land (two hops out, else one) into it.
        let pick = [2, 1].iter().find_map(|&hop| {
            (0..layout.len()).find(|&j| {
                near[j] == hop && matches!(biome[j], Biome::Plains | Biome::Desert | Biome::Coast | Biome::Ice)
            })
        });
        let pick = pick.or_else(|| (0..layout.len()).find(|&j| near[j] == 2 && j != home));
        if let Some(j) = pick {
            biome[j] = want;
        }
    }
    home
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::cells::layout;

    #[test]
    fn every_climate_makes_seas_continents_and_a_good_home() {
        let catalog = Catalog::builtin();
        let l = layout(642).unwrap();
        for climate in &catalog.climates {
            for seed in 0..20u64 {
                let (tiles, home) = generate(seed, climate, l);
                let count = |b: Biome| tiles.iter().filter(|t| t.biome == b).count();
                let ocean = count(Biome::Ocean) as f64 / tiles.len() as f64;
                assert!((ocean - climate.ocean).abs() < 0.04, "{} ocean {ocean}", climate.id);
                assert!(matches!(tiles[home].biome, Biome::Plains | Biome::Coast));
                let near = l.hops(&[home], |_| true);
                for want in [Biome::Forest, Biome::Mountain, Biome::Plains] {
                    assert!(
                        (0..tiles.len()).any(|j| j != home && near[j] <= 2 && tiles[j].biome == want),
                        "{} seed {seed}: no {want:?} near home",
                        climate.id
                    );
                }
                assert!(count(Biome::Coast) > 0);
                // The home continent has room to grow.
                let reach = l.hops(&[home], |j| tiles[j].biome != Biome::Ocean);
                let continent = reach.iter().filter(|&&d| d != usize::MAX).count();
                assert!(continent >= 40, "{} seed {seed}: continent of {continent}", climate.id);
            }
        }
    }

    #[test]
    fn the_same_seed_draws_the_same_planet() {
        let catalog = Catalog::builtin();
        let l = layout(642).unwrap();
        let climate = &catalog.climates[0];
        assert_eq!(generate(9, climate, l), generate(9, climate, l));
        assert_ne!(generate(9, climate, l).0, generate(10, climate, l).0);
    }
}

//! A small deterministic generator (SplitMix64). The simulation never uses a
//! global or time-seeded source: every draw comes from a generator built from
//! the planet seed and a step index, so results are reproducible.

#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    /// The generator for one simulation step of one planet.
    pub fn for_step(seed: u64, step: i64) -> Self {
        let mut rng = Rng(seed ^ (step as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        rng.next_u64(); // decorrelate neighbouring steps
        rng
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// An index picked with probability proportional to its weight; `None`
    /// when every weight is zero.
    pub fn weighted(&mut self, weights: &[u32]) -> Option<usize> {
        let total: u64 = weights.iter().map(|&w| w as u64).sum();
        if total == 0 {
            return None;
        }
        let mut pick = self.next_u64() % total;
        for (i, &w) in weights.iter().enumerate() {
            if pick < w as u64 {
                return Some(i);
            }
            pick -= w as u64;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_and_step_give_the_same_draws() {
        let a: Vec<u64> = (0..4).map(|_| Rng::for_step(7, 42).next_u64()).collect();
        assert!(a.windows(2).all(|w| w[0] == w[1]));
        assert_ne!(
            Rng::for_step(7, 42).next_u64(),
            Rng::for_step(7, 43).next_u64()
        );
    }

    #[test]
    fn weighted_never_picks_a_zero_weight() {
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            assert_eq!(rng.weighted(&[0, 5, 0]), Some(1));
        }
        assert_eq!(rng.weighted(&[0, 0]), None);
    }
}

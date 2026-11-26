//! Things that move and can be walked into: passers-by, cars, bikes. Each
//! module that moves something writes where its things are, every frame; the
//! player's step and the traffic read it. Nobody else needs to know who
//! moves what.

use bevy::prelude::*;

use crate::gen::SolidKind;

/// A moving thing's box on the ground this frame.
#[derive(Clone, Copy, Debug)]
pub struct Mover {
    pub at: Vec2,
    pub half: Vec2,
    pub kind: SolidKind,
    /// The model it is ("person" for a passer-by), for describing it.
    pub model: &'static str,
}

impl Mover {
    pub fn touches(&self, at: Vec2, radius: f32) -> bool {
        (at.x - self.at.x).abs() < self.half.x + radius
            && (at.y - self.at.y).abs() < self.half.y + radius
    }
}

/// Where everything that moves is.
#[derive(Resource, Default)]
pub struct Movers {
    pub people: Vec<Mover>,
    pub vehicles: Vec<Mover>,
}

impl Movers {
    pub fn all(&self) -> impl Iterator<Item = &Mover> {
        self.people.iter().chain(&self.vehicles)
    }

    /// The first moving thing a square at `at` would touch, whole.
    pub fn what(&self, at: Vec2, radius: f32) -> Option<&Mover> {
        self.all().find(|mover| mover.touches(at, radius))
    }

    /// The first moving thing a square at `at` would touch.
    pub fn hit(&self, at: Vec2, radius: f32) -> Option<SolidKind> {
        self.all()
            .find(|mover| mover.touches(at, radius))
            .map(|mover| mover.kind)
    }
}

/// A small generator of chance for things that need not be the same every
/// time (which street the next car comes down).
#[derive(Clone, Copy, Debug)]
pub struct Dice(pub u64);

impl Dice {
    pub fn roll(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 24) as u32
    }

    pub fn below(&mut self, n: u32) -> u32 {
        self.roll() % n.max(1)
    }

    pub fn unit(&mut self) -> f32 {
        (self.roll() % 10_000) as f32 / 10_000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mover_is_hit_like_a_box() {
        let movers = Movers {
            people: vec![Mover {
                at: Vec2::new(2.0, 0.0),
                half: Vec2::splat(0.3),
                kind: SolidKind::Person,
                model: "person",
            }],
            vehicles: vec![Mover {
                at: Vec2::new(0.0, 10.0),
                half: Vec2::new(0.9, 1.8),
                kind: SolidKind::Car,
                model: "car_small_red",
            }],
        };
        assert_eq!(
            movers.hit(Vec2::new(1.4, 0.0), 0.38),
            Some(SolidKind::Person)
        );
        assert_eq!(movers.hit(Vec2::new(1.2, 0.0), 0.38), None);
        assert_eq!(movers.hit(Vec2::new(0.0, 8.0), 0.38), Some(SolidKind::Car));
        assert_eq!(movers.hit(Vec2::new(1.4, 8.0), 0.38), None);
    }

    #[test]
    fn dice_stay_in_range() {
        let mut dice = Dice(42);
        for _ in 0..1000 {
            assert!(dice.below(7) < 7);
            let unit = dice.unit();
            assert!((0.0..1.0).contains(&unit));
        }
    }
}

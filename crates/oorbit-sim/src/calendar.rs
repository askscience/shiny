//! The planet's calendar: days, seasons, years and the festivals that come
//! back every year.
//!
//! A day is 24 planet hours ([`DAY_STEPS`]); a season lasts
//! `tuning.season_days` (see `step::season_of`); a year is four seasons,
//! counted from the planet's founding for its birthday. Halfway through each
//! season, at six in the evening, the people hold the season's festival:
//! flowers in spring, the sun in summer, lanterns in autumn, the stars in
//! winter. A festival costs food and lifts the mood while it lasts; when there is not enough, it is called
//! off and the people are sad about it. All of it is in the Chronicle.

use serde::Serialize;

use crate::planet::Celebration;
use crate::{Catalog, Planet, STEPS_PER_HOUR, STEP_MS};

/// Steps in one planet day.
pub const DAY_STEPS: i64 = 24 * STEPS_PER_HOUR;
/// The season's festival starts at this hour of the planet day.
pub const FESTIVAL_HOUR: i64 = 18;
/// And lasts this many planet hours.
pub const FESTIVAL_HOURS: i64 = 8;
/// The birthday's party, in planet hours.
pub const ANNIVERSARY_HOURS: i64 = 6;
/// Food eaten per person at a festival.
pub const FESTIVAL_FOOD_PER_PERSON: f64 = 0.5;
/// A festival is held only with this many times its food in store.
pub const FESTIVAL_RESERVE: f64 = 3.0;
/// Culture made per person at a festival.
pub const FESTIVAL_CULTURE_PER_PERSON: f64 = 0.3;
/// While a festival lasts, the mood heads this much higher; one called
/// off costs some mood at once.
pub const CELEBRATION_MOOD: f64 = 0.15;
pub const CALLED_OFF_MOOD: f64 = -0.03;
/// Each season's festival.
pub const FESTIVALS: [&str; 4] = ["flowers", "sun", "lanterns", "stars"];

/// Steps in one season.
pub fn season_steps(catalog: &Catalog) -> i64 {
    ((catalog.tuning.season_days * DAY_STEPS as f64) as i64).max(1)
}

/// Steps in one year (four seasons).
pub fn year_steps(catalog: &Catalog) -> i64 {
    season_steps(catalog) * 4
}

/// The first step of the season `step` falls in.
pub fn season_start(step: i64, catalog: &Catalog) -> i64 {
    let s = season_steps(catalog);
    step.div_euclid(s) * s
}

/// When the festival of the season starting at `start` begins: the
/// FESTIVAL_HOUR nearest the season's middle that leaves the whole festival
/// inside the season.
pub fn festival_step(start: i64, catalog: &Catalog) -> i64 {
    let season = season_steps(catalog);
    let middle = start + season / 2;
    let latest = start + season - FESTIVAL_HOURS * STEPS_PER_HOUR;
    let evening = FESTIVAL_HOUR * STEPS_PER_HOUR;
    let first = middle - middle.rem_euclid(DAY_STEPS) + evening - DAY_STEPS;
    (0..3)
        .map(|k| first + k * DAY_STEPS)
        .filter(|at| *at >= start && *at <= latest)
        .min_by_key(|at| (at - middle).abs())
        .unwrap_or(start)
}

/// What happens on the calendar at `step`: the season's festival, the
/// planet's birthday.
pub(crate) fn run(planet: &mut Planet, catalog: &Catalog, step: i64) {
    if planet
        .celebration
        .as_ref()
        .is_some_and(|c| step >= c.until_step)
    {
        planet.celebration = None;
    }
    let year = year_steps(catalog);
    let age = step - planet.created_step;
    if age > 0 && age % year == 0 {
        let years = (age / year).to_string();
        planet.log(catalog, step, "anniversary", &[("years", &years)]);
        celebrate(planet, "anniversary", step, ANNIVERSARY_HOURS);
    }
    let start = season_start(step, catalog);
    if step != festival_step(start, catalog) || age <= 0 {
        return;
    }
    let festival = FESTIVALS[crate::step::season_of(step, catalog)];
    let cost = (planet.population * FESTIVAL_FOOD_PER_PERSON).round();
    // Nobody feasts on the last of the stores: a festival needs food left
    // over for the days after.
    if planet.starving || planet.amount("food") < cost * FESTIVAL_RESERVE {
        planet.mood = (planet.mood + CALLED_OFF_MOOD).clamp(0.0, 1.0);
        planet.log(
            catalog,
            step,
            "festival_called_off",
            &[("festival", festival)],
        );
        return;
    }
    planet.add("food", -cost);
    planet.add(
        "culture",
        (planet.population * FESTIVAL_CULTURE_PER_PERSON).round(),
    );
    planet.log(catalog, step, "festival", &[("festival", festival)]);
    celebrate(planet, festival, step, FESTIVAL_HOURS);
}

fn celebrate(planet: &mut Planet, id: &str, step: i64, hours: i64) {
    planet.celebration = Some(Celebration {
        id: id.to_string(),
        from_step: step,
        until_step: step + hours * STEPS_PER_HOUR,
    });
}

/// The calendar as the window shows it (times in planet milliseconds).
#[derive(Debug, Clone, Serialize)]
pub struct CalendarView {
    /// 0 spring, 1 summer, 2 autumn, 3 winter.
    pub season: usize,
    pub season_start_ms: i64,
    pub season_ms: i64,
    pub day_ms: i64,
    /// Years since the founding (the first year is 1).
    pub year: i64,
    /// The next season's festival: its id and when it starts.
    pub next_festival: &'static str,
    pub next_festival_ms: i64,
}

pub fn view(planet: &Planet, catalog: &Catalog) -> CalendarView {
    let step = planet.step;
    let start = season_start(step, catalog);
    let mut next = festival_step(start, catalog);
    let mut season_of_next = start;
    if next <= step {
        season_of_next = start + season_steps(catalog);
        next = festival_step(season_of_next, catalog);
    }
    CalendarView {
        season: crate::step::season_of(step, catalog),
        season_start_ms: start * STEP_MS,
        season_ms: season_steps(catalog) * STEP_MS,
        day_ms: DAY_STEPS * STEP_MS,
        year: (step - planet.created_step).max(0) / year_steps(catalog) + 1,
        next_festival: FESTIVALS[crate::step::season_of(season_of_next, catalog)],
        next_festival_ms: next * STEP_MS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fed_planet_holds_its_festival_and_logs_it() {
        let catalog = Catalog::builtin();
        let mut planet = Planet::new(7, "temperate", 0, &catalog).unwrap();
        let start = season_start(planet.step, &catalog) + season_steps(&catalog);
        let at = festival_step(start, &catalog);
        planet.resources.insert("food".into(), 500.0);
        run(&mut planet, &catalog, at);
        let c = planet.celebration.clone().expect("a festival");
        assert_eq!(c.until_step, at + FESTIVAL_HOURS * STEPS_PER_HOUR);
        assert!(planet.chronicle.iter().any(|e| e.key == "festival"));
        run(&mut planet, &catalog, c.until_step);
        assert!(planet.celebration.is_none());
        // Without food it is called off.
        let next = festival_step(start + season_steps(&catalog), &catalog);
        planet.resources.insert("food".into(), 0.0);
        run(&mut planet, &catalog, next);
        assert!(planet.celebration.is_none());
        assert!(planet
            .chronicle
            .iter()
            .any(|e| e.key == "festival_called_off"));
    }

    #[test]
    fn festival_falls_in_its_season_in_the_evening() {
        let catalog = Catalog::builtin();
        for k in 0..12 {
            let start = k * season_steps(&catalog);
            let at = festival_step(start, &catalog);
            assert!(at >= start);
            assert!(at + FESTIVAL_HOURS * STEPS_PER_HOUR <= start + season_steps(&catalog));
            assert_eq!(at.rem_euclid(DAY_STEPS), FESTIVAL_HOUR * STEPS_PER_HOUR);
        }
    }
}

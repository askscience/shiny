# oorbit-sim

The simulation behind [Oorbit](../../plugins/oorbit/docs/README.md): a tiny
planet whose society grows in real time. Pure Rust, no I/O and no clock: every
call takes the current time, so the plugin, the tests and a balancing bot all
drive the same rules.

## Time and determinism

- Time is cut into 10-minute **steps aligned to the Unix epoch**
  (`STEP_MS`). The world changes only at step boundaries.
- Each step draws its randomness from `Rng::for_step(seed, step)`, a fixed
  number of draws per step whatever happens.
- `advance(planet, catalog, now_ms)` processes every boundary up to `now_ms`.
  Advancing from A to C in one call or stopping at B on the way gives the same
  planet, bit for bit. A computer switched off overnight catches up on opening
  with the result it would have had if it stayed on.
- `apply(planet, catalog, now_ms, action)` advances to `now_ms`, then checks
  and applies the action.
- The planet has 642 cells (`cells.rs`, embedded from `data/cells_642.json`)
  whose land and sea `terrain.rs` draws from the seed and climate. A planet
  saved on another layout (32 or 72 tiles before version 4) is reborn on its
  next advance: new land, its buildings moved around the new home, the rest
  kept.
- Saved planets round-trip through JSON exactly (`serde_json` with
  `float_roundtrip`): reloading a planet never changes its future.

## Time

Everything here runs on planet time. `clock.rs` turns wall time into it for
the plugin: 60 times faster while someone plays (`Clock::present`), half as
fast otherwise, counted in half milliseconds so it is exact in pieces.

## Rules in brief

| Part | How it works |
|---|---|
| Work | 80% of the population works. Each standing order (`orders.rs`: a cell and a resource) and each extracting building (a mine) takes a crew of one ninth of the workers (smaller with more than nine). Whoever has no crew thinks: two thirds make knowledge, one third culture. |
| Output | Per worker-hour rate × (1 + bonuses) × efficiency. Efficiency is `0.6 + 0.8 × mood`, halved after 36 hours with no action (the planet slows down, it never decays), times the settlers' zeal of a young planet. |
| Gathering | A crew gathers its resource on its cell: food on meadows, forests, beaches and the shallow sea; wood in forests; stone on mountains; clay on meadows once Pottery is known; marble on mountains once Masonry is. The cell's stock (0 to 1) drops 2% an hour while worked and refills 1.5% an hour at rest; output is `0.4 + 0.6 × stock` of a full cell's, and below 15% the crew moves to the nearest fuller cell. Mines yield copper and tin, later iron and coal. Bronze comes from a foundry's conversion. |
| Cities | The capital stands on the home (`cities.rs`). A colony (era 2) founds a city on free land at least 5 cells from the others, within 8 cells over land of a city or by any shore within 16 once a port stands. New buildings go within 4 cells (over land) of a city. A road (A* over land, mountains dear, 3 wood and 2 stone per cell, half an hour per cell) or a sea route between two cities' ports (20 wood plus 1.5 per cell, 3 hours) links two cities. Crews on the cells of a city with no way to the capital work at 60%; each linked colony adds 5% to knowledge and culture (up to four). |
| Food | Seasons follow a one-week year; the climate multiplies food. Each person eats a fixed amount. |
| People | Grow while fed and housed, shrink slowly while hungry, leave when there is no room; never below the floor. |
| Mood | Heads towards a target made of food, housing, culture per person, effects and traits. |
| Research | One discovery at a time; it completes when enough knowledge is banked. |
| Eras | Advance when the era's discoveries are known and its monument stands. |
| Events | About one or two a day, each with choices; unanswered for a day, the people take the first (prudent) choice themselves. |
| Notables | Named people who boost the category the people care most about, for three weeks. |
| Workshops | Converters (foundry, mint, glassworks, steel mill, power plant, data centre...) scale with the population and the materials bonuses, stop when their output is full, and leave a tenth of the storage of every input still needed to build. |
| Demolition | Any building but a monument can be cleared for something new; nothing is refunded. |
| Neighbours | Three simulated planets (`neighbors.rs`), one generous, one trader, one prankster, an era ahead, level and behind. They send gifts, ideas, visitors, pranks, trade offers and messages about every 14 hours, and answer the player's ships. Travel time and cargo by the sender's era (`travel_hours`, `ship_capacity`). Gifts and trades only carry resources of the eras the player has reached (`Catalog::resource_era`). |
| Chronicle | Every change is logged as a key plus parameters, for the window and the assistant to tell. |

## View and preview

`view::planet_view` is what the plugin's state route returns: the planet,
its rates, and the status of every discovery and building of the eras
reached. `examples/preview.rs` serves the same view from a local file, for
the window's preview server (`plugins/oorbit/scripts/preview-server.py`);
`preview FILE NOW autoplay DAYS [CLIMATE]` makes a planet DAYS old, played
by `autoplay`.

## Data

Everything tunable lives in `data/catalog.toml`: tuning constants, climates,
resources, eras, discoveries, buildings and events. `Catalog::parse` rejects a
catalog with a dangling id. The shipped catalog has all ten eras: Stone,
Agricultural, Bronze, Classical, Middle Ages, Renaissance, Industrial,
Electric, Neon and Orbital.

## Tests

```bash
cargo test -p oorbit-sim                                   # rules, determinism, pacing
cargo test -p oorbit-sim --test balance -- --nocapture     # print the era timeline
SEED=2 CLIMATE=arid FROM=30 TO=60 cargo test --release -p oorbit-sim --test trace -- --ignored --nocapture
```

`tests/balance.rs` plays 12 planets (four climates, three seeds) with the
steady player of `autoplay.rs` (two visits a day, waking hours only) and
checks the pacing: era 2 by day 1.5 to 5, era 3 by day 5 to 12, era 4 by day
10 to 22, era 7 by day 30 to 52, era 10 by day 45 to 72, the whole game done
by day 50 to 85. `tests/neighbors.rs` covers the neighbours, `tests/cities.rs` colonies, roads and sea routes.
`tests/first_session.rs` checks the first visit: a player looking in every
five minutes sees a discovery within 20 minutes, two within the hour and a
first building within 90 minutes. Settlers' zeal (`zeal_bonus`,
`zeal_hours`) makes a new planet work up to nine times faster at first,
fading over its first day.

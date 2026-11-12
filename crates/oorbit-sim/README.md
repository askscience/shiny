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
- Saved planets round-trip through JSON exactly (`serde_json` with
  `float_roundtrip`): reloading a planet never changes its future.

## Rules in brief

| Part | How it works |
|---|---|
| Work | 80% of the population works, split by four priorities (food, materials, knowledge, culture; 0 to 4 each). |
| Output | Per worker-hour rate × (1 + bonuses) × efficiency. Efficiency is `0.6 + 0.8 × mood`, halved after 36 hours with no action (the planet slows down, it never decays). |
| Materials | Split among the materials the planet can gather, weighted by the tiles of their biome (wood: forest, stone: mountain, clay: plains once Pottery is known) or by the buildings that extract them (copper and tin: mines). Bronze comes from a foundry's conversion. |
| Food | Seasons follow a one-week year; the climate multiplies food. Each person eats a fixed amount. |
| People | Grow while fed and housed, shrink slowly while hungry, leave when there is no room; never below the floor. |
| Mood | Heads towards a target made of food, housing, culture per person, effects and traits. |
| Research | One discovery at a time; it completes when enough knowledge is banked. |
| Eras | Advance when the era's discoveries are known and its monument stands. |
| Events | About one or two a day, each with choices; unanswered for a day, the people take the first (prudent) choice themselves. |
| Notables | Named people who boost the category the people care most about, for three weeks. |
| Chronicle | Every change is logged as a key plus parameters, for the window and the assistant to tell. |

## View and preview

`view::planet_view` is what the plugin's state route returns: the planet,
its rates, and the status of every discovery and building of the eras
reached. `examples/preview.rs` serves the same view from a local file, for
the window's preview server (`plugins/oorbit/scripts/preview-server.py`).

## Data

Everything tunable lives in `data/catalog.toml`: tuning constants, climates,
resources, eras, discoveries, buildings and events. `Catalog::parse` rejects a
catalog with a dangling id. The shipped catalog covers the first three eras:
Stone, Agricultural and Bronze.

## Tests

```bash
cargo test -p oorbit-sim                                   # rules, determinism, pacing
cargo test -p oorbit-sim --test balance -- --nocapture     # print the era timeline
SEED=2 CLIMATE=arid cargo test -p oorbit-sim --test trace -- --ignored --nocapture
```

`tests/balance.rs` plays 20 planets (four climates, five seeds) with a steady
bot (two visits a day, waking hours only) and checks the pacing: era 2 by day
2 to 8, era 3 by day 6 to 16, the first three eras done by day 12 to 27.

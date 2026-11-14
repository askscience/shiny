# Oorbit: game design

> "A planet in your pocket. A civilisation that grows while you sleep. Your
> friends are the planets next door."

Status of each part: **built** in `crates/oorbit-sim` (M1), or **planned**.

## Pillars

1. **Real time is the material.** Growth, research and travel take real hours
   and days. Slowness is the architecture, not a flaw.
2. **A society, not a company.** People with needs, moods and a culture, not
   numbers to optimise.
3. **Others are needed.** Some things come only from friends: their climate's
   resources, ideas, visiting people.
4. **Absence is never punished.** An idle planet slows down and waits. It
   never dies, never loses buildings or eras.
5. **Laura is the chronicler.** She tells and comments; the game never waits
   for the AI.

## Loop

- **Session (as long as you like):** while you play, an hour of the planet
  lasts a minute. Point at a forest, a hill, a meadow or the sea by the shore
  and a crew goes there; watch them walk back with their load, put up
  buildings and see them rise, answer events, research, load the dock.
- **Away:** the planet keeps going at half the speed of the wall clock: the
  crews keep working, ships arrive, events wait.
- **Days:** a discovery, a key building, a trade with a friend.
- **Weeks:** a new era, the era's monument, a shared project.

## The planet (built)

642 cells (12 pentagons, 630 hexagons): open sea and land of six biomes
(coast, plains, forest, mountain, desert, ice). Continents come from smooth
noise; the climate (temperate, arid, frozen, oceanic) sets the share of sea
and how the land splits between biomes. The home stands on the largest
continent with forest, hills and plains within two cells, and the village
grows outward from it. A planet year lasts one real week; the
seasons change food output. Planned: a low-poly sphere whose look changes with
the era.

## Cities (built)

The planet starts with one city on its home. From the Agricultural Age a
colony founds another on free land a few cells away; new buildings go up
near a city, so a growing people spreads into new cities. A road over land
(laid cell by cell, carts on it) or a sea route between two jetties links a
colony to the capital: until then its crews work at 60%, and every linked
colony brings a little more knowledge and culture.

## Society (built)

- Population starts at 8. 80% work. Up to nine crews, each a ninth of the
  workers, gather on the cells they are sent to (a new planet starts with
  three on food, two on wood and one on stone); the cell's stock runs down
  while it is worked and grows back at rest, and a crew on a spent cell
  moves on by itself. Mines keep a crew of their own. Everyone else thinks
  (knowledge) and makes culture.
- Needs: food, housing, culture. Mood follows them and drives output and
  growth; low mood means people leave or work less, never game over.
- Notable people are born now and then with a name, boost one category for
  three weeks and enter the Chronicle.
- Traits (peaceful, curious, traditional, open) grow from event choices and
  lean the society: two planets in the same era differ.

## Eras

Each era adds resources, buildings, things to do and **things to send**. An
era ends when its discoveries are known and its monument stands.

| # | Era | Travel | Sending (design; the dock sends gifts, ideas, visits, pranks, messages) | Status |
|---|---|---|---|---|
| 1 | Stone | seeds on the wind | seeds, a smoke signal | built |
| 2 | Agricultural | messenger birds | food, pottery, a message | built |
| 3 | Bronze | sky rafts | tools, tablets (an **idea**) | built |
| 4 | Classical | sky sailing ships | goods, visiting people, art | built |
| 5 | Medieval | sailing ships | envoys, relics, spies | built |
| 6 | Renaissance | balloons | books, maps | built |
| 7 | Industrial | airships | big cargo, machines | built |
| 8 | Electric | planes | radio messages, films | built |
| 9 | Neon | drones | data | built |
| 10 | Orbital | rockets | colonists, station modules | built |

- The first visit must already move: a discovery within minutes, the first
  buildings within the hour (settlers' zeal; `tests/first_session.rs`).
- Early eras take days, later ones weeks. Pacing target for a steady player:
  era 2 by day 1.5 to 5, era 3 by day 5 to 12, era 3 done by day 10 to 21
  (checked by `tests/balance.rs`).
- Travel gets faster and carries more with every era.
- Friends in different eras still help each other: the one ahead sends ideas,
  the one behind sends raw goods or "inspiration".
- From era 7: pollution and a choice between a fast and a green road to
  space.

## Discoveries (built for eras 1 to 3)

A tree per era, one discovery at a time, paid in banked knowledge. Events can
grant a discovery early (lightning teaches fire). Planned: social
discoveries unlocked by exchanges with other planets.

## Events (built)

One or two a day, each with two or three choices that move resources, mood,
population and traits. The text comes from the catalog, so it always reads
without the AI; Laura retells it. Unanswered for a day, the people take the
first, prudent choice themselves.

## Neighbours (built) and friends (planned)

Built: three simulated neighbours with a personality (generous, trader,
prankster) send gifts, ideas, visitors, harmless pranks, trade offers and
messages, and answer the player's ships; kinds unlock by era (gifts and
messages from the start, pranks and trades in era 2, ideas in era 3,
visitors in era 4).

Planned for friends:

The dock sends gifts, trades, visits, pranks (never destructive: stolen light,
a rained-out harvest, sky graffiti, goats eating a field), defences that send
a prank back with a taunt, and shared projects (a bridge of light, a sky
lighthouse, the orbital station). Friendship shows as lit routes in the sky.
Without friends, three or four simulated neighbour planets keep the sky alive.
Planets travel over Iroh peer to peer; a ship is a signed message with a
departure and an arrival time, delivered when the receiver is back online.

## Laura (planned)

A short spoken Chronicle at each opening, in a personality the player picks
(solemn historian, village gossip, grumpy grandpa, keen scientist); agent
tools ("how is my planet?", "send 20 food to Marco"); dictated notes rewritten
in the era's style. Always asynchronous: catalog text first, AI as colour.

## Sharing (planned)

An era timelapse video, a daily postcard, the archive of notable people,
invitation codes that make a friend's planet appear in the sky.

## Endgame (decided)

No forced reset. The orbital station closes the **First Orbit**. Then the
player keeps the planet, or founds a **colony** on a new moon that starts in
the stone age with a **relic** from the home planet (a permanent bonus); the
home planet stays in the sky. Target: 6 to 8 weeks to space with one or two
sessions a day.

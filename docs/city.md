# The city

`crates/ascitty-core/src/world.rs` and `arch.rs`.

## 1. It is a height field

Every cell carries one height, and a building is a run of cells that happen
to share a lot. Two things follow.

The renderer can walk it front to back in a single pass per column, and the
skyline falls out of it.

And **setbacks are free**: a tower that steps in as it rises is a lot whose
edge cells are shorter than its middle. That costs nothing at render time
and reads correctly from every angle.

Nothing is stored per floor. A sixty-storey tower is one lot record and a
handful of cell heights; its windows are a hash of (lot, face, floor, bay),
evaluated when a ray happens to land on one.

## 2. The street grid

Avenues run north–south and are wide; streets run east–west and are narrow.

```
AVE_PERIOD  14 cells        AVE_WIDTH  3
ST_PERIOD    9 cells        ST_WIDTH   2
```

That asymmetry is the single most Manhattan thing in the file: it gives long
sightlines one way and short ones the other, so turning ninety degrees
changes what the city looks like instead of just rotating it.

A cell is about six metres. An avenue is three cells — a real avenue, with
parking. A lot is two to five — a real frontage.

Blocks are found by scanning for maximal runs of cells that are neither road
nor sidewalk, rather than by stepping at the street period. That keeps the
generator correct when the two periods are changed independently, and it
guarantees forward progress on every iteration — which the arithmetic
version did not, and which hung the whole test suite until it was found.

## 3. Blocks, and what goes in them

One block in nine is left open as a park or a plaza, and it is likelier out
in the neighbourhoods than in the middle of downtown.

The rest are subdivided by recursive splitting of the longer axis until every
piece is small enough to be one address, and a building goes on each.

Height falls off from the middle of the map, and a big footprint can carry a
taller building than a narrow one — which is why the tall things cluster and
the gaps between them are filled with walk-ups.

## 4. The six archetypes

The eye reads this before it reads anything else. Two towers of the same
height and colour are still obviously different buildings if one is a glass
slab and the other is a brick walk-up with an iron staircase bolted to the
front of it.

| Archetype | Silhouette | Facade |
|---|---|---|
| `CurtainWall` | flat top | continuous glass grid, corners only |
| `Slab` | flat top | a pier every second bay, full height — the zipper |
| `Prewar` | flat top | punched windows, a spandrel course every fourth floor, a cornice, and a fire escape |
| `Setback` | loses a tier per ring in from the edge | glass, corners expressed |
| `Deco` | setback, plus corner piers that carry past the top | a pier every third bay |
| `LowRise` | two or three storeys | shopfront, and a fire escape |

`cell_height` is where the silhouette comes from, and it is worth reading:
the "ring" a cell sits in — its distance in from the lot edge — is the only
input. A slab ignores it. A setback loses a tier for each one. A Deco tower
does the same but keeps its corner piers, so the crown is a notch taller
than the shoulders.

## 5. The facade

Sampled, never stored. `arch::facade` is asked what is at an exact point on
a wall and answers from the archetype, the floor, the bay and a hash.

```
FLOORS_PER_UNIT  2     a floor is 3 m
BAYS_PER_UNIT    3     a bay is 2 m — a window and its pier
```

A sixty-unit tower is therefore 120 floors of 3 m: 360 m, an Empire State
Building.

In the order the sampler checks them:

- **The top two courses** — cornice, then parapet. Which one depends on the
  archetype; a brick building's is heavier than a glass one's.
- **The ground floor** — lit shopfront, warmer than anything above it,
  with a neon sign every eighth bay. Brighter, taller and more irregular than
  the floors above, which is what stops a street looking like a filing
  cabinet standing on end.
- **The fire escape**, if the building has one.
- **Piers** — every second bay on a slab, every third on a Deco tower.
- **Corners** — every building, whatever it is made of. The vertical line
  where two faces meet is the strongest cue that a tower is a box standing in
  space rather than a flat patch of light.
- **A spandrel course** every fourth floor on the prewar blocks, which is
  what makes a brick face read as brick rather than as a gradient.
- **An ordinary window.**

### The two rates

The last one is where most of the look comes from, and it takes two
decisions at deliberately different rates.

**Which tile** is a property of the *building*, from the lot's seed and
nothing else. One tower is drawn in `X`, its neighbour in `0`, a third in
`8`.

**Whether the light is on** is a property of the *window*, sampled at the
resolution the screen cell can actually resolve — the floor and bay indices
are quantised by the level of detail first.

Getting this the wrong way round was the first thing that looked wrong: with
the tile chosen per window, a skyline is one textured mass instead of a row
of buildings.

## 6. The fire escape

The most recognisable thing on a prewar building, and it is two glyph
families and a hash.

Which face carries it, and where along that face, is fixed per lot — it must
not move between frames or between viewing angles, and a test walks all four
faces at every level of detail to prove it does not. Within a floor: a
landing on the floor line, a rail at the outer edge of it, a zigzag between,
alternating direction each floor, and a counterweighted drop ladder at the
bottom. It stops below the cornice.

Only `Prewar` and `LowRise` carry one. A curtain wall has nowhere to bolt it.

## 7. Street markings

Worked out in **continuous coordinates across the carriageway**, not per
cell. A cell is six metres and a painted line is not, so asking "is this the
middle cell of the avenue" can only ever put the centre line down the middle
of a whole cell — and on a two-cell cross street the true centre is the
*boundary* between its two cells, where there is no cell to put it in. The
line ended up off to one side. Measuring across the road instead is correct
for any width, and it is the same arithmetic for both families of street.

| Marking | Where | Glyph |
|---|---|---|
| Double yellow centre line | the middle of the carriageway | `ROAD_CENTRE` |
| White dashed lane divider | a quarter and three quarters across, roads three cells and wider | `ROAD_DASH` |
| White edge line | an eighth of a cell from each kerb | `ROAD_DASH` |
| Crosswalk | just inside each of a junction's four edges | `ROAD_CROSSING` |

Three details are deliberate.

**The centre line is a sixth of a cell wide**, which is a metre — about three
times a real double yellow. It has to be: the ground is point-sampled once
per screen cell, and a line a twentieth of a cell across would fall between
samples past a few cells and flicker in and out as the camera moved.

**Lane dividers only appear on roads three cells and wider.** A two-cell
cross street is one lane each way; painting a divider inside each lane is
three lines across twelve metres of road.

**Junctions have no centre line and no dividers at all** — bare tarmac in the
middle, which is what a real junction has, and painting through one is the
quickest way to make a street grid look like a diagram of a street grid.
What a junction does have is a crosswalk on each of its four approaches, laid
just inside the edge of the box. The stripes run *with* the traffic and
repeat *across* it, which is the way round they are painted: a pedestrian
walking north over an avenue crosses a ladder of north–south bars.

## 8. The colours

Eight facade hues, deliberately narrow: a night city is mostly two or three
colours of glass with the odd lit brick face, and a wider palette reads as
confetti rather than as a place.

Shopfronts are warm — a lit shop at night is warm and a blue one looks like
an aquarium. Signs are saturated, because neon is.

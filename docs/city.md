# The city

Four layers over one grid, each its own structure:

| Layer | Module | Answers |
|---|---|---|
| Roads | `world::Plan` | where the streets are, how wide, what class |
| Mapping | `zone` | what this ground is *for* |
| Elevation | `elevation` | how high the ground is, and what stands on it |
| Walking | `walk` | where a person on foot may be, and how they get about |

They are separate because the questions are separate, and conflating them
produced real bugs: treating "not built on" as one walkable space put
pedestrians in the middle of the avenue, and deriving the district ring from
a cell distance rather than a block index made the zoning change half way
along a street.

## 0. The numbers

```text
cell             ~6 m
map              28 blocks square, 364 cells
inhabited world  26 blocks square       zone::WORLD_BLOCKS
built city       16 blocks square       zone::CITY_BLOCKS
downtown core     8 blocks square       zone::CORE_BLOCKS
suburb            3 rings               zone::SUBURB_RINGS
farmland          1 ring                zone::FARM_RINGS
last houses       1 ring                zone::OUTSKIRT_RINGS
coast            26 cells of the south  zone::SHORE_CELLS
block             at least 8 cells      zone::MIN_BLOCK
nominal pitch    13 cells               zone::BLOCK_PITCH
arterial         12-16 cells - 72-96 m, kerb to kerb
```

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

## 2. The street system

It used to be arithmetic: an avenue wherever `x % 14 < 3`, a cross street
wherever `y % 9 < 2`. That is one line of code, and it gives a city where
every block is the same size, every road is the same width and every
junction is the same junction. It reads as a diagram of a city.

So the plan is **generated** instead, once per city, as two independent axes.
Each is a list of roads with a class, a width and a gap after it:

| Class | Width | What it is |
|---|---:|---|
| `Alley` | 1 | service access — no pavement, no paint, but you can walk down it |
| `Street` | 2 | one lane each way |
| `Avenue` | 3 | |
| `Boulevard` | 4–5 | |
| `Arterial` | 12–16 | the better part of a hundred metres of carriageway |

There are one or two arterials in a city and they run its whole length.
They are placed *first*, before the rest of the axis is laid out, so they
land through the middle of the city rather than wherever the accumulated
sequence of gaps happened to arrive.

The grid is still a grid — this is Manhattan, not Boston — but no two blocks
are the same size and the roads have a **hierarchy**, which is what a street
system actually is.

### The two axes have different characters

North–south carries the big roads: mostly avenues, a boulevard every so
often, and long gaps after them. East–west is mostly streets with short
gaps. That asymmetry is the most Manhattan thing in the file — it gives long
sightlines one way and short ones the other, so turning ninety degrees
changes what the city looks like instead of just rotating it.

### The gap depends on what came before it

A bigger road gets a bigger block after it, which is what makes the
hierarchy visible from the ground: you can tell you have come out onto a
boulevard because you can see a long way in both directions and the next
crossing is a long way off. An alley gets a short gap, so it reads as a
service road splitting one block rather than as a thin street.

A cell is about six metres. So a boulevard is 24–30 m of carriageway, a lot
is 18–30 m of frontage, and the blocks run 40–160 m.

### What reads the plan

Everything, and that is the point of storing a width and an offset-from-kerb
per cell rather than recomputing them.

`City::generate` uses it to place road, pavement and buildable ground. The
renderer reads it directly for [markings](#7-street-markings), so widening a
boulevard moves its centre line with it and there is no second definition of
where the middle of a road is. The autopilot asks it whether it is standing
in a junction.

### Blocks

A block is a maximal run of buildable cells in both directions, found by
scanning rather than by stepping at a fixed period — which is what lets the
roads be irregular in the first place.

**No block is smaller than eight cells.** Every gap the generator produces is
clamped to `MIN_BLOCK + 2` — the block itself plus a cell of pavement on each
side. Clamped rather than merely chosen from a suitable range, so the
guarantee survives the figures being retuned; below eight there is no room
for a building with a front and a back, and a subdivision that tried would
produce lots one cell deep.

Each block is reached exactly once, at its **top row**: a run is only filled
when the cell above its left end is not buildable. Without that test a block
is refilled once per row it spans, and the symptom is subtle — buildings
quietly reroll their height and colour, and plazas turn into towers on the
second pass.

## 2a. Zoning, and the rings

`zone.rs`. Three questions that keep getting confused, kept apart:

- **Zone** — what this ground is for. A property of the *place*: downtown,
  commercial, residential, civic, park, outskirts.
- **Use** — what a particular building is. An office or a home. Follows from
  the zone but not slavishly: there are flats downtown and shops in the
  suburbs.
- **Archetype** — how it is built. Curtain wall, brick walk-up, setback
  tower. Follows from the use *and then* the height.

Keeping them apart is what stops the generator collapsing into "tall things
are blue glass and short things are brown brick". A twelve-storey residential
slab and a twelve-storey office slab are different buildings and look it.

The city is laid out as rings of blocks measured from the middle:

```text
   ring 0-3     the downtown core - towers, the arterials, full intensity
   ring 4-7     the rest of the city, intensity falling to a fifth
   ring 8-10    suburb - one-storey houses on wide plots, gardens between
   ring 11      farmland - fields, with a farmhouse in about half of them
   ring 12      the last houses, and no road runs through them
   ring 13+     nothing
```

Measured in **blocks**, not cells. Dividing a cell distance by the block
pitch looks equivalent and is not: the middle of the map is rarely on a
block boundary, so the ring steps somewhere in the middle of a block and the
zone changes half way along a street.

The ring is the "decreasing height outwards" the whole layout is arranged
around. It multiplies with the zone's own ceiling and with the lot's
footprint, so an office tower on a big downtown lot may be ninety cells and
the same use on a small lot at the edge may be six.

### Why there is a world outside the city

Five rings of it, and they exist to answer a question the map edge asks and
nothing used to answer: *why can I not drive that way?*

A city that stops at a kerb with nothing beyond it reads as a diorama on a
table. So it thins instead — three rings of single-storey houses on plots
wide enough to see between, then a ring of fields with the occasional
farmhouse standing in one, then one last row of houses that no road runs
through. By the time you have driven far enough to find the edge, the answer
to "why not further" is "there is nothing out there", which is the true
answer and does not need a wall to make it.

The rings are zoned by ring rather than by dice. Inside the city a park is a
roll of the die per block, because a city is a mixture; outside it the whole
point is that the bands are legible from a long way off, and a scatter of
farmland through the suburbs would read as gaps rather than as countryside.

**The last ring has no roads in it.** The street plan is two one-dimensional
axes laid over the whole map, so it cannot express "no roads here" — every
road runs edge to edge by construction. They are painted out afterwards
instead, and the block filler never notices, because it walks the *plan*
looking for buildable runs: the houses still get built and the ground
between them is green rather than tarmac. What that costs is a flood fill:
cutting roads leaves stubs — a street whose only way back to the rest of the
map went through ground that is now a field — and a stub is worse than no
road, because it is somewhere the traffic can be generated and never leave,
and somewhere a fare can be placed that no cab can reach. So the carriageway
is flooded from the middle and anything the flood does not reach stops being
road. Seven components, in the first city this was measured on.

**And the draw distance went up by a block**, at every haze setting, for the
same reason the rings are there: from the fields the towers have to be
*visible*, so that the way back to the middle is something you can see
rather than something you have to remember.

### The south is a coast

The last twenty-six cells of the south edge are beach and then sea. One
coast rather than four: an island reads as a model of a city and a coastline
reads as a city that has a sea on one side of it, which is what most of them
have. It is always the south, so "drive towards the water" is a direction
you can learn.

The sea is levelled to the datum rather than following the terrain — it is
not ground that has been built on, it is the thing ground height is measured
from — and it is painted after the buildings, because the plan lays roads
across the whole map and a block that straddles the tide line gets built
before the coast is drawn. Whatever is on that ground when the coast pass
arrives, it is beach now.

Sand is walkable and not drivable; water is neither.

## 2b. Elevation

`elevation.rs`. Two byte arrays over the grid — ground level and what stands
on it — kept together because they are the two the inner loop touches most.

Ground is stored in **thirty-seconds of a cell**. A whole-unit ground level
would step the streets in six-metre cliffs. An eighth - 75 cm - was the unit
until the pavement had to stand above the road beside it: a kerb is about
18 cm, which is a thirty-second, and a unit that cannot express a kerb cannot
draw one.

The kerb built is two steps rather than one. One step is also the steepest
gradient the terrain generator produces, so a one-step kerb survives only
where the ground happens not to fall the other way, and a kerb present on
part of a street and absent from the rest reads worse than a high one.

The terrain is deliberately almost flat: two units of relief across the whole
map, and slow. Not because hills would be hard to generate but because of
what the renderer does with them. The floor pass works out how far away a row
of ground is from the camera's height *above its own footing*, then samples
whatever cell that lands on. Over a gentle grade the error is far less than a
character cell. Over a hill it would not be, and the floor would visibly
swim. So this is a city on a river plain with a rise in it, which is most
cities, and it says so rather than pretending to be San Francisco.

A building does not follow the contour: `Elevation::level` cuts its whole
footprint to one pad before anything stands on it. Without that, a lot
spanning a grade has its corners at different heights, and the roofline —
which is one number per lot — ends up a different distance above the ground
on each side.

## 2c. The walking system

`walk.rs`. The driving network and the walking network are not the same
network and never were. A car belongs on the carriageway and nowhere else; a
person belongs on the pavement, in the parks and plazas, down the alleys, and
on the carriageway only where it is painted for them to cross.

| `Foot` | Where |
|---|---|
| `Path` | pavement, park, plaza, alley |
| `Crossing` | carriageway where a crossing is painted |
| `Blocked` | buildings, and the open road |

**Crossings sit outside the junction box**, one cell back along the road, at
the stop line. Putting them inside — which is the obvious thing, and was
wrong here for a while — produces a crossing that no pavement touches: every
orthogonal neighbour of a junction cell is either more junction or plain
carriageway. The symptom was that each block's pavement was an isolated ring
of about forty cells and the network had no connected component larger than
one block. They come from `Plan::crossing_at`, which is also what the
renderer paints the zebra bars from, so the two cannot drift apart.

Two ways to ask for a route, and the distinction matters:

- `step_toward` is a greedy step: prefer the long axis, accept a crossing,
  turn along the kerb when blocked. Four lookups, no memory, and **explicitly
  not guaranteed to arrive** — a U-shaped dead end holds it forever. That is
  the right trade for a crowd of pedestrians who have somewhere to be and no
  opinion about the shortest way there.
- `route` is a breadth-first search. Shortest path, costs a visited set the
  size of the map, and is what the tests use to assert the network is
  actually connected.

## 3. Blocks, and what goes in them

One block in nine is left open as a park or a plaza, and it is likelier out
in the neighbourhoods than in the middle of downtown.

The rest are subdivided by recursive splitting of the longer axis until every
piece is small enough to be one address, and a building goes on each.

### How tall, and what colour

Two fields decide what a block can carry, and they are added together
because one on its own is not a city.

A single falloff from the centre of the map gives a perfectly conical
skyline: tallest in the middle, monotonically shorter in every direction.
Real cities have a downtown *and* secondary clusters — a second business
district, a tall patch around a station — with quiet ground between. So the
falloff is two thirds of it and the rest is a smooth value-noise field,
which puts the clusters somewhere different in every city. Both are
integer-only: the generator has to be transcribable to a machine with no
floating point.

Height is then drawn from a **skewed** distribution, not a uniform one.
Uniform gives a city where every height is equally common, and the eye reads
that as noise — there is no general roofline for anything to stand above.
Real heights are closer to a power law, so four bands approximate it:

| Band | Share | What |
|---|---:|---|
| 2 – ceiling/5 | 52% | the fabric: walk-ups and low commercial |
| ceiling/5 – ceiling/2 | 32% | mid-rise, the bulk of a real skyline |
| ceiling/2 – ceiling | 14.5% | towers |
| ceiling – ceiling×1.5 | 1.5% | a landmark, gated on a footprint that could carry it |

Measured on a generated district: median 9, ninetieth percentile 26,
ninety-ninth 42, tallest 50.

Colour is a **district palette**, not a per-building roll. Each district
draws from a small set of hues that belong together — glass towers blue and
cyan, prewar blocks brick and ochre, a strip of neon — because a flat list
of sixteen hues picked at random per building reads as confetti whatever the
individual colours are. The palette comes from the same noise lattice as the
height, so it drifts across the map instead of changing at every kerb.

On top of the hue, each building gets its own **brightness** and its own
**occupancy** — how many of its windows are lit. Hue alone is not enough
variety: a street of buildings differing only in colour reads as a colour
chart, and what tells two real buildings apart at a glance is as often how
bright one is. The occupancy spread is deliberately wide, because the towers
that are nearly dark are what make the ones that are nearly full look full.

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

## 7a. The pavement, in bands

A pavement is drawn in four bands from the kerb to the building line, in
continuous coordinates like the markings, because a pavement seen in
perspective is mostly its edges.

| Band | Width from the kerb | What |
|---|---|---|
| Kerb | a sixth of a cell, 1 m | `ROAD_KERB`, white, and the brightest thing on the ground |
| Verge | to half a cell, 3 m | grass and hedge; where the street trees are planted |
| Paving | to the building line | cement, stained cell by cell from a hash |
| Seam | an eighth of a cell at the wall | a dark line, which is what stops a wall appearing to float |

Putting the trees in the verge rather than in the paving is the difference
between a street with trees on it and a street with obstacles on it.

The brightness of the first three is not arbitrary and was raised once,
because the street read as ending at the kerb: cement at night is the thing
under the street lights, and the band people are on should be the band you
can see. Near the camera, and with the moon up:

| | was | is |
|---|---|---|
| Kerb | 200,200,200 | 255,255,255 |
| Paving | 126,126,126 | 156,156,156 |
| Grass | 77,107,77 | 162,226,162 |

The grass is `H_GREEN` rather than `H_LIGHT_GREEN`, which is the wrong way
round until you look at what the two do at the top of their ramps. This
palette scales chroma with luminance, so green at six is a pale, nearly
white green and light green at six is olive. The bright green is the greener
of the two; the light-green tufts are what keeps a verge from being one flat
colour.

## 7b. The sky, and the day

The sky is not black any more. It is a gradient — palest at the horizon,
where the light is, darkening towards the zenith — and it turns through
twelve phases on a cycle: night, morning, awakening, sunrise, dust, noon,
afternoon, overcast, sunset, afterglow, gloaming, deep night, and round
again. `DAY` in `atmos.rs` is the table, and no two adjacent phases share a
hue, so the sky always visibly moves.

Three things about it are worth writing down.

**The gradient is the right way up.** A hue at the top of this palette's
ramp is a washed, almost white version of itself, and that is what the
bottom of a sky looks like; the zenith is the darker end. Getting it the
other way round produces a ceiling.

**A phase change rises rather than cross-fading.** Two hues cannot be mixed
in a palette that gives a cell one colour, and dithering them together costs
a colour escape per cell across half the frame — the sky is half the frame.
So the new colour climbs out of the horizon over the first half of a phase
and holds for the second, and every row stays one colour, which is one
escape and a row of characters. The boundary carries a row of noise so it is
a weather front and not a ruled line.

**Brightness is coverage as well as luminance.** The sky is drawn with the
haze and dither families, densest at the horizon, which is what lets it have
a gradient in ASCII with no colour at all — a blank sky at night grading up
through `. : - = +` at noon. It stops short of solid: in ASCII a solid fill
is `@`, which reads as a building rather than as air.

The city is lit by the sky it is under, at up to two luminance steps of
ambient — `Atmos::daylight`, read off the phase's own brightness so the two
cannot disagree. It is ambient rather than directional because a directional
daylight would want a second shadow sweep, which is a real thing to want and
is in the backlog.

## 8. The colours

Eight facade hues, deliberately narrow: a night city is mostly two or three
colours of glass with the odd lit brick face, and a wider palette reads as
confetti rather than as a place.

Shopfronts are warm — a lit shop at night is warm and a blue one looks like
an aquarium. Signs are saturated, because neon is.

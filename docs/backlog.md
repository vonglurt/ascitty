# Backlog

Ordered, with the reasoning kept. An item leaves this list when it is built
or when it is decided against — and if it is decided against, the entry stays
with the reason, because "we tried that" is worth more than a shorter list.

Status: **now** (next up) · **soon** · **later** · **maybe** · **dropped**

---

## Lighting

The formulas, the costings and the reasoning are in
[`raytracing.md`](raytracing.md). The items below are what falls out of it.

### ~~A directional light and a five-entry Lambert table~~ — **built**
`Atmos::lambert()` on the host, `lambert[4]` on the Plus/4. A height field
presents five normals and the renderer already knows which one it hit, so
`L·N` for a directional source is five numbers recomputed once per frame.
Measured cost: none, on either target. See `raytracing.md` §2.1.

Left here rather than deleted because the *shape* of the argument is the
reusable part: whenever a quantity is constant over a surface the renderer
can already identify, it belongs in a per-frame table and not in the inner
loop.

### ~~Directional shadows from a horizon sweep~~ — **built**
`shadow::ShadowMap`. One O(cells) sweep per light direction, O(1) per lookup,
no shadow rays and no bias term. It stores the shadow line as a *height*, so
a wall is dark at the bottom and lit above — a bit could not express a tower
standing in the shade of a nearer tower.

Baked into `city_s[]` for the Plus/4, which therefore sweeps nothing: the
line is a pure function of the height field and the light, and both are known
at bake time. Measured free on both targets. See `raytracing.md` §2.3.

### Soft shadows from the same sweep — **now**
The stored value is already a height, so how far a surface sits below the
line is known. Grading the luminance offset by that distance rather than
switching it gives the umbra/penumbra distinction that normally wants an area
light and many shadow rays. It is a change to one comparison and it is the
cheapest remaining item on this list.

### A light that moves — **soon**
`City::relight` exists and recasting is one sweep, so a day/night cycle or a
moon that tracks is a matter of calling it when the bearing changes by enough
to matter. The thing to get right is *when*: a sweep is ~2 ms, which is
nothing occasionally and eleven frames' worth if it happens every frame.

### Wet-road reflections as a vertical mirror — **soon**
The ground is a horizontal plane, so a reflected ray is the incident one with
its vertical component negated and its horizontal component unchanged. On
screen, for a camera with no roll, that is exactly:

> what is drawn at row `horizon + k` reflects what is at row `horizon − k`,
> in the same column.

A second read of a cell the renderer has already computed. No recursion, no
depth limit, no second trace. Fade it with distance and mask it with the
puddles that already exist. Probably the change that most looks like ray
tracing to somebody watching.

### Sub-cell gradient shading — **soon**
A character cell is one colour but **eight by eight sub-cells of shape**, and
the font is generated rather than drawn, so all of it is reachable.

Ordered dithering already uses this for magnitude - `catalog::shade(n)` picks
a glyph covering exactly `n/8` of the cell. The extension is to use it for
*direction*: the `half_plane` family gives sixteen edge orientations, and
`font::moments` already computes where a glyph's mass sits. Pick the glyph
whose centre of mass lies towards the brighter side and a wall lit from the
left gets glyphs weighted left.

This is the ASCII-native answer to Gouraud shading - the colour stays flat
because it must, and the *shape* carries the interpolation. It is the most
interesting unexplored idea in the renderer.

### Quadratic attenuation as a table — **later**
`Atmos::fade` is linear in distance on purpose: an eight-level ramp under
inverse-square spends six levels in the first three cells. The softened form
`1/(a + bd + cd²)` is worth *trying* as an 80-byte table indexed by whole
cells - not as a correction, as a different look.

### Specular via a halfway-vector table — **later**
`H = (L+V)/|L+V|` is a normalise, which is a square root, which is the thing
this renderer is built around not doing. But `N` is one of five and `V` is
constant per column, so `N·H` is a `width × 5` table rebuilt when the camera
turns - 800 bytes on a host, 200 on the Plus/4.

Worth it for wet asphalt and glass. Not worth it for brick: at one colour per
character cell a matte wall has no highlight to draw.

### Point lights with real attenuation — **maybe**
Street lamps and lit shopfronts as actual sources rather than as bright
glyphs. `L` is no longer constant so `L·N` is per hit per light, and the
five-normal trick still removes the normalise but not the direction
subtraction. Affordable on a host for a handful of lights, out of the
question on the Plus/4.

### Sphere primitives — **maybe**
The one place the quadratic

```
    (D·D)t² + 2(D·(O−C))t + ((O−C)·(O−C) − R²) = 0
```

would earn its discriminant and its square root: a moon that is a real
sphere, lamp glow as a shaded ball. Everything curved in the city is
currently a billboard, which is right for cost and slightly wrong for the
moon at the horizon.

### Naming

### A `no_std` core — **later**
`ascitty-core` is written so its *arithmetic* transcribes to the 6502, not so
that the crate compiles for one: frames and per-frame row tables are `Vec`.
Caller-provided buffers and `no_std` would be a step towards a shared source
rather than a transcription, and would make the claim in
[`lab-report.md`](lab-report.md) §VIII-C unnecessary.

### Audit the remaining identifiers — **soon**
Section IV of the lab report found three identifiers asserting something
other than what they measure, two of which preceded defects. The audit was
not exhaustive. Candidates not yet examined: `Stamp` (a coinage, not from the
specification), `Prop` (theatre jargon where the domain term is *street
furniture*), and the split between `Tour` on the host and `cast_demo` on the
target for one concept.

## Distribution ray tracing — **dropped**
Anti-aliasing, soft shadows, depth of field, glossy reflection and motion
blur from one mechanism: many jittered samples per pixel. Its slogan is that
it turns aliasing into *noise*.

That trade is backwards here, twice. On cost, sixteen samples on a machine
managing 1.3 frames a second is a different project. On the medium, noise is
worse than aliasing in a character grid, because a cell is coarse enough to
be legible: a pixel renderer's noise dissolves into texture, a character
renderer's noise is a visibly wrong letter, and re-sampled each frame it
crawls. It is the same argument that makes the dither ordered rather than
diffused.

The one part worth having is soft shadows, and the horizon sweep above gives
those without sampling anything.

## Renderer

### Roof surfaces, properly — **now**
Looking down from the copter, a roof is capped with a single row of
roofscape glyphs and the rest of the cell shows facade. It reads acceptably
at a glance and wrongly if you stop and look. The fix is a horizontal
surface pass in the same walk: when `cam.z > cell height`, cast the roof
plane the way the ground plane is already cast. The row-distance table
already exists; it needs a second one indexed from the roof height rather
than from zero.

### Ambient occlusion at the street canyon — **soon**
Real streets are darker at the bottom because the buildings shade them.
One extra luminance step for ground cells with tall buildings on both sides
would do most of the work, and the height field already knows.

### Sub-cell rooflines — **soon**
`G_FILL` exists — eight partial fills from the bottom up — and the renderer
does not use it. A tower top that falls between two character rows should
pick the eighth-step nearest its true height so the skyline steps in eighths
rather than in whole cells. This is the single largest available improvement
in how the picture reads at distance, and it is about ten lines.

### Terminal-font glyph matching — **later**
The ASCII and Unicode tables in `glyph.rs` are hand-authored. The bitmaps
carry a coverage-and-moments signature (`font::moments`) that was built to
support automatic matching, and nothing uses it yet. Doing it properly needs
the terminal's actual font, which is not knowable at build time — so this is
an offline tool that takes a font file and emits a table, not a runtime
feature.

### PETSCII fallback mode — **later**
A third terminal mode that draws using the *unmodified* Commodore character
ROM's screen codes, for running against a real machine's stock charset or a
PETSCII-faithful terminal. Deferred rather than guessed: shipping a table of
ROM screen codes that is subtly wrong is worse than not shipping one, and
verifying it needs the ROM in front of you.

---

## The city

### A kerb of the right height — **soon**
The pavement stands two elevation steps above the carriageway, which is
37 cm. A kerb is about 18 cm - one step. One step is also the steepest
gradient the terrain generator produces, so a one-step kerb disappears
wherever the ground falls the other way across the same boundary, and a kerb
that is present on part of a street and absent from the rest looks worse than
one that is uniformly too high. The fix is to level each carriageway and its
two pavements to a common footing first, then raise the pavements: the kerb
becomes a difference by construction rather than a hope about the terrain.
The same pass would give crossings a dropped kerb.

Neither the kerb nor the verge reaches the Plus/4. The bake carries building
heights only; the target has no terrain array to raise.

### Diagonal streets — **later**
The plan is two independent axes, so every road is north–south or east–west.
A Broadway cutting across the grid would be the single most characterful
thing that could be added to it, and it is also the one thing the current
representation cannot express at all: a diagonal is not a column or a row.
It would want a third structure — a list of line segments rasterised over
the grid after the two axes are laid — plus a marking path that can follow
it, and blocks that come out as triangles.

### The cabbie's lane keeping — **done**
Now 83, 79, 77 and 81 per cent of travelling ticks on the correct side of the
crown across the four test cities, up from 68, 55, 79 and 65 - and from 52,
63, 55 and 25 before that.

The last of it was not in the lane target, which is where every previous
attempt went. It was in the *fallback*: when there is no single lane line to
hold - inside a junction, and the crossing of two arterials is fourteen cells
of junction - the controller steered at the marker, twenty cells away on the
far side of a block. Aiming a few cells up the planned route instead
(`Cabbie::aim`) fixed both measurements at once and took one city from one
completed fare in five minutes to eight.

Three lane targets were tried along the way - the middle of the right-hand
half, the kerbside lane, and one cell past the crown - and the last is what
is in `road::lane` now. Scaling the gains by the width of the carriageway is
still the obvious next thing if this needs to go further.

Ruled out along the way: the sign conventions in `road::lane`,
`Cabbie::track` and the measurement agree when checked by hand against all
four combinations of axis and direction; `across` measures from the
low-coordinate kerb on every road in every city, which is now asserted; and
the controller was reading the road at its route's cursor rather than the
road under the car, which was a real bug and fixing it did not fix this.

### The cabbie on the pavement — **maybe**
Was about 40 per cent of travelling ticks with the cab's centre on a cell
that is not carriageway; now 2, 0, 0 and 2. The cause was not the controller
either: cornering radius now grows with the square of the speed, the cab was
still cruising at a speed that used to corner and no longer does, and it
understeered onto the pavement on the far side of every junction. Dropping
the cruising speed to what the radius allows removed almost all of it.

The underlying gap is still there and is what would need doing if this comes
back: the physics has no notion of a vehicle footprint, so nothing stops a
car from ending up anywhere its centre can reach. Giving `integrate` the hull
and resolving against it would close that and the wall-wedging in
`Cabbie::unstick` at the same time.

What has been added in the meantime is a second stuck check. Wedged is not
always *stopped*: a car that has climbed a kerb and is grinding along a shop
front at a cell a second passes every speed test there is, and was measured
doing it for 1,000 ticks of one run. A second off the carriageway now backs
the car out the way a stall does.

### Traffic that follows the road — **mostly done**
Traffic keeps the right-hand lane, is put down facing the way that side of
the road goes, eases off for whatever is ahead in its own corridor, gives way
to anything crossing from its right, and collides with itself as well as with
you. Measured: 98 to 100 per cent of car-ticks on the correct side of the
crown, and deep overlaps between cars down from 366 car-ticks to 33 over the
same 1,800.

What is left is *routing*: a car still goes wherever the street it is on
goes, so it never turns a corner - it is recycled when it falls behind
instead. Turning at junctions, and stopping at the signals that are already
modelled as street furniture, is the rest of this. Neither should be allowed
to make traffic hard to hit, which is the reason the earlier version was
scenery with momentum on purpose.

### A gear indicator, and a reason to have gears — **maybe**
The engine now has a torque curve, which is the half of a gearbox that
changes how the car drives. The other half - a shift, a moment of no drive,
and a curve that starts again - would be audible on a machine with sound and
is invisible on one without. What it would buy here is a number on the status
line and a reason for the top of the range to feel earned. Not obviously
worth it; written down because the curve makes it possible for the first
time.

### Keys the terminal cannot send — **later**
Two keys at once now works on terminals that speak the progressive keyboard
protocol, and is approximated with a half-second grace on the rest. What no
terminal will report is a *bare modifier*: Shift on its own sends nothing at
all, which is why `z` descends rather than Shift. Nor is there any analogue
axis - a key is down or it is not - so the ramp in `Axis` is standing in for
a pedal's travel. A gamepad over a socket would give both, and would be a
dependency and a daemon for a program whose whole point is that it runs in a
terminal.

### Distance should fade to the sky, not to black — **soon**
The depth cue subtracts luminance until a surface is black, which was right
when the sky was black too. It is not right under a yellow noon: a tower
fades to black and then the bright sky appears behind it, which is a hard
edge where aerial perspective should be a soft one. What it should fade
*towards* is the sky colour at that row. On the host that is a lerp per cell
in a palette that does not lerp; the honest version is to pick the nearer of
the two hues and cross the boundary as a dither, which is what the sky's own
phase change does. On the Plus/4 it is a second table.

### Daylight with a direction — **later**
The sky lights the city ambiently: two steps at noon, none at night. A sun
that moved with the phase would want its own shadow sweep - the sweep is
`O(cells)` once per light per bearing, so it is affordable - and would give
long shadows at sunrise and short ones at noon, which is most of what a day
cycle is for. What stops it today is that the moon owns the bearing and the
shadow map, and there is exactly one of each.

### Pedestrians that react — **soon**
They walk in a straight line and ignore everything, including buildings and
the taxi. Scattering when a car comes at them is a few lines and a large
amount of life.

### Interiors at street level — **later**
Ground-floor glazing that shows a lit interior a cell deep, rather than a
brighter facade tile. The renderer would need to keep walking past the wall
for one cell, which the front-to-back walk can already express.

### Bridges, elevated track, an expressway — **maybe**
The height field cannot represent anything you can pass *under*, which rules
all of this out without a second layer. A second height field for "floor
height" would do it and would roughly double the walk's inner loop. Worth it
only if there is something to drive on up there.

---

## Driving and the fare

### Scoring and grades — **now**
Combo counting and money exist; there is no grade at the end of a shift, no
tally screen, and no reward for a long drift. The `Car::slip` value is
computed every tick and nothing reads it.

### Air time and jumps — **soon**
There is no z axis on the car at all: the streets are flat and it never
leaves them. Kerbs and plaza steps are the obvious source of a hop.

### Passenger reactions — **soon**
Nothing about the passenger is expressed except a marker and a payout.

### Damage that shows on your own car — **soon**
`Car::damage` accumulates on the taxi and is only ever drawn on *other*
cars, because you are inside yours. A dented chase-camera sprite, or a
cracked-screen overlay, would close the loop.

### Rival taxis — **maybe**
Another cab racing you to the same fare. Fun, and a large amount of AI.

---

## Atmosphere

### Time of day — **soon**
Everything is night. The palette, the window-lit fraction and the moon are
all already parameters; a day/dusk/night cycle is mostly a matter of driving
them from one clock.

### Lightning — **soon**
A frame or two at full luminance across the whole palette, keyed off the
rain intensity. Cheap, and enormous.

### Wet-road reflections — **later**
Puddles brighten but do not reflect. Mirroring the column's own wall sample
downward from the horizon would give a real reflection for the cost of one
extra lookup per wet cell.

### Snow, fog banks, wind — **maybe**
Snow is the rain glyphs with a different fall vector and a slower scroll.
Fog banks need a per-cell density and a reason to exist.

---

## The Plus/4 build

### An assembly inner loop — **now**
The measured figure is about **1.3 frames a second**: fifty frames complete
between 75 and 80 million cycles with under 15 million of that being boot.
The C is already written to avoid what cc65 is bad at — the district is 64
wide so a grid index is a shift, the divisions are tables, the trigonometry
is hoisted out of the column loop — and the remaining cost is in code cc65
generates for the DDA and the wall fill. A hand-written `cast.s` for those
two loops is the obvious next factor of three to five, and it is exactly the
move this codebase's sibling made for the same reason.

### Double buffering — **now**
A frame takes longer than the raster, so the display always catches the
screen part-way through. Drawing each column completely before starting the
next turned that from a blank half-screen into a wipe, which is a large
improvement and not a fix. Two screen matrices and a `$FF14` flip would fix
it properly; the Plus/4 has the RAM.

### Floor casting — **soon**
The ground is drawn as bands shaded by distance. The host casts it properly
and gets lane markings, crosswalks, kerbs and puddles for it. On the target
it was dropped because it roughly halved the frame rate. Worth revisiting
after the assembly loop.

### Cars in the baked district — **later**
The vehicles are now boxes rather than cards - the silhouette of a rectangle
seen at an angle, so a car crossing the view stretches and shortens - and
none of that reaches the Plus/4, which has no sprites at all. The silhouette
itself is two multiplies and would transcribe; the blocker is the same one as
for every other sprite.

### Sprites — **soon**
No street furniture, traffic or people on the Plus/4 at all. The billboard
projection is simple integer arithmetic and the depth buffer is already one
byte per column; the blocker is that there is no frame rate to spend.

### Driving — **later**
The physics is fixed point already and would transcribe. It needs the frame
rate first: a car at 1.3 fps is not a car.

### Streaming a larger city — **later**
A 64x64 district is baked; the host generates 128x128. Streaming from disk
would give the whole map, and the `.d64` has room. The generator is
deterministic from a seed, so an alternative is to *generate* on the machine
rather than bake — much slower, and much smaller.

### C64 and VIC-20 targets — **maybe**
The C64 is nearly free: same 6502, same cc65, a 2 KB character set instead
of 1 KB, and a colour model with 16 fixed colours rather than 16x8. The
depth cue would have to change from "drop the luminance" to a hand-picked
ramp per hue, which is the interesting part.

---

## Naming

### A `no_std` core — **later**
`ascitty-core` is written so its *arithmetic* transcribes to the 6502, not so
that the crate compiles for one: frames and per-frame row tables are `Vec`.
Caller-provided buffers and `no_std` would be a step towards a shared source
rather than a transcription, and would make the claim in
[`lab-report.md`](lab-report.md) §VIII-C unnecessary.

### Audit the remaining identifiers — **soon**
Section IV of the lab report found three identifiers asserting something
other than what they measure, two of which preceded defects. The audit was
not exhaustive. Candidates not yet examined: `Stamp` (a coinage, not from the
specification), `Prop` (theatre jargon where the domain term is *street
furniture*), and the split between `Tour` on the host and `cast_demo` on the
target for one concept.

## Distribution

### Checksums and a release manifest — **now**
A build manifest recording the compiler, the exact command line and every
source file's git blob SHA, plus sidecar `.sha256` and `.md5` files for the
published artifacts.

### A browser build — **soon**
Two routes and they are very different. Compile the Rust core to WebAssembly
and render to a canvas — fast, faithful, and a new front end to maintain. Or
put an emulated Plus/4 in the page running the `.prg` — slow, and *exactly*
the artifact. The second is more interesting and cheaper.

### Golden-frame conformance — **soon**
Nothing currently checks that the host and the target agree. They cannot
agree exactly — different resolutions, different fixed-point widths — but
they can be made to agree on the *city*: same seed, same heights, same lot
colours. A test that bakes the district and compares it against what the
core generates would catch the whole class of "the two halves drifted".

---

## Dropped

### Floating point in the core
Considered and rejected before the first commit. See
[`adr/0001-fixed-point.md`](adr/0001-fixed-point.md). The host could afford
it; the point is that a renderer written twice in two number systems is a
renderer whose halves cannot be diffed.

### Rust on the 6502
`rustc` can target MOS 6502 through the llvm-mos fork, and it is genuinely
impressive work. It is not in Homebrew core, it needs a nightly toolchain
and a custom target spec, and `core` on a machine with a 256-byte stack is a
fight. cc65 is in `brew`, has shipped 6502 code for twenty years, and is
what the sibling project in the next directory already uses. See
[`adr/0002-rust-and-cc65.md`](adr/0002-rust-and-cc65.md). Revisit if
llvm-mos lands in Homebrew.

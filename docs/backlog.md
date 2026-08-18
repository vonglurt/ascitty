# Backlog

Ordered, with the reasoning kept. An item leaves this list when it is built
or when it is decided against — and if it is decided against, the entry stays
with the reason, because "we tried that" is worth more than a shorter list.

Status: **now** (next up) · **soon** · **later** · **maybe** · **dropped**

---

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

### Diagonal streets — **later**
The plan is two independent axes, so every road is north–south or east–west.
A Broadway cutting across the grid would be the single most characterful
thing that could be added to it, and it is also the one thing the current
representation cannot express at all: a diagonal is not a column or a row.
It would want a third structure — a list of line segments rasterised over
the grid after the two axes are laid — plus a marking path that can follow
it, and blocks that come out as triangles.

### Traffic that follows the road — **soon**
Traffic currently drives in a straight line at a constant throttle and is
recycled when it falls behind. It is scenery with momentum. Giving it lane
following and a stop at signals would make the streets read as busy rather
than as populated. Deliberately *not* a priority: the current behaviour makes
traffic easy to hit, and hitting it is the point.

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

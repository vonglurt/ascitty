# ASCITTY: A Height-Field Character-Cell Renderer for Modern Terminals and the Commodore Plus/4

**Paul Richeson and Claude**

Released under the MIT License. © 2026 Paul Richeson and Claude.
Source and version history: https://github.com/vonglurt/ascitty

---

## Abstract

We describe ASCITTY, a real-time three-dimensional city renderer whose output
alphabet is character cells, targeting a colour terminal and an unmodified
Commodore Plus/4 (MOS 7501, 1.76 MHz PAL, 64 KB). The world is an
axis-aligned height field traversed by a per-column digital differential
analyser that continues past the first intersection, carrying one integer of
occlusion state. We report three consequences of that geometric restriction:
the ray parameterisation removes the fisheye correction and every square root
from the render path; the surface normal set has cardinality five, reducing a
directional diffuse term to a five-entry table recomputed once per frame; and
cast shadows reduce to one O(n) horizon sweep per light direction, removing
secondary rays and their bias term. A Rust generator emits the character set,
trigonometric and reciprocal tables, a district of the city and its
precomputed shadow map as C headers, so the 8-bit target holds no independent
copy of any derived quantity. Measured: 0.16 ms per frame at 160×48 on the
host; 687,500 cycles per frame (2.58 fps) at 40×25 on the target, 2.8× faster
than the initial target implementation. We report four defects that survived
visual inspection, one performance figure that was an artefact of an
incorrect optimisation, and the diagnostic procedures that identified each.
The system was specified conversationally; the prompt corpus is reproduced in
Appendix A, the version history in Appendix C.

**Index Terms** — real-time rendering, ray casting, height fields, character
cell graphics, retrocomputing, 6502, fixed-point arithmetic, ordered
dithering, program nomenclature.

---

## I. Scope

This document is the design and measurement section of a larger record. It
covers what was built, how it was specified, what was measured, and what
failed. It does not cover a user study, and makes no claim about the
generality of the construction method described in Section III.

Sections II–IV cover origin, procedure and naming. Sections V–VII cover the
system. Section VIII covers Rust on the target processor. Sections IX–XI
report results, defects and limitations. Section XII invites review.

---

## II. Inspiration and Prior Art

### A. Originating artefact

The project began from a circulated prototype: a city rendered entirely in
ASCII symbols, implemented as a single HTML file with a bespoke JavaScript
engine and no external graphics library. Its author described it as a
grid-based three-dimensional world storing roads, buildings, trees, cars and
pedestrians with per-cell building height, rendered by casting rays from the
camera across the grid each frame, taking the first thing each ray meets, and
using distance to derive perspective, scale and occlusion. Nearby objects
were drawn as larger and brighter clusters of characters; distant ones faded.

Two properties of that description directed this work. First, the world was a
grid with per-cell height, which is a height field. Second, the renderer took
the *first* intersection. We retained the first property and departed from
the second (Section VI-B).

A second reference set was supplied as images: a night skyline in which each
building is drawn in a distinct repeating character, with visible window
grids and colour varying per structure. This determined the facade model
(Section VI-F). A third reference established scale and architectural
variety, and a fourth established the vehicle behaviour requested.

### B. Technical prior art

The per-column traversal derives from the ray-casting architecture of
*Wolfenstein 3D* and is formalised for voxel grids by Amanatides and Woo [1].
Continuing past first intersection with a running occlusion horizon is the
voxel-space technique of contemporaneous terrain renderers, applied here to a
city.

Shading follows Phong [2]; Blinn's halfway modification [3] was analysed and
deferred (Section VII-D). Gouraud interpolation [4] is shown to be
inexpressible in colour under the character-cell constraint and partially
recoverable in glyph shape (Section VI-E). Ordered dithering follows
Bayer [5]. The shadow method is horizon mapping in the sense of Max [6],
applied to a macro-scale height field. Secondary-ray shadowing [7] and
distributed ray tracing [8] were analysed and rejected on cost and on
medium-specific grounds respectively (Sections VII-C, X-D). Canonical
formulations of ray generation, intersection and distance are taken from the
POV-Ray documentation [9] and Angel [10], as these were the formulations
supplied in the specification.

---

## III. Method of Construction

### A. Procedure

The system was specified in natural language across 23 prompts (Appendix A)
in a single session, and implemented as it was specified. The human author
supplied intent, reference images, domain constraints and corrections. The
machine author supplied implementation, tests, measurement and defect
analysis. Every change was committed to git with a message recording the
reasoning; the version history is in Appendix C.

The working loop was:

1. A prompt states an intent, frequently as a correction of an observed
   result rather than as a feature request.
2. The intent is implemented, with assertions written to fail.
3. The result is rendered and inspected, or measured.
4. Discrepancies are reported to the human author with the measurement, not
   summarised.

Step 4 is the step that distinguishes this from code generation. Four of the
defects in Section X were found at step 3 or 4 and none at step 2.

### B. Specification arrived non-monotonically

Atmospheric effects, a helicopter camera, vehicle dynamics, a fare mission,
a zoning model, a lighting model and cast shadows were each introduced after
the architecture they had to fit was committed. We observe that the
height-field decision (Section VI) absorbed all of them without revision,
while the initially uniform street grid was replaced twice — first by a
generated plan with varying road classes, then by a four-layer model with
separate zoning, elevation and pedestrian networks.

We attribute the difference to the height field being a decision about
*representation* and the street grid being a decision about *content*. The
prompts constrained content repeatedly and representation never.

### C. Corrections were more productive than features

Nine of the 23 prompts corrected an observed rendering rather than requesting
new capability: excessive rain, rain occluding buildings, absent street
markings, trees overhanging the carriageway, insufficient field of view,
undersized vehicles, and three reports that the target build was rendering
incorrectly. These produced three of the four defects in Section X.

### D. Version control as the record

Thirteen commits, each with the reasoning and, where applicable, the measured
figures in the message body. Where a change was reverted — the quarter-square
multiplication of Section X-B — the reversion is recorded with the reason
rather than removed from history.

---

## IV. Nomenclature: From User Verbs to Program Identifiers

### A. Observation

The specification arrived as natural language containing verbs (*walk*,
*drive*, *get in*, *knock over*, *paint*, *sweep*, *bake*), objects
(*sidewalk*, *street light*, *taxi*, *block*, *fare*, *crosswalk*) and
anthropomorphic descriptions of behaviour ("the car wants to go forward like
a boat"; "the trees grow along the sidewalk"; "other cars get knocked around
like pins to a bowling ball").

These terms were translated into identifiers. Where the translation is
faithful, the program reads as a restatement of the specification and a
reader can check one against the other. Where the translation drifts, we
observed that defects followed, because the identifier then asserts something
the code does not do.

### B. Retained mappings

| Specification term | Identifier | Kind |
|---|---|---|
| "walk the streets" | `walk::WalkMap`, `Foot::Path` | object |
| "get in / out of the taxi" | `Sim::park_near`, `Stamp::Taxi` | verb |
| "light poles all get knocked over" | `Prop::standing`, `Car::shove` | verb |
| "the car wants to go forward like a boat" | `drive::Car::step` ordering | behaviour |
| "paint street lines" | `raycast::lines`, `zebra` | verb |
| "shadow sweep" | `shadow::ShadowMap::cast` | verb |
| "spreadsheet table of lookups" | `gen/tables.h` | object |
| "a fare" | `sim::Fare`, `Sim::hail` | object |
| "crosswalk" | `Plan::crossing_at`, `Crossing` | object |

Anthropomorphic identifiers were retained deliberately. `Tour::doing`
takes the values `Strolling`, `Admiring`, `Turning`, `Waiting`; `Sim::hail`
is what a passenger does to a cab. These carry the specification's intent
into the code, and each is constrained by a test named as a sentence — for
example `it_looks_up_at_something_at_some_point`,
`a_taxi_scatters_a_parked_car_and_barely_moves_a_bus`.

### C. Drifted mappings, and the changes made

Three identifiers were found to assert something other than what they
measure. All three were corrected after the review that produced this
section.

**1. `City::walkable` → `City::open`.** The predicate returns whether a cell
is unbuilt. It was named for a hypothetical user of that fact rather than for
the fact, and the hypothetical user was wrong: it is the test a *vehicle*
requires, whereas a pedestrian requires `WalkMap`. Two defects trace directly
to reading the name as a claim about people. Pedestrians were placed by
"is it walkable", which is how they came to walk down the middle of avenues;
the unattended camera moved by the same predicate, which is how it wandered
into parks — enclosed clearings inside blocks — and spent extended periods
facing the backs of buildings. The predicate is unchanged; the name now
states what it measures, and the two questions are answered by two
differently-named things.

**2. `--tour` gains `--demo`.** The unattended mode is called a *demo* by the
human author, by the Makefile targets (`demo`, `demo4`) and by the target
implementation (`cast_demo`). The host command-line flag alone called it a
tour. Both spellings are now accepted. We note this is a naming defect with
no behavioural component, detectable only by comparing the vocabulary of the
specification against the vocabulary of the interface.

**3. `rows_` → `rowshade`.** A row-base pointer table for the shadow array,
named with a trailing underscore to avoid colliding with a local variable
named `rows`. The name recorded a compilation constraint rather than a
meaning.

### D. Statement

An identifier is a claim. `walkable` claimed that a predicate answered a
question about pedestrians; it did not, and the code that trusted the claim
was wrong in exactly the way the claim was wrong. We did not observe the same
class of error where the identifier was drawn directly from the
specification's own vocabulary.

---

## V. System Architecture

### A. Components

```
                    ┌──────────────────┐
                    │   ascitty-core   │  10,206 lines, Rust
                    │ no I/O, no clock │  no external dependencies
                    └────────┬─────────┘
                   ┌─────────┴──────────┐
                   ▼                    ▼
          ┌────────────────┐   ┌──────────────────┐
          │  ascitty-tty   │   │  ascitty-bake    │
          │  1,308 lines   │   │    568 lines     │
          │  terminal      │   │  generator       │
          └────────────────┘   └────────┬─────────┘
                                        ▼
                              ┌──────────────────┐
                              │ targets/plus4    │  2,847 lines, C (cc65)
                              │  .prg  /  .d64   │
                              └──────────────────┘
```

`ascitty-core` holds the number system, world model, camera, glyph
catalogue, traversal, vehicle dynamics and atmosphere. It performs no I/O,
reads no clock and has no external dependencies. The last is a consequence of
the second target: the module is the specification the 6502 build
transcribes, and a dependency graph must be understood before it can be
transcribed.

### B. World model: four layers

The world is four separate structures over one grid, because the questions
are separate.

| Layer | Module | Question answered |
|---|---|---|
| Roads | `world::Plan` | where the streets are, how wide, what class |
| Zoning | `zone::ZoneMap` | what this ground is *for* |
| Elevation | `elevation::Elevation` | how high the ground is; what stands on it |
| Walking | `walk::WalkMap` | where a person on foot may be |

Zoning distinguishes three things previously conflated: *zone* (a property of
place), *use* (what a building is — office or dwelling), and *archetype* (how
it is constructed). Separating them prevents the generator collapsing to
"tall structures are glass, short structures are brick": a twelve-storey
residential slab and a twelve-storey office slab share an archetype and
differ in use, and are rendered differently.

### C. Numeric representation

Render-path arithmetic is fixed point throughout: Q16.16 on the host, Q8.8 on
the target. Floating point is confined to table generation, diagnostics and
tests.

The motivation is comparability rather than performance. An implementation
written twice in two number systems cannot be differenced: when the two
images disagree, a defect is indistinguishable from the expected consequence
of narrowing a float to eight fractional bits.

We observed a second effect that was not anticipated. The discipline forced
the host arithmetic into the shape the target requires — reciprocal rather
than divide, table rather than transcendental, octagonal norm rather than
square root — so that the transcription to C was mechanical rather than a
redesign.

### D. Generator/target bridge

`ascitty-bake` executes the generator and emits its results as C headers: the
1 KB character set in TED bit order, a 256-entry sine table in Q8.8, a
512-entry reciprocal table, a 64×64 district (heights, colours, facade tiles,
precomputed shadow line), screen geometry, projection scale per distance,
haze ramp, per-column camera-plane offset, ground colours, star field, field
of view, light bearing, and a 511-entry quarter-square table.

None is committed; all is regenerated before every target build.

The correctness argument is stronger than the performance one. Before
consolidation, the target computed several of these independently at boot.
One was the light bearing: the target was shading surfaces as if lit from one
direction while displaying shadows swept for another. No test detected it,
because each half was internally consistent. The defect is unavailable once
both halves read the same constant.

---

## VI. Renderer

### A. Ray parameterisation

```
    D = dir + plane · camx,     camx ∈ [−1, 1] across the frame
```

with `plane ⊥ dir`, scaled by a field of view expressed as a plane
half-width. The component of `D` along `dir` is therefore unity for every
column. Three consequences:

1. The distance reported by the traversal *is* the perpendicular distance;
   the `t·cos θ` fisheye correction is structurally absent.
2. `dir` and `plane` are computed per frame, not per column.
3. No square root occurs on the render path. The Euclidean norm — `vlength()`
   in [9] — is never required, because the projection consumes `t`.

Where a magnitude is genuinely required — depth ordering of billboards,
vehicle speed, contact normals — we substitute
`|v| ≈ max(|x|,|y|) + ⅜·min(|x|,|y|)`: two comparisons and a shift, error
bounded near 4%.

### B. Traversal with an occlusion horizon

The originating prototype took the first intersection. First-hit visibility
is sufficient for a maze and insufficient for a skyline, because a defining
feature of a city is a tall structure seen over a nearer one.

Traversal therefore continues, carrying one integer per column — the topmost
screen row claimed so far — and each subsequent structure may draw only above
it:

```
    ceiling ← H
    repeat
        step the DDA;  d ← perpendicular distance
        if d ≥ far or cell empty: continue
        top ← horizon − proj·(ground + height − eye)/d
        bot ← horizon + proj·(eye − ground)/d
        draw rows [max(top,0) … min(bot, ceiling−1)]
        ceiling ← min(ceiling, top)
    until ceiling ≤ 0
```

Clamping the span at both ends permits ground to remain visible in the gap
between two structures. This integer is the entire hidden-surface solution:
no depth buffer, no sort.

### C. Intersection cost

For axis-aligned unit cells, intersection with the plane `x = k` is
`t = (k − Oₓ)/Dₓ`, and the DDA amortises the division by precomputing
`Δt = 1/|D|` once per column. Each crossing then costs one addition and one
comparison. Against the standard result that intersection testing consumes
75–90% of a classical ray tracer's time, the corresponding cost here is not
measurable; the dominant per-frame costs are cell writes and traversal steps.

On the target the two reciprocals are themselves table lookups (`65536/n`,
512 entries), the processor having no divide instruction.

### D. Ground plane

Ground distance depends only on a row's displacement below the horizon, not
on the column. It is a per-frame table of `h/2` entries, reducing floor
casting from a division per cell to a division per row.

### E. Shading below the cell

At one colour per cell the system is confined to flat shading; interpolation
of *colour* below the cell is undefined.

The glyph, however, is an 8×8 bitmap generated by a function. Ordered
dithering already exploits this for magnitude — `shade(n)` selects a glyph
covering exactly n/8 of the cell. Extending it to a directional gradient, by
selecting the glyph whose first moment lies toward the brighter side, is the
character-cell analogue of Gouraud shading: colour remains flat and shape
carries the interpolation. The glyph generators support it; the renderer does
not yet drive it.

Ordered dithering is required rather than preferred. A cell is re-selected
each frame, and a diffused error has nowhere to propagate between frames
except into a temporally crawling artefact.

### F. Facade model

Two decisions are taken at deliberately different rates. *Which* facade tile
a structure is drawn in is a property of the lot, from its seed alone — one
tower drawn in `X`, its neighbour in `0`. *Whether* a given window is lit is
a property of the window, sampled at the resolution the screen cell can
resolve, with floor and bay indices quantised by level of detail first.

Taking both decisions at the per-window rate was the first observable defect
in the project: the skyline rendered as one textured mass rather than as a
row of distinct structures.

---

## VII. Constrained Target

### A. Platform

The TED video controller provides 121 colours as 16 hues × 8 luminances and
can source character definitions from RAM. When it does, it reads a 1 KB set
— 128 definitions — with bit 7 of a screen code reinterpreted as
reverse-video. The glyph catalogue is therefore exactly 128 entries, and the
colour byte is packed `luminance ≪ 4 | hue` in the *host* renderer as well,
so on the target the renderer's output byte is the hardware byte.

The palette organisation also supplies the depth cue: holding hue and
decrementing luminance is one subtraction on a nibble.

### B. Diffuse illumination in five values

An axis-aligned height field presents exactly five surface normals: four wall
orientations and a roof. The traversal already determines which, from the
grid plane crossed and the step sign.

For a directional source `L` is spatially invariant, so `L·N` is five values,
recomputed once per frame. Luminance being 3-bit, the term is stored as a
signed offset and added. The lookup is hoisted out of the per-row loop, every
cell of a wall span sharing a normal.

Measured cost on both targets: not detectable. Host frame time 0.16 ms before
and 0.16–0.18 ms after; target program grew 342 bytes.

### C. Cast shadows without secondary rays

Sweeping the grid once along the light's ground projection, carrying

```
    horizon ← horizon − slope_per_step
    shadow[cell] ← horizon                  (recorded before the max)
    horizon ← max(horizon, top(cell))
```

gives O(n) construction per light direction and O(1) evaluation per sample.

Two properties are required. The map stores a *height*, not a predicate: a
wall is dark below the line and lit above it, which is the appearance of a
tower in the shadow of a nearer tower, and a predicate cannot express it. And
the horizon is recorded *before* the cell's own height is folded in; omitting
this causes every structure to occlude itself and renders the city black. The
latter is the natural way to write the loop; a regression test constrains it.

On the target the sweep does not execute: the shadow line is a function of
the height field and light bearing, both known at bake time, and is emitted
as a district array. The machine performs one multiply per wall intersection
and one comparison per row.

Measured: sweep ≈ 2 ms for 234×234, once; frame cost not detectable; 48% of
open ground in shadow at 21° elevation, asserted as a bound.

### D. Deferred: specular

Blinn's `H = (L+V)/|L+V|` requires a normalisation and hence a square root.
`N` is one of five and `V` is constant per column, so `N·H` is a `width × 5`
table rebuilt on rotation — 800 bytes host, 200 target. Deferred: at one
colour per cell a matte surface has no resolvable highlight, and the case
rests on wet asphalt and glass.

---

## VIII. Rust on the Target Processor

The specification requested Rust where possible. The host renderer, the
generator and the terminal front end are Rust; the Plus/4 program is C
compiled by cc65 [11]. This section records what was verified.

### A. What was checked

On the development machine, `rustc 1.93.1`:

- `rustc --print target-list` contains no MOS 6502 target. A grep for `mos`
  returns `aarch64-unknown-illumos` and `x86_64-unknown-illumos`, which are
  false positives — a detail we record because it is the kind of match that
  makes an unverified claim.
- No `mos` directory exists under the toolchain's `rustlib`.
- Homebrew has no `llvm-mos` formula in core.

6502 code generation for LLVM exists as the out-of-tree llvm-mos project,
with a corresponding `rustc` fork. Using it would require building a
toolchain from source, a nightly compiler and a custom target specification,
and would place `core` on a processor with a 256-byte hardware stack. cc65 is
packaged, has produced 6502 code for two decades, and was already in use in a
sibling project on the same machine, so the hardware knowledge, linker
configuration and emulator harness were understood.

### B. Rust patterns that nonetheless serve the 6502

The decision not to compile Rust *to* the 6502 did not remove Rust from the
target's construction. Four patterns were used specifically to serve it.

**Generator as offline compiler.** The most consequential. Rust executes the
real generator — the character set, the city, the trigonometry, the shadow
sweep — and emits results as C data. The 8-bit target performs no generation
and holds no second implementation, so the two cannot disagree. This is the
sense in which the project is "Rust compiled down to a retro machine": Rust
is the offline stage, not the runtime.

**Fixed-point arithmetic as a shared contract.** `fixed.rs` defines Q16.16 in
terms the target's Q8.8 mirrors operation for operation. The Rust is written
in the arithmetic the 6502 can perform, not in the arithmetic the host makes
convenient.

**Structure-of-arrays for the baked data.** The district is emitted as four
parallel byte arrays rather than an array of records, because a 6502 indexes
a byte array with one instruction and a record array with a multiply it does
not have.

**Constants with a single definition.** Screen geometry, projection scale,
field of view and light bearing are emitted from the Rust constants rather
than restated in C. The class of defect this removes is described in
Section V-D.

### C. What is not claimed

`ascitty-core` is not `no_std` and allocates on the host: frames and
per-frame row tables are `Vec`. It is written so that the *arithmetic*
transcribes, not so that the crate compiles for a 6502. Making it `no_std`
with caller-provided buffers is a plausible step toward a shared source, and
is recorded in the backlog rather than claimed here.

---

## IX. Results

### A. Host

At 160×48 with approximately 7,000 traversal steps per frame: **0.16 ms per
frame**, approximately 6,000 fps. The renderer is not the limiting factor on
contemporary hardware; terminal output bandwidth is. Frame painting emits a
colour escape only on change, which on a predominantly black night scene
reduces most rows to a single escape.

### B. Target

Measured with `tools/frametime.sh` (Appendix B), which renders exactly *N*
frames, sets the border to a distinctive colour, and bisects the emulator
cycle budget at which that occurs; two runs at differing *N* cancel boot cost.

| Configuration | cycles/frame | fps |
|---|---:|---:|
| Initial target implementation | 1,910,156 | 0.93 |
| Baked tables + row-base pointers | **687,500** | **2.58** |

The 2.8× improvement is attributable almost entirely to one change. The
traversal's innermost expression was `city_h[(my ≪ 6) | mx]`; on a 6502,
shifting a 16-bit quantity six positions is approximately two dozen cycles,
executed on every step of every column of every frame. A table of row base
pointers — built at boot, addresses not being known until link time — reduces
it to an indexed access.

Baking the remaining tables removed 337 divisions and modulo operations from
the boot path and was performance-neutral; its value is the property in
Section V-D.

### C. Generated world

| Property | Value |
|---|---|
| Map | 234 × 234 cells (18 blocks, 16 built) |
| Cell scale | ≈ 6 m |
| Road classes | 5 (alley 1 cell … arterial 12–16 cells) |
| Minimum block | 8 cells, enforced by clamp |
| Building heights | median 9, p90 26, p99 42, max 50 |
| Pedestrian network | 25,261 of 25,261 passable cells, one component |
| Baked district | 64 × 64, 57% built |

Heights are drawn from four bands approximating a power law. A uniform
distribution was tried first and is perceived as noise: no general roofline
exists for any structure to exceed.

### D. Verification

290 assertions execute in approximately 0.2 s. The acceptance gate
additionally rebuilds both targets, regenerates the baked headers, renders a
host frame, and boots the target program in an emulator to confirm
non-trivial output. A target program that compiles is not evidence that it
renders; three of the four defects in Section X produced programs that
compiled.

---

## X. Observed Defects

### A. Four defects that survived visual inspection

**1. Inverted steering sign.** The unattended camera's lateral centring term
used `clearance(left) − clearance(right)` where the movement basis made
positive displacement rightward, so it steered toward the closed side. On a
wide symmetric avenue this is unobservable. Elsewhere it held the camera
against the kerb for 948 of 3,000 sampled ticks. The centring gain had
previously been raised by a factor of 2.5 in an attempt to improve behaviour,
compounding it. Diagnosis required recording *what the camera was standing
on* rather than reasoning about the expression: all 948 samples were on the
one-cell pavement. After correction: 0 of 3,000.

**2. Crossings unreachable from the pavement.** Pedestrian crossings were
placed inside the junction box. Every orthogonal neighbour of a junction cell
is either further junction or open carriageway, so no crossing was adjacent
to any pavement. The pedestrian network had no connected component larger
than one block: 46 cells of 25,261. Rendered frames appeared correct
throughout. Found by a connectivity assertion. Relocating crossings to the
stop line one cell outside the box — where they are also painted in practice
— produced a single connected component.

**3. Luminance underflow to background.** On the target three subtractions
compose: surface orientation, distance haze, cast shadow. Against eight
luminance levels these reach zero well inside the draw distance. A structure
rendered at luminance zero is not dark but absent — black glyphs on a black
field — which is indistinguishable from the traversal finding no geometry.
Two debugging sessions were spent on the traversal before the cause was
identified. The host clamps to a floor of one; the target did not.

**4. Program/character-set memory collision.** The character set was copied
to a fixed address `$7000`, above the program when that address was chosen.
Adding the baked tables and shadow map extended read-only data to `$77E8`,
and the program and its font overwrote each other. The symptom was again a
frame containing sky and ground and no geometry. The linker map identified it
in one line. The remedy is structural: the buffer is placed by the linker and
aligned at runtime to the 1 KB boundary the hardware register requires.

All four produced plausible output. Three were diagnosed only after
substituting a scalar measurement for inspection: what surface is underfoot,
how many cells are mutually reachable, where does the read-only segment end.

### B. A performance figure that was an artefact

Quarter-square multiplication, `a·b = ⌊(a+b)²/4⌋ − ⌊(a−b)²/4⌋`, is exact for
integers because `a+b` and `a−b` share parity, and is a standard method on
processors without a multiplier. Applied to the traversal's side-distance
initialisation it measured **5.40 fps against a 0.93 fps baseline**.

The implementation was incorrect. Every column terminated traversal
immediately, so the renderer performed a fraction of the work. The
measurement was of a program producing a blank screen.

We state the finding plainly: a frame-rate measurement from a build whose
output has not been inspected is not a measurement of rendering performance.
The correct figure with the optimisation reverted is 2.58 fps. The defect in
the implementation is unresolved and recorded in the backlog.

### C. Content-dependent failure attributed to the agent

The unattended target mode initially spent approximately half its frames
observing empty space. Three hypotheses — insufficient look-ahead, wandering
to the district boundary, an over-long leash — each produced partial
improvement and none resolved it.

The cause was in neither the autopilot nor the renderer. The 64×64 district
baked for the target was taken as the geometric centre of the map, downtown
being central. So is the intersection of the two arterial roads, which are
12–16 cells wide; the extracted district contained a thirteen-row band of
empty carriageway and was 33% built. Selecting the window by content instead
— a summed-area table over building occupancy makes candidate scoring
constant-time, so every offset is evaluated — raised occupancy to 57% and
frames containing visible geometry from 3-in-6 to 6-in-6.

Three iterations were spent modifying the agent when the defect was in its
environment. Rendering the district as an ASCII occupancy map, a two-minute
diagnostic, displayed it immediately.

### D. Distributed ray tracing: rejected

Distributed ray tracing [8] obtains anti-aliasing, soft shadows, depth of
field, glossy reflection and motion blur from one mechanism, characterised as
converting aliasing into noise.

The exchange is unfavourable twice. On cost, sixteen samples per cell on a
platform achieving 2.58 fps at one sample is not a quality setting. On the
medium, noise is worse than aliasing in a character grid: a pixel renderer's
noise resolves to texture at viewing distance, whereas a character renderer's
noise is a visibly incorrect glyph, and one re-sampled per frame crawls. The
single component worth retaining, soft shadows, is obtainable from the
horizon sweep without sampling by grading the offset by depth below the line.

### E. A controller defect that was not in the controller

The autopilot's preference for the right-hand lane was measured at between 25
and 63 per cent of travelling ticks across four cities and did not respond to
any change in the lane target. Three targets were tried - the middle of the
right-hand half of the carriageway, the kerbside lane, and one cell past the
crown - and each was clearly best on some cities and clearly worst on others.
Raising the cross-track gain to full authority made one city *worse*, 357
ticks correct against 1,209, which is the signature of a sign inversion
rather than of an under-tuned gain. The sign conventions were checked by hand
against all four combinations of axis and direction and were correct.

The defect was in neither the target nor the gains. The lane controller
regulates against a single lane line and therefore declines to act where
there is no single line: inside a junction, where both axes are streets. The
fallback in that case steered at the fare marker directly. On a grid whose
two arterials are twelve to sixteen cells wide, their crossing is a junction
box on the order of two hundred cells, and a car inside it was steering at a point
twenty cells away on the far side of a block, arriving on whatever side of
whatever street the geometry produced. Steering instead at a point a few
cells along the already-planned route raised the split to 70-88 per cent and,
on one city, took completed fares in five minutes from one to eight.  Giving
the engine an acceleration curve subsequently moved the figures a second time
- the car spends longer at low speed, where the cross-track term is divided
by a smaller number - and the gain was raised by half to settle them at 83,
79, 77 and 81.

The general form is the same as Section X-C. Three lane targets and a gain
were tuned in the agent when the defect was in the one case the agent
declines to handle at all. A
controller that is correct wherever it acts can still be wrong for most of a
run if the conditions under which it does not act are common and its
behaviour there is unconsidered.

### F. Input the byte stream could not express

A terminal reports a key going down and nothing at all when it comes up, and
autorepeats the most recently pressed key only. Two keys held at once is
therefore not a state the input can represent: pressing the second one stops
the first from arriving. For a driving mode this excludes accelerating
through a corner, which is not an edge case but the ordinary way a car is
driven.

The workaround in place was a decay - a press stayed live for five frames and
autorepeat renewed it - which addresses the first half of the problem and
cannot address the second at any setting.

Two changes were made. The controls became analogue axes that wind on while
held and off when not, which is worth doing on its own: an arcade throttle is
a thing you lean on. And the terminal is now *asked*, at startup, whether it
speaks the progressive keyboard protocol, which reports press, repeat and
release as distinct events; where it does, a held key is held and two are
two. The handshake is a single round trip and is decidable rather than
timed: the flags query is followed by a primary device attributes request,
which every terminal answers, so a terminal that has answered the second
without answering the first has given a definite no.

Where the answer is no, the decay remains and its window was set from the
system autorepeat parameters rather than by feel: half a second, because that
is the delay before the first repeat, and a shorter window is a dip in the
first half second of every hold. Measured against an emulated terminal at the
defaults - 500 ms to first repeat, then 33 ms - a quarter-second window read
43 and 52 mph at the two moments where half a second read 58 and 84.

### G. A projection that used the wrong height

Sprites - cars, people, street furniture, the fare markers - are billboards
placed by the same projection the walls use. Their feet were positioned with
the camera's *absolute* height where the ground plane uses its height **above
the ground**. The two are the same number only where the terrain is at zero.

The terrain generator produces about two cells of relief across the map, so
across four sampled places in one city the true eye height was 0.71 to 0.80
cells against an absolute height of 1.17 to 1.83 - the sprite projection was
using an eye two to two and a half times too large. Feet are drawn
`eye x scale / distance` rows below the horizon, so the error scales like
everything else in the projection: a car ten cells away sat about eleven rows
too low, while a distant one was nearly right.

The reported symptom was "the cars are in the wrong place and seem to drift
away as they recede", which is an exact description of an error proportional
to `1/distance` and is not a description of anything else. It survived
inspection for as long as it did because the city it was first looked at in
is nearly flat where the camera spawns, and because a sprite drawn too low is
still a sprite.

### H. A stuck check that only knew one kind of stuck

The same autopilot detects being wedged by measuring speed: below half a cell
per second for half a second, it reverses with opposite lock. A car that has
climbed a kerb and is grinding along a shop front travels at about one cell
per second and passes that test indefinitely - measured at 1,000 ticks of a
3,000-tick run, a third of it, with the speed never once falling far enough
to trip the check. The remedy is a second predicate over a different
quantity: a second spent with the car's centre off the carriageway is also
stuck. Clipping a kerb on the way round a junction lasts a few ticks and is
not.

---

## XI. Limitations

The world is a height field and cannot express any structure one may pass
beneath: bridges, elevated roadway, arcades, tunnels. A second height field
for floor elevation would express these at approximately double the traversal
inner-loop cost.

The street plan comprises two independent axes, so every road is axis
aligned. A diagonal thoroughfare is not merely absent but inexpressible in
the current representation.

Terrain relief is held below two cells across the map. The floor pass samples
ground assuming it is level with the camera's own footing; over a gentle
grade the error is below one character cell and over genuine topography it
would not be.

Target-side omissions: floor casting is replaced by distance-shaded bands;
there are no billboard sprites and therefore no vehicles or pedestrians; and
the frame rate is below the raster rate, so the display tears. Drawing each
column to completion before beginning the next reduces this from a blank
half-screen to a vertical wipe but does not remove it.

Street lights of increased height with an emissive halo, positioned at the
kerb, and a raised pavement with a planted verge, are implemented on the host
and observed in Section XI's frames. Two limitations are noted. The kerb is
two elevation steps - 37 cm - rather than the one step - 18 cm - the request
implies, because one step is also the steepest gradient the terrain generator
produces, and a one-step kerb is therefore cancelled wherever the ground falls
the other way across the same boundary. Levelling each carriageway with its
pavements before raising one would give a true 18 cm and is recorded in the
backlog. Neither the kerb nor the verge reaches the Plus/4: the bake carries
building heights only, and the target has no terrain array to raise.

An unattended mode driving a vehicle rather than walking is implemented and
is now what the program does when it is started and left alone: the cab takes
a randomly chosen fare, plans a route over the carriageway, drives to it and
pulls up at the kerb beside the circle, at which point the simulation hands
over the passenger and issues another. Both of the limitations previously
recorded here have been closed - see Section X-E - and the figures are now 70
to 88 per cent of travelling ticks on the correct side of the crown, and 0 to
2 per cent with the car's centre off the carriageway.

What remains is that the other traffic does not turn. It keeps its lane,
gives way to what is ahead and to what crosses from its right, and is
recycled when it falls behind, but it goes only where the street it was put
down on goes.

---

## XII. Peer Review and Reproduction

The source, version history and all measurement harnesses are public under
the MIT License at https://github.com/vonglurt/ascitty. Review is welcomed,
particularly on the following, where we hold a position we cannot fully
defend from measurement:

1. **The quarter-square implementation (Section X-B).** The decomposition
   `(f·d) ≫ 8 = ((f·lo) ≫ 8) + (f·hi)` is algebraically correct and the
   implementation is not. We have not located the fault.
2. **Sub-cell gradient shading (Section VI-E).** We assert this is the
   character-cell analogue of Gouraud shading. It is unimplemented, so the
   assertion is untested.
3. **The height-field restriction (Section XI).** We claim it absorbed every
   content change without revision. This is an observation over one project.
4. **Nomenclature (Section IV).** We claim identifier drift preceded two
   defects. The causal direction is argued, not demonstrated.

Reproduction requires `cargo`, `cc65` and VICE [12]:

```sh
git clone https://github.com/vonglurt/ascitty && cd ascitty
make            # host binary, .prg, .d64
make check      # 222 assertions, both targets, emulator boot
make bench      # host frame time
tools/frametime.sh 10   # target frame time, by bisection
make demo       # unattended, host
make demo4      # unattended, Plus/4 under emulation
```

The city is deterministic from a seed; `ascitty_core::DEFAULT_SEED` is the
one all figures here were taken with.

---

## XIII. Conclusion

Restricting world geometry to an axis-aligned height field is principally a
restriction on *arithmetic* rather than on the world. The normal set becomes
finite and small; distance becomes a by-product of the ray parameterisation
rather than a norm to evaluate; intersection becomes an addition; occlusion
becomes one integer per column; shadowing becomes a sweep. Individually these
are modest; together they are the difference between a technique being
available on a 1.76 MHz 8-bit processor and not.

The character-cell constraint proved less limiting than expected in one
respect and more in another. Colour interpolation below the cell is
unavailable and no effort recovers it. The glyph is an 8×8 bitmap produced by
a function, which is 64 bits of sub-cell structure beneath every colour — a
channel with no analogue in a pixel renderer, and the part of this work we
have explored least.

---

## Acknowledgment

The concept originated in a circulated prototype of a living ASCII city
implemented as a single HTML file with a bespoke JavaScript engine, described
by its author as a grid-based 3D world with a per-column ray caster
substituting letters and symbols for pixels. This work is an independent
implementation directed at fixed-point arithmetic, a generated character set
and a machine from 1984.

---

## References

[1] J. Amanatides and A. Woo, "A fast voxel traversal algorithm for ray
tracing," in *Proc. Eurographics '87*, 1987, pp. 3–10.

[2] B. T. Phong, "Illumination for computer generated pictures," *Commun.
ACM*, vol. 18, no. 6, pp. 311–317, Jun. 1975.

[3] J. F. Blinn, "Models of light reflection for computer synthesized
pictures," in *Proc. SIGGRAPH '77*, 1977, pp. 192–198.

[4] H. Gouraud, "Continuous shading of curved surfaces," *IEEE Trans.
Comput.*, vol. C-20, no. 6, pp. 623–629, Jun. 1971.

[5] B. E. Bayer, "An optimum method for two-level rendition of
continuous-tone pictures," in *Proc. IEEE Int. Conf. Communications*, 1973,
pp. 11–15.

[6] N. L. Max, "Horizon mapping: shadows for bump-mapped surfaces," *The
Visual Computer*, vol. 4, no. 2, pp. 109–117, Mar. 1988.

[7] T. Whitted, "An improved illumination model for shaded display," *Commun.
ACM*, vol. 23, no. 6, pp. 343–349, Jun. 1980.

[8] R. L. Cook, T. Porter, and L. Carpenter, "Distributed ray tracing," in
*Proc. SIGGRAPH '84*, 1984, pp. 137–145.

[9] Persistence of Vision Raytracer Pty. Ltd., *POV-Ray Documentation*.
[Online]. Available: https://www.povray.org/documentation/

[10] E. Angel and D. Shreiner, *Interactive Computer Graphics: A Top-Down
Approach with WebGL*, 7th ed. Boston, MA: Pearson, 2014, ch. 5.

[11] U. von Bassewitz *et al.*, *cc65 — a freeware C compiler for 6502 based
systems*. [Online]. Available: https://cc65.github.io/

[12] VICE Team, *VICE — the Versatile Commodore Emulator*. [Online].
Available: https://vice-emu.sourceforge.io/

---

## Appendix A: Specification Corpus

The complete sequence of specification prompts, verbatim including
orthography, in order of issue. Prompts 1, 2, 4 and 11 were accompanied by
images or pasted reference material, summarised in brackets rather than
reproduced. Prompt 1 additionally included a transcript of a video describing
the originating prototype.

**1.** *(with a screenshot of an ASCII cityscape, and a video transcript)*
"git init. MIT License Paul Richeson and Claude. Make published on git
project. Write Readme. Write docuemnts to this folder. Sync. Find a coding
environment. Propose a development setup. Start with brew on macos. We want
to compile ona Commodore Plus/4 . I believe any code should be most
performant Rust to clang to compiled program. Perhaps in a .prg and a .d64
image. Take the style of project organization like ~/code/urfinkel . Create
an index structure. keep a backlog. Create a compiled program. Programming in
rust in not required, but would be nice if we can find a raytracer algorithm
performanct for ascii rendering. Also the symbols, we need a method of
dithering, or converting shapes and edges to be like the fonts available. Can
we create our own procedure block font to allow a function to generate
different blocks, or applied transformations to blocks. A general font could
just be precomputed, or blocky font files might be needed. We want a city
scame to be raytraced. The colors must be like retro black terminal
background, where the screen is processed using typebale characters. or even
better pre computed shapes for characters. Using the commodore keyboard
symbols as a basis. We can also use ascii, because this is ascii in the name.
we should alsoways have an ascii tty mode. with colors. […] It looks like a
city scape. with box rectable buildings with windows, procedurally rendered.
Buildings like new york city, towers. Screets. Enough of a scene to spin the
camera and walk on foot pov first person . We can simulate this on commodore
64, or commodore plus/4 . Maybe even on PC. We want multi target since this
is mostly just a ascii camera system."

**2.** *(with four reference images)* "axpand. atmosphere effects, like rain,
moon. look at this grid system."

**3.** "not simcity in specific, we dont want that angle. its good
inspiration for the scale of the city. to expand on building archtiecture,
not just boxes. a look at how windows zipper down the size of the building ,
and the obvious fire escape attack to the outside of the building."

**4.** *(with a screenshot of an arcade taxi game)* "more top down look. also
allow camera to move into copter mode. like sim copter, where you can fly
above the maximum building height and fly around the city looking down at an
angle, or maybe even sim taxi mode. where you pick people up and take them to
their job .. where you would enter the car, the yellow taxies, and drive it
around like this, third perspective. where you have a circle to drive on, to
park to let you rider get in and out. There is a suggested travel coins that
spawn when your mission timer starts. The driver will collect the large
spinnning coins . along the ascii third person perspective. We need a GTA
style physics crazy taxi driving chaos simulator. Where the car is fast, and
slides out like a rear wheel drive. Reality does not matter. What does matter
is pace, and collisions, other things and mailboxes, and light poles all get
knocked over, and defect the car which does not get hurt. The car wants to go
foraward like a boat. drifting street corners when going fast. Other cars get
knocked around like pins to a bowling ball. The buildings are rigid. The
streets are mostly flat."

**5.** "what are you rendering. why so many attempts. are you having trouble
finding a fram?e"

**6.** "i done it done." *(confirming an interactive authentication step)*

**7.** "can we attempt to walk the streets and look around. how does camera
controls works. can we set an animation."

**8.** "less rain. too much. rain should mainly be against the black
background not the buildings. also can you paint street lines, for cars, a
yellow center line for each street, which turns into cross walks for
intersections."

**9.** "can we spend more effort in creating a reasonable street system. A
slightly larger world. A increasing variety of building height, and colors"

**10.** "so. give me a residential building, a commerical, make the blocks at
leats 8 by 8. with decreased building height after 8x8 radius, (smaller
buildings keep going till 16x16. where the streets are very large, like a
full 100m, these are big buildings and 100m new york city streets. Please
create a mapping, an elevation map structure, a roat and a walking system.
Also make the taxi be next to the starting position."

**11.** *(with an extended excerpt of ray-tracing and shading reference
material covering POV-Ray vector functions, sphere intersection, the Phong
model, distributed ray tracing and polygonal shading)* "the ray tracing might
have already been calculated correctly. Create a writeup and write items to
the backlog and incorporate fast ray tracing explain with these formulas.
[…] and a good sampling of what we can do efficiently in ascii city."

**12.** "do the shadow sweep next"

**13.** "can we , for common compute reasons, keep a cached table of
interpolation for the plus/4 to create a pre computed spreadsheet table for
calculations of lookups rather than computing difficult math specifically. we
want numerical reductions, and techniques to speed up the most common
critical calculations, for an approparate interval step. We also need to
increase the size of the cars, and increase the length of the taxi. in fact,
pressing t for taxi. and the driving is third person follow cam of yellow
taxi. entered by pressing t button (remap the t button for text, to be
something else)."

**14.** "can you draw sidewaks up to the buildings on the side of the
streets, both sides. the sidewalk is cement colored, but dirty ground."

**15.** "the trees go next to the size walk, never in the road."

**16.** "can you expand the fov, to be customizable. perhaps we have 110
degree of fov."

**17.** "the street lights need to be 3x as tell, the need a emmit a glow
effect directly around the light itself. perhaps the light bulb. the steet
lights need to go between the street and the sidewalk. turn the demo to be a
car driving on the right hand side of the road, making a random path of the
streets. Drive like a street car. the dmo itself shows a car driving."

**18.** "stop please consolidate all verions of this game. The terminal
version on mac/pc and the plus/4 version. how do i test demo on plus 4. can
you start it/?"

**19.** "ok. so. make demo4 for demo plus 4 mode."

**20.** "can you raise the height of the sidewalk by about 1 pixel. or about
1/10 the height of the camera. the trees grow along the side walk. see how
trees are in grass. along sidewalk seperating from street road from
buildings"

**21.** "the rendering on the commodore plus 4 needs fixing."

**22.** "i think it works. commit. push"

**23.** "write a lab-report ieee academic style. Paul Richeson and Claude
under MIT license. We constructed via the prompts. Keep a collection of
prompts used in this message history and this project of creating a crazy
taxi, with starting with a text rendering pov perhaps third person, computed
commodore plus 4 program. a prompt section, a creation section, an
inspiration and open sourced peer review is welcome, the procdure and
architectual decisions of the application strutures. do not talk with
puffery, instead make concise clear statements like an acemdic would in a
ieee lab report, we state what we observe and reactions to input. So a desgin
of a program has a reaction by a user using it, then the 'verbs' and 'object'
and aspects antropormorphic are translated into the code and wording we use.
after consideration, make the code changes for more accurate wording, but
also this is just one section of a lab report, and its structure, we are
tracking it in git, creating a version history, we created this in rust, but
we need to look into rust patterns used for commodore plus 4. save this with
title. markdown as a file."

Prompts 17 and 20, and portions of 13, were issued while a prior request was
in progress and were deferred. The street-light and planted-verge items were
completed subsequently; the driving demonstration in prompt 17 remained
outstanding at the time of writing (Section XI).

---

## Appendix B: Measurement Methodology

**Host frame time.** `ascitty --bench` renders 200 frames with camera
rotation, painting each to an ANSI string, and reports the mean. Figures are
the median of three runs. A single anomalous reading of 0.33 ms was traced to
a concurrent compilation and discarded.

**Target frame time.** Wall-clock timing is meaningless under emulator warp
mode. `tools/frametime.sh` compiles a variant that renders exactly *N*
frames, sets the border to a distinctive colour, then resumes. The emulator
runs under a cycle limit and the resulting frame is captured; the cycle count
at which the border changes is located by exponential expansion followed by
bisection to 3×10⁵ cycles. Run at *N* and 5*N*, with per-frame cost taken as
the difference over 4*N*, cancelling boot and injection cost.

**Target correctness.** `tools/viceshot.sh` boots a program under a cycle
limit and writes a framebuffer capture. Program injection
(`-autostartprgmode 1`) is used in preference to emulated disk loading, which
consumes approximately ninety seconds of machine time and causes captures to
record the loader. Captures are validated by the border colour, which the
program sets to black at initialisation and which the BASIC prompt leaves
light; a failing capture is retried up to three times.

**World statistics** are computed in-process by assertions over the generated
world, not by image analysis.

**A note on image-derived measurement.** An early attempt to quantify the
diffuse term by comparing mean pixel brightness between regions of a capture
produced differences within noise and supported an incorrect conclusion that
the term was not applied. The measurement averaged across regions containing
different structures, hues and dithered surfaces. Subsequent verification
used in-process assertions on the lighting table and on rendered cell
colours. Image statistics were retained only for whole-frame difference
counts, where the question is binary.

---

## Appendix C: Version History

Thirteen commits, in order. Each message body records the reasoning and,
where applicable, the measured figures.

| # | Commit | Subject |
|---:|---|---|
| 1 | `576c378` | Renderer core: height-field raycaster, procedural block font, generated city |
| 2 | `7d9d05a` | Driving, sprites and the fare: the city moves |
| 3 | `1c095ee` | The Plus/4 build, the bake bridge, and the documentation |
| 4 | `c33d132` | Clean the clippy gate, and put the default seed in one place |
| 5 | `6123bcc` | An autopilot that walks the streets, and a way to record it |
| 6 | `5890ffd` | README: link the downloads the .gitignore promised |
| 7 | `c2f1361` | Lighter rain that stays off the buildings, and real street markings |
| 8 | `531052b` | A generated street system, a larger world, and real variety in the buildings |
| 9 | `beddbb7` | Four layers: a street system, zoning, elevation and a walking network |
| 10 | `b43dd05` | A writeup on ray tracing, and the one lighting term that was missing |
| 11 | `b0c4c98` | Cast shadows from a horizon sweep, and no shadow rays anywhere |
| 12 | `a0e4b79` | Consolidate the two builds onto one set of baked numbers |
| 13 | `335fc58` | make demo4: the Plus/4 build drives itself, and three reasons it did not |

The nomenclature changes of Section IV-C were applied after this sequence and
are recorded in the commit accompanying this document.

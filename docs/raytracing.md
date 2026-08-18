# Ray tracing, and what an ASCII city can afford

The classical formulas, what ASCITTY actually computes, and which of the
things it does not compute are cheap enough to add.

The short version: **the ray tracing is already done, and it is done in a
form that avoids almost every expensive operation the textbook version
needs.** What was missing was not the tracing. It was the *lighting* — until
this document was written there was no light source in the renderer at all
— and adding one turns out to be far cheaper here than it is anywhere else,
for reasons specific to a height field drawn on a character grid.

The diffuse term of §2.1 and the cast shadows of §2.3 are now implemented on
both targets. Between them they cost five numbers per frame, one array read
per surface, and one sweep of the grid when the light moves. The measured
frame time did not change on either target.

---

## 1. The ray equation, and where ours differs

The fundamental equation is the same one everywhere:

```
    P(t) = O + t·D
```

`O` is the origin, `D` the direction, `t` the distance along the ray. This
is exactly what `raycast::column` solves. What differs is three choices, and
all three are about not doing arithmetic.

### 1.1 D is not normalised, and that is the point

A ray is built as

```
    D = dir + plane · camx           camx ∈ [-1, 1] across the screen
```

where `plane` is perpendicular to `dir`, scaled by the field of view. The
component of `D` along `dir` is **exactly one for every column**. So the `t`
the walk reports is already the *perpendicular* distance — the quantity the
projection needs.

Three things fall out:

- **No fisheye correction.** The usual `t · cos(θ)` is unnecessary because
  the parameterisation has done it.
- **No trigonometry in the inner loop.** `dir` and `plane` are computed once
  per frame, not once per column.
- **No square root, ever.** Which brings us to the next point.

### 1.2 We never compute `vlength()`

POV-Ray's `vlength(V)` and `distance(P1, P2)` are the Euclidean norm:

```
    |V| = sqrt(x² + y² + z²)
```

ASCITTY does not evaluate that anywhere on the render path. The distance the
renderer wants is `t`, which comes out of the walk as an accumulation of
additions. A square root is a division-class operation on a host and roughly
two hundred cycles of software on a 7501; the design is arranged so that the
question never arises.

Where a magnitude genuinely *is* needed — sorting billboards by depth, a
speedometer, the contact normal between two cars — the octagonal
approximation is used instead:

```
    |v| ≈ max(|x|,|y|) + 3/8 · min(|x|,|y|)
```

Two comparisons and a shift, accurate to about 4%. That is well inside what
a depth sort or a speedometer needs, and `drive::normalise` and
`Car::speed` use the same ruler so "how far apart are they" and "how fast is
it going" are measured consistently.

### 1.3 The intersection is closed form and costs one addition

The textbook sphere intersection is a quadratic:

```
    (D·D)t² + 2(D·(O−C))t + ((O−C)·(O−C) − R²) = 0
```

Two dot products, a discriminant, and a square root, per ray per object.
Intersection tests are 75–90% of a classical ray tracer's time.

ASCITTY's geometry is axis-aligned unit cells, so the intersection with a
grid plane `x = k` is just

```
    t = (k − Oₓ) / Dₓ
```

and even that division is amortised. The DDA precomputes the spacing between
successive crossings of each family of planes,

```
    Δtₓ = 1 / |Dₓ|          Δt_y = 1 / |D_y|
```

once per column, and then every subsequent crossing is **one addition and
one comparison**:

```rust
if side_x < side_y { dist = side_x; side_x += delta_x; map_x += step_x; }
else               { dist = side_y; side_y += delta_y; map_y += step_y; }
```

On the Plus/4 even the two reciprocals are gone: `reciptab[|D|]` is a
512-entry table of `65536/n`, because a 6502 has no divide instruction and
cc65's software one is upwards of two thousand cycles.

Measured: **7038 grid steps per frame** at 160×48, whole frame **0.16 ms**.
The renderer is not the bottleneck on anything made this century.

### 1.4 It does not stop at the first hit

`trace()` in POV-Ray returns the first intersection. First-hit is enough for
a maze and not enough for a skyline, because what makes a city look like a
city is a tall building seen over a near one.

So the walk continues, front to back, carrying one number — the topmost
screen row anything has claimed — and each further building may only draw
above that line. This is the Comanche voxel-space idea rather than the
Wolfenstein one. It costs the same as first-hit in the common case (a wall
right in front of you closes the column immediately) and it is what produces
a skyline down an avenue.

That single number is also the whole of the hidden-surface problem here.
There is no depth buffer and no sorting.

---

## 2. Lighting: the term that is missing

### 2.0 Where it started

The Phong model is

```
    I = kₐLₐ + 1/(a + bd + cd²) · [ k_d L_d max(L·N, 0)
                                  + k_s L_s max(R·V, 0)^α ]
```

and what ASCITTY computed before this document was

```
    I = luma(building) − fade(d)          fade(d) = 8d / draw_distance
```

An **ambient term and a linear attenuation term**. There was no `L`, no `N`
and no `N·L` anywhere in the renderer. Every face of every building was lit
identically; what varied was the building's own base brightness, its window
occupancy, and how far away it was.

It read better than it had any right to, because a night city really is
mostly self-luminous — the windows *are* the light sources — and because the
distance fade does the depth work. But building corners were expressed by a
pier *glyph* rather than by shading, and that is a drawing, not a lighting
model.

### 2.1 The diffuse term is nearly free in a height field

This is the important observation, and the reason this document exists.

A height field of axis-aligned cells has exactly **five possible normals**:

```
    N ∈ { +x, −x, +y, −y, +z }        four walls and a roof
```

and the renderer *already knows which one it hit* — that is `arch::Face`,
computed from which grid plane the DDA crossed and which way it was going:

```rust
Face::of(vertical, step_x, step_y)
```

For a **directional** light — the moon, which is the only light source this
city would want — `L` is constant across the whole scene. So

```
    L·N        is five numbers.
```

Five. Computed once per frame when the moon moves, from the existing sine
table. Not per pixel, not per hit — per *frame*. At render time the diffuse
term is one array index by a value the renderer has already computed:

```rust
let lambert = LAMBERT[face as usize];        // 0..255, precomputed
let luma = (base as u32 * lambert as u32 >> 8) as u8;
```

Since luminance is a three-bit nibble on both targets, it is cheaper still
to precompute the table as *luminance offsets* — north face −2, moonward
face +1 — and add:

```rust
let luma = (base as i32 + LAMBERT_STEP[face as usize]).clamp(0, 7) as u8;
```

**One table lookup per wall hit and one add per cell.** The lookup is hoisted
out of the per-row loop because every cell of a wall span shares a normal,
which is the whole reason a height field can afford lighting.

#### It is implemented, and here is what it cost

`Atmos::lambert()` returns `[i8; 5]`, computed once per frame from the moon's
bearing. `raycast::render_to` hoists it; `raycast::column` indexes it by
`Face` for walls and by `arch::ROOF` for roofs.

| | Before | After |
|---|---|---|
| Frame time, 160×48 | 0.16 ms | 0.16–0.18 ms |
| Plus/4 `.prg` | 19 795 bytes | 20 137 bytes |
| Lines of Rust | — | about 40, half of them the table |

The host frame time did not move outside measurement noise. The Plus/4
version is four numbers rather than five — there is no top-down view there,
so no roof normal — computed at boot because that build's light does not
move, and the DDA already knows which grid plane it crossed.

#### How much it shows

Honestly: less than the effort suggests, in the views this city produces
most. Looking down a street you see the two *opposing* faces of the
buildings flanking it, so the effect is one wall lighter and the other
darker, and the distance fade is already subtracting more than the light
term adds. Switching it off changes 2.3% of the pixels of a Plus/4 frame.

Where it pays is a building **corner**, where two faces meet in the same
column and the tonal step between them is now lighting rather than a pier
glyph. That is the thing the eye reads as three-dimensional, and it is worth
the forty lines even though a whole-frame pixel count undersells it.

### 2.2 Attenuation: why ours is linear on purpose

The physically correct falloff for a point source is inverse-square:

```
    i(p, p₀) = I(p₀) / |p − p₀|²
```

POV-Ray, and every practical renderer, softens it to

```
    f(d) = 1 / (a + b·d + c·d²)
```

precisely because pure inverse-square produces harsh, high-contrast images —
"objects appear either bright or dark".

ASCITTY has an eight-level luminance ramp. Inverse-square across it would
spend six of the eight levels in the first three cells and leave everything
past that at zero. So `Atmos::fade` is **linear in `d`**, which distributes
all eight levels evenly over the draw distance, and the haze setting scales
the draw distance rather than the curve. That is the `b` term of the
softened model with `a` and `c` at zero, and it is a deliberate choice about
a three-bit quantity rather than an approximation of physics.

A quadratic version is available for the cost of a table: 80 bytes indexed
by whole-cell distance. It is in the backlog as something to *try*, not as a
correction.

### 2.3 Shadows without shadow rays

The classical method is a second trace per light per hit:

```
    cast a ray from the hit point towards the light
    bias the start to avoid surface acne
    if it hits anything with bias ≤ t ≤ 1, the point is in shadow
```

That roughly doubles the cost of the renderer, and the bias term is a
notorious source of artefacts.

**A height field with a directional light does not need any of it.** Shadow
is a horizon problem. Sweep the grid once along the light's ground
direction, carrying a running maximum of

```
    horizon = max(horizon − slope_per_cell, height_here)
```

A cell is in shadow exactly when its height is below the running horizon
when the sweep reaches it. That is **O(cells) once per light direction and
O(1) per lookup at render time** — one byte per cell, or one bit if only
hard shadows are wanted.

For a city of blocks lit by a low moon this gives exactly the right thing:
long shadows down the avenues, sunlit tower tops above a shaded street.

#### It is implemented, on both targets

`shadow::ShadowMap::cast` sweeps once; `City` holds the result and
`City::relight` recasts when the light moves. The renderer reads
`line_at(x, y)` once per wall hit and once per ground sample.

**It stores a height, not a flag**, and that is the whole reason it is worth
doing. A wall is dark below the shadow line and lit above it, which is what
a tower standing behind a nearer tower actually looks like. A bit could not
express that, and the picture would be a city of uniformly black or
uniformly lit buildings.

Recording the horizon *before* folding in the cell's own height is what
stops every building shadowing itself. There is a test for exactly that,
because getting it wrong turns the entire city black and it is the obvious
way to write the loop.

| | |
|---|---|
| Sweep cost | ~2 ms for 234×234, once, at generation |
| Frame cost, host | none measurable (0.18 ms before and after) |
| Frame cost, Plus/4 | one multiply per wall hit, one compare per row |
| Open ground in shade | 48% at a 21° light — asserted in a test |

**The Plus/4 does not sweep anything.** The shadow line is a pure function
of the height field and the light direction, and both are known at bake
time, so `ascitty-bake` emits it as `city_s[]` alongside the heights and the
colours. The machine reads an array. A textbook renderer traces a second ray
per light per hit; this one indexes.

One trap, and both targets fell into it: a shadow line of zero means
*nothing is upstream*, not *shadowed up to the ground*. Without a guard the
"below the line" test darkens the foot of every wall, because the base of a
near building is drawn below the horizon.

#### Still free: the penumbra

The same sweep gives the *soft* version for nothing extra. The stored value
is already a height, so a surface only just below the line is only just in
shadow — grading the offset by how far below rather than switching it gives
the umbra/penumbra distinction that normally requires an area light and a
great many shadow rays. It is a change to one comparison.

### 2.4 Specular, and the halfway vector

Phong's specular term needs the mirror direction

```
    r = 2(L·N)N − L
```

and then `max(R·V, 0)^α`. Blinn's modification replaces it with the halfway
vector

```
    H = (L + V) / |L + V|            and uses  max(N·H, 0)^α
```

which avoids recomputing `r` — but `|L + V|` is a normalise, which is a
square root, which is the thing this renderer is built around not doing.

The saving grace is the same as for diffuse: `N` is one of five, and `V` is
constant *per column* (it is the ray direction, which the walk already has).
So `N·H` is a table of `width × 5` entries, rebuilt when the camera turns.
At 160 columns that is 800 bytes on a host and 200 on the Plus/4.

Whether it is worth it is a different question. A matte brick wall has no
specular highlight worth drawing at one colour per character cell. Where it
would pay is **wet asphalt** and **glass**, which is why it sits below the
diffuse term and the shadows in the backlog.

### 2.5 Reflections are a vertical mirror

The recursive step of a real ray tracer — trace a new ray in direction `r`,
bounded by depth or by contribution — is not affordable here.

But look at what the geometry allows. The ground is a horizontal plane. For
a ray reflecting off it, `r` is the incident direction with its vertical
component negated, and the horizontal component **unchanged**. In screen
terms, for a camera with no roll, that is exactly:

> the reflection of the ground at screen row `horizon + k` is what is drawn
> at screen row `horizon − k` in the same column.

So a wet road reflecting the lit windows above it costs **a second read of a
cell the renderer has already computed**, in the same column, mirrored about
the horizon. No second trace, no recursion, no depth limit. Fade it with
distance and blend it with the puddle mask that already exists.

This is the cheapest large visual win after the diffuse term, and it is the
one that most looks like ray tracing to a viewer.

---

## 3. Shading models, and the ASCII twist

Chapter 5 of the standard treatment distinguishes three:

| | What is interpolated | Cost |
|---|---|---|
| **Flat** | nothing; one colour per polygon | one lighting calculation per polygon |
| **Gouraud** | vertex *colours*, across the face | one per vertex |
| **Phong** | vertex *normals*, then light per fragment | one per fragment |

**ASCITTY is forced into flat shading, per character cell.** A cell has one
foreground colour. There is nothing below it to interpolate a colour across,
so Gouraud and Phong shading of colour are not merely expensive here, they
are meaningless.

Except that they are not, because of one thing this renderer has that a
pixel renderer does not.

### 3.1 The glyph is the interpolation

A character cell is one colour, but it is **eight by eight sub-cells of
shape**. That is 64 bits of intensity information underneath a single
colour, and the font is generated rather than drawn, so any of it is
reachable.

Ordered dithering already uses this for *magnitude*: `catalog::shade(n)`
picks a glyph covering exactly `n/8` of the cell, so intensity between two
luminance levels is representable. That is sub-cell shading already, in one
dimension.

The extension is to use it for **gradient**. The `half_plane` glyph family
exists — sixteen edge directions at several offsets — and `font::moments`
already computes where a glyph's mass sits within its cell. Choosing the
glyph whose centre of mass lies towards the brighter side gives a directional
sub-cell shade: a wall lit from the left picks glyphs weighted left.

This is the ASCII-native answer to Gouraud shading. The colour stays flat
because it must; the *shape* carries the interpolation. It is in the backlog
and it is the most interesting unexplored idea in the renderer.

### 3.2 Mach bands, and why the dither is ordered

Lateral inhibition makes the eye overshoot at an intensity step, so a
sequence of flat bands shows stripes at its boundaries that are not in the
signal. With eight luminance levels across a whole draw distance, ASCITTY
would band badly.

The ordered dither between adjacent levels is the fix, and it has to be
*ordered* rather than diffused for a reason specific to this medium: a
character cell is re-chosen from scratch every frame, and a diffused error
has nowhere to go between frames except into a crawling shimmer. An ordered
matrix is stable — the same intensity always yields the same glyph — so a
wall that is not moving does not sparkle.

---

## 4. Distribution ray tracing: declined, with reasons

Distribution ray tracing takes many jittered samples per pixel and buys
anti-aliasing, soft shadows, depth of field, glossy reflection and motion
blur with one mechanism. Its slogan is that it **turns aliasing into noise**.

That trade is the wrong way round here, twice over.

**On cost.** The Plus/4 has 1000 character cells and 1.76 MHz and currently
manages 1.3 frames a second at one sample per cell. Sixteen samples is not a
quality setting, it is a different project.

**On the medium.** Noise is *worse* than aliasing in a character grid,
because the grid is coarse enough that individual cells are legible. A pixel
renderer's noise dissolves into a texture at normal viewing distance; a
character renderer's noise is a visibly wrong letter, and if it is re-sampled
each frame it crawls. This is the same argument as §3.2 and it is the reason
the entire renderer is deterministic: same city, same camera, same frame,
every time.

The one place the idea earns its keep is the soft-shadow variant, and §2.3
gets that from a horizon sweep without any sampling at all.

---

## 5. What an ASCII city can afford

Costs are per frame at 160×48 on a host, and the Plus/4 column assumes the
assembly inner loop that is at the top of the backlog.

| Technique | Classical cost | Cost here | Host | Plus/4 |
|---|---|---|---|---|
| Ray generation | trig per pixel | 2 mul per column, trig per *frame* | free | free |
| Primary intersection | 75–90% of render time | 1 add + 1 cmp per grid step | free | affordable |
| Perpendicular distance | `vlength`, a sqrt | falls out of the parameterisation | free | free |
| Hidden surface | z-buffer or sort | one integer per column | free | free |
| **Diffuse `N·L`** *(built)* | dot per fragment | **5-entry table, 1 lookup per wall hit** | measured free | measured free |
| **Directional shadows** *(built)* | a second trace per hit | **one O(cells) sweep per light** | measured free | baked, free |
| Attenuation | divide per fragment | 80-byte table | free | free |
| **Ground reflection** | recursive trace | **mirror the column about the horizon** | cheap | affordable |
| Sub-cell gradient shading | n/a | glyph choice from a moment table | cheap | cheap |
| Specular `N·H` | normalise per fragment | `width × 5` table per frame | cheap | marginal |
| Point-light attenuation | per light per fragment | per light per hit, no sqrt | affordable | too slow |
| Sphere primitives | quadratic + sqrt | quadratic + sqrt | affordable | too slow |
| Shadow rays, general | a trace per light per hit | the same | too slow | too slow |
| Radiosity, path tracing | a great deal | a great deal | too slow | too slow |
| Distribution sampling | n× everything | n× everything | possible, unwanted | too slow |

The pattern is not subtle. **Everything that is cheap here is cheap because
the geometry is an axis-aligned height field and the normal set has five
members.** The moment a curved surface or an arbitrary normal enters, the
square roots come back and the advantage is gone — which is the argument for
keeping the world a height field, recorded in
[`adr/0003-height-field.md`](adr/0003-height-field.md).

---

## 6. Where each formula lives, if you want to read the code

| Formula | Code |
|---|---|
| `P(t) = O + t·D` | `raycast::column`, the DDA loop |
| camera plane, `D = dir + plane·camx` | `camera::Camera::plane` |
| grid intersection, `Δt = 1/\|D\|` | `raycast::column`, `delta_x` / `delta_y` |
| perpendicular distance | the `dist` returned by each DDA step |
| projection, rows per unit at distance | `raycast::projection` |
| octagonal norm | `drive::normalise`, `Car::speed`, `sprite::draw_all` |
| the horizon sweep | `shadow::ShadowMap::cast` |
| attenuation | `atmos::Atmos::fade` |
| the five normals | `arch::Face`, `Face::of` |
| ordered dither | `font::BAYER8`, `font::dither`, `catalog::shade` |
| glyph moments | `font::moments` |
| billboard projection | `sprite::draw`, the inverse of `[plane \| dir]` |
| fixed-point arithmetic | `fixed` |

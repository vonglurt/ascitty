# The renderer

`crates/ascitty-core/src/raycast.rs`, and its transcription in
`targets/plus4/src/cast.c`.

## 1. Why not first-hit

The video that started this project describes casting a ray per column and
taking the first thing it hits. First-hit is enough for a maze. It is not
enough for a skyline, because the thing that makes a city look like a city is
a **tall building visible over the top of a near one**.

So the walk does not stop. It keeps going, front to back, carrying one number
— the topmost screen row anything has claimed so far — and each further
building may only draw above that line. When the line reaches the top of the
screen the column is finished and the walk stops.

```
    ceiling ──┐                     ceiling ──┐
              │  ░░░░                         │  ░░░░   far tower,
              │  ░░░░                         ▼  ░░░░   drawn above
    ▓▓▓▓▓▓    │                        ▓▓▓▓▓▓░░░░░░
    ▓▓▓▓▓▓    │                        ▓▓▓▓▓▓░░░░░░
    ▓▓▓▓▓▓ near, closes                ▓▓▓▓▓▓
    ──────    down to here             ──────
```

That is the Comanche voxel-space idea rather than the Wolfenstein one. It
costs the same as first-hit in the common case — a wall right in front of you
closes the column immediately — and produces a real skyline when you are
looking down an avenue.

The span each hit draws is clamped at *both* ends: `[top, min(bottom,
ceiling - 1)]`. Clamping the bottom as well as the top is what lets the
ground show through the gap between two buildings.

## 2. Why there is no per-column cosine

A ray is

```
    rayDir = dir + plane × camx        camx ∈ [-1, 1] across the screen
```

where `plane` is perpendicular to `dir` and scaled by the field of view. The
component of that vector along `dir` is **exactly one for every column**, so
the distance the DDA reports is already the perpendicular distance. No
fisheye correction, no trigonometry in the inner loop, and — on the target —
the two products `plane × camx` stay inside a 16-bit integer, which is the
whole reason the field of view is expressed as a plane half-width rather
than as an angle.

## 3. Where the divisions are

Three, and each is dealt with differently.

**The DDA's step distances**, `1/|rayDir|`. Two per column. On the host, a
fixed-point divide. On the target, `reciptab[|rayDir|]` — a 512-entry table
of `65536/n`, because the 6502 has no divide instruction and cc65's 32-bit
one is upwards of two thousand cycles. 512 and not 256: the component
reaches 1.0 for the direction plus 0.67 for the plane, about 427, so a
256-entry table would run off the end for every ray in the outer third of
the screen.

**The perspective divide**, rows per world unit at a distance. One per wall
hit, hoisted out of the per-row loop: height falls by a constant `dz` per
row down the screen, so the loop is a subtraction. On the target it is
`projtab[dist]`, built at boot.

**The ground distance**, which depends only on how far a row is below the
horizon — *not* on which column you are in. So it is a per-frame table of
`h/2` entries, which is what turns floor casting from a division per cell
into a division per row.

## 4. The passes, in order

1. **Sky and ground**, per column. Sky is black with stars hashed from the
   column's compass bearing, so they are fixed to the world and the same
   stars come back when you turn around. Ground is cast from the row table.
2. **The moon**, so it is behind the buildings that stand in front of it.
3. **The buildings**, per column, overwriting.
4. **Sprites**, clipped against the per-column wall depth the walk left
   behind.
5. **Rain**, last, because it is in front of the traffic as well as in front
   of the buildings.

`render_to` does 1–3 and records the depths; `render` is the convenience
wrapper that also does 5. Anything with sprites wants the former, so the
billboards go on before the weather rather than under it.

## 5. Sprites

Standard billboard projection: invert the `[plane | dir]` matrix to get the
sprite into camera space, where the second component comes out as the
perpendicular distance in the same units the wall depths are in — which is
what makes the comparison meaningful.

Clipping is per column, not per cell. That is the usual approximation and is
wrong only when something should be visible over the top of a *near*
building, which for a hydrant it never is.

Billboards are sorted furthest-first with an octagonal norm — `max + 3/8 ×
min` — rather than a squared distance, which would overflow Q16.16 at city
scale. Sorting only needs an ordering.

## 6. The character aspect ratio

`CELL_ASPECT = 2`: a character cell is twice as tall as it is wide. Every
terminal and the Plus/4 alike are close enough that one constant covers
both. Getting it wrong does not distort the picture so much as make every
building the wrong proportion, which is worse.

Rows per world unit at unit distance is therefore

```
    proj = w / (4 × fov)
```

— the horizontal figure `(w/2)/fov` divided by the aspect.

## 7. What the frame costs

On the host, at 160×48 with about 3200 grid steps a frame: **0.14 ms**, which
is roughly 7000 frames a second. The renderer is not the bottleneck on
anything made this century; the terminal is.

On the Plus/4, at 40×25: about **1.3 frames a second**. See
[`plus4.md`](plus4.md).

# ASCITTY — Architecture

One renderer, two machines, and a generator in between.

---

## 1. The shape

```
                    ┌──────────────────┐
                    │   ascitty-core   │   the renderer
                    │  no I/O, no clock│   Rust, zero dependencies
                    └────────┬─────────┘
                   ┌─────────┴──────────┐
                   ▼                    ▼
          ┌────────────────┐   ┌──────────────────┐
          │  ascitty-tty   │   │  ascitty-bake    │
          │ a colour       │   │ writes C headers │
          │ terminal       │   └────────┬─────────┘
          └────────────────┘            ▼
                              ┌──────────────────┐
                              │ targets/plus4    │  C, via cc65
                              │ .prg  and  .d64  │
                              └──────────────────┘
```

`ascitty-core` owns the number system, the city, the camera, the glyph
catalogue, the per-frame cast, the driving physics and the weather. It
touches no terminal, no file and no clock. Everything around it decides only
where the bytes go.

The 6502 does not run that code — it runs a transcription of it against
tables the crate generated. What crosses the gap is the *arithmetic*: the
same operations in the same order, at Q16.16 on one side and Q8.8 on the
other. A frame that disagrees is a bug rather than a difference of opinion
about floating point.

## 2. Why there is a bake step

The alternative is two copies of everything: a character set drawn twice, a
city generator written twice, a decision about what colour a building is
made twice. Both copies start correct and one of them drifts.

So `ascitty-bake` runs the *real* generator and writes out what it produced:

| Header | Bytes | What |
|---|---:|---|
| `charset.h` | 1024 | the 128 glyphs, in TED bit order |
| `trig.h` | 512 | sine over one turn, Q8.8, 256 entries |
| `recip.h` | 1024 | 65536/n, for the DDA's step distances |
| `city.h` | 12288 | a 64×64 district: heights, colours, tiles |
| `glyphs.h` | — | the catalogue's names as `#define`s |

Nothing in `targets/plus4/gen` is committed. `make bake` runs before every
target build, and the generator is the definition.

## 3. The renderer

Detailed in [`renderer.md`](renderer.md). The short version:

**It is a height field, walked front to back, and it does not stop at the
first hit.** First-hit is enough for a maze; a skyline needs a tall building
seen over the top of a near one. So each column carries one number — the
topmost screen row anything has claimed — and each further building may only
draw above that line. When the line reaches the top the column is finished.

**A ray is `dir + plane × camx`**, where the plane is perpendicular to the
direction. The ray's component along the direction is exactly one for every
column, so the distance the DDA reports *is* the perpendicular distance. No
fisheye correction and no trigonometry in the inner loop.

**Ground distance depends only on the row**, not the column, so it is a
per-frame table of `h/2` entries rather than a division per cell.

## 4. What a glyph is

Detailed in [`glyphs.md`](glyphs.md). Nothing in this program ships a
picture of a character. A glyph is an 8×8 bitmap produced by a *function* —
a dither level, a quadrant mask, a half-plane, a window bay — and the
catalogue is that function evaluated over its parameters.

The renderer emits *catalogue indices*, and each target renders an index its
own way:

| Target | How an index becomes a shape |
|---|---|
| Plus/4 | screen code `index`, from the baked character set in RAM |
| Terminal, ASCII | the typeable character in `glyph::ASCII` |
| Terminal, Unicode | the block element in `glyph::UNICODE` |

The Plus/4 mapping is the identity, and the colour byte is already in the
TED's own packing, so on the machine the renderer's output byte *is* the
hardware byte.

## 5. Colour, and the depth cue

The TED gives 16 hues at 8 luminances. That layout is a gift to a renderer
that wants depth: **hold the hue, drop the luminance.** A blue tower stays
blue as it recedes and simply gets darker, which is what a night city does,
and it costs one subtraction rather than a colour-space interpolation.

So the shading model is the Plus/4's and the terminal emulates *it*, not the
other way round. `palette::to_rgb` converts a TED colour byte to sRGB once,
into a 128-entry table, and the ANSI painter reads that.

## 6. The three cameras

| Mode | What it is | Constraints |
|---|---|---|
| Walk | eye height, 1.8 m | cannot leave the pavement |
| Drive | third person, behind the taxi | boom shortens out of walls |
| Copter | free flight above the roofline | floor at the tallest building |

The camera is one struct in all three; what differs is who moves it. Walk
calls `Camera::walk`, which slides along walls rather than stopping at them.
Drive runs the physics and then puts the camera behind the result, with the
heading *lagging* the car's so a drift is watched from outside the spin.
Copter ignores buildings horizontally, because it is above them.

## 7. The simulation

[`driving.md`](driving.md) covers the physics. The layer above it holds:

- **Street furniture** — about four hundred items, generated deterministically
  from cell coordinates so restarting a shift does not move the lamp posts.
  Frangible: they go over, take a velocity and a lean, and stay down. None of
  it slows the car.
- **Traffic and pedestrians** — a fixed-size pool that is *recycled*: anything
  more than a few blocks behind you is picked up and put down ahead. A fixed
  pool is the shape a 64 KB machine can also run, which is why it is the
  shape here.
- **The fare** — a pickup, a destination, a trail of coins along a Manhattan
  path between them, and a clock that only ever gains time from those three
  things. There is no other source, so the only way to keep playing is to
  keep moving.

## 8. The Plus/4 half

[`plus4.md`](plus4.md) in full. What changed, and why:

- **Q16.16 became Q8.8.** A cell coordinate needs eight bits of integer for a
  64-cell district; cc65 does 16-bit arithmetic in about a dozen cycles and
  32-bit in hundreds.
- **Division became a table.** The 6502 has no divide instruction.
- **The projection scale is Q4.4, not Q8.8.** A sixty-unit tower one cell
  away projects to nine hundred rows; at Q8.8 that product overflows an
  `int` and the tower draws upside down.
- **The district is 64 cells, and it must be a power of two.** `y * 48 + x`
  is a call to cc65's software multiply on every step of every column of
  every frame. `(y << 6) | x` is free.
- **Floor casting was dropped**, and there are no sprites. Both are in the
  backlog behind an assembly inner loop.

Measured: about **1.3 frames a second**. Honest, and the reason the top of
the backlog is `cast.s`.

## 9. Testing

131 tests, about a tenth of a second. They are written to fail rather than
to pass — three real bugs were found by tests written before the code was
believed:

- The two-body collision impulse applied mass twice, so a taxi at 40 mph
  moved a parked car about a foot.
- The yaw-rate filter was written `spin = spin * 7/8 + turn`, which looks
  like smoothing and is a filter with a gain of eight. Every steering input
  spun the car like a top.
- The block scanner could fail to advance, and generated cities of certain
  street periods hung forever.

The gate is `make check`: tests, both builds, the tables regenerated, a host
frame rendered, and the target booted in the emulator to confirm it still
draws something. A `.prg` that compiles is not evidence.

## 10. Decisions worth not re-litigating

- [0001](adr/0001-fixed-point.md) — fixed point everywhere
- [0002](adr/0002-rust-and-cc65.md) — Rust for the host, C via cc65 for the 6502
- [0003](adr/0003-height-field.md) — a height field, not a set of boxes
- [0004](adr/0004-plus4-charset.md) — 128 glyphs, and no text on the Plus/4
- [0005](adr/0005-no-dependencies.md) — no third-party crates

# Contributing

## The gate

```sh
make check
```

Tests, both builds, the tables regenerated, a host frame rendered, and the
target booted in the emulator to confirm it still draws something. If it
passes, the repository is in a state somebody else can clone.

**A `.prg` that compiles is not evidence.** The most common way to break the
Plus/4 build is to change something in `ascitty-core` that the bake step
carries across, and find out three weeks later that the machine has been
drawing a black screen. `make check` boots it.

## How to think about a change

### The core owns everything, and touches nothing

`ascitty-core` has no I/O, no clock, no terminal and no randomness that is
not seeded. That is what lets the same code answer for both targets and what
makes the tests instant. If a change wants to read the time or print
something, it belongs in `ascitty-tty`.

### Nothing gets a second copy

If the Plus/4 needs to know something the host already knows — a glyph, a
table, a city, a colour — it gets it from `ascitty-bake`, not from a
hand-written constant that happens to match. Two copies start correct and one
drifts.

Nothing in `targets/plus4/gen` is committed, for the same reason.

### Fixed point, everywhere

No floating point on any render path. `fixed::to_f32` and `from_f64` are for
diagnostics, tests and table generation only. See
[`docs/adr/0001-fixed-point.md`](docs/adr/0001-fixed-point.md) — the reason
is not performance, it is that a renderer written twice in two number systems
is a renderer whose halves cannot be diffed.

Watch overflow. The one that has already bitten: a sixty-unit tower one cell
away projects to nine hundred rows, and nine hundred rows in the wrong
fixed-point format wraps a 16-bit integer and draws the tower upside down.

### No new dependencies

See [`docs/adr/0005-no-dependencies.md`](docs/adr/0005-no-dependencies.md).
If you need one, that is a conversation, not a commit.

## Tests

Write them to fail. Three real bugs were caught by tests written before the
code was believed:

- the two-body collision impulse applied mass twice, so a taxi at 40 mph
  moved a parked car about a foot
- the yaw-rate filter was written `spin = spin * 7/8 + turn`, which looks
  like smoothing and is a filter with a gain of eight — every steering input
  spun the car like a top
- the block scanner could fail to advance, and hung the whole suite

A test that asserts the code does what the code does is worse than no test,
because it has to be updated every time and it never fails. A test that
asserts a *property* — the setback steps inward, the ASCII table stays inside
seven bits, the fire escape does not move when the camera does, the car never
ends up inside a building — keeps earning.

Name them as sentences. `a_taxi_scatters_a_parked_car_and_barely_moves_a_bus`
tells you what broke without opening the file.

## Comments

Explain the *why*, especially where the code looks odd and is right.

The ones already there that were worth writing: why the heading is rotated
after the velocity is recombined; why the dither is ordered rather than
diffused; why the facade tile is chosen per building and the lit window per
window; why the reciprocal table has 512 entries and not 256; why the Plus/4
district must be a power of two.

None of those are obvious from the code, and all of them would be
reintroduced as bugs by somebody tidying up.

## Style

Rust: whatever `cargo fmt` does, four spaces, `#![warn(missing_docs)]` on the
core and it is not turned off.

C: the surrounding style — four spaces, `/* */` comments, `**` continuation,
declarations at the top of a block, because cc65 is a C89 compiler and will
tell you so.

Commit messages: a subject line that says what changed, then prose that says
why. If a bug was found, say what the symptom was — that is the part that is
useful in three years.

## Where things live

```
crates/ascitty-core/src/
├── fixed.rs     Q16.16
├── trig.rs      angles and the sine table
├── rng.rs       xorshift32 and a stateless hash
├── palette.rs   the TED's 16 hues x 8 luminances
├── font.rs      the glyph generators
├── catalog.rs   the 128 shapes
├── glyph.rs     ASCII and Unicode stand-ins
├── frame.rs     the two-bytes-a-cell buffer
├── world.rs     streets, blocks, lots, archetypes
├── arch.rs      what is at a point on a wall
├── camera.rs    where you are standing
├── raycast.rs   the walk
├── sprite.rs    billboards, as ASCII art
├── drive.rs     arcade physics
├── sim.rs       furniture, traffic, people, the fare
└── atmos.rs     rain, moon, stars, haze
```

`crates/ascitty-tty` is the terminal front end — `term.rs` for raw mode and
keys, `paint.rs` for ANSI, `hud.rs` for the status line, `main.rs` for the
loop and the three camera modes.

`crates/ascitty-bake` is the bridge. `targets/plus4` is the machine.

## Filing something you are not going to build

Put it in [`docs/backlog.md`](docs/backlog.md), with the reasoning. An entry
that says what was considered and why it was not done is worth more than a
shorter list.

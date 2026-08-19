# ASCITTY — documentation index

A raytraced ASCII city, for colour terminals and for a Commodore Plus/4.

This page is the map. Every other document is reachable from here, and
nothing important lives only in a commit message.

## Start here

| If you want to | Read |
|---|---|
| Run it | [`../INSTALL.md`](../INSTALL.md) |
| Know what it is and what it does | [`../README.md`](../README.md) |
| Understand how it works | [`architecture.md`](architecture.md) |
| Set up a machine to work on it | [`dev-setup.md`](dev-setup.md) |
| Know what is coming | [`backlog.md`](backlog.md) |
| Change something | [`../CONTRIBUTING.md`](../CONTRIBUTING.md) |

## The documents

### [`lab-report.md`](lab-report.md)
The design and measurement record, in IEEE form: origin, method of
construction, the naming analysis, architecture, results, the four defects
that survived visual inspection, and the prompt corpus the system was
specified from. Start here if you want to know *why* rather than *how*.

### [`architecture.md`](architecture.md)
How the whole thing is put together: the one renderer, the two targets, the
bake step that joins them, and the reasoning behind the shape. Start at §1
if you have never seen the repository before.

### [`glyphs.md`](glyphs.md)
The procedural block font. What a glyph is, how one is generated rather than
drawn, how the catalogue is laid out, what dithering means here, and how the
same 128 shapes become a character set on the Plus/4 and printable ASCII on
a terminal.

### [`camera.md`](camera.md)
The three camera modes and how each is driven, why pitch is measured in
screen rows, and the autopilot that walks the streets and looks around on its
own — including how to record it as an animation.

### [`raytracing.md`](raytracing.md)
The classical ray-tracing and shading formulas, what this renderer actually
computes, and which of the things it does not compute are cheap enough to
add. Includes a costing table of what an ASCII city can afford. Read this
before proposing a lighting feature.

### [`renderer.md`](renderer.md)
The height-field walk in detail — why it does not stop at the first hit,
where the perspective divide happens, why there is no per-column cosine, and
the arithmetic that has to be identical on both targets.

### [`city.md`](city.md)
What a city is made of, in four layers: the generated street system, the
zoning that says what ground is for, the elevation map, and the pedestrian
network. Then blocks, lots, the six building archetypes, setbacks, window
zippers and fire escapes.

The four layers are separate structures on purpose, and the module each
lives in says why:

| Layer | Module | Answers |
|---|---|---|
| Roads | [`world::Plan`](../crates/ascitty-core/src/world.rs) | where the streets are, how wide, and what class |
| Mapping | [`zone`](../crates/ascitty-core/src/zone.rs) | what this ground is *for* |
| Elevation | [`elevation`](../crates/ascitty-core/src/elevation.rs) | how high the ground is and what stands on it |
| Walking | [`walk`](../crates/ascitty-core/src/walk.rs) | where a person on foot may be, and how they get about |

### [`driving.md`](driving.md)
The arcade physics: the four properties that are modelled, the three that
deliberately are not, and the update ordering that produces the drift.

### [`plus4.md`](plus4.md)
The Commodore Plus/4 build. What the machine gives you, what it takes away,
what had to change, and what the measured numbers are.

### [`dev-setup.md`](dev-setup.md)
Homebrew, Rust, cc65, VICE. The proposed workflow, the make targets, and
what to run before pushing.

### [`backlog.md`](backlog.md)
Everything not built yet, ordered, with the reasoning kept. This is the
living document; it is where an idea goes when it arrives mid-flight.

### [`adr/`](adr/)
Architecture decision records — the choices that would otherwise be
re-litigated every few months.

| # | Decision |
|---|---|
| [0001](adr/0001-fixed-point.md) | Fixed point everywhere, not floating point |
| [0002](adr/0002-rust-and-cc65.md) | Rust for the host, C via cc65 for the 6502 |
| [0003](adr/0003-height-field.md) | A height field, not a set of boxes |
| [0004](adr/0004-plus4-charset.md) | 128 glyphs, and no text on the Plus/4 |
| [0005](adr/0005-no-dependencies.md) | No third-party crates |

## The pictures

`make shot` regenerates all of these from the current build.

| File | What it is |
|---|---|
| [`media/walk-ascii.txt`](media/walk-ascii.txt) | Street level, 7-bit ASCII |
| [`media/walk-blocks.txt`](media/walk-blocks.txt) | The same, in block elements |
| [`media/drive-ascii.txt`](media/drive-ascii.txt) | Behind the taxi |
| [`media/copter-blocks.txt`](media/copter-blocks.txt) | Above the roofline |
| [`media/tour-strip.txt`](media/tour-strip.txt) | the autopilot's walk, sampled every few seconds |
| [`media/glyph-sheet.txt`](media/glyph-sheet.txt) | All 128 glyphs, as bitmaps |
| [`media/plus4.png`](media/plus4.png) | The Plus/4 build, in VICE |

## The layout of the repository

```
ascitty/
├── crates/
│   ├── ascitty-core/     the renderer.  No I/O, no clock, no terminal.
│   ├── ascitty-tty/      the terminal front end
│   └── ascitty-bake/     generates the 6502 build's tables from the core
├── targets/
│   └── plus4/
│       ├── src/          the Commodore Plus/4 program, in C
│       └── gen/          baked headers - generated, never committed
├── docs/                 you are here
├── tools/                scripts the Makefile calls
└── build/                artifacts
```

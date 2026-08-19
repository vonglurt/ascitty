# The Commodore Plus/4 build

`targets/plus4/`, built by `cl65`, output `build/ascitty.prg` and
`build/ascitty.d64`.

![The Plus/4 build running in VICE](media/plus4.png)

## 1. What the machine gives you

**A character set in RAM.** The TED can take character definitions from RAM
instead of ROM, so the 128 procedurally generated glyphs are copied to
`$7000` at boot and the machine draws shapes no PETSCII set contains — at no
per-frame cost, because they are still just screen codes.

**121 colours, laid out as 16 hues × 8 luminances.** That layout is the
reason the depth cue in this renderer is "hold the hue, drop the luminance":
it is one subtraction on a nibble. The host emulates the TED's palette
rather than the other way round.

**Two parallel 40×25 byte matrices**, screen codes at `$0C00` and colour at
`$0800`, exactly `$0400` apart. A colour byte is `luminance << 4 | hue`,
which is the packing `ascitty-core::palette` already uses — so a colour that
comes out of the renderer goes straight into colour RAM with no conversion.

## 2. What it takes away

**The alphabet.** When characters come from RAM the set is 1 KB — 128
definitions — and bit 7 of a screen code becomes a reverse-video flag rather
than an address bit. Installing the custom set costs the ROM font, so this
program has no text on screen at all. 128 shapes of city are worth more than
a status line. See [`adr/0004`](adr/0004-plus4-charset.md).

**Division.** The 6502 has no divide instruction and cc65's software one is
upwards of two thousand cycles for 32 bits.

**Multiplication**, for that matter. `y * 48 + x` is a call.

**32-bit arithmetic.** cc65 does 16-bit in about a dozen cycles and 32-bit in
hundreds.

## 3. What changed from the host renderer

The algorithm did not change. `cast.c` is `raycast.rs` transcribed: a DDA per
column, front to back, past the first hit, carrying the topmost claimed row.

What changed:

| Host | Target | Why |
|---|---|---|
| Q16.16 | Q8.8 | a 64-cell district needs 8 bits of integer; the camera fits in `int` |
| fixed-point divide | `reciptab[n]`, 512 entries | no divide instruction |
| Q16.16 projection | **Q4.4** projection | a 60-unit tower one cell away is 900 rows; at Q8.8 that product overflows `int` and the tower draws upside down |
| 128×128 city | **64×64** district, baked | 12 KB fits; 48 KB does not |
| any grid size | a **power of two** grid | `(y << 6) \| x` instead of a software multiply on every DDA step |
| floor casting | ground drawn as shaded bands | roughly halved the frame rate |
| sprites | none yet | there is no frame rate to spend |

Everything that could be pulled out of a loop was. The trigonometry and the
camera plane are computed once per frame, not once per column. The per-column
camera-plane offset, the ground row colours and the star positions are boot-
time tables. The sky and ground are walked with the two video pointers rather
than indexed, because `SCREEN[y * SCR_W + sx]` looks harmless and is a
software multiply — forty per column, a thousand per frame, for an address
the previous iteration already knew.

## 4. Tearing

A frame takes longer than the raster, so the display always catches the
screen part-way through.

The obvious structure — clear the whole screen, then draw all the columns —
tears horribly, because the part the display catches is *blank*: half a city
and half an empty street, every frame. Doing one column completely before
starting the next means a torn frame shows old city on one side and new city
on the other, which reads as a wipe rather than as a fault.

That is an improvement, not a fix. Two screen matrices and a `$FF14` flip
would fix it properly and the machine has the RAM; it is near the top of the
backlog.

## 5. The measured frame rate

**About 2.6 frames a second**, at 40×25 — up from 0.93 after the table work
below.

### What the tables bought

Everything the machine used to work out at boot now comes from
`gen/tables.h`, which `ascitty-bake` writes from the same figures the host
renderer uses: the projection scale per distance, the haze ramp, the
camera-plane offset per column, the ground colours, the star field, the
field of view and the light bearing. That removed 337 divisions and modulos
from boot, but the reason it matters is correctness — they were a second
copy of formulas the host already has, and the two could drift.

The speed came from somewhere else. The DDA's innermost line was

```c
    city_h[(my << CITY_SHIFT) | mx]
```

and that shift is a 16-bit value shifted six times, on every step of every
column of every frame. A table of row base pointers — built at boot, because
the addresses are not known until the linker has run — turns it into an
index off a pointer.

| | cycles/frame | fps |
|---|---:|---:|
| Before | 1 910 156 | 0.93 |
| Baked tables + row pointers | 687 500 | **2.58** |

Measured with `tools/frametime.sh`, which builds a variant that renders
exactly *N* frames and then turns the border white, and bisects the cycle
budget at which that happens. Two runs at different *N* cancel the boot
cost, which is otherwise most of the answer.

### One that did not work

The side-distance set-up is still a 32-bit multiply. It was briefly a pair
of quarter-square products — `a*b = (a+b)²/4 − (a−b)²/4`, two reads of a
baked table and a subtraction, which is the classic way to multiply on a
machine that cannot. The algebra checks out and the screen came up blank:
every column bailed out of the walk immediately.

It is worth recording *how* that nearly got shipped. The first measurement
of the quarter-square build said **5.4 fps**, which looked like a triumph and
was an artefact: the columns were terminating early, so the renderer was
doing a fraction of the work. A frame-rate number from a build whose output
has not been looked at is not a measurement of anything.

`tools/viceshot.sh` boots and screenshots; `tools/frametime.sh` measures. Note `-autostartprgmode 1`, which injects
the program straight into memory: loading a 19 KB program through an emulated
1541 takes about ninety seconds of machine time, and the screenshot catches
the `LOADING` message rather than the city.

## 5a. The lighting is four numbers

A directional light on a height field costs almost nothing here, and the
argument is worth reading in full in [`raytracing.md`](raytracing.md) §2.1.

The DDA already knows which grid plane it crossed and which way it stepped,
so it knows which of the four wall normals is facing the ray. The light does
not move in this build, so `N·L` for each of them is computed once at boot
into `lambert[4]` and applied as a luminance offset — one array index and
one addition, hoisted out of the per-row loop because every cell of a wall
span shares a normal.

Cost: 342 bytes of program and no measurable frame time. A textbook renderer
evaluates a dot product per fragment; there is nothing left of one here.

## 5b. The shadows are baked

Cast shadows, for nothing at all at runtime.

The shadow line over a height field lit by a directional source is a pure
function of the heights and the light bearing — see
[`raytracing.md`](raytracing.md) §2.3 — and both are known when
`ascitty-bake` runs. So the sweep happens on a laptop and the machine gets
`city_s[]`, one byte per cell of district, alongside the heights and the
colours.

At render time it is one multiply per wall hit to project the shadow line to
a screen row, and one comparison per row to decide which side of it a cell
is on. The wall above the line is lit and the wall below it is not, which is
what a tower standing behind a nearer tower looks like.

Cost: 4 KB of baked data, and the program grew to 24 384 bytes.

## 5c. The attract mode

`make demo4` boots the program in `xplus4` and leaves it alone. It drives
itself from boot and stops the moment a key is touched.

The autopilot is the host's idea cut down to what a 7501 can spare: walk
until something is in the way, turn at the junction, keep to the right of
the road. It reads the city rather than following a path, because a path
baked for one district is wrong for every other seed. No trigonometry beyond
the two table reads the walk already does, and no state but a heading and a
target.

Three things it had to learn, each of which showed up as "the demo is
looking at nothing":

**Follow roads, not open ground.** Parks and plazas are unbuilt cells in the
middle of a block, and a walker that treats them as passable strolls into
one and spends the rest of the demo pressed against the back of a building.
The district carries no cell kind, but it carries the tile each cell is
drawn with, and only carriageway is drawn in asphalt — so the renderer's own
data answers the question.

**The leash is a hard boundary, not a preference.** Steering back towards
the middle only works when there is a road pointing that way. When there is
not, the walk carries on outwards to the edge of the district, where most of
the view is off the end of the world. Refusing the move instead makes the
boundary behave like a wall, which is the behaviour that was wanted: the
walk turns at it. This took three attempts — a preference, then a shorter
preference, then a constraint — and only the constraint worked.

**The district is chosen by content.** See below.

## 5d. Which 64×64 to bake

It used to be "the middle of the map", on the reasoning that downtown is in
the middle. Downtown is in the middle — and so is the crossing of the two
arterials, which are twelve to sixteen cells wide each. The baked district
came out with a thirteen-row band of empty carriageway straight through it,
33% built, and the attract mode spent half its time looking down it at
nothing.

The window was then chosen by *content*: a summed-area table over "is there a
building here" makes scoring a candidate constant-time, so every offset is
tried rather than sampled, and the densest one won. That took the district
from 33% built to 57%, and the attract mode from three frames in six with
something in shot to six in six.

It also looked worse than what it replaced, which took a side-by-side
comparison of the two screenshots to see. The densest 64×64 of a city is a
solid block of towers, so the camera boots hard against a wall: at forty
columns a near facade is one flat colour across half the screen, and the
frame carries no sky, no distance and no second building to compare the
first with. The earlier build's frame was liked *because* the arterial
crossing put everything far away, small and full of window texture.

So the score is now content **multiplied by the length of the longest
straight run of carriageway out of the district's centre**, and that run is
probed all the way to the draw distance rather than to some shorter figure —
a twenty-cell street sounds long and is not, because the target draws forty
and a tower at twenty-one closes the end of it. Multiplied again by the width
of that street, because depth alone gives an alley: two walls four metres
away and nothing between them.

That is what "size the world to the resolution" means here. The district is
64 cells either way; which 64 has to be decided by what fits in forty
columns, not by what is densest.

## 5e. Where the camera starts

Three numbers in `gen/tables.h`: `START_X`, `START_Y`, `START_A`.

The machine used to spiral out from the middle of the district looking for a
road cell, then probe four directions for the longest street. Both are pure
functions of data the bake already holds, so both moved into it. The target
reads three numbers, and the boot code lost 621 bytes.

The boot-time version was not slow enough to matter on its own. What it was
is *worse*: it could only see 24 cells, which is not far enough to tell a
street that runs off into the haze from one a tower closes at 25, so the
first frame anybody saw was a facade across the middle of the screen with the
actual street off to one side.

## 6. What is next

A hand-written `cast.s` for the DDA and the wall fill. The C is already
written to avoid what cc65 is bad at, and the remaining cost is in the code
cc65 generates for those two loops. That is the next factor of three to five,
and it is exactly the move this codebase's sibling made for the same reason.

## 7. Memory

```
$1001   program, tables and the baked district      about 27 KB
BSS     the character set, aligned to 1 KB at boot
$0C00   screen matrix
$0800   colour matrix
        C stack grows down from HIMEM
```

`$7000` is above everything cc65 puts in the program and well below the
stack. It has to be 1 KB aligned, because `$FF13` holds address bits 15–10.

## 8. Putting the machine back

`Q` restores the ROM character set before returning to BASIC. Without it the
`READY.` prompt is drawn in dither patterns and fire escapes, which is funny
once.

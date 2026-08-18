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

**About 1.3 frames a second**, at 40×25.

Measured rather than estimated, and the method matters because warp mode
makes wall-clock meaningless. A build that renders exactly *N* frames and
then sets the border white is run under `-limitcycles`, and the cycle count
at which the border turns is found by bisection:

- boot, injection and the character-set copy complete under **15 million**
  cycles
- fifty frames complete between **75 and 80 million**

which is roughly 1.3 million cycles a frame against a PAL clock of
1 773 447 Hz.

`tools/viceshot.sh` is the harness. Note `-autostartprgmode 1`, which injects
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

## 6. What is next

A hand-written `cast.s` for the DDA and the wall fill. The C is already
written to avoid what cc65 is bad at, and the remaining cost is in the code
cc65 generates for those two loops. That is the next factor of three to five,
and it is exactly the move this codebase's sibling made for the same reason.

## 7. Memory

```
$1001   program, tables and the baked district      about 19 KB
$7000   the character set, 1 KB, aligned            copied at boot
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

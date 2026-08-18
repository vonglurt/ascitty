# 0004 — 128 glyphs, and no text on the Plus/4

**Status:** accepted

## Context

The Plus/4's TED can fetch character definitions from RAM instead of ROM,
which is what lets this program draw shapes the Commodore character ROM does
not contain.

When it does, it reads a **1 KB** character set: 128 definitions of eight
bytes. Bit 7 of a screen code becomes a reverse-video flag rather than an
address bit, so codes 128–255 are the inverses of 0–127 and there is no
second half to put anything in.

Installing a custom set therefore costs the ROM alphabet.

## Decision

The catalogue is **exactly 128 glyphs**, all of them city. The Plus/4 build
has no text on screen: no status line, no score, no menu.

The base screen code is **0**, so the mapping from catalogue index to screen
code is the identity.

## Consequences

The renderer's output byte *is* the screen byte, and — because
`ascitty-core::palette` packs colour the way the TED does, `luminance << 4 |
hue` — the colour byte is the hardware byte too. The machine has no glyph
selection cost and no colour conversion cost at all. On a 1.76 MHz machine
drawing a thousand cells a frame, that is not a rounding error.

It also forces a useful discipline on the catalogue: 128 is the budget, so
every slot has to earn itself. A test asserts `N_GLYPHS == 128` and
`N_GLYPHS * 8 == 1024`, because "we can always add one more" is exactly the
thought that would break the build in a way nobody would notice until the
character set silently wrapped.

### The alternatives that were considered

**Keep the ROM font and use only the graphics characters.** This is what
most Commodore programs do, and it is why most Commodore programs look alike.
The whole premise here is a font generated to suit the picture.

**Carve out 32 slots for a small alphabet.** Sixteen digits and a few letters
would fit. It was rejected because a 40-column status line drawn in a 3×5
font on a screen with no other text is worse than no status line, and
because the four glyph families that would have to go — the moon, the halo,
the slopes, the haze — are all things the picture uses.

**Two character sets, flipped per raster line.** Possible, and the sibling
project does raster tricks. Not for a first version.

The host build has a full status line, so nothing is lost there.

/* ------------------------------------------------------------------------
 * plus4.h - the machine, as ASCITTY uses it.
 *
 * Text mode on the Plus/4 is two parallel 40x25 byte matrices: screen codes
 * at $0C00 and colour at $0800, exactly $0400 apart.  A colour byte is
 * luminance in the high nibble and hue in the low, which is the packing the
 * Rust core already uses - so a colour that comes out of the renderer goes
 * straight into colour RAM with no conversion.
 *
 * THE CHARACTER SET
 *
 * TED can take character definitions from RAM instead of ROM, and when it
 * does it reads a 1 KB set: 128 definitions of eight bytes, with bit 7 of a
 * screen code becoming a reverse-video flag rather than an address bit.
 * That is why the catalogue is exactly 128 glyphs, and it is also why this
 * program has no text on screen: installing the set costs the alphabet, and
 * 128 shapes of city are worth more than a status line.
 *
 * $FF12 bit 2   0 = characters from RAM, 1 = from ROM
 * $FF13 bits7-2 character base, address bits 15-10
 *
 * The set lives at $7000, which is above everything cc65 puts in this
 * program (about 20 KB of code, tables and city) and well below the C stack
 * that grows down from HIMEM.  It is 1 KB aligned, which the register
 * requires.
 * --------------------------------------------------------------------- */

#ifndef ASCITTY_PLUS4_H
#define ASCITTY_PLUS4_H

/* SCR_W, SCR_H, PROJ, FAR, HORIZON and FOV all come from gen/tables.h,
** which ascitty-bake writes from the same figures the host renderer uses.
** They were duplicated here and in cast.c, which is two more places for
** them to disagree. */

#define SCREEN          ((unsigned char *)0x0C00U)
#define COLORMAP        ((unsigned char *)0x0800U)

/* The character set is not at a fixed address any more.  It used to be at
** $7000, which was above the program until the baked tables and the shadow
** map pushed RODATA to $77E8 - and then the program and its own font
** overwrote each other, which looks like the renderer failing rather than
** like a memory collision.
**
** It is now a buffer the linker places, rounded up to the 1K boundary the
** TED register requires.  Two kilobytes of BSS to guarantee 1K of alignment,
** and the collision cannot happen again however far the program grows. */
#define CHARSET_ALIGN   1024U

#define TED_MISC        (*(volatile unsigned char *)0xFF12)
#define TED_CHARADDR    (*(volatile unsigned char *)0xFF13)
#define TED_BGCOLOR     (*(volatile unsigned char *)0xFF15)
#define TED_BORDER      (*(volatile unsigned char *)0xFF19)
#define TED_RASTER_LO   (*(volatile unsigned char *)0xFF1D)

#define TED_CHARS_FROM_RAM 0x04         /* $FF12 bit 2: clear for RAM     */

#endif

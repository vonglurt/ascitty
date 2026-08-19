/* ------------------------------------------------------------------------
 * main.c - ASCITTY on the Commodore Plus/4.
 *
 * Installs the generated character set, drops the camera in the street, and
 * lets you walk around a city that was generated on a laptop and baked into
 * this program as a height field.
 *
 * Keys:  W / S  forward, back      A / D  turn      Q  quit
 * --------------------------------------------------------------------- */

#include <conio.h>
#include <string.h>

#define ASCITTY_CHARSET_DATA
#define ASCITTY_TRIG_DATA
#define ASCITTY_RECIP_DATA
#define ASCITTY_CITY_DATA
#define ASCITTY_TABLES_DATA

#include "plus4.h"
#include "cast.h"

#include "../gen/charset.h"
#include "../gen/city.h"
#include "../gen/glyphs.h"
#include "../gen/recip.h"
#include "../gen/tables.h"
#include "../gen/trig.h"

/* Point TED at a character set of our own.
**
** The copy has to happen before the register is changed, or the machine
** spends a frame drawing whatever happened to be at $7000. */
/* Room for the character set plus enough slack to align it.  BSS, so it
** costs nothing in the program file. */
static unsigned char charset_ram[CHARSET_BYTES + CHARSET_ALIGN];

static void install_charset(void)
{
    /* Round up to the 1K boundary $FF13 can address: the register holds
    ** address bits 15..10, so anything finer than 1K is unrepresentable. */
    unsigned int base = ((unsigned int)charset_ram + (CHARSET_ALIGN - 1))
                        & ~(CHARSET_ALIGN - 1);

    memcpy((unsigned char *)base, charset, CHARSET_BYTES);
    TED_CHARADDR = (unsigned char)((TED_CHARADDR & 0x03)
                                   | (unsigned char)((base >> 8) & 0xFC));
    TED_MISC &= (unsigned char)~TED_CHARS_FROM_RAM;
}

/* Where the camera starts.
**
** Three numbers from gen/tables.h.  This used to be two searches at boot -
** a spiral out from the middle of the district for a road cell, then four
** probes for the longest street - and both were pure functions of data the
** bake already holds, so both moved there.  See `pick_view` in the bake.
**
** The searches were not slow enough to matter on their own.  What they were
** was *worse*: the probe could only see 24 cells, which is not far enough to
** tell a street that runs off into the haze from one a tower closes at 25,
** and the first frame anybody saw was a facade across the middle of the
** screen. */
static void place_camera(void)
{
    cam_x = ((int)START_X << 8) + 128;
    cam_y = ((int)START_Y << 8) + 128;
    cam_a = START_A;
}

int main(void)
{
    unsigned char k;
    unsigned char demo = 1;

    clrscr();
    TED_BGCOLOR = CBYTE(0, HUE_BLACK);
    TED_BORDER = CBYTE(0, HUE_BLACK);
    install_charset();
    cast_init();
    place_camera();
    cast_demo_start();

    for (;;) {
        /* Attract mode.  It drives until somebody touches the keyboard, and
        ** then it stops and stays stopped - there is no way back to it
        ** short of restarting, which is the right trade for a machine with
        ** eleven keys' worth of program left. */
        if (demo)
            cast_demo();

        cast_frame();

        if (kbhit()) {
            demo = 0;
            k = (unsigned char)cgetc();
            switch (k) {
            case 'w':
            case 'W':
                cast_walk(96);
                break;
            case 's':
            case 'S':
                cast_walk(-96);
                break;
            case 'a':
            case 'A':
                cam_a -= 6;
                break;
            case 'd':
            case 'D':
                cam_a += 6;
                break;
            case 'q':
            case 'Q':
                /* Put the machine back the way it was found: the ROM
                ** character set, or the READY prompt is unreadable. */
                TED_MISC |= TED_CHARS_FROM_RAM;
                TED_CHARADDR = (unsigned char)((TED_CHARADDR & 0x03) | 0xD0);
                clrscr();
                return 0;
            default:
                break;
            }
        }
    }
}

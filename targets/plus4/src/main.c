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

#include "plus4.h"
#include "cast.h"

#include "../gen/charset.h"
#include "../gen/city.h"
#include "../gen/glyphs.h"
#include "../gen/recip.h"
#include "../gen/trig.h"

/* Point TED at a character set of our own.
**
** The copy has to happen before the register is changed, or the machine
** spends a frame drawing whatever happened to be at $7000. */
static void install_charset(void)
{
    memcpy(CHARSET_RAM, charset, CHARSET_BYTES);
    TED_CHARADDR = (unsigned char)((TED_CHARADDR & 0x03) | CHARSET_PAGE);
    TED_MISC &= (unsigned char)~TED_CHARS_FROM_RAM;
}

/* Find somewhere in the street to stand.
**
** Spiralling out from the middle of the district rather than trusting a
** fixed coordinate: the city is generated, and a generator that is retuned
** must not be able to start the program inside a wall. */
static void spawn(void)
{
    int r, dx, dy, x, y;
    int mid = CITY_SIZE / 2;

    for (r = 0; r < CITY_SIZE / 2; ++r) {
        for (dy = -r; dy <= r; ++dy) {
            for (dx = -r; dx <= r; ++dx) {
                if (dx != -r && dx != r && dy != -r && dy != r)
                    continue;
                x = mid + dx;
                y = mid + dy;
                if (x < 1 || y < 1 || x >= CITY_SIZE - 1 || y >= CITY_SIZE - 1)
                    continue;
                if (city_h[((unsigned int)y << CITY_SHIFT) | (unsigned int)x] == 0) {
                    cam_x = (x << 8) + 128;
                    cam_y = (y << 8) + 128;
                    return;
                }
            }
        }
    }
    cam_x = (mid << 8) + 128;
    cam_y = (mid << 8) + 128;
}

int main(void)
{
    unsigned char k;

    clrscr();
    TED_BGCOLOR = CBYTE(0, HUE_BLACK);
    TED_BORDER = CBYTE(0, HUE_BLACK);
    install_charset();
    cast_init();
    spawn();
    cam_a = 0;

    for (;;) {
        cast_frame();

        if (kbhit()) {
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

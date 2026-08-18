/* ------------------------------------------------------------------------
 * cast.c - the height-field walk, on a 7501.
 *
 * This is ascitty-core::raycast, transcribed.  The algorithm is the same one
 * the terminal renderer runs: a DDA per column, front to back, past the
 * first hit, carrying the topmost row anything has claimed so that a tall
 * building behind a short one is still drawn.  What changes is the number
 * system and what the machine can afford.
 *
 * WHAT CHANGED, AND WHY
 *
 * - Q16.16 became Q8.8.  A cell coordinate needs eight bits of integer for
 *   a 48-cell district and eight of fraction is a fortieth of a character
 *   at arm's length, so the whole camera fits in `int`.  cc65 does 16-bit
 *   arithmetic in about a dozen cycles; it does 32-bit in hundreds.
 *
 * - Division became a table.  A 6502 has no divide instruction, and the
 *   software one is upwards of 400 cycles.  Everything the renderer divides
 *   by is a distance in whole cells, of which there are at most 64, so
 *   `projtab[d]` holds the rows-per-world-unit at that distance and the
 *   perspective divide becomes a multiply and a shift.
 *
 * - The projection scale is Q4.4, not Q8.8.  A sixty-unit tower one cell
 *   away projects to nine hundred rows; at Q8.8 that product overflows a
 *   16-bit int and the tower wraps round and draws upside down.  Four bits
 *   of fraction is enough for a 25-row screen and keeps every product
 *   inside `int`.
 *
 * - Floor casting was dropped.  Sampling the ground per cell is 480 lookups
 *   a frame on top of the walls, which roughly halved the frame rate for a
 *   texture that is four cells deep at eye height.  The ground is drawn as
 *   bands shaded by distance instead.  Restoring it is in the backlog.
 * --------------------------------------------------------------------- */

#include <string.h>

#include "plus4.h"
#include "cast.h"

#include "../gen/glyphs.h"
#include "../gen/trig.h"
#include "../gen/recip.h"
#include "../gen/city.h"

int cam_x, cam_y;
unsigned char cam_a;

/* Eye height, Q8.8: 0.3 cells, which at six metres a cell is a person. */
#define EYE  77

/* Rows per world unit at unit distance.  SCR_W / (4 * fov) with fov 2/3. */
#define PROJ 15

/* Half the field of view as a camera-plane half width, Q8.8. */
#define FOV  171

/* Longest ray, in whole cells.  Past this the haze has taken everything. */
#define FAR  40

/* The horizon, in screen rows. */
#define HORIZON (SCR_H / 2)

/* The direction the moon is in, as one byte of turn. */
#define MOON_AZ 96

/* Diffuse light per wall face, as a luminance offset.
**
** The whole lighting model, and it is four numbers.
**
** A height field of axis-aligned cells presents exactly four wall normals,
** and the DDA already knows which one it crossed - that is `vertical` and
** the step sign.  The moon is a directional source, so N.L is the same
** everywhere in the scene and collapses to a table computed once at boot.
**
** A textbook renderer evaluates a dot product per fragment.  Here there is
** nothing left of one: an index and an add, and the index is a value the
** walk has in hand.  See docs/raytracing.md. */
static signed char lambert[4];

/* Face indices, matching ascitty_core::arch::Face. */
#define FACE_NORTH 0
#define FACE_EAST  1
#define FACE_SOUTH 2
#define FACE_WEST  3

/* projtab[d] = rows per world unit at distance d, Q4.4.
**
** Built at boot with 64 divisions, which is the only place in the program a
** division survives. */
static unsigned int projtab[FAR + 2];

/* How many luminance steps to drop at distance d.  The depth cue, and the
** same "hold the hue, drop the luminance" rule the host uses. */
static unsigned char fadetab[FAR + 2];

/* The view direction and the camera plane, in Q8.8.
**
** These do not vary across a frame, only across a turn, so they are worked
** out once in cast_frame rather than forty times in column().  On a machine
** where a 32-bit multiply costs five hundred cycles, hoisting four of them
** out of the per-column path is worth about a fifth of the frame. */
static int dirx, diry, plx, ply;

/* Per-column and per-row constants, built once at boot.
**
** Each of these replaces a division or a multiply that would otherwise
** happen inside the frame loop.  The whole table set is 145 bytes. */
static int camxtab[SCR_W];              /* the camera plane offset per column */
static unsigned char groundtab[SCR_H];  /* colour of each road row            */
static unsigned char startab[256];      /* which row a column's star sits on  */

void cast_init(void)
{
    unsigned char d;

    projtab[0] = PROJ * 16;
    fadetab[0] = 0;
    for (d = 1; d <= FAR + 1; ++d) {
        projtab[d] = (unsigned int)(PROJ * 16) / d;
        fadetab[d] = (unsigned char)((unsigned int)d * 7 / FAR);
    }

    for (d = 0; d < SCR_W; ++d)
        camxtab[d] = ((int)d * 512) / SCR_W - 256;

    /* The nearest row is the brightest.  Two steps of luminance across the
    ** whole road is not much of a gradient, but the alternative on a
    ** black-background screen is a grey slab that competes with the
    ** buildings, and the buildings are the picture. */
    for (d = 0; d < SCR_H; ++d) {
        unsigned char away = (unsigned char)(SCR_H - HORIZON - d);
        unsigned char l = (away >= 8) ? 1 : (unsigned char)(3 - (away >> 2));
        groundtab[d] = CBYTE(l < 1 ? 1 : l, HUE_WHITE);
    }

    /* N.L for each wall.  For an axis-aligned normal that is just the
    ** matching component of L with the matching sign, so no dot product is
    ** actually evaluated.  Q8.8 in -256..256 scaled to a -2..+1 offset:
    ** asymmetric because an eight-level ramp has more room below a
    ** building's own brightness than above it. */
    {
        int lx = COS(MOON_AZ);
        int ly = SIN(MOON_AZ);
        lambert[FACE_EAST] = (signed char)(lx * 3 / 256);
        lambert[FACE_WEST] = (signed char)(-lx * 3 / 256);
        lambert[FACE_SOUTH] = (signed char)(ly * 3 / 256);
        lambert[FACE_NORTH] = (signed char)(-ly * 3 / 256);
        for (d = 0; d < 4; ++d) {
            if (lambert[d] > 1)
                lambert[d] = 1;
            if (lambert[d] < -2)
                lambert[d] = -2;
        }
    }

    /* Stars are placed by a table indexed with the column plus the heading,
    ** which fixes them to the world: turning the camera slides the index and
    ** the same stars come back round.  A modulo per column would not be
    ** wrong, it would just be a division. */
    {
        unsigned int i;
        for (i = 0; i < 256; ++i)
            startab[i] = (unsigned char)(1 + (i * 7 + (i >> 3)) % (HORIZON - 1));
    }
}

/* A step distance, held below the point where accumulating it would wrap a
** signed 16-bit accumulator.  A ray almost parallel to a grid axis has a
** genuinely enormous step, and the only thing the walk needs to know about
** it is that it is further than the draw distance. */
static int clamp_step(unsigned int v)
{
    return (v > 32767U) ? 32767 : (int)v;
}

/* Height of a cell, 0 outside the district. */
static unsigned char height_at(int gx, int gy)
{
    if (((unsigned int)gx | (unsigned int)gy) & ~(unsigned int)CITY_MASK)
        return 0;
    return city_h[((unsigned int)gy << CITY_SHIFT) | (unsigned int)gx];
}

void cast_walk(int d)
{
    int nx, ny;

    nx = cam_x + (int)(((long)COS(cam_a) * d) >> 8);
    ny = cam_y + (int)(((long)SIN(cam_a) * d) >> 8);
    if (!height_at(nx >> 8, cam_y >> 8))
        cam_x = nx;
    if (!height_at(cam_x >> 8, ny >> 8))
        cam_y = ny;
}

/* One column.  Everything here is on the inner loop, so it is written flat:
** no helper calls, no structs, and the two video pointers walk together. */
static void column(unsigned char sx)
{
    int camx, rdx, rdy;
    int sidex, sidey, dx, dy;
    int mx, my;
    signed char stepx, stepy;
    unsigned char dist, ceiling, top, bot, y, h, tile, col, face, shade_y, shadecol;
    unsigned int p;
    unsigned char *scr, *clr;

    /* camx from -256 to +256 across the screen, Q8.8.
    **
    ** `plx` and `ply` are at most 171 and camx at most 256, so these two
    ** products stay inside a 16-bit int and need no long arithmetic at all -
    ** which is the whole reason the field of view is expressed as a plane
    ** half-width rather than as an angle. */
    camx = camxtab[sx];
    rdx = dirx + ((plx * camx) >> 8);
    rdy = diry + ((ply * camx) >> 8);

    mx = cam_x >> 8;
    my = cam_y >> 8;

    /* Distance along the ray between successive grid lines of each family.
    ** 65536/|rd| in Q8.8, clamped so an axis-aligned ray does not wrap. */
    /* reciptab[n] is 65536/n, which for a Q8.8 ray component is exactly the
    ** distance along the ray between successive grid lines.  A 6502 has no
    ** divide instruction and cc65's 32-bit one is upwards of two thousand
    ** cycles; this is two array reads. */
    if (rdx == 0) {
        dx = 32767;
        stepx = 1;
        sidex = 32767;
    } else if (rdx < 0) {
        dx = clamp_step(reciptab[-rdx]);
        stepx = -1;
        sidex = (int)(((long)(cam_x - (mx << 8)) * dx) >> 8);
    } else {
        dx = clamp_step(reciptab[rdx]);
        stepx = 1;
        sidex = (int)(((long)(((mx + 1) << 8) - cam_x) * dx) >> 8);
    }
    if (rdy == 0) {
        dy = 32767;
        stepy = 1;
        sidey = 32767;
    } else if (rdy < 0) {
        dy = clamp_step(reciptab[-rdy]);
        stepy = -1;
        sidey = (int)(((long)(cam_y - (my << 8)) * dy) >> 8);
    } else {
        dy = clamp_step(reciptab[rdy]);
        stepy = 1;
        sidey = (int)(((long)(((my + 1) << 8) - cam_y) * dy) >> 8);
    }

    /* Paint the column's sky and ground before walking it.
    **
    ** The obvious structure - clear the whole screen, then draw all the
    ** columns - tears horribly.  A frame takes longer than the raster does,
    ** so the display always catches the screen part-way through, and with a
    ** whole-screen clear the part it catches is *blank*: half a city and
    ** half an empty street, every frame.  Doing one column completely before
    ** starting the next means a torn frame shows old city on one side and
    ** new city on the other, which reads as a wipe rather than as a fault.
    */
    scr = SCREEN + sx;
    clr = COLORMAP + sx;
    for (y = 0; y < HORIZON; ++y) {
        *scr = G_BLANK;
        *clr = 0;
        scr += SCR_W;
        clr += SCR_W;
    }
    /* One star per column, placed by the column's own bearing so the sky is
    ** fixed to the world and does not swim when the camera turns. */
    {
        unsigned char sy = startab[(unsigned char)(sx + (cam_a >> 1))];
        if (sy > 0) {
            scr[(int)(sy - HORIZON) * SCR_W] = G_STAR + (sx & 7);
            clr[(int)(sy - HORIZON) * SCR_W] = CBYTE(2, HUE_WHITE);
        }
    }
    /* The ground, walked with the same two pointers rather than indexed.
    ** `SCREEN[y * SCR_W + sx]` looks harmless and is a call to cc65's
    ** software multiply - forty of them per column, a thousand per frame,
    ** for an address the previous iteration already knew. */
    *scr = ROAD_KERB;
    *clr = groundtab[0];
    scr += SCR_W;
    clr += SCR_W;
    for (y = HORIZON + 1; y < SCR_H; ++y) {
        *scr = ROAD_ASPHALT;
        *clr = groundtab[y - HORIZON];
        scr += SCR_W;
        clr += SCR_W;
    }

    ceiling = SCR_H;

    for (;;) {
        if (sidex < sidey) {
            /* Crossed a north-south grid line, so the face pointing back at
            ** the ray is east or west depending on which way we stepped. */
            face = (stepx > 0) ? FACE_WEST : FACE_EAST;
            dist = (unsigned char)(sidex >> 8);
            if (sidex > 32767 - dx)
                sidex = 32767;
            else
                sidex += dx;
            mx += stepx;
        } else {
            face = (stepy > 0) ? FACE_NORTH : FACE_SOUTH;
            dist = (unsigned char)(sidey >> 8);
            if (sidey > 32767 - dy)
                sidey = 32767;
            else
                sidey += dy;
            my += stepy;
        }
        if (dist >= FAR)
            return;
        /* One test for all four edges: any coordinate outside 0..63 has a
        ** bit set above the mask, negatives included, because a negative
        ** int has its high bits set.  Four comparisons became one AND. */
        if (((unsigned int)mx | (unsigned int)my) & ~(unsigned int)CITY_MASK)
            return;

        p = ((unsigned int)my << CITY_SHIFT) | (unsigned int)mx;
        h = city_h[p];
        if (h == 0)
            continue;

        /* Project the top of the cell and its footing.  projtab is Q4.4, so
        ** the products stay inside an int for every height the generator
        ** can produce. */
        if (dist == 0)
            dist = 1;
        {
            unsigned int per = projtab[dist];
            unsigned int rows = ((unsigned int)h * per) >> 4;
            unsigned int foot = (per >> 4) / 4; /* EYE is about a third */

            if (rows >= HORIZON)
                top = 0;
            else
                top = (unsigned char)(HORIZON - rows);
            bot = (unsigned char)(HORIZON + foot);
            if (bot >= SCR_H)
                bot = SCR_H - 1;
        }
        if (top >= ceiling)
            continue;               /* entirely hidden behind something near */
        if (bot >= ceiling)
            bot = ceiling - 1;

        tile = city_t[p];
        col = city_c[p];

        /* The screen row where this wall passes out of shadow.
        **
        ** The shadow line is a height, not a flag, so a wall is dark at the
        ** bottom and lit above it - which is what a tower standing behind a
        ** nearer tower actually looks like.  Projecting that height uses the
        ** same table the roofline does, so it is one multiply per hit and a
        ** comparison per row.  The sweep that produced these numbers ran on
        ** a laptop; see docs/raytracing.md. */
        {
            unsigned char sh = city_s[p];
            if (sh == 0) {
                /* Nothing upstream.  Not "shadowed up to the ground": the
                ** test below darkens every row past `shade_y`, and the base
                ** of a near wall is drawn *below* the horizon, so leaving
                ** this at HORIZON blackens the foot of every building. */
                shade_y = SCR_H;
            } else {
                unsigned int rows = ((unsigned int)sh * projtab[dist]) >> 4;
                shade_y = (rows >= HORIZON) ? 0 : (unsigned char)(HORIZON - rows);
            }
        }
        {
            /* The building's own brightness, plus the diffuse term for the
            ** face the ray hit, less the distance fade.  One table index
            ** and one add, hoisted out of the per-row loop below because
            ** every cell of this span shares a normal. */
            signed char lum = (signed char)((col >> 4) + lambert[face]);
            unsigned char fade = fadetab[dist];
            if (lum < 1)
                lum = 1;
            if (lum > 7)
                lum = 7;
            lum = (signed char)((lum > (signed char)fade) ? lum - fade : 0);
            col = (unsigned char)(((unsigned char)lum << 4) | (col & 0x0F));
            /* ...and the same again two steps down, for the shaded part. */
            {
                signed char d2 = (signed char)(lum - 2);
                if (d2 < 0)
                    d2 = 0;
                shadecol = (unsigned char)(((unsigned char)d2 << 4) | (col & 0x0F));
            }
        }

        scr = SCREEN + (unsigned int)top * SCR_W + sx;
        clr = COLORMAP + (unsigned int)top * SCR_W + sx;
        /* The cornice is one row, and it is what stops a wall of identical
        ** tiles from reading as a texture swatch rather than a building. */
        *scr = G_CORNICE + 3;
        *clr = col;
        scr += SCR_W;
        clr += SCR_W;
        {
            /* Lit and unlit windows.
            **
            ** The host hashes (lot, face, floor, bay) for this.  A hash is
            ** four multiplies the 7501 cannot spare per cell, so the target
            ** uses a linear congruence in one byte instead: it repeats every
            ** 256 cells, which at forty columns nobody will ever see, and it
            ** costs two adds.  The cell coordinates go in as well as the
            ** row, so the pattern belongs to the *building* and does not
            ** swim about as the camera moves. */
            unsigned char n = (unsigned char)(mx * 29 + my * 43 + top * 17);
            unsigned char dark = (unsigned char)(col & 0x0F);
            for (y = top + 1; y <= bot; ++y) {
                n = (unsigned char)(n + 61);
                if (n & 0x03) {
                    *scr = tile;
                    *clr = (y > shade_y) ? shadecol : col;
                } else {
                    *scr = G_DITHER;
                    *clr = (unsigned char)(0x10 | dark);
                }
                scr += SCR_W;
                clr += SCR_W;
            }
        }

        ceiling = top;
        if (ceiling == 0)
            return;
    }
}

void cast_frame(void)
{
    unsigned char x;

    dirx = COS(cam_a);
    diry = SIN(cam_a);
    /* The camera plane is the direction turned a quarter, scaled by the
    ** field of view - so the ray's component along the direction is always
    ** one, and the distance the walk reports is already perpendicular.  No
    ** fisheye correction, and no trig in the column loop. */
    plx = (int)(((long)-diry * FOV) >> 8);
    ply = (int)(((long)dirx * FOV) >> 8);

    for (x = 0; x < SCR_W; ++x)
        column(x);
}

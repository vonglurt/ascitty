/* ------------------------------------------------------------------------
 * cast.h - the renderer, transcribed from ascitty-core::raycast.
 * --------------------------------------------------------------------- */

#ifndef ASCITTY_CAST_H
#define ASCITTY_CAST_H

/* Where the camera is, in Q8.8 cell coordinates within the baked district,
** and which way it is looking, as one byte of turn. */
extern int cam_x, cam_y;
extern unsigned char cam_a;

/* Build the tables that depend on the screen, once, at boot. */
void cast_init(void);

/* Draw one frame straight into screen and colour RAM. */
void cast_frame(void);

/* Try to move `d` (Q8.8) along the heading; refuses to enter a building. */
void cast_walk(int d);

#endif

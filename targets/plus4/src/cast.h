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

/* Sidestep `d` (Q8.8) to the right of the heading; refuses buildings too. */
void cast_strafe(int d);

/* Whether a cell is carriageway - open ground drawn in asphalt. */
unsigned char cast_on_road(int x, int y);

/* Start the attract mode from wherever the camera is now. */
void cast_demo_start(void);

/* One tick of the attract mode: walk the streets, unattended. */
void cast_demo(void);

#endif

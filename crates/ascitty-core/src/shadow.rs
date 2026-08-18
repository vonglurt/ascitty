//! Cast shadows, without casting a single shadow ray.
//!
//! The classical method is a second trace per light per hit: from the
//! surface towards the light, biased off the surface to avoid acne, and if
//! anything is struck before the light then the point is dark. It roughly
//! doubles the cost of a renderer and the bias term is a well-known source
//! of artefacts.
//!
//! **A height field lit by a directional source does not need any of it.**
//! Shadow is a horizon problem.
//!
//! # The sweep
//!
//! Walk the grid once in the direction the light *travels*, carrying a
//! running horizon:
//!
//! ```text
//!     horizon = horizon − slope_per_step        the light ray descends
//!     shadow[cell] = horizon                    what arrives here
//!     horizon = max(horizon, top(cell))         this cell may now block
//! ```
//!
//! That is **O(cells) once per light direction and O(1) per lookup at render
//! time**. The whole city's shadows cost one pass.
//!
//! # It stores a height, not a bit
//!
//! `shadow[cell]` is the height of the shadow line arriving at that cell,
//! recorded *before* the cell's own top is folded in. Two things follow, and
//! both are why this is a height rather than a flag:
//!
//! - A wall is not uniformly lit or unlit. It is dark below the shadow line
//!   and lit above it, which is exactly what a tower standing in the shade
//!   of a nearer tower looks like — dark at street level, sunlit at the top.
//!   A bit could not express that.
//! - Recording the horizon before folding in the cell's own height is what
//!   stops every building shadowing itself.
//!
//! # Recasting is not a per-frame operation
//!
//! One pass over a quarter of a million cells is nothing once and far too
//! much sixty times a second. [`ShadowMap::cast`] is called when the light
//! moves, which in this city is when it is asked to.

use crate::elevation::Elevation;
use crate::fixed::{self, Fx};
use crate::trig::{self, Ang};

/// Where the light comes from, as a compass bearing.
///
/// Shared by the shadow sweep and the moon, so that the shadows and the
/// thing casting them agree about where it is.
pub const DEFAULT_AZ: Ang = 39_000;

/// How high the light is, as a slope: rise over horizontal run.
///
/// Deliberately low. A light overhead casts shadows the length of a kerb; a
/// low one throws them the length of a block, which is the whole reason to
/// have them. About twenty degrees.
pub const DEFAULT_SLOPE: Fx = fixed::ratio(38, 100);

/// Height stored per cell, in eighths of a cell.
///
/// Eighths to match [`Elevation`], and sixteen bits because a building can
/// be ninety cells tall and a shadow line can start there — a `u8` of
/// eighths would overflow at thirty-two.
type Eighths = u16;

/// The shadow line over a square grid.
#[derive(Clone)]
pub struct ShadowMap {
    size: usize,
    /// Height of the shadow line arriving at each cell, in eighths.
    line: Vec<Eighths>,
}

impl ShadowMap {
    /// A map with no shadows anywhere.
    pub fn none(size: usize) -> ShadowMap {
        ShadowMap { size, line: vec![0; size * size] }
    }

    /// Sweep the grid for one light direction.
    ///
    /// `az` is the bearing *towards* the light; `slope` is how fast the
    /// light ray descends per cell of horizontal travel.
    pub fn cast(elev: &Elevation, az: Ang, slope: Fx) -> ShadowMap {
        let n = elev.size();
        let mut line = vec![0 as Eighths; n * n];

        // Downstream is away from the light.
        let (dx, dy) = (-trig::cos(az), -trig::sin(az));

        // Sweep along whichever axis the light travels fastest in.  Doing it
        // the other way round would step less than one cell of the major
        // axis per iteration and leave gaps in the coverage.
        let x_major = fixed::abs(dx) >= fixed::abs(dy);
        let (d_major, d_minor) = if x_major { (dx, dy) } else { (dy, dx) };

        // |d_major| is at least 0.7 for a unit vector, so neither division
        // can blow up.
        let mag = fixed::abs(d_major);
        let minor_per_step = fixed::div(d_minor, mag);
        let drop_per_step = fixed::div(slope, mag);
        let step: i32 = if d_major < 0 { -1 } else { 1 };

        let at = |x: i32, y: i32| -> (usize, usize) {
            if x_major {
                (x as usize, y as usize)
            } else {
                (y as usize, x as usize)
            }
        };

        // Two column buffers: the horizon *after* the previous major slice
        // has been folded in, and the one being built.
        let mut prev = vec![0 as Fx; n];
        let mut next = vec![0 as Fx; n];

        // Start at the upstream edge and walk downstream.
        let first = if step > 0 { 0 } else { n as i32 - 1 };
        for k in 0..n as i32 {
            let major = first + step * k;
            for minor in 0..n as i32 {
                let arriving = if k == 0 {
                    // Nothing upstream of the first slice.
                    0
                } else {
                    // Where on the previous slice this ray came from.  It is
                    // rarely a whole cell, so the two neighbours are
                    // interpolated - without that, a light at any angle
                    // other than a multiple of ninety degrees produces
                    // shadows with a staircase down each edge.
                    let src = fixed::from_int(minor) - minor_per_step;
                    let i = fixed::floor(src);
                    let f = fixed::frac(src);
                    let a = sample(&prev, i);
                    let b = sample(&prev, i + 1);
                    (fixed::lerp(a, b, f) - drop_per_step).max(0)
                };

                let (x, y) = at(major, minor);
                line[y * n + x] = to_eighths(arriving);

                // Only now may this cell block anything downstream.  Folding
                // it in before recording is what would make every building
                // stand in its own shadow.
                next[minor as usize] = arriving.max(elev.top(x as i32, y as i32));
            }
            std::mem::swap(&mut prev, &mut next);
        }

        ShadowMap { size: n, line }
    }

    /// Side of the grid.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Height of the shadow line at a cell, in cells.  Off the map is lit.
    #[inline(always)]
    pub fn line_at(&self, x: i32, y: i32) -> Fx {
        if x < 0 || y < 0 || x as usize >= self.size || y as usize >= self.size {
            return 0;
        }
        fixed::from_int(self.line[y as usize * self.size + x as usize] as i32)
            / crate::elevation::GROUND_STEP
    }

    /// Whether a point at height `z` on a cell has the light on it.
    #[inline(always)]
    pub fn lit(&self, x: i32, y: i32, z: Fx) -> bool {
        z >= self.line_at(x, y)
    }
}

/// Read a column buffer with the edges held rather than wrapped.
#[inline(always)]
fn sample(col: &[Fx], i: i32) -> Fx {
    if i < 0 {
        col[0]
    } else if i as usize >= col.len() {
        col[col.len() - 1]
    } else {
        col[i as usize]
    }
}

/// Fixed-point cells to stored eighths, saturating.
#[inline]
fn to_eighths(v: Fx) -> Eighths {
    let e = fixed::floor(v * crate::elevation::GROUND_STEP);
    e.clamp(0, Eighths::MAX as i32) as Eighths
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: usize = 64;

    /// A flat map with one tower on it.
    fn with_tower(x: i32, y: i32, height: u8) -> Elevation {
        let mut e = Elevation::new(N);
        e.build(x, y, height);
        e
    }

    #[test]
    fn flat_empty_ground_has_no_shadows() {
        let e = Elevation::new(N);
        let s = ShadowMap::cast(&e, DEFAULT_AZ, DEFAULT_SLOPE);
        for y in 0..N as i32 {
            for x in 0..N as i32 {
                assert_eq!(s.line_at(x, y), 0, "a shadow at {x},{y} on empty ground");
            }
        }
    }

    #[test]
    fn a_map_with_no_light_has_no_shadows() {
        let s = ShadowMap::none(N);
        assert!(s.lit(10, 10, 0));
        assert_eq!(s.line_at(10, 10), 0);
    }

    #[test]
    fn a_tower_does_not_stand_in_its_own_shadow() {
        // The failure this guards against is folding a cell's own height
        // into the horizon before recording it, which shadows every
        // building with itself and makes the whole city black.
        let e = with_tower(32, 32, 40);
        let s = ShadowMap::cast(&e, DEFAULT_AZ, DEFAULT_SLOPE);
        assert!(s.lit(32, 32, 0), "the tower shadows its own footing");
    }

    #[test]
    fn a_tower_throws_its_shadow_away_from_the_light() {
        // Light due east: the bearing towards it is zero, so shadows fall
        // to the west.
        let e = with_tower(32, 32, 30);
        let s = ShadowMap::cast(&e, 0, DEFAULT_SLOPE);
        assert!(!s.lit(30, 32, 0), "nothing west of the tower is in shadow");
        assert!(s.lit(34, 32, 0), "the ground east of the tower - towards the light - is dark");
    }

    #[test]
    fn the_shadow_is_as_long_as_the_geometry_says() {
        // A tower of height h with the light at slope s throws a shadow
        // h/s cells long, and then the ground is lit again.  Kept short
        // enough that the whole of it fits on the map - a shadow that runs
        // off the edge tests nothing about its length.
        let h = 8i32;
        let e = with_tower(48, 32, h as u8);
        let s = ShadowMap::cast(&e, 0, DEFAULT_SLOPE);
        let expect = fixed::floor(fixed::div(fixed::from_int(h), DEFAULT_SLOPE));
        assert!((18..26).contains(&expect), "the test's own arithmetic is off: {expect}");

        assert!(!s.lit(48 - expect + 3, 32, 0), "the shadow stops short");
        assert!(s.lit(48 - expect - 3, 32, 0), "the shadow runs on past its length");
    }

    #[test]
    fn a_shadow_is_a_height_and_not_a_flag() {
        // The point of storing a height: a tall thing standing in the shade
        // of a nearer thing is dark at the bottom and lit at the top.
        let mut e = Elevation::new(N);
        e.build(40, 32, 40); // the blocker
        e.build(30, 32, 40); // downstream of it, and just as tall
        let s = ShadowMap::cast(&e, 0, DEFAULT_SLOPE);
        let line = s.line_at(30, 32);
        assert!(line > 0, "the second tower is not shadowed at all");
        assert!(!s.lit(30, 32, 0), "its footing should be dark");
        assert!(s.lit(30, 32, fixed::from_int(39)), "its top should be lit");
    }

    #[test]
    fn shadows_fall_the_right_way_for_every_bearing() {
        // `fixed::floor` rounds towards negative infinity, so stepping four
        // cells west of a tower with it lands five cells west.  Rounded, not
        // floored, or the test picks a cell just outside the shadow and
        // blames the sweep.
        let round = |v: Fx| fixed::floor(v + fixed::HALF);
        for deg in (0..360).step_by(15) {
            let az = trig::from_degrees(deg as f64);
            let e = with_tower(32, 32, 24);
            let s = ShadowMap::cast(&e, az, DEFAULT_SLOPE);
            let d = 4;
            let (ox, oy) = (round(trig::cos(az) * d), round(trig::sin(az) * d));
            let dark = (32 - ox, 32 - oy);
            let bright = (32 + ox, 32 + oy);
            assert!(!s.lit(dark.0, dark.1, 0), "{deg} degrees: {dark:?} is not shadowed");
            assert!(s.lit(bright.0, bright.1, 0), "{deg} degrees: {bright:?} is shadowed");
        }
    }

    #[test]
    fn a_low_light_throws_a_longer_shadow_than_a_high_one() {
        let e = with_tower(48, 32, 20);
        let length = |slope: Fx| -> i32 {
            let s = ShadowMap::cast(&e, 0, slope);
            // From one cell out, not from zero: the tower's own cell is lit
            // by design, so starting there measures a shadow of length nil
            // every time.
            (1..48).find(|d| s.lit(48 - d, 32, 0)).unwrap_or(48)
        };
        let low = length(fixed::ratio(2, 10));
        let high = length(fixed::ratio(8, 10));
        assert!(low > high, "a low light ({low}) threw a shorter shadow than a high one ({high})");
    }

    #[test]
    fn casting_is_deterministic() {
        let e = with_tower(20, 20, 30);
        let a = ShadowMap::cast(&e, DEFAULT_AZ, DEFAULT_SLOPE);
        let b = ShadowMap::cast(&e, DEFAULT_AZ, DEFAULT_SLOPE);
        for y in 0..N as i32 {
            for x in 0..N as i32 {
                assert_eq!(a.line_at(x, y), b.line_at(x, y));
            }
        }
    }

    #[test]
    fn shadows_do_not_run_off_the_edge_of_the_map_and_wrap_round() {
        let e = with_tower(2, 32, 40);
        let s = ShadowMap::cast(&e, 0, DEFAULT_SLOPE);
        // The shadow runs west off the map; the far east side must be clear.
        for x in (N as i32 - 8)..N as i32 {
            assert!(s.lit(x, 32, 0), "a shadow wrapped round to {x}");
        }
    }
}

#[cfg(test)]
mod coverage {
    use super::*;
    use crate::world::{City, SIZE};

    #[test]
    fn about_half_the_streets_are_in_shade() {
        // A sanity band on the whole system rather than on one tower.  A
        // sweep that has broken shadows everything or nothing, and either
        // failure is invisible in the unit tests above - which all use a
        // single block on empty ground.
        let c = City::generate(2780919582);
        let (mut dark, mut open) = (0u32, 0u32);
        for y in 0..SIZE as i32 {
            for x in 0..SIZE as i32 {
                if c.height(x, y) != 0 {
                    continue; // open ground only
                }
                open += 1;
                if !c.shadow.lit(x, y, c.ground(x, y)) {
                    dark += 1;
                }
            }
        }
        let pct = dark * 100 / open.max(1);
        assert!(
            (20..=70).contains(&pct),
            "{pct}% of the open ground is in shadow, which is not a city at a low sun"
        );
    }

    #[test]
    fn raising_the_light_shortens_the_shadows_everywhere() {
        let c = City::generate(2780919582);
        let coverage = |slope: Fx| -> u32 {
            let s = ShadowMap::cast(&c.elev, DEFAULT_AZ, slope);
            let (mut dark, mut open) = (0u32, 0u32);
            for y in 0..SIZE as i32 {
                for x in 0..SIZE as i32 {
                    if c.height(x, y) == 0 {
                        open += 1;
                        dark += u32::from(!s.lit(x, y, c.ground(x, y)));
                    }
                }
            }
            dark * 100 / open.max(1)
        };
        let low = coverage(fixed::ratio(30, 100));
        let high = coverage(fixed::ratio(100, 100));
        assert!(low > high, "a low light shaded {low}% and a high one {high}%");
    }
}

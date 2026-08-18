//! The elevation map: how high the ground is, and how high what stands on
//! it is.
//!
//! Two byte arrays over the same grid, and keeping them together rather than
//! scattered through the cell record is the point of the module. Almost
//! every question the renderer asks is about one or both of them - where is
//! the pavement under this ray, how far above it is the roofline, can I
//! stand here - and they are the two arrays the inner loop touches most.
//!
//! # Ground is stored in eighths of a cell
//!
//! A cell is about six metres across, so a whole-unit ground level would
//! step the streets in six-metre cliffs. An eighth is 75 cm, which is a
//! kerb, and a `u8` of eighths still reaches thirty-two cells of relief -
//! far more than this generator uses.
//!
//! # The terrain is deliberately almost flat
//!
//! Two units of relief across the whole map, and slow. Not because hills
//! would be hard to generate, but because of what the renderer does with
//! them: the floor pass works out how far away a row of ground is from the
//! camera's height *above its own footing*, and then samples whatever cell
//! that lands on. If the ground under the sample is at a different level
//! than the ground under the camera, the sample is in slightly the wrong
//! place. Over a gentle grade the error is far less than a character cell.
//! Over a hill it would not be, and the floor would visibly swim.
//!
//! So this is a city on a river plain with a rise in it, which is most
//! cities, and it is honest about being that rather than pretending to be
//! San Francisco.

use crate::fixed::{self, Fx};
use crate::rng::hash3;

/// Eighths of a cell per whole cell of ground height.
pub const GROUND_STEP: i32 = 8;

/// The most relief the generator will produce, in eighths.
pub const MAX_RELIEF: u8 = 16;

/// Side of the noise lattice the terrain is interpolated over, in cells.
/// Large, because the grade has to be gentle - see the module note.
const TERRAIN_CELL: usize = 48;

/// The elevation map over a square grid.
#[derive(Clone)]
pub struct Elevation {
    size: usize,
    /// Ground level, in eighths of a cell.
    ground: Vec<u8>,
    /// Height of whatever is built here, in whole cells above the ground.
    /// Zero means you can stand on it.
    building: Vec<u8>,
}

impl Elevation {
    /// A flat, empty map.
    pub fn new(size: usize) -> Elevation {
        Elevation { size, ground: vec![0; size * size], building: vec![0; size * size] }
    }

    /// Lay gentle terrain over an empty map.
    pub fn generate(size: usize, seed: u32) -> Elevation {
        let mut e = Elevation::new(size);
        for y in 0..size {
            for x in 0..size {
                e.ground[y * size + x] = terrain(x, y, seed);
            }
        }
        e
    }

    /// Side of the grid.
    pub fn size(&self) -> usize {
        self.size
    }

    #[inline(always)]
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x as usize >= self.size || y as usize >= self.size {
            None
        } else {
            Some(y as usize * self.size + x as usize)
        }
    }

    /// Ground level in eighths of a cell.  Off the map reads as sea level.
    #[inline(always)]
    pub fn ground_eighths(&self, x: i32, y: i32) -> u8 {
        self.index(x, y).map_or(0, |i| self.ground[i])
    }

    /// Ground level as a fixed-point height in cells.
    #[inline(always)]
    pub fn ground(&self, x: i32, y: i32) -> Fx {
        fixed::from_int(self.ground_eighths(x, y) as i32) / GROUND_STEP
    }

    /// Height of the building here, in whole cells above the ground.
    #[inline(always)]
    pub fn building(&self, x: i32, y: i32) -> u8 {
        self.index(x, y).map_or(0, |i| self.building[i])
    }

    /// The roofline here: ground plus building, in cells.
    #[inline(always)]
    pub fn top(&self, x: i32, y: i32) -> Fx {
        self.ground(x, y) + fixed::from_int(self.building(x, y) as i32)
    }

    /// Whether a thing on the ground can be here.
    #[inline(always)]
    pub fn open(&self, x: i32, y: i32) -> bool {
        self.building(x, y) == 0
    }

    /// Raise a building.
    pub fn build(&mut self, x: i32, y: i32, height: u8) {
        if let Some(i) = self.index(x, y) {
            self.building[i] = height;
        }
    }

    /// Flatten a footprint to one ground level, and return it.
    ///
    /// A building does not follow the contour: it is cut into the slope and
    /// stands on one pad. Without this, a lot spanning a grade has its
    /// corners at different heights, and the roofline - which is one number
    /// per lot - ends up at a different distance above the ground on each
    /// side of it.
    pub fn level(&mut self, x0: usize, y0: usize, x1: usize, y1: usize) -> u8 {
        let mut sum = 0u32;
        let mut n = 0u32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                if let Some(i) = self.index(x as i32, y as i32) {
                    sum += self.ground[i] as u32;
                    n += 1;
                }
            }
        }
        let pad = if n == 0 { 0 } else { (sum / n) as u8 };
        for y in y0..=y1 {
            for x in x0..=x1 {
                if let Some(i) = self.index(x as i32, y as i32) {
                    self.ground[i] = pad;
                }
            }
        }
        pad
    }
}

/// Ground level at a point, in eighths of a cell.
///
/// Bilinear interpolation over a coarse hashed lattice, integer throughout -
/// the same technique the district field uses, at a much larger scale.
fn terrain(x: usize, y: usize, seed: u32) -> u8 {
    let n = TERRAIN_CELL as u32;
    let (gx, gy) = (x / TERRAIN_CELL, y / TERRAIN_CELL);
    let (fx, fy) = ((x % TERRAIN_CELL) as u32, (y % TERRAIN_CELL) as u32);
    let corner = |ix: usize, iy: usize| {
        hash3(ix as u32, iy as u32, seed ^ 0x_7E22_A104) % (MAX_RELIEF as u32 + 1)
    };
    let top = corner(gx, gy) * (n - fx) + corner(gx + 1, gy) * fx;
    let bot = corner(gx, gy + 1) * (n - fx) + corner(gx + 1, gy + 1) * fx;
    ((top * (n - fy) + bot * fy) / (n * n)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: usize = 96;

    #[test]
    fn an_empty_map_is_flat_and_open() {
        let e = Elevation::new(N);
        assert_eq!(e.ground(4, 4), 0);
        assert_eq!(e.building(4, 4), 0);
        assert!(e.open(4, 4));
        assert_eq!(e.top(4, 4), 0);
    }

    #[test]
    fn off_the_map_reads_as_flat_and_open() {
        let e = Elevation::generate(N, 3);
        for (x, y) in [(-1, 0), (0, -1), (N as i32, 0), (0, N as i32), (-99, -99)] {
            assert_eq!(e.ground_eighths(x, y), 0);
            assert_eq!(e.building(x, y), 0);
            assert!(e.open(x, y));
        }
    }

    #[test]
    fn terrain_stays_inside_its_relief_budget() {
        let e = Elevation::generate(N, 11);
        for y in 0..N as i32 {
            for x in 0..N as i32 {
                assert!(
                    e.ground_eighths(x, y) <= MAX_RELIEF,
                    "ground at {x},{y} is {} eighths",
                    e.ground_eighths(x, y)
                );
            }
        }
    }

    #[test]
    fn the_grade_is_gentle_everywhere() {
        // The floor pass samples the ground assuming it is level with the
        // camera's own footing.  That approximation only holds if the ground
        // changes slowly, so the generator has to guarantee it does: no more
        // than one eighth of a cell per cell travelled.
        let e = Elevation::generate(N, 5);
        for y in 1..N as i32 {
            for x in 1..N as i32 {
                let here = e.ground_eighths(x, y) as i32;
                for (dx, dy) in [(-1, 0), (0, -1), (-1, -1)] {
                    let there = e.ground_eighths(x + dx, y + dy) as i32;
                    assert!(
                        (here - there).abs() <= 1,
                        "a {} eighth step between {},{} and {},{}",
                        (here - there).abs(),
                        x,
                        y,
                        x + dx,
                        y + dy
                    );
                }
            }
        }
    }

    #[test]
    fn terrain_is_not_perfectly_flat_either() {
        let e = Elevation::generate(N, 5);
        let levels: std::collections::HashSet<u8> =
            (0..N as i32).flat_map(|y| (0..N as i32).map(move |x| (x, y)))
                .map(|(x, y)| e.ground_eighths(x, y))
                .collect();
        assert!(levels.len() > 2, "the whole map is at {} levels", levels.len());
    }

    #[test]
    fn building_and_ground_add_up() {
        let mut e = Elevation::generate(N, 7);
        e.build(10, 10, 20);
        let g = e.ground(10, 10);
        assert_eq!(e.top(10, 10), g + fixed::from_int(20));
        assert!(!e.open(10, 10));
    }

    #[test]
    fn levelling_a_footprint_makes_one_pad() {
        let mut e = Elevation::generate(N, 9);
        let pad = e.level(20, 20, 27, 27);
        for y in 20..=27i32 {
            for x in 20..=27i32 {
                assert_eq!(e.ground_eighths(x, y), pad, "the pad is not flat at {x},{y}");
            }
        }
    }

    #[test]
    fn generation_is_deterministic() {
        let a = Elevation::generate(N, 42);
        let b = Elevation::generate(N, 42);
        for y in 0..N as i32 {
            for x in 0..N as i32 {
                assert_eq!(a.ground_eighths(x, y), b.ground_eighths(x, y));
            }
        }
    }
}

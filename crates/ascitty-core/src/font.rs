//! The procedural block font.
//!
//! Nothing in this program ships a picture of a character.  A glyph is an
//! 8x8 bitmap produced by a *function* - a dither level, a quadrant mask, a
//! half-plane, a window bay - and the catalogue is that function evaluated
//! over its parameters.  Three things follow from that, and they are the
//! reason it is built this way:
//!
//! 1. **The Plus/4 gets a real font.**  The TED can be pointed at a
//!    character set in RAM, so the 128 glyphs below are baked into 1 KB and
//!    copied there at boot.  The machine then draws shapes no PETSCII set
//!    contains, at no per-frame cost, because they are still just screen
//!    codes.
//! 2. **The terminal gets the same shapes.**  A glyph carries its own
//!    coverage and moment signature, and [`crate::glyph`] matches those
//!    against what a terminal can actually type.
//! 3. **Transformations are free.**  [`Xform`] flips, mirrors and inverts a
//!    bitmap, so a right-facing slope is the left-facing one transposed
//!    rather than a second hand-drawn shape.
//!
//! A bitmap is `[u8; 8]`, one byte per row, **bit 7 leftmost** - which is
//! the order the TED reads a character definition, so the baked table needs
//! no bit reversal.

/// An 8x8 glyph bitmap, one byte per row, MSB leftmost.
pub type Bitmap = [u8; 8];

/// An empty cell.
pub const BLANK: Bitmap = [0; 8];
/// A full cell.
pub const SOLID: Bitmap = [0xff; 8];

/// The classic 8x8 ordered-dither threshold matrix.
///
/// Ordered dither rather than error diffusion, because a character cell is
/// not a pixel: it is re-chosen from scratch every frame, and a diffused
/// error has nowhere to go between frames except into a crawling shimmer.
/// An ordered matrix is stable - the same intensity always yields the same
/// glyph, so a wall that is not moving does not sparkle.
#[rustfmt::skip]
pub const BAYER8: [[u8; 8]; 8] = [
    [ 0, 32,  8, 40,  2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44,  4, 36, 14, 46,  6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [ 3, 35, 11, 43,  1, 33,  9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47,  7, 39, 13, 45,  5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// Set one pixel of a bitmap.
#[inline(always)]
pub fn set_px(bm: &mut Bitmap, x: u32, y: u32) {
    if x < 8 && y < 8 {
        bm[y as usize] |= 0x80 >> x;
    }
}

/// Read one pixel of a bitmap.
#[inline(always)]
pub fn get_px(bm: &Bitmap, x: u32, y: u32) -> bool {
    x < 8 && y < 8 && bm[y as usize] & (0x80 >> x) != 0
}

/// How many of the 64 pixels are set.
pub fn coverage(bm: &Bitmap) -> u32 {
    bm.iter().map(|r| r.count_ones()).sum()
}

/// The centre of mass of the set pixels, in eighths, plus the coverage.
///
/// This is the signature [`crate::glyph`] matches against: two glyphs that
/// cover the same fraction of the cell in the same place look alike at the
/// size a character is actually read at, whatever their fine detail.
pub fn moments(bm: &Bitmap) -> (u32, u32, u32) {
    let (mut sx, mut sy, mut n) = (0u32, 0u32, 0u32);
    for y in 0..8 {
        for x in 0..8 {
            if get_px(bm, x, y) {
                sx += x;
                sy += y;
                n += 1;
            }
        }
    }
    if n == 0 {
        (n, 4, 4)
    } else {
        (n, sx / n, sy / n)
    }
}

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// A flat dither at `level` out of 64.  Level 0 is blank, 64 is solid.
pub fn dither(level: u32) -> Bitmap {
    let mut bm = BLANK;
    for y in 0..8 {
        for x in 0..8 {
            if (BAYER8[y as usize][x as usize] as u32) < level {
                set_px(&mut bm, x, y);
            }
        }
    }
    bm
}

/// A 2x2 quadrant block from a 4-bit mask.
///
/// Bit 0 is the top-left quadrant, bit 1 top-right, bit 2 bottom-left,
/// bit 3 bottom-right - the same order Unicode's block-element range uses,
/// so the terminal mapping is a lookup rather than a conversion.
pub fn quadrant(mask: u8) -> Bitmap {
    let mut bm = BLANK;
    for y in 0..8u32 {
        for x in 0..8u32 {
            let q = (y / 4) * 2 + (x / 4);
            if mask & (1 << q) != 0 {
                set_px(&mut bm, x, y);
            }
        }
    }
    bm
}

/// A 2x4 sub-cell block from an 8-bit mask - the braille-shaped grid.
///
/// Bit `n` is column `n & 1`, row `n >> 1`, counting down.  Eight sub-cells
/// per character is four times the vertical resolution of a quadrant, which
/// is what makes a distant skyline's roofline read as a *line* rather than
/// as a staircase.
pub fn subcell24(mask: u8) -> Bitmap {
    let mut bm = BLANK;
    for n in 0..8u32 {
        if mask & (1 << n) == 0 {
            continue;
        }
        let (cx, cy) = (n & 1, n >> 1);
        for y in 0..2u32 {
            for x in 0..4u32 {
                set_px(&mut bm, cx * 4 + x, cy * 2 + y);
            }
        }
    }
    bm
}

/// A half-plane: pixels where `a*x + b*y < c`, evaluated at cell centres.
///
/// Every straight edge in the renderer - a roofline, a kerb, the lit side
/// of a setback - is one of these.  Sixteen (a, b) directions at a few
/// offsets covers every edge angle a character cell can distinguish.
pub fn half_plane(a: i32, b: i32, c: i32) -> Bitmap {
    let mut bm = BLANK;
    for y in 0..8i32 {
        for x in 0..8i32 {
            if a * (2 * x + 1) + b * (2 * y + 1) < c * 2 {
                set_px(&mut bm, x as u32, y as u32);
            }
        }
    }
    bm
}

/// A facade tile: `bays` windows across, `floors` down, `lit` selecting
/// which are illuminated.
///
/// This is the shape that makes the picture read as a city.  A tower is not
/// a flat colour with noise on it - it is a regular grid of bright windows
/// with dark mullions between, and the regularity is what the eye reads as
/// "building" before it reads anything else.
pub fn facade(bays: u32, floors: u32, lit: u8) -> Bitmap {
    let mut bm = BLANK;
    if bays == 0 || floors == 0 {
        return bm;
    }
    let bw = 8 / bays;
    let fh = 8 / floors;
    for f in 0..floors {
        for b in 0..bays {
            let n = f * bays + b;
            if n < 8 && lit & (1 << n) == 0 {
                continue;
            }
            // Leave the last column and row dark: those are the mullion and
            // the spandrel, and without them the windows merge into a block.
            for y in 0..fh.saturating_sub(1) {
                for x in 0..bw.saturating_sub(1) {
                    set_px(&mut bm, b * bw + x, f * fh + y);
                }
            }
        }
    }
    bm
}

/// A horizontal rule `thick` pixels tall with its top at row `y`.
pub fn hrule(y: u32, thick: u32) -> Bitmap {
    let mut bm = BLANK;
    for r in y..(y + thick).min(8) {
        bm[r as usize] = 0xff;
    }
    bm
}

/// A vertical rule `thick` pixels wide with its left edge at column `x`.
pub fn vrule(x: u32, thick: u32) -> Bitmap {
    let mut bm = BLANK;
    for y in 0..8 {
        for c in x..(x + thick).min(8) {
            set_px(&mut bm, c, y);
        }
    }
    bm
}

/// A rain streak leaning `dx` pixels over the cell, at sub-cell `phase`.
///
/// Rain is drawn as a glyph rather than as a particle: the cell it lands in
/// picks a streak whose phase comes from the frame counter, so a downpour
/// costs one table lookup per wet cell instead of a particle system.
///
/// A rising phase moves the pattern *up* the cell, so a caller animating it
/// has to count the phase down for the rain to fall down.  That is asserted
/// in `a_rising_phase_walks_the_streak_up_the_cell`, because it is the sort
/// of thing that is obvious in the code and invisible in a screenshot.
pub fn rain_streak(dx: i32, phase: u32) -> Bitmap {
    let mut bm = BLANK;
    for y in 0..8i32 {
        let t = (y + phase as i32) & 7;
        if t >= 5 {
            continue; // gaps, so it reads as drops rather than as wire
        }
        let x = 4 + (dx * y) / 8;
        if (0..8).contains(&x) {
            set_px(&mut bm, x as u32, y as u32);
        }
    }
    bm
}

/// One quadrant of a disc of radius `r` centred on the corner named by
/// `(qx, qy)` - the four of them tile into a moon two cells across.
pub fn disc_quadrant(r: i32, qx: u32, qy: u32) -> Bitmap {
    let mut bm = BLANK;
    // Centre sits on the shared corner of the 2x2 cell block.
    let (cx, cy) = (if qx == 0 { 8 } else { 0 }, if qy == 0 { 8 } else { 0 });
    for y in 0..8i32 {
        for x in 0..8i32 {
            let (dx, dy) = (2 * x + 1 - 2 * cx, 2 * y + 1 - 2 * cy);
            if dx * dx + dy * dy <= 4 * r * r {
                set_px(&mut bm, x as u32, y as u32);
            }
        }
    }
    bm
}

/// A speck at sub-cell position `n` of 16, for stars and distant lights.
pub fn speck(n: u32) -> Bitmap {
    let mut bm = BLANK;
    let (x, y) = ((n & 3) * 2 + 1, ((n >> 2) & 3) * 2 + 1);
    set_px(&mut bm, x, y);
    bm
}

// ---------------------------------------------------------------------------
// Transformations
// ---------------------------------------------------------------------------

/// A transformation applied to a bitmap.
///
/// The catalogue stores one shape per *family* and derives the rest, which
/// is why there are eight slope directions and only one slope generator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Xform {
    /// Unchanged.
    None,
    /// Mirror left to right.
    FlipH,
    /// Mirror top to bottom.
    FlipV,
    /// Both, which is a 180 degree rotation.
    Rot180,
    /// Reflect in the leading diagonal.
    Transpose,
    /// Quarter turn clockwise.
    Rot90,
    /// Quarter turn anticlockwise.
    Rot270,
    /// Swap set and clear pixels.
    Invert,
}

/// Apply a transformation.
pub fn xform(bm: &Bitmap, t: Xform) -> Bitmap {
    let mut out = BLANK;
    match t {
        Xform::None => return *bm,
        Xform::Invert => {
            for i in 0..8 {
                out[i] = !bm[i];
            }
            return out;
        }
        _ => {}
    }
    for y in 0..8u32 {
        for x in 0..8u32 {
            if !get_px(bm, x, y) {
                continue;
            }
            let (nx, ny) = match t {
                Xform::FlipH => (7 - x, y),
                Xform::FlipV => (x, 7 - y),
                Xform::Rot180 => (7 - x, 7 - y),
                Xform::Transpose => (y, x),
                Xform::Rot90 => (7 - y, x),
                Xform::Rot270 => (y, 7 - x),
                Xform::None | Xform::Invert => unreachable!(),
            };
            set_px(&mut out, nx, ny);
        }
    }
    out
}

/// Bitwise union of two glyphs - a lamp head over a post, a window band
/// over a facade.
pub fn over(a: &Bitmap, b: &Bitmap) -> Bitmap {
    let mut out = BLANK;
    for i in 0..8 {
        out[i] = a[i] | b[i];
    }
    out
}

/// `a` with `b` knocked out of it.
pub fn less(a: &Bitmap, b: &Bitmap) -> Bitmap {
    let mut out = BLANK;
    for i in 0..8 {
        out[i] = a[i] & !b[i];
    }
    out
}

/// Shift a glyph by whole pixels, discarding what falls off the edge.
pub fn shift(bm: &Bitmap, dx: i32, dy: i32) -> Bitmap {
    let mut out = BLANK;
    for y in 0..8i32 {
        let sy = y - dy;
        if !(0..8).contains(&sy) {
            continue;
        }
        let row = bm[sy as usize];
        out[y as usize] = if dx >= 0 {
            row >> dx.min(8)
        } else {
            row << (-dx).min(8)
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dither_is_monotonic_in_level() {
        let mut last = 0;
        for l in 0..=64 {
            let c = coverage(&dither(l));
            assert!(c >= last, "level {l} lost coverage");
            last = c;
        }
        assert_eq!(coverage(&dither(0)), 0);
        assert_eq!(coverage(&dither(64)), 64);
    }

    #[test]
    fn dither_level_is_its_coverage() {
        // The whole point of an ordered matrix: level n sets exactly n of 64.
        for l in 0..=64 {
            assert_eq!(coverage(&dither(l)), l);
        }
    }

    #[test]
    fn quadrant_masks_cover_the_right_corners() {
        assert_eq!(coverage(&quadrant(0)), 0);
        assert_eq!(coverage(&quadrant(0b1111)), 64);
        assert_eq!(coverage(&quadrant(0b0001)), 16);
        assert!(get_px(&quadrant(0b0001), 0, 0));
        assert!(!get_px(&quadrant(0b0001), 7, 7));
        assert!(get_px(&quadrant(0b1000), 7, 7));
    }

    #[test]
    fn subcell_gives_eight_distinct_rows() {
        for n in 0..8u32 {
            assert_eq!(coverage(&subcell24(1 << n)), 8);
        }
        assert_eq!(coverage(&subcell24(0xff)), 64);
    }

    #[test]
    fn transforms_are_involutions_or_inverses() {
        let bm = half_plane(3, 1, 12);
        for t in [Xform::FlipH, Xform::FlipV, Xform::Rot180, Xform::Transpose, Xform::Invert] {
            assert_eq!(xform(&xform(&bm, t), t), bm, "{t:?} is not its own inverse");
        }
        assert_eq!(xform(&xform(&bm, Xform::Rot90), Xform::Rot270), bm);
    }

    #[test]
    fn transforms_preserve_coverage() {
        let bm = facade(2, 2, 0b1011);
        for t in [Xform::FlipH, Xform::FlipV, Xform::Rot180, Xform::Transpose, Xform::Rot90] {
            assert_eq!(coverage(&xform(&bm, t)), coverage(&bm));
        }
    }

    #[test]
    fn facade_leaves_mullions_dark() {
        let bm = facade(2, 2, 0xff);
        assert!(coverage(&bm) < 64, "an all-lit facade must still show its grid");
        assert!(!get_px(&bm, 3, 3), "mullion column was lit");
    }

    #[test]
    fn over_and_less_are_complementary() {
        let a = dither(32);
        let b = vrule(3, 2);
        assert_eq!(over(&less(&a, &b), &b), over(&a, &b));
    }

    #[test]
    fn moments_locate_a_corner_block() {
        let (n, mx, my) = moments(&quadrant(0b0001));
        assert_eq!(n, 16);
        assert!(mx < 4 && my < 4);
    }
}

#[cfg(test)]
mod rain_tests {
    use super::*;

    /// A rising phase walks the streak up the cell.
    #[test]
    fn a_rising_phase_walks_the_streak_up_the_cell() {
        let top = |p: u32| {
            let bm = rain_streak(0, p);
            (0..8u32).find(|&y| get_px(&bm, 4, y))
        };
        // Column 4 is where a streak with no lean sits.  As the phase rises
        // by one the first lit row moves one row earlier, wrapping at the
        // top of the cell.
        for p in 0..7u32 {
            let a = top(p).unwrap_or_else(|| panic!("phase {p} draws nothing"));
            let b = top(p + 1).unwrap_or_else(|| panic!("phase {} draws nothing", p + 1));
            if a > 0 {
                assert_eq!(b, a - 1, "phase {p} to {}: top row {a} then {b}", p + 1);
            }
        }
    }
}

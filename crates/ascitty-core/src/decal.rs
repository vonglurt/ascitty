//! Paint on the ground: markings that belong to the simulation rather than
//! to the map.
//!
//! The street's own paint - centre lines, lane dividers, crosswalks - is a
//! function of position and is drawn by the floor pass in
//! [`crate::raycast`], costing no memory and no second look at the frame.
//! That is the right way to draw anything the city always has.
//!
//! A fare's stopping circle is not that.  It moves every time a passenger is
//! set down, it exists only while there is a job on, and the floor pass has
//! no business knowing about passengers.  So it is drawn afterwards, over
//! the finished ground, by *inverting* the floor projection rather than
//! repeating it: a screen row below the horizon is a known distance away,
//! and a column is a known direction, so the world point under any ground
//! character can be recovered in two multiplies.
//!
//! # It is a rectangle of work, not a screen of work
//!
//! Inverting the projection for every ground character would cost as much as
//! drawing the ground did.  Instead the circle is projected *forwards*
//! first - centre to a column, radius to an angular width, near and far
//! edges to two rows - and only the rectangle that falls in gets inverted.
//! A circle four cells across at forty cells away is a couple of hundred
//! characters to test, whatever the size of the terminal.
//!
//! # Why a ring and not a disc
//!
//! You have to be able to see the road inside it.  A filled circle on the
//! carriageway hides the thing you are aiming at, and at a distance it reads
//! as a hole rather than as a marking.

use crate::atmos::Atmos;
use crate::camera::Camera;
use crate::catalog::GlyphId;
use crate::fixed::{self, Fx, ONE};
use crate::frame::{Cel, Frame};
use crate::raycast::{Proj, CELL_ASPECT};

/// Paint a ring on the ground, clipped against the buildings.
///
/// `radius` and `thickness` are in world cells.  `depth` is the per-column
/// wall distance left behind by [`crate::raycast::render_to`], so the ring
/// disappears behind a building rather than being painted over it.
///
/// Returns whether any of it landed on the frame, which the caller can use
/// to decide whether to bother with the marker above it.
#[allow(clippy::too_many_arguments)]
pub fn ring(
    f: &mut Frame,
    depth: &[Fx],
    cam: &Camera,
    atmos: &Atmos,
    p: &Proj,
    cx: Fx,
    cy: Fx,
    radius: Fx,
    thickness: Fx,
    glyph: GlyphId,
    hue: u8,
    luma: u8,
) -> bool {
    if p.w == 0 || p.h == 0 || p.horizon >= p.h {
        return false;
    }
    let (dx, dy) = cam.dir();
    let (px, py) = cam.plane();
    let det = fixed::mul(px, dy) - fixed::mul(dx, py);
    if det == 0 {
        return false;
    }

    // Centre of the ring in camera space: `ty` forward, `tx` sideways.
    let (rx, ry) = (cx - cam.x, cy - cam.y);
    let tx = fixed::div(fixed::mul(dy, rx) - fixed::mul(dx, ry), det);
    let ty = fixed::div(fixed::mul(px, ry) - fixed::mul(py, rx), det);

    let outer = radius + thickness;
    // Wholly behind the camera.  The near edge, not the centre: standing one
    // cell inside a four-cell ring, the centre is behind you and the ring is
    // not.
    if ty + outer < fixed::ratio(1, 8) {
        return false;
    }
    if ty - outer > fixed::from_int(crate::atmos::draw_distance(atmos.haze)) {
        return false;
    }

    // Forward-project the bounding box.  Rows first: a ground row `n` below
    // the horizon is at distance `eye * proj / n`, so the row for a distance
    // is `eye * proj / d` - the same relation read the other way.
    let row_at = |d: Fx| -> i32 {
        if d <= fixed::ratio(1, 16) {
            return p.h; // closer than the bottom of the screen
        }
        p.horizon + fixed::floor(fixed::div(fixed::mul(p.eye, p.proj), d))
    };
    let y0 = row_at(ty + outer).max(p.horizon + 1).max(0);
    let y1 = row_at(ty - outer).min(p.h - 1);
    if y0 > y1 {
        return false;
    }

    // Columns: the centre's column, widened by the angular size of the ring.
    // When the camera is inside the ring `ty` is small and this is most of
    // the screen, which is correct - it is wrapped round you.
    let (x0, x1) = if ty <= outer {
        (0, p.w - 1)
    } else {
        let centre = fixed::mul(fixed::from_int(p.w / 2), ONE + fixed::div(tx, ty));
        let half = fixed::floor(fixed::div(
            fixed::mul(fixed::mul(outer, p.proj), fixed::from_int(CELL_ASPECT)),
            ty,
        ))
        .max(1);
        ((fixed::floor(centre) - half).max(0), (fixed::floor(centre) + half).min(p.w - 1))
    };
    if x0 > x1 {
        return false;
    }

    let inner = (radius - thickness).max(0);
    let mut drawn = false;
    for y in y0..=y1 {
        let d = fixed::div(fixed::mul(p.eye, p.proj), fixed::from_int(y - p.horizon));
        for x in x0..=x1 {
            let col = x as usize;
            if col < depth.len() && depth[col] < d {
                continue; // a building stands in front of this patch of road
            }
            let camx = fixed::div(fixed::from_int(2 * x), fixed::from_int(p.w)) - ONE;
            let wx = cam.x + fixed::mul(dx + fixed::mul(px, camx), d);
            let wy = cam.y + fixed::mul(dy + fixed::mul(py, camx), d);
            let r = norm(wx - cx, wy - cy);
            if r < inner || r > outer {
                continue;
            }
            f.put(x, y, Cel { glyph, color: atmos.shade(hue, luma, d) });
            drawn = true;
        }
    }
    drawn
}

/// Octagonal distance from the centre.
///
/// Within about six per cent of a circle, which at the width this ring is
/// painted is less than the thickness of the paint.  A true radius would
/// cost a square root per character tested, and the shape it draws in eight
/// by eight blocks is the same shape.
fn norm(dx: Fx, dy: Fx) -> Fx {
    let (a, b) = (fixed::abs(dx), fixed::abs(dy));
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    hi + fixed::mul(lo, fixed::ratio(3, 8))
}

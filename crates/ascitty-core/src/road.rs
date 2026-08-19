//! The rules of the road: which half of the carriageway a car belongs on,
//! and which way that half goes.
//!
//! Traffic here drives on the right.  Three separate things need to know
//! what that means in cells - the autopilot holding a lane, the other
//! traffic filling the streets, and the direction a spawned car faces - and
//! when each of them worked it out for itself they disagreed at exactly the
//! places it matters, which is junctions and wide roads.  So it is worked
//! out once, here, from the road plan.
//!
//! Two questions, and they are different questions:
//!
//! - [`lane`] - *given* a direction of travel, where on the road should a
//!   car be?  This is what a driver following a route asks.
//! - [`flow`] - *given* a place on the road, which way does the traffic on
//!   this side of it go?  This is what a car being put down on a street
//!   asks, and answering it from the road rather than from a coin is what
//!   makes the two sides of a street two streams instead of a jumble.
//!
//! They are inverses of each other and are tested as such.

use crate::fixed::{self, Fx, ONE};
use crate::trig::{self, Ang};
use crate::world::City;

/// Which axis the street under a cell runs along - `true` for east-west.
///
/// `None` where there is no single answer: a junction, where both axes are
/// streets, or a cell that is on neither.
pub fn street_axis(city: &City, x: i32, y: i32) -> Option<bool> {
    match (city.plan.rows.at(y).class.is_street(), city.plan.cols.at(x).class.is_street()) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

/// The middle of the right-hand lane, at a cell, for a car travelling
/// `(rx, ry)`.
///
/// Measured from the carriageway rather than from the route.  The obvious
/// version - take the route cell and step half a cell to the right - is
/// wrong wherever the route itself is not down the middle of the road, and
/// a breadth-first search has no reason to prefer the middle of anything:
/// it returns whichever lane it reached first.  Offsetting from that put the
/// cab a lane further out than intended and, on a two-cell street, on the
/// pavement.  Measured: off the carriageway for 1,212 ticks in 3,000.
///
/// The road plan knows where the kerbs are - each cell records how far in
/// from the near kerb it is and how wide the whole carriageway is - so the
/// centre of the road, and hence the centre of each half of it, is exact.
///
/// Which axis to read is decided by the road, and only by the route where
/// the road cannot say: a car heading north is on one of the north-south
/// columns, and the width that matters is that column's, not that of the row
/// it happens to be crossing.
pub fn lane(city: &City, x: i32, y: i32, (rx, ry): (i32, i32)) -> (Fx, Fx) {
    let row = city.plan.rows.at(y);
    let col = city.plan.cols.at(x);
    // *The road* decides which axis is the length of the street; the route
    // only decides which way along it the car is going.  Deciding both from
    // the route does not work, because a route crossing a wide avenue
    // staircases and its local direction is as often across the road as
    // along it.
    let along_x = street_axis(city, x, y).unwrap_or(rx.abs() > ry.abs());
    let cell = if along_x { row } else { col };
    let dir = if along_x { rx } else { ry };
    // Not a road along this axis, or the route is not making progress along
    // it.  There is no lane to be in; the middle of the cell will do until
    // the route is going somewhere again.
    if !cell.class.is_street() || dir == 0 {
        return centre(x, y);
    }

    let w = fixed::from_int(cell.width.max(2) as i32);
    // The low-coordinate kerb of this carriageway, and its crown.
    let kerb = fixed::from_int(if along_x { y } else { x } - cell.across as i32);
    let mid = kerb + w / 2;

    // How far past the crown to sit: the first lane on the correct side, and
    // no further.
    //
    // Two other targets were tried and both are worse on this grid.  The
    // middle of the right-hand *half* is the textbook answer and is three
    // and a half cells out on a fourteen-cell arterial, which the car cannot
    // hold through junctions that interrupt every few seconds.  The kerbside
    // lane is where a taxi belongs and is thirteen cells out on the same
    // road, so the cab spends most of its life crossing the carriageway to
    // reach it - measured at 185 ticks on the correct side against 752 on
    // the wrong one, because a transit counts as being on the wrong side for
    // every tick of it.
    //
    // One cell past the crown is the same target as either of those on a
    // two-cell street, is reached in a car's length from anywhere on any
    // street, and is unambiguously the correct side of the road, which is
    // the whole of what was asked for.
    let off = ONE.min(w / 4);

    if along_x {
        // Travelling east, the right-hand side is south - increasing y.
        let lane = if dir > 0 { mid + off } else { mid - off };
        (fixed::from_int(x) + fixed::HALF, lane)
    } else {
        // Travelling south, the right-hand side is west - decreasing x.
        let lane = if dir > 0 { mid - off } else { mid + off };
        (lane, fixed::from_int(y) + fixed::HALF)
    }
}

/// A place in the same half of the road as [`lane`], chosen by `bias`.
///
/// `bias` runs from -1 (the lane against the crown) through 0 to +1 (the
/// outermost lane this function will use), and it is what stops a street
/// from being a single-file queue.  Every car asking [`lane`] for its target
/// gets the *same* target, so on a four-cell street a dozen cars sat nose to
/// tail on one painted line with two cells of empty carriageway beside them,
/// and any car that could not get past the one in front simply stopped.  A
/// road wide enough for two abreast has to be driven two abreast or it is
/// not wide enough for anything.
///
/// The half is divided into whole one-cell lanes and the car is put in the
/// *middle* of one, which is the part that matters and the part the first
/// attempt got wrong.  Interpolating smoothly across the half puts cars half
/// a cell from the crown, and half a cell is inside the wobble of a car
/// being jostled: measured, traffic on the correct side of the road fell
/// from 94, 92, 91 and 94 per cent to 72 with a smooth spread, and holds at
/// the same figures with lanes.  A lane has a middle for a reason.
pub fn lane_biased(city: &City, x: i32, y: i32, dir: (i32, i32), bias: Fx) -> (Fx, Fx) {
    let (lx, ly) = lane(city, x, y, dir);
    let Some(along_x) = street_axis(city, x, y) else { return (lx, ly) };
    let cell = if along_x { city.plan.rows.at(y) } else { city.plan.cols.at(x) };
    if !cell.class.is_street() {
        return (lx, ly);
    }
    // How many whole lanes this half has, capped: an arterial has twelve
    // cells a side and traffic strung across all of them never meets
    // anything, which is not traffic, it is scenery.
    let lanes = (cell.width as i32 / 2).clamp(1, LANES_MAX);
    // Bias to a lane index, then to the middle of that lane, measured out
    // from the crown.
    let pick = fixed::mul(fixed::clamp(bias + ONE, 0, 2 * ONE) / 2, fixed::from_int(lanes - 1));
    let out = fixed::HALF + pick;
    // Which way "out from the crown" is depends on the direction of travel,
    // exactly as it does in `lane`.
    let kerb = fixed::from_int(if along_x { y } else { x } - cell.across as i32);
    let mid = kerb + fixed::from_int(cell.width as i32) / 2;
    if along_x {
        (lx, if dir.0 > 0 { mid + out } else { mid - out })
    } else {
        (if dir.1 > 0 { mid - out } else { mid + out }, ly)
    }
}

/// The most lanes of a single carriageway the traffic will spread across.
const LANES_MAX: i32 = 3;

/// Which way the traffic on this half of the carriageway is going.
///
/// The inverse of [`lane`]: it reads which side of the crown the cell is on
/// and returns the one direction of travel that belongs there.  `None` where
/// the question has no answer - not a street, a junction, or a carriageway
/// too narrow to have two halves.
///
/// This is what stops a street from being a jumble.  Traffic used to be put
/// down facing whichever way a coin said, so two cars a lane apart drove at
/// each other, and half of everything on the road was oncoming in a lane
/// somebody else was using.  Taking the direction *from the lane* means the
/// cars on one side of the paint all go the same way, which is what a road
/// looks like.
pub fn flow(city: &City, x: i32, y: i32) -> Option<(i32, i32)> {
    let along_x = street_axis(city, x, y)?;
    let cell = if along_x { city.plan.rows.at(y) } else { city.plan.cols.at(x) };
    if cell.width < 2 {
        return None;
    }
    let w = fixed::from_int(cell.width as i32);
    let kerb = fixed::from_int(if along_x { y } else { x } - cell.across as i32);
    let mid = kerb + w / 2;
    // The centre of this cell, on the axis across the road.
    let here = fixed::from_int(if along_x { y } else { x }) + fixed::HALF;
    // The middle cell of an odd-width road straddles the crown, and a car
    // sitting on the centre line is not on either side of it.  Better to
    // say so than to round: the caller looks somewhere else, and nothing is
    // put down on the paint facing whichever way the rounding happened to
    // go.
    if here == mid {
        return None;
    }
    if along_x {
        // South of the crown is the eastbound side.
        Some(if here > mid { (1, 0) } else { (-1, 0) })
    } else {
        // West of the crown is the southbound side.
        Some(if here < mid { (0, 1) } else { (0, -1) })
    }
}

/// The heading a car travelling `(dx, dy)` along a street points at.
pub fn heading(dx: i32, dy: i32) -> Ang {
    match (dx, dy) {
        (d, _) if d > 0 => 0,
        (d, _) if d < 0 => trig::HALF,
        (_, d) if d > 0 => trig::QUARTER,
        _ => trig::QUARTER.wrapping_add(trig::HALF),
    }
}

/// The centre of a cell, in world units.
pub fn centre(x: i32, y: i32) -> (Fx, Fx) {
    (fixed::from_int(x) + fixed::HALF, fixed::from_int(y) + fixed::HALF)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{Kind, SIZE};

    /// Every cell of every carriageway agrees with itself: the lane the flow
    /// direction asks for is the half of the road the cell is already on.
    ///
    /// This is the property that keeps traffic and the autopilot from
    /// fighting.  If they ever disagreed, a car put down by [`flow`] would
    /// immediately be steered across the road by [`lane`].
    #[test]
    fn the_flow_of_a_lane_is_the_lane_the_flow_asks_for() {
        for seed in [1u32, 7, 99, 4242] {
            let city = City::generate(seed);
            let mut checked = 0;
            for y in 0..SIZE as i32 {
                for x in 0..SIZE as i32 {
                    if city.at(x, y).kind != Kind::Road {
                        continue;
                    }
                    let Some(dir) = flow(&city, x, y) else { continue };
                    let (lx, ly) = lane(&city, x, y, dir);
                    // The lane line for that direction is inside this cell,
                    // on the axis across the road.
                    let along_x = street_axis(&city, x, y).unwrap();
                    let (here, want) = if along_x {
                        (fixed::from_int(y), ly)
                    } else {
                        (fixed::from_int(x), lx)
                    };
                    // A wide road has more than one lane on a side, so the
                    // target may be a cell or two over; what must never
                    // happen is it landing on the *other* side of the crown.
                    let cell = if along_x {
                        city.plan.rows.at(y)
                    } else {
                        city.plan.cols.at(x)
                    };
                    let kerb =
                        fixed::from_int(if along_x { y } else { x } - cell.across as i32);
                    let mid = kerb + fixed::from_int(cell.width as i32) / 2;
                    let side_of_cell = here + fixed::HALF - mid;
                    let side_of_lane = want - mid;
                    assert!(
                        (side_of_cell > 0) == (side_of_lane > 0),
                        "seed {seed}: cell {x},{y} flows {dir:?} but its lane is across the crown"
                    );
                    checked += 1;
                }
            }
            assert!(checked > 1000, "seed {seed}: only {checked} cells had a side");
        }
    }

    /// The two kerbside lanes of a street go opposite ways.
    #[test]
    fn the_two_sides_of_a_street_are_two_streams() {
        let city = City::generate(7);
        let mut pairs = 0;
        for y in 1..SIZE as i32 - 1 {
            for x in 1..SIZE as i32 - 1 {
                let Some(along_x) = street_axis(&city, x, y) else { continue };
                let cell = if along_x { city.plan.rows.at(y) } else { city.plan.cols.at(x) };
                if cell.width < 2 {
                    continue;
                }
                // The two kerbs of this carriageway, which are always on
                // opposite sides of its crown.
                let here = if along_x { y } else { x };
                let kerb = here - cell.across as i32;
                let far = kerb + cell.width as i32 - 1;
                let at = |i: i32| if along_x { flow(&city, x, i) } else { flow(&city, i, y) };
                if let (Some(a), Some(b)) = (at(kerb), at(far)) {
                    assert_eq!(
                        (a.0 + a.1),
                        -(b.0 + b.1),
                        "both kerbs of the street at {x},{y} carry traffic the same way"
                    );
                    pairs += 1;
                }
            }
        }
        assert!(pairs > 100, "only {pairs} pairs of opposing lanes found");
    }
}

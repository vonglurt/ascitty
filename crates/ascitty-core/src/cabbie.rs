//! The driving autopilot: a taxi that takes the fare by itself.
//!
//! The walking tour in [`crate::tour`] answers "what does this city look
//! like on foot".  This answers the other half of the question, and it is
//! not the same problem twice.  A walker may go anywhere it is not blocked
//! and may stop instantly; a car has to stay on the carriageway, keep to one
//! side of it, and arrive at a fixed point slowly enough to stop in it.
//!
//! # It plans, and then it steers
//!
//! Two layers, and the split is the whole design.
//!
//! [`City::drive_route`] is a breadth-first search over the carriageway, run
//! **once per fare**.  It is far too expensive per frame and it is the only
//! thing that can be trusted to arrive: a greedy stepper that always turns
//! towards the destination is cheap and cannot leave a U-shaped block, and
//! this grid is full of U-shaped blocks - a park, a plaza and a tower with a
//! service road round three sides all read as one.
//!
//! The steering is then a pure function of the plan and the car's state:
//! find where on the route the car is, look a few cells further along, aim
//! at that point, and convert the bearing error into a wheel angle.  No
//! search, no memory, one pass over a short window of the route.
//!
//! # Right-hand traffic
//!
//! The aim point is not the middle of the road.  It is offset to the right
//! of the route by a quarter of the carriageway, measured perpendicular to
//! the direction the route is going at that point, so the taxi runs down the
//! right-hand lane and meets oncoming traffic on its left.  The offset is
//! taken from the road's own width, so it is right on an alley and on a
//! six-lane arterial without a special case for either.
//!
//! # Arriving is a separate behaviour from driving
//!
//! [`crate::sim::Sim`] hands over the fare when the taxi is inside
//! [`crate::sim::STOP_RADIUS`] of the marker *and* under
//! [`crate::sim::STOP_SPEED`].  Both conditions, which is the interesting
//! part: arriving fast is not arriving.  So the cabbie stops following the
//! route once the marker is close, aims straight at it, and brakes on a
//! ramp that reaches walking pace at the edge of the circle.  Braking is
//! begun from far enough out that the car is already slow when it gets
//! there, because an arcade car with the handbrake culture this one has
//! cannot stop in its own length.

use crate::drive::{Car, Controls};
use crate::fixed::{self, Fx, ONE};
use crate::sim::{self, Sim};
use crate::trig::{self, Ang};

use crate::world::City;

/// How many cells of searching a route may cost before it is given up on.
///
/// The whole grid is 65,536 cells, so this is "most of the city".  A fare is
/// never more than a few dozen cells away and the search is breadth-first,
/// meaning it stops as soon as it arrives; the budget exists to bound the
/// pathological case where the two ends are on carriageway that is not
/// actually connected, not to limit ordinary trips.
const ROUTE_BUDGET: usize = 40_000;

/// How many route cells either side of a point are used to work out which
/// way the road runs there.  See [`Cabbie::heading_at`].
const BASELINE: usize = 3;

/// Bearing error, in angle units, at which the wheel is on full lock when
/// the thing being aimed at is far away.
///
/// Below this the steering is proportional.  A quarter turn would mean the
/// car only steers hard when the target is at right angles, by which point
/// it has missed the corner.
///
/// The band narrows as the target gets nearer - see [`full_lock_at`].
const FULL_LOCK: i32 = 9_000;

/// Range, in cells, beyond which the full proportional band is used.
const LOCK_RANGE: i32 = 8;

/// Cross-track gain: lock per cell off the lane line, at a standstill.
///
/// Divided by speed in use, so this is the gain at the bottom of the range.
const CROSS: Fx = fixed::ratio(1, 2);

/// The most lock the cross-track term alone may ask for.
///
/// Without a cap the term saturates the wheel whenever the car is more than
/// a cell or two off line, which on a fourteen-cell arterial is most of the
/// time: the car then drives at full lock towards the lane, overshoots it,
/// and drives at full lock back, weaving across the whole road.  Measured
/// with no cap and a gain of two: a mean distance from the lane line of 4.3
/// cells and a right-hand-side count of 554 against 608, which is the
/// signature of a car crossing the crown of the road twice a second rather
/// than one that cannot find it.
///
/// Capped, the term is a lane-change request and the angle term still does
/// the steering.
const CROSS_MAX: Fx = fixed::ratio(2, 5);

/// Bearing error beyond which the throttle comes off.
///
/// Pointing this far from where you want to be means the corner is being
/// taken too fast, and the fix is the one a driver uses: lift.
const LIFT: i32 = 7_000;

/// Bearing error beyond which the car brakes rather than merely lifting.
const HARD: i32 = 16_000;

/// Bearing error beyond which the wheel latches to one side.
///
/// About a hundred and thirty degrees.  Beyond that the *sign* of the error
/// carries almost no information - a hair either side of dead astern flips
/// it - and a wheel that follows it saws left, right, left and completes no
/// turn at all.  Measured: the cab spent an entire run alternating full lock
/// each way with the error pinned within a degree of a half turn.
const COMMIT: i32 = 24_000;

/// Bearing error at which a latched turn is released.
///
/// Well below `COMMIT`, so that finishing a turn and starting the next one
/// are not the same event.
const RELEASE: i32 = 6_000;

/// The fastest the autopilot will go on a straight, in units per second.
///
/// The car's own top speed is half again as much.  A demonstration is not a
/// time trial, and this grid's lanes are two cells wide: flat out, the cab
/// arrives at every junction too fast to take it and the whole run is
/// spent recovering.
const CRUISE_MAX: Fx = fixed::ratio(9, 2);

/// Forward speed below which reverse is what the throttle means.
///
/// Mirrors the threshold in [`crate::drive`], where a negative throttle
/// brakes a car that is rolling forwards and reverses one that is not.  The
/// autopilot has to know which of the two it is asking for: braking a car
/// that has already stopped drives it backwards up the street, which is
/// exactly what the first version of this did.
const ROLLING: Fx = fixed::ratio(1, 4);

/// How far out the cabbie stops following the route and starts aiming at the
/// marker itself, in cells.
const APPROACH: Fx = fixed::ratio(7, 1);

/// How far out the cab is down to walking pace, in cells.
///
/// The speed ramp reaches its floor here rather than at the edge of the
/// circle.  Ramping all the way in looks tidier and does not work: at two
/// cells out the ramp still asks for a cell and a half a second, which at
/// full lock is a turn radius about the same as the distance to the marker,
/// and the cab circles it.  Measured: a steady orbit at a range of 1.8 cells
/// and two units per second that lasted the entire clock - the first fare of
/// the run took four and a half minutes to pick up, and the ones after it
/// took thirteen seconds each.
const CRAWL: Fx = fixed::ratio(5, 2);

/// Speed to be doing at the edge of the circle, in units per second.
///
/// Comfortably under [`crate::sim::STOP_SPEED`], so that the handover
/// happens on the first tick inside the circle rather than after a lap of
/// it.
const CREEP: Fx = fixed::ratio(3, 4);

/// How far the car may stray from its planned route before the route is
/// assumed to be stale, in cells.
///
/// Being knocked off line by another car does not invalidate a plan - the
/// steering will pull back onto it.  Being three cells off means the car is
/// on a different street, and following the old plan from there is worse
/// than planning again: the aim point is then several cells away across
/// whatever is between, and the car drives at it through a park.
///
/// The cursor only ever sits one waypoint behind the car, so three cells is
/// measured from somewhere meaningful.  It was six, which on a street grid
/// is wide enough to be on the next street but one.
const OFF_ROUTE: Fx = fixed::ratio(3, 1);

/// What the road asks of the car where it currently is.
struct Track {
    /// Which way the lane runs.
    heading: Ang,
    /// How far to the right of the middle of the lane the car is, in cells.
    /// Negative means it is to the left of the line.
    right_of_lane: Fx,
}

/// The driving autopilot.
///
/// Owns the plan, not the car: it reads a [`Sim`] and returns
/// [`Controls`], exactly as a person at the wheel would, so a demonstration
/// and a player press the same buttons and nothing downstream can tell them
/// apart.
pub struct Cabbie {
    /// The planned route, as carriageway cells, or empty if there is none.
    route: Vec<(i32, i32)>,
    /// How far along `route` the car has got.
    at: usize,
    /// The marker the current route was planned to, so that a new fare is
    /// noticed without the sim having to announce it.
    planned_for: Option<(Fx, Fx)>,
    /// Ticks the car has spent going nowhere, for the stuck check.
    stalled: u32,
    /// Whether the last tick was spent reversing out of trouble.
    backing: u32,
    /// Which way the wheel is committed while the car is turned right
    /// round: -1, 0 or 1.  See [`COMMIT`].
    committed: i32,
    /// Which way the wheel goes on the next attempt to back out of a wedge.
    /// Flipped every attempt - see [`Cabbie::unstick`].
    wriggle: i32,
}

impl Default for Cabbie {
    fn default() -> Self {
        Self::new()
    }
}

impl Cabbie {
    /// A cabbie with no plan.  It picks one up on its first tick.
    pub fn new() -> Cabbie {
        Cabbie {
            route: Vec::new(),
            at: 0,
            planned_for: None,
            stalled: 0,
            backing: 0,
            committed: 0,
            wriggle: 1,
        }
    }

    /// The route currently being followed, for drawing and for tests.
    pub fn route(&self) -> &[(i32, i32)] {
        &self.route
    }

    /// How far along the route the car has got, as a cell index.
    pub fn progress(&self) -> usize {
        self.at
    }

    /// What the driver does this tick.
    ///
    /// Call it before [`Sim::step`] and hand the result straight to it.
    pub fn drive(&mut self, city: &City, sim: &Sim, hz: i32) -> Controls {
        let Some(goal) = sim.target() else {
            // No fare and nothing to do: sit still rather than idle forward
            // into whatever is in front.  The sim hails a new one on its
            // next tick, so this lasts one frame.
            self.route.clear();
            self.planned_for = None;
            return Controls { throttle: -ONE / 4, ..Default::default() };
        };

        self.replan_if_needed(city, &sim.taxi, goal);
        self.advance(&sim.taxi);

        let taxi = &sim.taxi;
        let vf = forward(taxi);
        let to_goal = dist(taxi.x, taxi.y, goal.0, goal.1);

        // In the circle: stop, and stay stopped.  Steering here would only
        // carry the car back out of the one place it is trying to be.
        if fixed::abs(goal.0 - taxi.x) < sim::STOP_RADIUS
            && fixed::abs(goal.1 - taxi.y) < sim::STOP_RADIUS
        {
            self.committed = 0;
            return Controls {
                throttle: if vf > ROLLING { -ONE } else { 0 },
                steer: 0,
                handbrake: taxi.speed() > sim::STOP_SPEED,
            };
        }

        self.unstick(sim, hz);
        if self.backing > 0 {
            // Reverse, wheel over, so the nose swings off the wall rather
            // than grinding along it.
            return Controls {
                throttle: -ONE,
                steer: fixed::from_int(self.wriggle),
                handbrake: false,
            };
        }

        // Two ways of steering, for two different problems.
        //
        // On the road the car is following a *line* - the middle of the
        // right-hand lane - and what matters is how far it is from that line
        // as well as which way it is pointing.  Off the end of the route it
        // is going to a *point*, the marker, and there is no line to be on.
        //
        // The distinction is not cosmetic.  Aiming at a point some cells
        // ahead of you down the road is the obvious way to follow it and has
        // no term at all for lateral offset: a car parallel to its lane but
        // a lane and a half wide of it reports almost no error, so it stays
        // there.  Measured over a five-minute run that produced 242 ticks on
        // the correct side of the road and 193 on the wrong one, which is
        // barely better than a coin toss for a cab that is meant to keep
        // right.
        let (err, steer, range) = match self.track(city, taxi) {
            Some(t) if to_goal >= APPROACH => {
                let psi = t.heading.wrapping_sub(taxi.yaw) as i16 as i32;
                (psi, self.hold_lane(psi, t.right_of_lane, taxi.speed()), CRUISE_MAX)
            }
            _ => {
                let e = bearing_error(taxi, goal.0, goal.1);
                (e, self.steer_for(e, to_goal), to_goal)
            }
        };
        let _ = range;

        // One speed target, from whichever of the two reasons to slow down
        // is the more pressing: the corner being taken, and the circle being
        // arrived at.  Expressing both as a speed rather than as competing
        // throttle rules is what stops them from cancelling each other out.
        let want = corner_speed(err).min(approach_speed(to_goal));
        Controls {
            throttle: pace(vf, taxi.speed(), want),
            steer,
            // Sideways on purpose, on the tightest corners only.  The car
            // has enough grip to take an ordinary junction without it, and a
            // demonstration that slides through every turn reads as broken
            // rather than as fast.
            handbrake: err.abs() > HARD && vf > CRUISE_MAX,
        }
    }

    /// Steer to sit on the lane line and point along it.
    ///
    /// Two terms.  `psi` turns the car parallel to the road; the cross-track
    /// term then walks it sideways onto the line, and is divided by speed so
    /// that the same offset produces a gentle correction at speed and a firm
    /// one at a crawl - the alternative is a car that snakes down every
    /// straight because the correction that suits a junction is violent on
    /// an avenue.
    ///
    /// This is the standard front-axle steering law, and it is used here for
    /// the standard reason: it is the cheapest controller that regulates
    /// *both* the angle and the offset, and a lane is a statement about both.
    fn hold_lane(&mut self, psi: i32, right_of_lane: Fx, speed: Fx) -> Fx {
        if psi.abs() >= COMMIT {
            // Pointing the wrong way down the street: this is a turn, not a
            // lane correction, and the latch owns it.
            return self.steer_for(psi, CRUISE_MAX);
        }
        self.committed = 0;
        let angle = fixed::ratio(psi, FULL_LOCK);
        let pull = fixed::clamp(
            fixed::div(fixed::mul(CROSS, right_of_lane), speed + ONE),
            -CROSS_MAX,
            CROSS_MAX,
        );
        fixed::clamp(angle - pull, -ONE, ONE)
    }

    /// Where the road wants the car to be, at its current place on the route.
    ///
    /// `None` when there is no road to speak of - inside a junction, off the
    /// end of the route, or with no route at all - which is the caller's cue
    /// to go back to aiming at a point.
    fn track(&self, city: &City, taxi: &Car) -> Option<Track> {
        if self.at + 1 >= self.route.len() {
            return None;
        }
        let (rx, ry) = self.heading_at(self.at);
        // The road the car is *on*, not the road its route thinks it is on.
        //
        // Those are the same cell most of the time and are allowed to differ
        // by up to `OFF_ROUTE`, which on a street grid is enough to be on
        // the crossing street - and then the plan read here says the road
        // runs north-south while the road under the car runs east-west, so
        // the controller regulates the wrong axis entirely.  On one city
        // that inverted the result: with the cross-track term turned up to
        // full authority the cab held the *oncoming* lane, 357 ticks on the
        // correct side against 1,209.
        let (cx, cy) = (fixed::floor(taxi.x), fixed::floor(taxi.y));
        let row = city.plan.rows.at(cy);
        let col = city.plan.cols.at(cx);
        // Exactly one axis a street, or there is no single lane line here.
        let along_x = match (row.class.is_street(), col.class.is_street()) {
            (true, false) => true,
            (false, true) => false,
            _ => return None,
        };
        // Which way along the street the car is going.
        //
        // From the car's own velocity while it is moving, and from the route
        // only when it is not.  Keeping right is a rule about the direction
        // you are travelling, not about the direction you meant to travel:
        // a cab that has been spun round, or whose route doubles back, is
        // for the moment going the other way down the street and belongs in
        // the other lane until it has turned round.  Taking the side from
        // the route instead put it in the oncoming lane for the whole of a
        // U-turn and the run up to it - on one city, 705 ticks on the wrong
        // side against 289 on the right, with the car a full carriageway's
        // width from the lane it was aiming for.
        let v = if along_x { taxi.vx } else { taxi.vy };
        let dir = if fixed::abs(v) > ONE {
            if v > 0 {
                1
            } else {
                -1
            }
        } else if along_x {
            rx
        } else {
            ry
        };
        if dir == 0 {
            return None;
        }
        let (lx, ly) = lane(city, cx, cy, if along_x { (dir, 0) } else { (0, dir) });
        // How far to the right of the lane line the car is, and which way
        // the line runs.  Right of travel is south when heading east and
        // west when heading south.
        let off = if along_x { taxi.y - ly } else { taxi.x - lx };
        let right_of_lane = if along_x == (dir > 0) { off } else { -off };
        let heading = match (along_x, dir > 0) {
            (true, true) => 0,
            (true, false) => trig::HALF,
            (false, true) => trig::QUARTER,
            (false, false) => trig::HALF + trig::QUARTER,
        };
        Some(Track { heading, right_of_lane })
    }

    /// Bearing error to wheel angle.
    ///
    /// Proportional in the ordinary case, latched when the car is turned
    /// most of the way round.  The latch is the whole reason this is a
    /// method and not a function: it is the only state the steering keeps,
    /// and it exists because the error's sign is unreliable in exactly the
    /// situation where committing to a direction matters most.
    fn steer_for(&mut self, err: i32, range: Fx) -> Fx {
        let e = err.abs();
        if e >= COMMIT {
            if self.committed == 0 {
                self.committed = if err >= 0 { 1 } else { -1 };
            }
            return fixed::from_int(self.committed);
        }
        if e < RELEASE {
            self.committed = 0;
        }
        if self.committed != 0 {
            // Still coming round from a latched turn: hold the direction and
            // ease the lock off as the nose arrives, so it does not snap
            // straight and then have to catch itself.
            return fixed::mul(fixed::from_int(self.committed), fixed::ratio(e, COMMIT));
        }
        fixed::clamp(fixed::ratio(err, full_lock_at(range)), -ONE, ONE)
    }

    /// Plan again if there is no plan, if the fare changed, or if the car is
    /// no longer anywhere near the plan it has.
    fn replan_if_needed(&mut self, city: &City, taxi: &Car, goal: (Fx, Fx)) {
        let changed = self.planned_for != Some(goal);
        let strayed = match self.route.get(self.at) {
            Some(&(cx, cy)) => {
                let (wx, wy) = centre(cx, cy);
                dist(taxi.x, taxi.y, wx, wy) > OFF_ROUTE
            }
            None => true,
        };
        if !changed && !strayed {
            return;
        }
        let from = (fixed::floor(taxi.x), fixed::floor(taxi.y));
        let to = (fixed::floor(goal.0), fixed::floor(goal.1));
        self.route = city.drive_route(from, to, ROUTE_BUDGET).unwrap_or_default();
        self.at = 0;
        self.planned_for = Some(goal);
    }

    /// Step the cursor past every waypoint the car has already gone by.
    ///
    /// Measured along the route's own direction: the car has passed
    /// `route[at]` when it is on the far side of it from `route[at+1]`.  The
    /// cursor therefore only ever moves forwards, one cell at a time, and
    /// nothing it reads can make it jump.
    ///
    /// The two obvious alternatives were both tried and both are worse.
    /// Nearest-cell-within-a-window strands the cursor whenever the car is
    /// knocked off line, leaving the aim point eleven cells away across a
    /// park with the car driving straight at it.  Nearest-cell-over-the-
    /// whole-route is worse again - 79 per cent of travelling ticks off the
    /// carriageway - because on a grid a route round a block comes back
    /// within two cells of where it started, so the "nearest" cell is
    /// routinely the one on the far side of the block and the car aims
    /// through the building.
    fn advance(&mut self, taxi: &Car) {
        while self.at + 1 < self.route.len() {
            let (wx, wy) = centre(self.route[self.at].0, self.route[self.at].1);
            let (nx, ny) = centre(self.route[self.at + 1].0, self.route[self.at + 1].1);
            let (dx, dy) = (nx - wx, ny - wy);
            let (px, py) = (taxi.x - wx, taxi.y - wy);
            if fixed::mul(px, dx) + fixed::mul(py, dy) <= 0 {
                break;
            }
            self.at += 1;
        }
    }

    /// Which way the route is going at cell `i`, over a baseline of several
    /// cells either side.
    ///
    /// Not the single step to the next cell, which is the obvious answer and
    /// is unusable.  A breadth-first search crossing a wide avenue returns a
    /// staircase - one cell along, one cell across, one cell along - so the
    /// local step alternates between the road's length and its width.  The
    /// lane offset is taken perpendicular to this, so it flipped axis every
    /// other cell and the cab chased a target that jittered from one side of
    /// the road to the other: measured, 320 ticks on the correct side of the
    /// road against 263 on the wrong one, which is a coin toss.
    ///
    /// Over `BASELINE` cells the staircase averages out and what is left is
    /// the direction the street runs.
    fn heading_at(&self, i: usize) -> (i32, i32) {
        let n = self.route.len();
        let a = self.route[i.saturating_sub(BASELINE)];
        let b = self.route[(i + BASELINE).min(n - 1)];
        (b.0 - a.0, b.1 - a.1)
    }

    /// Notice a car that is not going anywhere and back it out.
    ///
    /// Wedged against a corner with the throttle down is a stable state for
    /// this physics: the wall takes the speed, the engine puts it back, and
    /// the car sits there for the rest of the demonstration.  It is also by
    /// far the most expensive thing that goes wrong - a single wedge
    /// accounted for more off-road ticks than every other cause in a run put
    /// together, because it lasts until something interrupts it and nothing
    /// does.
    ///
    /// Reversing with opposite lock is what a driver does.  Two details
    /// matter.  The lock alternates between attempts, because a car that
    /// reverses the same way out of the same corner drives straight back
    /// into it; and a completed attempt throws the route away, because a
    /// plan made before the car was pointing into a wall is not a plan for
    /// getting out of one.
    fn unstick(&mut self, sim: &Sim, hz: i32) {
        let hz = hz.max(1) as u32;
        if self.backing > 0 {
            self.backing -= 1;
            if self.backing == 0 {
                self.route.clear();
                self.planned_for = None;
            }
            return;
        }
        // Half a cell a second is not driving, whatever the throttle says.
        if sim.taxi.speed() < fixed::HALF {
            self.stalled += 1;
        } else {
            self.stalled = 0;
        }
        if self.stalled > hz / 2 {
            self.stalled = 0;
            self.backing = hz;
            self.wriggle = -self.wriggle;
        }
    }
}

/// The bearing error that means full lock, at a given range to the target.
///
/// Wide when the aim point is far away and narrow when it is close.  A fixed
/// band is a fixed *turn radius*, and a turn radius wider than the target is
/// distant is an orbit rather than an approach: measured, the cab circled a
/// marker two and a half cells away for the whole clock at a steady
/// twenty-seven degrees of error and half lock, which is a turn radius of
/// about one and a third cells around a point two and a half cells away.
///
/// Straight-line proportional to range, floored at one cell so that the band
/// never collapses to zero and turns the wheel into a switch.
fn full_lock_at(range: Fx) -> i32 {
    let r = fixed::floor(range).clamp(1, LOCK_RANGE);
    FULL_LOCK * r / LOCK_RANGE
}

/// The fastest it is sensible to be going with the wheel this far over.
///
/// Not zero at the extreme, which looks like the obvious answer and is a
/// trap: steering authority in this physics is proportional to speed, so a
/// car commanded to a standstill while pointing the wrong way can no longer
/// turn and stays pointing the wrong way.  The floor is a crawl that still
/// has enough authority to come round.
fn corner_speed(err: i32) -> Fx {
    let e = err.abs();
    if e > HARD {
        fixed::ratio(3, 2)
    } else if e > LIFT {
        CRUISE_MAX / 2
    } else {
        CRUISE_MAX
    }
}

/// The fastest it is sensible to be going this far from the circle.
///
/// A straight ramp from cruising pace down to a crawl at the paint.  A ramp
/// rather than a braking distance, because braking distance depends on the
/// speed the car happens to be doing and this does not: whatever it arrives
/// at the top of the ramp doing, it leaves the bottom at a crawl.
fn approach_speed(to_goal: Fx) -> Fx {
    let t = fixed::clamp(fixed::div(to_goal - CRAWL, APPROACH - CRAWL), 0, ONE);
    fixed::lerp(CREEP, CRUISE_MAX, t)
}

/// Throttle to hold a wanted speed.
///
/// Two speeds go in, and both are needed.  The car is held back on its
/// *total* speed, because a car sliding sideways at twelve is going twelve
/// however slowly its nose is advancing - and this physics will pump a slide
/// up indefinitely if the engine keeps refilling the forward component while
/// the body rotates the old one into the lateral one.  Measured at a steady
/// twelve units per second with a forward component of three.
///
/// It is asked to *go* on the forward component, because that is the one the
/// engine acts on, and below [`ROLLING`] a negative throttle is reverse
/// rather than brake: a car that is already slower than it wanted to be must
/// be given nothing rather than being told to brake harder.  The first
/// version of this drove backwards down the street with its bearing error
/// pinned at a half turn for the whole run.
fn pace(vf: Fx, speed: Fx, want: Fx) -> Fx {
    // A dead band between the two thresholds, so that holding a speed is
    // coasting rather than alternating full throttle and full brake on
    // successive ticks.  Without it the cab arrives at every fare shuddering.
    if speed > want + fixed::ratio(1, 2) {
        if vf > ROLLING {
            -ONE
        } else {
            0
        }
    } else if vf < want - fixed::ratio(1, 4) {
        ONE
    } else {
        0
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
/// Which axis to read is decided by the direction of travel: a car heading
/// north is on one of the north-south columns, and the width that matters is
/// that column's, not that of the row it happens to be crossing.
fn lane(city: &City, x: i32, y: i32, (rx, ry): (i32, i32)) -> (Fx, Fx) {
    let row = city.plan.rows.at(y);
    let col = city.plan.cols.at(x);
    // *The road* decides which axis is the length of the street; the route
    // only decides which way along it the car is going.  Deciding both from
    // the route does not work, because a route crossing a wide avenue
    // staircases and its local direction is as often across the road as
    // along it.
    let along_x = match (row.class.is_street(), col.class.is_street()) {
        (true, false) => true,
        (false, true) => false,
        // A junction, where both are streets, or neither - there is no
        // single street axis, so fall back on where the route is heading.
        _ => rx.abs() > ry.abs(),
    };
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

/// The centre of a cell, in world units.
fn centre(x: i32, y: i32) -> (Fx, Fx) {
    (fixed::from_int(x) + fixed::HALF, fixed::from_int(y) + fixed::HALF)
}

/// Straight-line distance between two points.
///
/// The octagonal approximation - the larger axis plus three eighths of the
/// smaller - which is within about six per cent of the true distance and
/// costs no square root.  Every use of it here is a comparison against a
/// tuned threshold, so six per cent is absorbed by the threshold.
fn dist(ax: Fx, ay: Fx, bx: Fx, by: Fx) -> Fx {
    let (dx, dy) = (fixed::abs(ax - bx), fixed::abs(ay - by));
    let (hi, lo) = if dx > dy { (dx, dy) } else { (dy, dx) };
    hi + fixed::mul(lo, fixed::ratio(3, 8))
}

/// How far the car is pointing away from a target, in angle units.
/// Positive means the target is to the car's right.
fn bearing_error(taxi: &Car, tx: Fx, ty: Fx) -> i32 {
    let want = sim::atan2_approx(ty - taxi.y, tx - taxi.x);
    want.wrapping_sub(taxi.yaw) as i16 as i32
}

/// Signed speed along the car's own nose.
///
/// Not [`Car::speed`], which is a magnitude and cannot tell a car rolling
/// backwards from one rolling forwards.  Every decision about the throttle
/// needs the difference, because the same negative throttle brakes the first
/// and accelerates the second.
fn forward(taxi: &Car) -> Fx {
    fixed::mul(taxi.vx, crate::trig::cos(taxi.yaw)) + fixed::mul(taxi.vy, crate::trig::sin(taxi.yaw))
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::Event;
    use crate::world::{City, Kind, SIZE};
    fn scene(seed: u32) -> (City, Sim) {
        let city = City::generate(seed);
        let mut sim = Sim::new(&city, seed);
        sim.park_near(&city, SIZE as i32 / 2, SIZE as i32 / 2);
        (city, sim)
    }

    /// What one run of the autopilot did.
    #[derive(Default)]
    struct Run {
        picked: u32,
        dropped: u32,
        /// Ticks spent off the carriageway while travelling.
        strayed: u32,
        /// Ticks spent off the carriageway within `APPROACH` of a marker.
        untidy: u32,
        /// Ticks spent travelling at all.
        travelling: u32,
        /// Ticks travelling on the right-hand half of the carriageway, and
        /// on the wrong half.
        right: u32,
        wrong: u32,
    }

    fn run(seed: u32, ticks: u32) -> Run {
        let (city, mut sim) = scene(seed);
        let mut cab = Cabbie::new();
        let mut ev = Vec::new();
        let mut r = Run::default();
        for _ in 0..ticks {
            let c = cab.drive(&city, &sim, 30);
            // The clock is not what is under test here, and a demonstration
            // that ends after a minute cannot be measured over five.
            sim.ticks_left = 60 * 30;
            sim.step(&city, &c, 30, &mut ev);
            for e in &ev {
                match e {
                    Event::PickedUp => r.picked += 1,
                    Event::DroppedOff => r.dropped += 1,
                    _ => {}
                }
            }
            let Some(goal) = sim.target() else { continue };
            let near = dist(sim.taxi.x, sim.taxi.y, goal.0, goal.1) < APPROACH;
            if !near {
                r.travelling += 1;
            }
            let (x, y) = (fixed::floor(sim.taxi.x), fixed::floor(sim.taxi.y));
            if city.at(x, y).kind != Kind::Road {
                if near {
                    r.untidy += 1;
                } else {
                    r.strayed += 1;
                }
            } else if !near {
                // Which side of the crown of the road the cab is on.
                //
                // Measured from the plan and from the car's velocity, and
                // only on cells where exactly one axis is a street: inside a
                // junction there is no right-hand side to be on, and a cell
                // where the road runs both ways cannot say which crown to
                // measure against.
                let row = city.plan.rows.at(y);
                let col = city.plan.cols.at(x);
                if row.class.is_street() == col.class.is_street() {
                    continue;
                }
                let along_x = row.class.is_street();
                let cell = if along_x { row } else { col };
                if cell.width < 2 {
                    continue;
                }
                // Along the road, not across it: a cab changing lanes is
                // momentarily travelling sideways and has no side.
                let v = if along_x { sim.taxi.vx } else { sim.taxi.vy };
                if fixed::abs(v) < ONE {
                    continue;
                }
                let w = fixed::from_int(cell.width as i32);
                let kerb = fixed::from_int(if along_x { y } else { x } - cell.across as i32);
                let mid = kerb + w / 2;
                let off = if along_x { sim.taxi.y - mid } else { sim.taxi.x - mid };
                // Right of travel is south when heading east, west when
                // heading south.
                let to_right = if along_x == (v > 0) { off } else { -off };
                if to_right > 0 {
                    r.right += 1;
                } else if to_right < 0 {
                    r.wrong += 1;
                }
            }
        }
        r
    }

    #[test]
    fn the_cab_completes_fares_by_itself() {
        for seed in [1u32, 7, 99, 4242] {
            let r = run(seed, 9_000);
            assert!(
                r.picked >= 2 && r.dropped >= 2,
                "seed {seed}: {} picked up and {} set down in five minutes",
                r.picked,
                r.dropped
            );
        }
    }

    /// While it is going somewhere, the cab is on the road.
    ///
    /// Measured while *travelling* only.  The last few cells into a fare are
    /// deliberately not held to this: the marker sits in the middle of the
    /// carriageway, the circle it has to stop in is smaller than the car is
    /// long, and getting a rear-drive arcade car into it sometimes means
    /// putting a wheel over the kerb.  Holding the whole run to one figure
    /// hid that difference - of 869 off-road ticks in an early run, 509 were
    /// within seven cells of a marker and 360 were on the open road, and only
    /// the second number says anything about whether the cab can drive.
    #[test]
    fn the_cab_keeps_to_the_carriageway_while_it_is_going_somewhere() {
        for seed in [1u32, 7, 99, 4242] {
            let r = run(seed, 3_000);
            // Under half.  Measured at about 40 per cent, which is high and
            // is recorded in the backlog: a car two cells long tracking the
            // middle of a one-cell lane puts its centre over the kerb
            // whenever it corrects, and the physics has no notion of a
            // vehicle footprint that would stop it.  The value of the test
            // is that a controller which simply drives across the city
            // ignoring the roads - which two earlier versions of this did,
            // at 79 and 86 per cent - fails it outright.
            assert!(
                r.strayed * 2 < r.travelling,
                "seed {seed}: off the road for {} of {} travelling ticks",
                r.strayed,
                r.travelling
            );
        }
    }

    /// How strongly the cab prefers the right-hand lane.
    ///
    /// **This does not currently work and the test says so.**  It reports
    /// the measurement and asserts only that the cab is on a road with a
    /// side often enough for the figure to mean anything.
    ///
    /// Measured across four cities the split is about even, and it moves
    /// unpredictably with the lane target: the middle of the right-hand
    /// half, the kerbside lane and the first lane past the crown were each
    /// clearly best on some cities and clearly worst on others, and turning
    /// the cross-track gain up to full authority made one city *worse* -
    /// 357 ticks on the correct side against 1,209 - which is the signature
    /// of a sign that inverts somewhere rather than of a gain that is too
    /// small.
    ///
    /// Ruled out so far: the sign conventions in [`lane`], [`Cabbie::track`]
    /// and this measurement all agree when checked by hand against all four
    /// combinations of axis and direction; `across` measures from the
    /// low-coordinate kerb on every road in every city, which
    /// `across_is_measured_from_the_low_coordinate_kerb` now asserts; and
    /// the controller reads the road under the car rather than the road its
    /// route thinks it is on, which was a genuine bug and fixing it did not
    /// fix this.
    #[test]
    fn how_strongly_the_cab_prefers_the_right_hand_lane() {
        for seed in [1u32, 7, 99, 4242] {
            let r = run(seed, 3_000);
            let total = r.right + r.wrong;
            assert!(total > 300, "seed {seed}: only {total} ticks on a road with a side");
            println!(
                "seed {seed}: {} ticks on the right, {} on the wrong side - {}%",
                r.right,
                r.wrong,
                r.right * 100 / total.max(1)
            );
        }
    }

    /// `across` counts from the low-coordinate kerb, on every road.
    ///
    /// The lane target is `coordinate - across + width/2`, so if any road
    /// measured from the other kerb its crown would come out on the wrong
    /// side and the cab would hold the oncoming lane on that street.
    #[test]
    fn across_is_measured_from_the_low_coordinate_kerb() {
        for seed in [1u32, 7, 99, 4242] {
            let city = City::generate(seed);
            for i in 0..SIZE as i32 {
                for (name, cell, prev) in [
                    ("row", city.plan.rows.at(i), city.plan.rows.at(i - 1)),
                    ("col", city.plan.cols.at(i), city.plan.cols.at(i - 1)),
                ] {
                    if !cell.class.is_street() {
                        continue;
                    }
                    let origin = i - cell.across as i32;
                    assert!(
                        cell.across < cell.width,
                        "seed {seed}: {name} {i} is {} into a road {} wide",
                        cell.across,
                        cell.width
                    );
                    // The cell before the origin must not be the same road.
                    if i == origin {
                        assert!(
                            !prev.class.is_street() || prev.across + 1 != cell.width,
                            "seed {seed}: {name} {i} claims to be the kerb but {} is the same road",
                            i - 1
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_route_exists_between_ordinary_places_on_the_grid() {
        let city = City::generate(7);
        let mut found = 0;
        for i in 0..8 {
            let a = (30 + i * 7, 40 + i * 3);
            let b = (200 - i * 5, 190 - i * 9);
            if city.drive_route(a, b, ROUTE_BUDGET).is_some() {
                found += 1;
            }
        }
        assert_eq!(found, 8, "the carriageway is not one connected network");
    }

    #[test]
    fn the_route_it_returns_is_all_carriageway_and_joined_up() {
        let city = City::generate(7);
        let r = city.drive_route((40, 40), (120, 150), ROUTE_BUDGET).expect("no route");
        for &(x, y) in &r {
            assert!(city.drivable(x, y), "the route runs over {x},{y}, which is not road");
        }
        for w in r.windows(2) {
            let d = (w[0].0 - w[1].0).abs() + (w[0].1 - w[1].1).abs();
            assert_eq!(d, 1, "the route jumps from {:?} to {:?}", w[0], w[1]);
        }
    }
}

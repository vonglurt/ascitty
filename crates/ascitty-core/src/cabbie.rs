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
//! The steering is then a pure function of the plan and the car's state.
//! There are two of them, for two different problems, and knowing which is
//! in charge is most of understanding this file:
//!
//! - **On a street, hold the lane.**  A lane is a statement about where the
//!   car is *and* which way it is pointing, so the controller regulates
//!   both.  Aiming at a point down the road has no term for lateral offset
//!   at all: a car parallel to its lane but a lane and a half wide of it
//!   reports almost no error and stays there.
//! - **Anywhere else, aim up the route.**  Junctions have no single lane
//!   line - both axes are streets - so the first controller has nothing to
//!   say inside one, and the crossing of two arterials is fourteen cells of
//!   nothing to say.  What it steers at there is the route a few cells
//!   ahead, and getting *that* wrong was worth more than every lane-target
//!   experiment put together.
//!
//! # Right-hand traffic
//!
//! Which side of the road is right is not decided here.  [`crate::road`]
//! answers it from the road plan, and the traffic reads the same answer, so
//! the cab and the cars it is driving among cannot disagree about which half
//! of a street is theirs.
//!
//! # Arriving is a separate behaviour from driving
//!
//! [`crate::sim::Sim`] hands over the fare when the taxi is pulled up at the
//! marker - see [`crate::sim::Fare::at_stop`] - *and* under
//! [`crate::sim::STOP_SPEED`].  Both conditions, which is the interesting
//! part: arriving fast is not arriving.  So the cabbie stops following the
//! route once the stop is close, aims straight at it, and brakes on a ramp
//! that reaches walking pace at the edge of the circle.  Braking is begun
//! from far enough out that the car is already slow when it gets there,
//! because an arcade car with the handbrake culture this one has cannot stop
//! in its own length.
//!
//! What it aims at is the *kerb* beside the marker, not the marker: the
//! passenger is standing on the pavement, and driving onto the pavement to
//! collect them is both wrong and slower.

use crate::drive::{Car, Controls};
use crate::fixed::{self, Fx, ONE};
use crate::road::{self, centre};
use crate::sim::{self, Sim};
use crate::trig::{self, Ang};

use crate::world::{City, Kind, SIZE};

/// How many cells of searching a route may cost before it is given up on.
///
/// A fraction under a third of the grid, which is "most of the city": the
/// search is breadth-first and stops as soon as it arrives, so the budget
/// exists to bound the pathological case where the two ends are on
/// carriageway that is not actually connected, not to limit ordinary trips.
///
/// Written against [`SIZE`] rather than as a number, because it is a
/// fraction of the map and the map has changed size twice.  At a flat 40,000
/// it was most of a 364-cell grid and a third of a 728-cell one, and a
/// budget that is a third of the way across the city is a budget that
/// answers "no route" to any fare on the far side of it - which read as a
/// cab that sat still.
const ROUTE_BUDGET: usize = SIZE * SIZE * 3 / 10;

/// How many route cells either side of a point are used to work out which
/// way the road runs there.  See [`Cabbie::heading_at`].
///
/// It has to be at least as long as the widest staircase the route can take,
/// which is the width of the road it is crossing.  At three, on roads twice
/// as wide as the ones it was chosen for, it averaged out nothing: the
/// heading it reported alternated between the length of the street and its
/// width every few cells, so the lane target flipped sides and - worse - the
/// straight-road detector in [`Cabbie::open_road`] never saw two consecutive
/// cells agree.  The cab was pinned at its cornering pace for 85 per cent of
/// a run down streets that were straight for a block and a half.
const BASELINE: usize = 6;

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
///
/// Half a block, which is what it has always been - the number moved when
/// the block did.
const LOCK_RANGE: i32 = 16;

/// Cross-track gain: lock per cell off the lane line, at a standstill.
///
/// Divided by speed in use, so this is the gain at the bottom of the range.
///
/// Doubled when the car stopped being a boat, and raised again by half when
/// the engine got a curve.  Both changes are the same change: the division
/// by speed was written against a physics where the wheel bought the same
/// yaw at every speed above a crawl, and where the car was at its cruising
/// speed a quarter of a second after any junction.  It now buys less the
/// faster you go (see [`crate::drive`]) and takes over a second to get back
/// up to speed, so the same command moves the car a good deal less and the
/// gain has to make it up.  Measured over four five-minute runs: at the
/// original gain, 500 ticks on the correct side of the road against 600 on
/// the wrong one, which is a cab with no opinion; at two thirds of it, 75,
/// 71, 51 and 81 per cent; at three halves, 83, 79, 77 and 81.
///
/// Then halved, because the roads doubled.  This is lock *per cell* off the
/// line, and a lane is now two cells wide rather than one: the same distance
/// off the middle of it is half as much of a mistake, and a gain that did
/// not know that asked for twice the correction it needed and sawed.
const CROSS: Fx = fixed::ratio(2, 1);

/// The speed above which the lane controller starts easing off, in cells a
/// second.
///
/// Twice the cornering pace rather than at it.  Softening from the cornering
/// pace up means the controller is already backing off at the speed it
/// spends most of its life at, which is the speed it was tuned for; what the
/// softening is actually for is the top of the range, where a correction
/// travels three times as far before it can be seen.
const HASTE_FROM: Fx = fixed::ratio(17, 2);

/// Where in its own half of the road the cab likes to sit, from -1 against
/// the crown to +1 against the kerb.
///
/// This is the bias on a *straight*, and it is faded out as the road ahead
/// runs out - see [`Cabbie::track`].
///
/// Well over towards the kerb, which is where a taxi belongs and is also the
/// half of the half that costs the least when something goes wrong: the cab
/// pulls out to pass parked cars, slow cars and anything it has to dodge,
/// and every one of those movements is *towards the crown*.  Aiming at the
/// middle of its half meant a routine overtake put the car across the paint;
/// aiming at the kerb, the same overtake uses the second lane, which is what
/// the second lane is for.  Measured over four five-minute runs, ticks on
/// the correct side of the road: 57 per cent in the worst city aiming at the
/// middle of the half, 68 aiming at the kerb.
///
/// Held all the way into the corners it was worse than useless.  The kerb
/// line is the *outside* of a left-hander and the inside of a right one, so
/// a cab arriving at a junction hugging it has the least room exactly where
/// it needs the most: off the carriageway for 903 of 2,041 travelling ticks
/// on one city, against 40 once the bias fades.  A driver moves back towards
/// the middle of the road before a turn, and so does this.
const CAB_BIAS: Fx = fixed::ratio(4, 5);

/// Damping: how much lock is taken back per unit of the car's own rate of
/// turn.
///
/// The derivative term, and the one the controller never had.  `CROSS` says
/// how far off the line the car is and `psi` says how far off parallel; both
/// are positions, and a controller made only of positions holds full lock
/// all the way to the line and arrives pointing across it.  Then it corrects
/// the other way.  That is the weave, and no amount of retuning the two
/// proportional gains removes it - it makes them either slow or unstable,
/// which is exactly the pair of failures this went through.
///
/// A half: a car turning as hard as it can gives back half the wheel, which
/// is enough to stop the overshoot and not so much that the cab cannot
/// commit to a corner.  Measured over four five-minute runs, ticks on the
/// correct side of the road: 59 per cent in the worst city with no damping
/// at all, against 64 with this - and the mean speed went from 2.6 cells a
/// second to 3.8, because a controller that is not fighting itself does not
/// have to be slowed down to stay on the road.
const DAMP: Fx = fixed::ratio(1, 2);

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
/// the steering.  Raised with the gain, and by less than the gain, so the
/// cap still bites before the wheel is on the stop.
const CROSS_MAX: Fx = fixed::ratio(3, 5);

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

/// The fastest the autopilot will take a corner, in units per second.
///
/// The figure is what the corner radius says it should be rather than a
/// guess.  Cornering radius in this car grows with the *square* of the
/// speed - about `v^2 / 3.45` cells - so the widest corner the road will
/// take sets the fastest the cab may arrive at one, and nothing else does.
/// Measured over four five-minute runs at the old three-cell lanes, ticks
/// spent off the carriageway while travelling: 9, 54, 41 and 24 per cent at
/// a guessed figure, and 2, 0, 0 and 2 once it came from the radius.
///
/// Which is why it went up when the roads did.  A junction was four cells
/// across and is now eight, so the radius budget doubled and the speed that
/// fits it goes up by the square root of two - not by two.  Cornering pace
/// is the one number on this page that does *not* scale with the city.
const CRUISE_MAX: Fx = fixed::ratio(17, 4);

/// What it will do on a long straight, in cells a second.
///
/// It is only ever asked for where the route runs straight, and the same
/// look-ahead brings it back down before the corner rather than at it - see
/// [`Cabbie::open_road`].
///
/// Three times the cornering pace was tried on the old grid, and four, and
/// both were worse than twice: the blocks were thirteen cells apart, so the
/// straights were short and the extra speed was spent arriving at the
/// junction rather than crossing the city.  Ticks on the correct side of the
/// road over four five-minute runs: 74, 72, 51 and 47 per cent at three
/// times, against 77, 71, 69 and 86 at twice.
///
/// The blocks are twenty-six cells apart now, and a straight that is twice
/// as long is a straight that pays for the braking distance twice over, so
/// three times the cornering pace is what the grid will take.  That is most
/// of the car's unwound top speed and about a third of what it will do with
/// the throttle held, which is the band this was always supposed to be
/// driven in: a demonstration that never leaves the bottom third of the
/// range is not a demonstration of the car.
const OPEN_ROAD: Fx = fixed::ratio(51, 4);

/// How far ahead a straight has to run for the cab to use all of it, in
/// cells.
///
/// Rather more than a block, so it is a real straight and not the far side
/// of one junction seen from the near side of it.
const STRAIGHT_FOR: i32 = 26;

/// How much straight road is needed before any of it is spent going faster
/// than the cornering pace, in cells.
///
/// This is the braking distance, and it is what makes the extra speed usable
/// rather than a way of arriving at the junction sideways.  It goes up with
/// the speed rather than with the city: the car brakes at a fixed rate, so
/// the room it needs grows with the square of what it is coming down from.
const BRAKING_ROOM: i32 = 6;

/// How much speed a cell of clear road ahead is worth, in cells a second per
/// cell.
///
/// Two, so the cab needs three and a half cells of clear road in front of
/// its bumper to be doing [`OPEN_ROAD`] and one to be walking pace.  That
/// also sets how far it has to look - past the point where the answer would
/// exceed the speed it wanted anyway there is nothing to learn - and a short
/// line is the point: this is aimed along the *car*, and a line seven cells
/// long swings a whole cell sideways for four degrees of yaw, so a long one
/// reports the kerb it is passing rather than the wall it is heading for.
const WALL_GAIN: Fx = fixed::ratio(2, 1);

/// How far apart the samples along that line are, in cells.
///
/// Half a cell: a building corner clipped diagonally is about that wide, and
/// stepping a whole cell at a time steps straight over it.
const WALL_STEP: Fx = fixed::HALF;

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
///
/// Half a block, as it always was.
const APPROACH: Fx = fixed::ratio(14, 1);

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
///
/// It is a braking distance from a marker, so it went up with the speed the
/// cab now arrives at rather than with the size of the block.
const CRAWL: Fx = fixed::ratio(5, 1);

/// Speed to be doing at the edge of the circle, in units per second.
///
/// Comfortably under [`crate::sim::STOP_SPEED`], so that the handover
/// happens on the first tick inside the circle rather than after a lap of
/// it.
const CREEP: Fx = fixed::ratio(3, 4);

/// How far ahead the cab looks for something to go round.
///
/// A second and a half at the speed it cruises, which is about as far ahead
/// as a decision to change lanes is worth making: further out and it is
/// dodging cars that will have moved by the time it arrives.  So it follows
/// the cruising speed, not the city.
const DODGE_LOOK: Fx = fixed::ratio(13, 2);
/// Half the width of the corridor in front that counts as blocked.
const DODGE_WIDE: Fx = fixed::ratio(11, 10);
/// The most lock a dodge asks for, on top of whatever the lane wants.
///
/// Deliberately less than half.  A dodge is a *lean* past something, not a
/// swerve round it: the lane controller is still steering, and a term that
/// could overpower it would take the car across the crown of the road to
/// avoid a parked van.
const DODGE_LOCK: Fx = fixed::ratio(3, 10);
/// How long a decision to go one way round is kept, in ticks at 30 Hz.
///
/// Four tenths of a second: long enough that the choice of side cannot
/// dither, short enough that the cab is not still leaning at something it
/// passed half a second ago.
const DODGE_HOLD: u32 = 12;
/// How close something has to be in front before the cab lifts off for it.
const DODGE_CLOSE: Fx = fixed::ratio(7, 2);
/// Below this, a car is not slow, it is stopped - and worth crossing the
/// crown of a narrow street to get round.
const STOPPED: Fx = fixed::ratio(3, 4);
/// How much slower than the cab a car has to be to be worth going round.
const CLOSING: Fx = fixed::ratio(1, 2);
/// How far under its target speed the cab has to be for full throttle.
///
/// A unit and a half a second.  Wide enough that it eases in rather than
/// stamping, and narrow enough that it still gets up to speed out of a
/// junction - which now takes longer, because the engine has a launch curve
/// and the target is higher.
const PACE_BAND: Fx = fixed::ratio(3, 2);

/// How far ahead the cab will look for a coin.
const COIN_LOOK: Fx = fixed::ratio(18, 1);
/// And how far off its line one may be and still be worth having.
///
/// A lane's width, which is now four cells.  Wider than this is a detour,
/// and a cab that detours for coins arrives late, which costs more than the
/// coin is worth.
const COIN_WIDE: Fx = fixed::ratio(4, 1);

/// How far out to the side, and how far ahead, there has to be road for a
/// dodge to be worth starting.
const DODGE_ROOM: Fx = fixed::ratio(3, 1);

/// How far the car may stray from its planned route before the route is
/// assumed to be stale, in cells.
///
/// Being knocked off line by another car does not invalidate a plan - the
/// steering will pull back onto it.  Being three cells off means the car is
/// on a different street, and following the old plan from there is worse
/// than planning again: the aim point is then several cells away across
/// whatever is between, and the car drives at it through a park.
///
/// The cursor only ever sits one waypoint behind the car, so this is
/// measured from somewhere meaningful.  It was six, which on a street grid
/// is wide enough to be on the next street but one, and then three, which
/// turned out to be narrower than a deliberate lane change.
///
/// Five was the figure on the old grid, because the cab *pulls out* to pass
/// things.  A dodge is a lane's width and at three cells it invalidated the
/// plan, so the route was thrown away and rebuilt from wherever the car had
/// swerved to - and a breadth-first search asked the same question from two
/// slightly different places answers it two different ways.  The cab
/// wandered: measured, 921 cells driven and one completed fare in five
/// minutes, against five fares once a plan could survive an overtake.  A
/// lane is twice as wide now, so this is ten.
const OFF_ROUTE: Fx = fixed::ratio(10, 1);

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
    /// Ticks it has spent off the carriageway, for the other stuck check.
    beached: u32,
    /// Whether the last tick was spent reversing out of trouble.
    backing: u32,
    /// Which way the wheel is committed while the car is turned right
    /// round: -1, 0 or 1.  See [`COMMIT`].
    committed: i32,
    /// Which way the wheel goes on the next attempt to back out of a wedge.
    /// Flipped every attempt - see [`Cabbie::unstick`].
    wriggle: i32,
    /// Which side it decided to pass the thing in front on: -1 left, +1
    /// right, 0 not passing anything.
    dodge: i32,
    /// Ticks left on that decision.  A dodge is *committed to*, because the
    /// alternative is a car that picks left, sees the gap on the right,
    /// picks right, and drives into the middle of what it was avoiding.
    dodge_for: u32,
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
            beached: 0,
            backing: 0,
            committed: 0,
            wriggle: 1,
            dodge: 0,
            dodge_for: 0,
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
        // The kerb beside the marker, not the marker: the passenger is on
        // the pavement and the cab is not allowed there.
        let Some(goal) = sim.drive_target() else {
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
        // How fast the car is already turning, for the damping term in both
        // steering laws.  Read once, here, because it is a property of the
        // car and not of whichever controller happens to be driving.
        let rate = taxi.turn_rate(hz);
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

        self.unstick(city, sim, hz);
        if self.backing > 0 {
            // Off the wall, wheel over, so the nose swings clear rather than
            // grinding along it.  Alternate attempts go *forwards* instead:
            // the back bumper is solid now - a car cannot reverse into a
            // building any more than it can drive into one - so a cab wedged
            // with its tail against a wall has nowhere to back into, and
            // reversing at it forever is how one fare in five minutes
            // happens.  A driver in that position pulls forward and tries
            // again.
            return Controls {
                throttle: if self.wriggle > 0 { -ONE } else { ONE },
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
        // How much straight road there is to use.  Read before the steering
        // rather than after it, because where in its half of the road the
        // cab wants to be depends on it - see [`CAB_BIAS`].
        let open = self.open_road();
        let (err, steer, range) = match self.track(city, taxi, open) {
            Some(t) if to_goal >= APPROACH => {
                let psi = t.heading.wrapping_sub(taxi.yaw) as i16 as i32;
                (psi, self.hold_lane(psi, t.right_of_lane, taxi.speed(), rate), CRUISE_MAX)
            }
            _ => {
                // The route first, and a coin only if the route is
                // more or less straight ahead.  Leaning at a coin while the
                // car is already turning adds the two corners together and
                // takes it over the kerb: measured, coin-seeking through
                // corners spent 21 per cent of travelling ticks off the
                // carriageway against 2.
                let route = self.aim(goal);
                let straight = bearing_error(taxi, route.0, route.1).abs() < LIFT;
                let (ax, ay) = match self.coin(sim) {
                    Some(c) if straight => c,
                    _ => route,
                };
                // The lock band is set by how far away the point being
                // steered at is, which is what makes this a pursuit rather
                // than a bearing hold: the same lateral error needs more
                // lock to correct over three cells than over thirty.  Using
                // the range to the marker instead was tried and costs both
                // measurements - the car understeers out of junctions
                // because the marker is still a long way off.
                let range = dist(taxi.x, taxi.y, ax, ay);
                let e = bearing_error(taxi, ax, ay);
                (e, self.steer_for(e, range, rate), range)
            }
        };
        let _ = range;

        // Something in the way, and which side it is being passed on.
        let (lean, cap) = self.avoid(city, sim, hz, open);

        // One speed target, from whichever of the three reasons to slow down
        // is the more pressing: the corner being taken, the circle being
        // arrived at, and the car in front.  Expressing them as a speed
        // rather than as competing throttle rules is what stops them from
        // cancelling each other out.
        // And a speed that leaves room to stop before whatever the car is
        // actually pointed at, which is what keeps it out of the buildings
        // when it is not where its route thinks it is.
        let look = fixed::div(open, WALL_GAIN);
        let room = Self::room_ahead(city, taxi, look);
        let wall = CREEP + fixed::mul(room, WALL_GAIN);

        let want = corner_speed(err, open)
            .min(approach_speed(to_goal, open))
            .min(cap)
            .min(wall);
        Controls {
            throttle: pace(vf, taxi.speed(), want),
            steer: fixed::clamp(steer + lean, -ONE, ONE),
            // Sideways on purpose, on the tightest corners only.  The car
            // has enough grip to take an ordinary junction without it, and a
            // demonstration that slides through every turn reads as broken
            // rather than as fast.
            handbrake: err.abs() > HARD && vf > CRUISE_MAX,
        }
    }

    /// Where to aim when there is no lane line to hold.
    ///
    /// A few cells up the route, not the marker.  Aiming at the marker is
    /// the obvious thing and is only right when the two are the same
    /// direction, which they are not the moment the route has to go round
    /// anything.  The case that made this unmissable is the crossing of two
    /// arterials: that junction is fourteen cells square, no cell in it
    /// belongs to a single street, so the lane controller has nothing to say
    /// for the two seconds the car is inside it - and the car spent those
    /// two seconds driving at a marker twenty cells away on the far side of
    /// a block, arriving at the kerb of a street it had no route down.
    /// Measured on one city: one fare in five minutes, against eight.
    ///
    /// Far enough ahead to be worth steering at, near enough to still be on
    /// the road: the route is cells, and a point three of them up it is
    /// about a car's length past the junction exit.
    fn aim(&self, goal: (Fx, Fx)) -> (Fx, Fx) {
        if self.route.is_empty() {
            return goal;
        }
        let i = self.at + BASELINE;
        // Past the end of the route is the goal itself, which is where the
        // route was going: the last stretch is aimed at the stopping circle
        // rather than at the cell it is painted in.
        if i >= self.route.len() {
            return goal;
        }
        // The middle of that cell, and not the middle of the lane it is in.
        // Aiming at the lane was tried, on the reasoning that a junction is
        // where a car picks which side of the next street to come out on,
        // and it is worse: the route through a junction staircases, so the
        // local heading a lane offset would be taken perpendicular to is as
        // often across the road as along it, and the aim point jumps a lane
        // and a half from tick to tick.  On one city it cost five fares out
        // of six.  The lane is regulated by `hold_lane` a moment later, from
        // the road under the car, where the heading is not a guess.
        centre(self.route[i].0, self.route[i].1)
    }

    /// A coin worth going slightly out of the way for.
    ///
    /// Coins are strung along the route the cab is already driving, so most
    /// of them are collected by driving; this is for the ones a lane change
    /// or a lane's width of drift would pick up.  It only looks at coins in
    /// front, within a couple of car lengths of the line it is already on,
    /// and it takes the nearest - so it is a *lean*, not a detour, and the
    /// cab never turns round for money.
    ///
    /// Worth doing because a coin is worth three things: two seconds on the
    /// clock, a unit of money, and three seconds of boost.  A cab that
    /// drives past them because they are not exactly on its route is leaving
    /// the fare's whole margin on the road.
    fn coin(&self, sim: &Sim) -> Option<(Fx, Fx)> {
        let fare = sim.fare.as_ref()?;
        let taxi = &sim.taxi;
        let (fx, fy) = (trig::cos(taxi.yaw), trig::sin(taxi.yaw));
        let (rx, ry) = (-fy, fx);
        let mut best: Option<(Fx, Fx, Fx)> = None;
        for c in &fare.coins {
            if c.taken {
                continue;
            }
            let (dx, dy) = (c.x - taxi.x, c.y - taxi.y);
            let lon = fixed::mul(dx, fx) + fixed::mul(dy, fy);
            if lon <= 0 || lon > COIN_LOOK {
                continue;
            }
            let lat = fixed::mul(dx, rx) + fixed::mul(dy, ry);
            if fixed::abs(lat) > COIN_WIDE {
                continue;
            }
            if best.is_none_or(|(l, _, _)| lon < l) {
                best = Some((lon, c.x, c.y));
            }
        }
        best.map(|(_, x, y)| (x, y))
    }

    /// Steer to sit on the lane line and point along it.
    ///
    /// Three terms, and they are a PD controller on the lane.  `psi` turns
    /// the car parallel to the road and `right_of_lane` walks it sideways
    /// onto the line - those are the proportional halves, one on angle and
    /// one on offset - and [`DAMP`] takes lock back in proportion to how
    /// fast the car is already turning, which is the derivative.
    ///
    /// The cross-track term is divided by speed as well, so the same offset
    /// produces a gentle correction at speed and a firm one at a crawl: the
    /// alternative is a car that snakes down every straight because the
    /// correction that suits a junction is violent on an avenue.
    ///
    /// There is no integral term and there should not be one.  The thing an
    /// integrator fixes is a steady-state offset from a constant
    /// disturbance, and a lane has none: the only persistent offsets here
    /// are deliberate ones - a dodge, an overtake - and an integrator would
    /// spend them winding up and then unwind into the kerb on the way out.
    fn hold_lane(&mut self, psi: i32, right_of_lane: Fx, speed: Fx, rate: Fx) -> Fx {
        if psi.abs() >= COMMIT {
            // Pointing the wrong way down the street: this is a turn, not a
            // lane correction, and the latch owns it.
            return self.steer_for(psi, CRUISE_MAX, rate);
        }
        self.committed = 0;
        // Softer with speed, and both terms by the *same* divisor.  The lock
        // that holds a lane at the cornering pace is a swerve at three times
        // it: the car turns the same radius per cell of road either way, but
        // it covers three times as many cells before the correction it just
        // made has shown up in the measurement.
        //
        // The two terms used to be softened by two different things - the
        // angle by the speed ratio, the offset by the speed itself, which is
        // four times larger at the top of the range - so at open-road pace
        // the offset term was a fortieth of what it is at a crawl and the
        // controller was almost pure damping: stable, and with no opinion
        // about which lane it was in.  The cab wandered across the crown on
        // every long straight and the keeping-right measurement fell to a
        // coin toss.
        let haste = fixed::div(speed, HASTE_FROM).max(ONE);
        let angle = fixed::div(fixed::ratio(psi, FULL_LOCK), haste);
        let pull = fixed::clamp(
            fixed::div(fixed::mul(CROSS, right_of_lane), haste),
            -CROSS_MAX,
            CROSS_MAX,
        );
        // And the derivative: take lock back in proportion to how fast the
        // car is already turning, so the correction stops when the car has
        // started to answer it rather than when it has finished.
        let damp = fixed::mul(DAMP, rate);
        fixed::clamp(angle - pull - damp, -ONE, ONE)
    }

    /// Where the road wants the car to be, at its current place on the route.
    ///
    /// `None` when there is no road to speak of - inside a junction, off the
    /// end of the route, or with no route at all - which is the caller's cue
    /// to go back to aiming at a point.
    fn track(&self, city: &City, taxi: &Car, open: Fx) -> Option<Track> {
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
        let d = if along_x { (dir, 0) } else { (0, dir) };
        // Out to the kerb on a straight, back to the middle of the half for
        // a corner.  See [`CAB_BIAS`].
        let t = fixed::clamp(fixed::div(open - CRUISE_MAX, OPEN_ROAD - CRUISE_MAX), 0, ONE);
        let (lx, ly) = road::lane_biased(city, cx, cy, d, fixed::mul(CAB_BIAS, t));
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
    fn steer_for(&mut self, err: i32, range: Fx, rate: Fx) -> Fx {
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
        // Damped, for the same reason the lane controller is: a pursuit
        // law with no derivative arrives at its aim point with the wheel
        // still wound on and has to catch itself on the far side.  The
        // latched branches above are deliberately left undamped - a latch is
        // a decision to complete a turn, and damping it is asking the car to
        // fight the corner it has just committed to.
        let damp = fixed::mul(DAMP, rate);
        fixed::clamp(fixed::ratio(err, full_lock_at(range)) - damp, -ONE, ONE)
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

    /// How fast the road ahead allows, from how far it runs straight.
    ///
    /// The cab used to pick one speed and hold it everywhere, which is a
    /// tidy way to drive and wastes the whole top half of the car: the
    /// throttle's wind-up only happens to somebody who *holds* the throttle,
    /// and a driver pacing himself at a constant three never does.
    ///
    /// So the target is the road: a block of straight ahead is worth
    /// [`OPEN_ROAD`], a corner is worth [`CRUISE_MAX`], and everything
    /// between is between.  The braking falls out of it - the count shrinks
    /// as the corner approaches, so the target comes down over the cells
    /// before it rather than at it, which is what makes it possible to use
    /// the speed at all.
    ///
    /// "Straight" is measured with the same coarse heading the lane
    /// controller uses, over `BASELINE` cells, because a route across a wide
    /// road staircases and every other cell of it is a turn.
    fn open_road(&self) -> Fx {
        if self.at + 1 >= self.route.len() {
            return CRUISE_MAX;
        }
        let (hx, hy) = self.heading_at(self.at);
        let (hx, hy) = (hx.signum(), hy.signum());
        let mut clear = 0;
        while clear < STRAIGHT_FOR && self.at + clear as usize + 1 < self.route.len() {
            let (nx, ny) = self.heading_at(self.at + clear as usize);
            if (nx.signum(), ny.signum()) != (hx, hy) {
                break;
            }
            clear += 1;
        }
        let room = fixed::from_int((clear - BRAKING_ROOM).max(0));
        let t = fixed::div(room, fixed::from_int(STRAIGHT_FOR - BRAKING_ROOM));
        fixed::lerp(CRUISE_MAX, OPEN_ROAD, t.clamp(0, ONE))
    }

    /// How far it is to the first thing the cab cannot drive on.
    ///
    /// Measured from the bumper, straight along the way the car is pointing,
    /// which is deliberately not the way the *route* goes: the route is
    /// always on the road, and the whole point of this is to catch the times
    /// when the car is not on the route - understeering out of a junction,
    /// halfway through a dodge, sliding.  The route's own corners are dealt
    /// with in [`Cabbie::open_road`], well before this notices them.
    ///
    /// Returns the look-ahead distance itself when the line is clear.
    fn room_ahead(city: &City, taxi: &Car, look: Fx) -> Fx {
        let (fx, fy) = (trig::cos(taxi.yaw), trig::sin(taxi.yaw));
        let nose = taxi.kind.half_len();
        let mut d = 0;
        while d < look {
            let ahead = nose + d;
            let x = fixed::floor(taxi.x + fixed::mul(fx, ahead));
            let y = fixed::floor(taxi.y + fixed::mul(fy, ahead));
            if !city.drivable(x, y) {
                return d;
            }
            d += WALL_STEP;
        }
        look
    }

    /// Go round the thing in front rather than into it.
    ///
    /// Returns the lock to add to whatever the lane wants, and the fastest
    /// it should be going.
    ///
    /// # Committing
    ///
    /// The decision is which *side* to pass on, and it is held for
    /// [`DODGE_HOLD`] ticks whatever happens in between.  Deciding it fresh
    /// every tick is the obvious version and it is much worse than doing
    /// nothing: a car a little to the left is passed on the right, which
    /// moves it to the right in the frame, which asks for a pass on the
    /// left, and the cab drives up the middle of what it was avoiding at
    /// full lock in alternating directions.
    ///
    /// The side is chosen from where the obstacle sits: something on your
    /// left is passed on the right.  Where it is dead ahead, the tie is
    /// broken towards the middle of the road rather than towards the kerb,
    /// because the kerb is where the lamp posts are.
    fn avoid(&mut self, city: &City, sim: &Sim, hz: i32, open: Fx) -> (Fx, Fx) {
        let taxi = &sim.taxi;
        let (fx, fy) = (trig::cos(taxi.yaw), trig::sin(taxi.yaw));
        let (rx, ry) = (-fy, fx);

        // How far ahead to look, and how close is too close, both in cells
        // and both scaled by how fast the cab is going.  The distances were
        // tuned when it drove everywhere at the cornering pace, where a
        // fixed number of cells is a fixed number of *seconds*; at three
        // times that speed the same four and a half cells is a sixth of a
        // second's warning, which is no warning at all.
        let haste = fixed::div(taxi.speed(), CRUISE_MAX).max(ONE);
        let look = fixed::mul(DODGE_LOOK, haste);
        let close = fixed::mul(DODGE_CLOSE, haste);

        // The nearest thing in the corridor ahead.
        // (how far up, how far right, how fast it is going)
        let mut near: Option<(Fx, Fx, Fx)> = None;
        for c in &sim.traffic {
            let (dx, dy) = (c.x - taxi.x, c.y - taxi.y);
            let lon = fixed::mul(dx, fx) + fixed::mul(dy, fy);
            if lon <= 0 || lon > look {
                continue;
            }
            let lat = fixed::mul(dx, rx) + fixed::mul(dy, ry);
            let room = DODGE_WIDE + c.kind.half_len();
            if fixed::abs(lat) > room {
                continue;
            }
            // Only things it is actually catching.  A car ahead doing the
            // same speed is not an obstacle, it is the traffic, and pulling
            // out for it means spending the whole street in the wrong lane:
            // measured, dodging everything in front took the cab's
            // right-hand-lane figure from 85 per cent to 59.
            let theirs = fixed::mul(c.vx, fx) + fixed::mul(c.vy, fy);
            let mine = fixed::mul(taxi.vx, fx) + fixed::mul(taxi.vy, fy);
            if theirs > mine - CLOSING {
                continue;
            }
            // Bumper to bumper, so a bus is felt where its back is.
            let gap = lon - taxi.kind.half_len() - c.kind.half_len();
            if near.is_none_or(|(g, _, _)| gap < g) {
                near = Some((gap, lat, c.speed()));
            }
        }

        if self.dodge_for > 0 {
            self.dodge_for -= 1;
        }
        let Some((gap, lat, _)) = near else {
            if self.dodge_for == 0 {
                self.dodge = 0;
            }
            return (0, open);
        };

        // On a two-cell street the only room to pass is the oncoming lane,
        // and pulling into it to get round traffic that is merely slower
        // than you is how a cab spends half its life on the wrong side of
        // the road - measured, 57 per cent on the correct side against 85.
        // So a narrow street is only overtaken on for something that has
        // actually stopped: a wreck, a queue, a bus at a stop.
        let (cx, cy) = (fixed::floor(taxi.x), fixed::floor(taxi.y));
        let lanes = match road::street_axis(city, cx, cy) {
            Some(true) => city.plan.rows.at(cy).width,
            Some(false) => city.plan.cols.at(cx).width,
            None => 2,
        };
        let stopped = near.map(|(_, _, v)| v < STOPPED).unwrap_or(false);
        if lanes < 3 && !stopped {
            self.dodge = 0;
            self.dodge_for = 0;
            let (gap, _, _) = near.unwrap();
            let cap = if gap < close {
                fixed::div(fixed::mul(open, gap.max(0)), close)
            } else {
                open
            };
            return (0, cap);
        }

        if self.dodge == 0 || self.dodge_for == 0 {
            // Pass on the side it is not on, and take the kerb side when it
            // is dead ahead.  The other way round is the tidier-looking
            // choice and it is wrong: on a road where the traffic keeps
            // right, the space to the left of the thing in front is the
            // oncoming lane, and the space to the right is a kerb with lamp
            // posts on it that go over when you touch them.
            let first = if lat < -fixed::ratio(1, 8) { 1 } else { -1 };
            // ...but only where there is road to do it on.  Without this the
            // cab pulls out onto the pavement to get round a parked van,
            // which is worse than waiting behind it: measured, dodging with
            // no regard for the kerb spent 43 per cent of its travelling
            // ticks off the carriageway, against 2.
            let room = |side: i32| {
                let (rx, ry) = (-fy, fx);
                let out = fixed::mul(fixed::from_int(side), DODGE_ROOM);
                let x = taxi.x + fixed::mul(rx, out) + fixed::mul(fx, DODGE_ROOM);
                let y = taxi.y + fixed::mul(ry, out) + fixed::mul(fy, DODGE_ROOM);
                city.drivable(fixed::floor(x), fixed::floor(y))
            };
            self.dodge = if room(first) {
                first
            } else if room(-first) {
                -first
            } else {
                0
            };
            self.dodge_for = DODGE_HOLD * hz.max(1) as u32 / 30;
        }

        // Harder the closer it is, and nothing at all once it is behind the
        // bumper - by then the lane controller has it.
        let urgency = fixed::clamp(fixed::div(look - gap.max(0), look), 0, ONE);
        let lean = fixed::mul(fixed::mul(fixed::from_int(self.dodge), DODGE_LOCK), urgency);
        // And lift off if it is close enough that steering alone will not do
        // it, which is what stops the cab from rear-ending a queue.
        let cap = if gap < close {
            fixed::div(fixed::mul(open, gap.max(0)), close)
        } else {
            open
        };
        (lean, cap)
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
    fn unstick(&mut self, city: &City, sim: &Sim, hz: i32) {
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
        // Nor is crawling along a pavement.  Wedged is not always *stopped*:
        // a car that has climbed a kerb and is grinding down a shop front at
        // a cell a second passes every speed test there is, and it can do it
        // for a minute - measured, 1,000 ticks of one run, which is a third
        // of it, and the speed never once fell far enough to trip the check
        // above.  Being off the carriageway at all is only worth noticing
        // after a second or so, because clipping a corner on the way round a
        // junction is normal and is over in a few ticks.
        if city.at(fixed::floor(sim.taxi.x), fixed::floor(sim.taxi.y)).kind == Kind::Road {
            self.beached = 0;
        } else {
            self.beached += 1;
        }
        if self.stalled > hz / 2 || self.beached > hz {
            self.stalled = 0;
            self.beached = 0;
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
/// trap: below its reference speed this car's steering authority is
/// proportional to speed, so a car commanded to a standstill while pointing
/// the wrong way can no longer turn and stays pointing the wrong way.  The
/// floor is a crawl - and it is the speed at which the car turns its very
/// hardest, which is the right place for the sharpest corners to be taken.
fn corner_speed(err: i32, open: Fx) -> Fx {
    let e = err.abs();
    if e <= LIFT {
        return open;
    }
    // Between lifting off and standing on it, a ramp rather than a step.
    //
    // It was a step - full speed below `LIFT`, half the cornering pace
    // above it - and a step is a controller that has only two opinions.
    // Every error of thirty-nine degrees was treated as an emergency, so a
    // cab tidying up its line out of a junction threw away two thirds of its
    // speed for a tenth of a second and then had to build it again; measured,
    // that branch was the binding limit on half of all driving ticks and the
    // mean speed sat at two thirds of the cornering pace on roads wide
    // enough to take all of it.
    let t = fixed::ratio((e - LIFT).min(HARD - LIFT), HARD - LIFT);
    fixed::lerp(open, TIGHT, t.clamp(0, ONE)).max(TIGHT)
}

/// The pace for a corner being taken at the limit of the wheel, in cells a
/// second.
///
/// Below this there is nothing left to slow down to: the car turns inside
/// five metres at any speed under [`crate::drive`]'s reference, so a tighter
/// corner is not bought by going slower still, it is bought by stopping.
const TIGHT: Fx = fixed::ratio(3, 2);

/// The fastest it is sensible to be going this far from the circle.
///
/// A straight ramp from cruising pace down to a crawl at the paint.  A ramp
/// rather than a braking distance, because braking distance depends on the
/// speed the car happens to be doing and this does not: whatever it arrives
/// at the top of the ramp doing, it leaves the bottom at a crawl.
fn approach_speed(to_goal: Fx, open: Fx) -> Fx {
    let t = fixed::clamp(fixed::div(to_goal - CRAWL, APPROACH - CRAWL), 0, ONE);
    fixed::lerp(CREEP, open, t)
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
    // Over the target: brake, unless it is already crawling, in which case
    // a negative throttle is reverse.
    if speed > want + fixed::HALF {
        return if vf > ROLLING { -ONE } else { 0 };
    }
    // Under it: throttle in proportion to how far under, full only when a
    // whole unit short.
    //
    // Not the flat "full throttle until you are nearly there" this used to
    // be, and the reason is that the pedal now has a memory: holding it down
    // for half a second raises the car's top speed a step, and the cab held
    // it down as a matter of course, so it wound itself up past its own
    // cruising speed and arrived at every corner too fast.  Measured, that
    // took one city from 79 per cent of travelling ticks on the correct side
    // of the road to 50.  Easing in is also what a driver does.
    fixed::clamp(fixed::div(want - vf, PACE_BAND), 0, ONE)
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
    use crate::world::{City, Kind};
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
        /// The fastest it went, in cells a second.
        peak: Fx,
        /// Speed summed over driving ticks, and how many there were, for a
        /// mean.
        sum: i64,
        driving: u32,
        /// Driving ticks spent above twice the cornering pace.
        fast: u32,
        /// The highest step of the throttle's wind-up it reached.
        stepped: u32,
        /// Ticks spent on a coin's boost.
        boosted: u32,
        /// Coins collected.
        coins: u32,
        /// Cars and walls hit.
        bumps: u32,
        /// Ticks spent escaping.
        escaping: u32,
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
                    Event::Coin => r.coins += 1,
                    Event::Rammed | Event::Crunched => r.bumps += 1,
                    _ => {}
                }
            }
            // Only while it is driving: the escape manoeuvre holds the
            // throttle wide open against a wall, which winds the engine all
            // the way up and says nothing at all about how the cab drives.
            if cab.backing > 0 {
                r.escaping += 1;
            } else {
                let speed = dist(0, 0, sim.taxi.vx, sim.taxi.vy);
                r.peak = r.peak.max(speed);
                r.stepped = r.stepped.max(sim.taxi.stepped);
                r.sum += speed as i64;
                r.driving += 1;
                if speed > CRUISE_MAX * 2 {
                    r.fast += 1;
                }
            }
            if sim.taxi.boost > 0 {
                r.boosted += 1;
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

    /// The demonstration drives the whole car.
    ///
    /// The autopilot is what most people will ever see of this program, and
    /// for a long time it drove like a milk float: it held one speed
    /// everywhere, so the engine's wind-up - which only happens to a driver
    /// who *keeps* the throttle down - never started, and the top half of
    /// the speed range existed only for somebody who took the wheel.
    ///
    /// So this is a test about the demonstration rather than about the
    /// driving: that the cab spends real time above twice its cornering
    /// pace, that its average is not the one speed it used to hold, and that
    /// it picks up coins and spends the boost.  Measured over five minutes:
    /// 4, 3, 4 and 4 per cent of driving ticks above twice the cornering
    /// pace, against 1, 0, 1 and 1 for the milk float; means of 3.35, 2.85,
    /// 3.00 and 3.08 cells a second; and 3,000 to 6,000 of the 9,000 ticks
    /// on a coin's boost.
    ///
    /// The mean is asserted against a *fraction* of the cornering pace, and
    /// the fraction moved when the roads doubled.  It used to be four
    /// fifths, on a grid where the open-road target was twice the cornering
    /// pace and there was nowhere to use it; the target is now three times
    /// the cornering pace and the straights are long enough to reach it, so
    /// the cab's speed is spread over a much wider band and its *mean* sits
    /// lower against the top of that band while being higher in absolute
    /// terms - 3.1 cells a second against 2.65.  Two thirds is the bar.
    ///
    /// Peak speed is deliberately not what is asserted on, and the escape
    /// manoeuvre is why: it holds the throttle wide open against a wall, so
    /// a badly stuck cab peaks higher than a well driven one.  Every figure
    /// here is measured with the escape excluded.
    #[test]
    fn the_demonstration_drives_the_whole_car() {
        let mut worst = (u32::MAX, Fx::MAX, u32::MAX);
        for seed in [1u32, 7, 99, 4242] {
            let r = run(seed, 9_000);
            let mean = (r.sum / r.driving.max(1) as i64) as Fx;
            println!(
                "seed {seed}: mean {}.{:02} fast {}% peak {}.{:02} step {} boost {} coins {} bumps {} escaping {}",
                mean / ONE,
                (mean % ONE) * 100 / ONE,
                r.fast * 100 / r.driving.max(1),
                r.peak / ONE,
                (r.peak % ONE) * 100 / ONE,
                r.stepped,
                r.boosted,
                r.coins,
                r.bumps,
                r.escaping
            );
            worst = (worst.0.min(r.fast * 100 / r.driving.max(1)), worst.1.min(mean), worst.2.min(r.boosted));
        }
        assert!(worst.0 >= 2, "only {}% of driving ticks above twice the cornering pace", worst.0);
        assert!(worst.1 > CRUISE_MAX * 2 / 3, "mean speed only {}", worst.1 / ONE);
        assert!(worst.2 > 0, "never used a coin's boost");
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
            // Under a fifth.  Measured at 2, 0, 0 and 2 per cent, which is
            // most of a car's width of margin: it was about 40 per cent
            // while the cab cruised faster than it could corner, and 79 and
            // 86 for two early versions that simply drove across the city
            // ignoring the roads.  A fifth is therefore a long way from
            // where the cab drives and a long way from where a broken one
            // does, which is what a bar on a chaotic measurement is for.
            assert!(
                r.strayed * 5 < r.travelling,
                "seed {seed}: off the road for {} of {} travelling ticks",
                r.strayed,
                r.travelling
            );
        }
    }

    /// The cab keeps right.
    ///
    /// Measured only while travelling, on cells where exactly one axis is a
    /// street - inside a junction there is no right-hand side to be on - and
    /// only when the car is moving along the road rather than across it.
    ///
    /// It was a coin toss for a long time and the test said so.  What fixed
    /// it was not the lane target, which is where the effort went: it was
    /// the *aim point* when there is no lane to hold.  The car fell back to
    /// steering at the marker, so every junction - and the crossing of two
    /// arterials is fourteen cells of junction - was a stretch of driving at
    /// a point on the far side of a block, arriving at whatever lane the
    /// geometry happened to produce.  Aiming up the route instead, in
    /// [`Cabbie::aim`], took the split from 68, 55, 79 and 65 per cent to
    /// 70, 84, 88 and 82.  The engine's acceleration curve then moved them
    /// again, because a car that takes a second and three quarters to get
    /// back up to speed spends longer at the speeds where the cross-track
    /// term is divided by a smaller number, and [`CROSS`] was raised by half
    /// to settle them at 83, 79, 77 and 81.
    ///
    /// The bar is three ticks on the right for every two on the wrong side.
    /// It is deliberately well below the measurement: what is being defended
    /// is that the car has a side and holds it, and the figure moves by a
    /// few per cent whenever anything about the driving changes.
    #[test]
    fn the_cab_keeps_right() {
        // Every seed is measured before any of them is judged.  A chaotic
        // measurement that stops at the first city under the bar tells you
        // nothing about the other three, and retuning against one number at
        // a time is how a controller ends up fitted to seed 1.
        let mut worst = 100;
        for seed in [1u32, 7, 99, 4242] {
            // Nine thousand ticks - five minutes - and not the three
            // thousand this used to run.  The measurement is chaotic:
            // whether a given fare happens to be down a street the cab
            // reaches on the wrong side is worth ten points either way, and
            // over fifty seconds there are not enough fares for that to
            // average out.  Two settings a hair apart read 38 and 65 per
            // cent on the same city at three thousand, which is a
            // measurement of the seed rather than of the driving.
            let r = run(seed, 9_000);
            let total = r.right + r.wrong;
            assert!(total > 300, "seed {seed}: only {total} ticks on a road with a side");
            let pct = r.right * 100 / total.max(1);
            println!("seed {seed}: {} ticks on the right, {} on the wrong side - {pct}%", r.right, r.wrong);
            worst = worst.min(pct);
        }
        assert!(worst >= 60, "worst city was only {worst} per cent on the right");
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

    /// Anywhere in the built city can be driven to from anywhere else in it.
    ///
    /// The coordinates are measured from the middle rather than written
    /// down, because the map grew a suburb, a ring of farmland and a coast
    /// around the outside of it: the absolute numbers that used to be
    /// "ordinary places on the grid" are now somebody's field.
    #[test]
    fn a_route_exists_between_ordinary_places_on_the_grid() {
        let city = City::generate(7);
        let mid = SIZE as i32 / 2;
        let reach = crate::zone::CITY_EDGE * crate::zone::BLOCK_PITCH as i32 - 8;
        let mut found = 0;
        for i in 0..8 {
            let a = (mid - reach + i * 7, mid - reach + i * 3);
            let b = (mid + reach - i * 5, mid + reach - i * 9);
            if city.drive_route(a, b, ROUTE_BUDGET).is_some() {
                found += 1;
            }
        }
        assert_eq!(found, 8, "the carriageway is not one connected network");
    }

    #[test]
    fn the_route_it_returns_is_all_carriageway_and_joined_up() {
        let city = City::generate(7);
        let mid = SIZE as i32 / 2;
        let r = city
            .drive_route((mid - 40, mid - 40), (mid + 40, mid + 60), ROUTE_BUDGET)
            .expect("no route");
        for &(x, y) in &r {
            assert!(city.drivable(x, y), "the route runs over {x},{y}, which is not road");
        }
        for w in r.windows(2) {
            let d = (w[0].0 - w[1].0).abs() + (w[0].1 - w[1].1).abs();
            assert_eq!(d, 1, "the route jumps from {:?} to {:?}", w[0], w[1]);
        }
    }
}




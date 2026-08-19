//! The city that moves: street furniture, traffic, pedestrians, and the
//! fare.
//!
//! # What the rules are
//!
//! - **Buildings are rigid.** Nothing you can do moves one.
//! - **Everything else on the pavement is not.**  Lamp posts, mailboxes,
//!   hydrants, meters and signals go over when you hit them, take a
//!   velocity and a lean, and stay down.  None of it stops the car.
//! - **Traffic is skittles, but it is not scenery.**  Other cars take a full
//!   impulse exchange and go spinning; a bus does not.  Between shunts they
//!   keep the right-hand lane, ease off for what is in front of them, give
//!   way to anything crossing from their right, and collide with each other
//!   as well as with you - see [`crate::road`] for the lane rules, which the
//!   autopilot reads from the same place.
//! - **People are not on the road.**  A fare waits on the pavement, in a
//!   plaza or in a park, and the circle is painted where they stand.  The
//!   cab pulls up at the kerb beside it; the last step is theirs.
//! - **The car never breaks.**  Damage accumulates and shows, and that is
//!   all it does.  A run that ends because the vehicle failed is a run that
//!   stopped being about pace.
//! - **The fare is the clock.**  Time only comes from picking up, dropping
//!   off, and the coins strung along the route.  There is no other source,
//!   so the only way to keep playing is to keep moving.
//!
//! # What is streamed and what is not
//!
//! The whole city's furniture exists from the start - about four hundred
//! items, which is nothing - and is culled at draw time by distance.
//! Traffic and pedestrians are a fixed-size pool that is *recycled*: when
//! one gets more than a few blocks behind you it is picked up and put back
//! down somewhere ahead.  A fixed pool is the shape a 64 KB machine can also
//! run, which is why it is the shape here.

use crate::atmos::Atmos;
use crate::camera::Camera;
use crate::drive::{self, Car, CarKind, Controls};
use crate::fixed::{self, Fx, ONE};
use crate::frame::Frame;
use crate::palette;
use crate::raycast::Proj;
use crate::rng::{hash3, Rng};
use crate::road;
use crate::sprite::{Billboard, Stamp};
use crate::trig::{self, Ang};
use crate::walk::Foot;
use crate::world::{City, Kind, SIZE};

/// How many other vehicles are in the pool.
///
/// This is really a *density*: the pool is recycled within [`RECYCLE`] cells
/// of the cab, so what the number sets is how many cars share that disc.
/// Twelve was right when the disc was thirty-four cells across and the
/// streets were two cells wide - and it was arrived at the hard way, because
/// the autopilot's completion rate is an unusually sharp measurement of
/// traffic density: at thirty-six the cab managed one fare in five minutes,
/// at twenty and sixteen it managed two, and at twelve it was back to six.
/// A cab that cannot get down the street is not something you tune your way
/// out of by a few per cent.
///
/// The disc is now four blocks, because cars have to appear at the far end
/// of the street rather than in the lane beside you, and four blocks of
/// doubled streets is nine times the road.  Sixty-four keeps the same cars
/// per cell of carriageway as twelve did, and the streets are twice as wide,
/// so the queue that number was defending against cannot form: there is room
/// to go round.
pub const TRAFFIC: usize = 64;
/// How many pedestrians are in the pool.
pub const PEDS: usize = 48;
/// Beyond this many cells, a pooled actor is recycled somewhere nearer.
///
/// Four blocks, and it has to be more than [`SPAWN_NEAR`] or the pool would
/// recycle a car the moment it arrived.  What sets it is the draw distance:
/// a car that vanishes while you can still see it is the same failure as one
/// that appears while you can, so this is beyond where the haze has taken
/// the city.
pub const RECYCLE: i32 = 4 * crate::zone::BLOCK_PITCH as i32;
/// How near the cab the next fare may be waiting, in cells.
///
/// A block.  It was four cells - the length of two cars - which puts the
/// circle for the next job on the pavement you are already stopped at, so
/// the whole job is "turn round".  A fare is somewhere you have to *drive*
/// to, and the shortest distance that means anything on this grid is one
/// block: [`crate::zone::BLOCK_PITCH`] is the nominal road-to-road spacing,
/// an eight-cell block with a pavement each side and a road.
pub const HAIL_NEAR: i32 = crate::zone::BLOCK_PITCH as i32;
/// And how far, so the pickup is a short drive rather than a trek.
pub const HAIL_FAR: i32 = HAIL_NEAR * 2;
/// How far the drop-off has to be from the pickup, in cells.
///
/// A block again, and measured from the *pickup* rather than from the cab -
/// which is the distance the passenger is paying for.  Both being far from
/// the cab says nothing about them being far from each other: a pickup a
/// block north and a drop-off a block and a half north is a fare of four
/// cells.
pub const FARE_MIN: i32 = HAIL_NEAR;
/// Seconds on the clock at the start of a shift.
pub const START_TIME: i32 = 60;
/// Seconds a coin is worth.
pub const COIN_TIME: i32 = 2;
/// Ticks of boost a coin is worth.
///
/// Three seconds of twice the engine and twice the top speed, spent only
/// while the throttle is down.  A coin is therefore worth three things at
/// once - time, money, and speed - which is what makes a trail of them worth
/// following rather than worth ignoring in favour of the shortest line to
/// the marker.
pub const COIN_BOOST: u32 = 3 * drive::HZ as u32;
/// How often a tick of the meter costs a unit of money, in ticks.
///
/// Once a second.  The fare pays a fixed amount for the distance and the
/// clock pays nothing at all, so without a running cost the fastest route
/// and the slowest are worth the same and there is no reason to hurry beyond
/// the clock.  With one, every second of a job is a coin burned, and a trip
/// is profitable to the extent that it was quick and that you picked things
/// up on the way.
pub const FUEL_TICKS: u32 = drive::HZ as u32;
/// Seconds picking up a fare is worth.
pub const PICKUP_TIME: i32 = 12;
/// How close, and how slow, you have to be to pick up or drop off.
pub const STOP_RADIUS: Fx = fixed::ratio(3, 4);
/// Above this speed the passenger will not get in or out.
pub const STOP_SPEED: Fx = fixed::ratio(3, 2);
/// How near the marker the cab has to be to pick up or set down.
///
/// Wider than [`STOP_RADIUS`], because the passenger stands on the pavement
/// and the cab may not: a car pulled up at the kerb is a cell from the
/// person on it.  A quarter of a cell of slop on top of that and no more -
/// it was the whole of a stopping circle, which is a box three and a half
/// cells across, and on a two-cell street that is the entire road.  A fare
/// that pays out for being *near* the circle is a fare you never actually
/// arrive at.
///
/// The autopilot is not held to this: it stops in the circle at the kerb and
/// [`Fare::at_stop`] takes either.  This number is the one a player is
/// asked to hit.
pub const REACH: Fx = fixed::ratio(5, 4);

/// How far from the kerb the cab may stop and still be paid.
///
/// Half a cell more than the circle the autopilot aims at, which is the slop
/// between stopping and having stopped.
pub const KERB_REACH: Fx = STOP_RADIUS + fixed::HALF;

/// How much clear road in front ends a reverse.
///
/// Two car lengths.  Enough to pull away into, and short enough that a car
/// stops reversing the moment it can.
const CLEAR: Fx = fixed::ratio(4, 1);

/// The longest a driver spends reversing after being hit, in ticks.
///
/// A quarter of a shift.  Fifteen seconds is a long time to be backing up -
/// long enough to get clear of whatever it was, and long enough that on a
/// street the cab is still on, the car is recycled somewhere ahead before it
/// finishes.  That is the intent: a shunt clears itself off the road rather
/// than becoming a permanent obstacle in the middle of it.
pub const BACKING: i32 = START_TIME * drive::HZ / 4;

/// How far ahead a driver in the traffic looks for a reason to lift off.
///
/// Six cells is about thirty-five metres, which at the speeds the traffic
/// keeps is a couple of seconds - a following distance rather than a
/// braking distance, which is the point: the car should be easing off long
/// before it needs the brake.
const LOOK: Fx = fixed::ratio(6, 1);
/// Half the width of the corridor ahead that a driver treats as its own.
///
/// Just under a cell, so a car in the next lane is not something to brake
/// for and a car in this one is.
const CORRIDOR: Fx = fixed::ratio(9, 10);
/// How far out a driver looks for something crossing its path from the
/// right, which is the one that has priority.
const JUNCTION: Fx = fixed::ratio(9, 2);
/// Slower than this and a car is stopped rather than creeping.
const ROLLING: Fx = fixed::ratio(1, 4);
/// How much either side of the speed it wants a driver will tolerate before
/// touching a pedal.  Without a dead band the throttle chatters between full
/// and full brake every tick and the traffic bucks down the street.
const SLACK: Fx = fixed::ratio(1, 5);
/// The throttle a driver in the traffic uses.  Not full: this is a car going
/// somewhere, not a car being raced.
const CRUISE_THROTTLE: Fx = fixed::ratio(2, 5);
/// How much of the city the coin trail's route search may look at.  The same
/// figure the autopilot plans with, and for the same reason: a fare across
/// the map is a long route and a search that gives up produces no trail.
const COIN_BUDGET: usize = 40_000;
/// The bearing error, in angle units, at which a traffic driver has the
/// wheel on full lock.  A tenth of a turn: these cars are correcting a lane,
/// not taking a hairpin.
const LANE_LOCK: i32 = 6_500;
/// How much lock the cross-track term may ask for on its own.  Bounded, so
/// that a car knocked a long way off line rejoins its lane at an angle
/// rather than turning across the road to get back to it.
const LANE_CROSS: Fx = fixed::ratio(1, 2);

/// Damping on the traffic's steering: lock given back per unit of the car's
/// own rate of turn.
///
/// The same term, and for the same reason, as the autopilot's - see
/// `cabbie::DAMP`.  Half rather than two fifths because traffic has no
/// route to commit to and nothing it needs to throw the car into: for a car
/// whose only job is to sit in its lane, more damping is strictly better.
const LANE_DAMP: Fx = fixed::ratio(1, 2);

/// How near the cab a recycled vehicle may be put down, in cells.
///
/// Three blocks.  It was eight cells - a car length and a half - so the pool
/// spent its life materialising a saloon in the next lane while you were
/// looking at it, which is the one thing a traffic system must never do.  At
/// three blocks a car appears at or beyond the far end of the street you are
/// on, where the frame is already a haze of distant buildings, and drives
/// towards you the way traffic does.
pub const SPAWN_NEAR: i32 = 3 * crate::zone::BLOCK_PITCH as i32;

/// A piece of street furniture.
#[derive(Clone, Copy, Debug)]
pub struct Prop {
    /// How it is drawn and where it stands.
    pub board: Billboard,
    /// Velocity, non-zero only while it is tumbling.
    pub vx: Fx,
    /// Velocity.
    pub vy: Fx,
    /// Whether it is still standing.
    pub standing: bool,
}

/// Somebody on the pavement.
#[derive(Clone, Copy, Debug)]
pub struct Ped {
    /// Where they are.
    pub x: Fx,
    /// Where they are.
    pub y: Fx,
    /// Which way they are walking.
    pub dir: Ang,
    /// Where they are going, in cells.  Reached, they pick somewhere else.
    pub goal: (i32, i32),
    /// Stride phase.
    pub phase: u8,
    /// Hue.
    pub hue: u8,
}

/// One of the coins strung along the route to the drop-off.
#[derive(Clone, Copy, Debug)]
pub struct Coin {
    /// Where it hangs.
    pub x: Fx,
    /// Where it hangs.
    pub y: Fx,
    /// Whether it has been taken.
    pub taken: bool,
}

/// The current job.
///
/// Four places, not two.  The passenger waits on the pavement and is set
/// down on one - that is where the marker and the circle go - and a car
/// cannot be either of those places, so each end also carries the kerb the
/// cab actually pulls up at.  Keeping both means the picture and the driving
/// can each be right about a different thing: the circle is where somebody
/// is standing, the stop is where a car fits.
#[derive(Clone, Debug)]
pub struct Fare {
    /// Where the passenger is waiting, on foot.
    pub from: (Fx, Fx),
    /// Where they are going, on foot.
    pub to: (Fx, Fx),
    /// The kerb beside `from`.
    pub from_stop: (Fx, Fx),
    /// The kerb beside `to`.
    pub to_stop: (Fx, Fx),
    /// Whether they are in the car.
    pub aboard: bool,
    /// The coins between here and there.
    pub coins: Vec<Coin>,
    /// What the fare is worth, in whole units of money.
    pub value: u32,
}

impl Fare {
    /// Where the passenger is - the end of the job that is drawn.
    pub fn marker(&self) -> (Fx, Fx) {
        if self.aboard { self.to } else { self.from }
    }

    /// Where a car stops to reach them.
    pub fn stop(&self) -> (Fx, Fx) {
        if self.aboard { self.to_stop } else { self.from_stop }
    }

    /// Whether a car is pulled up close enough to hand the passenger over.
    ///
    /// Either near the person or in the stopping circle at the kerb: the
    /// first is what a player aims at, the second is what the autopilot
    /// drives to, and a rule that only accepted one of them would fail
    /// whichever of the two is at the wheel.
    pub fn at_stop(&self, x: Fx, y: Fx) -> bool {
        let (mx, my) = self.marker();
        let (sx, sy) = self.stop();
        // The kerb clause is a little wider than the circle the autopilot
        // aims at.  It stops when it is inside that circle and then coasts,
        // so it comes to rest just outside it as often as not - and a cab
        // that has arrived, stopped, and is not paid sits there until the
        // stuck check reverses it into the road.  Measured: a third of one
        // city's travelling ticks were spent off the carriageway doing
        // exactly that.
        (fixed::abs(mx - x) < REACH && fixed::abs(my - y) < REACH)
            || (fixed::abs(sx - x) < KERB_REACH && fixed::abs(sy - y) < KERB_REACH)
    }
}

/// Something that just happened and that the front end may want to say out
/// loud.  Returned from [`Sim::step`] rather than printed, because the core
/// does not own a screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A piece of street furniture went over.
    Flattened,
    /// Another car was hit.
    Rammed,
    /// A wall was hit.
    Crunched,
    /// A coin was collected.
    Coin,
    /// The passenger got in.
    PickedUp,
    /// The passenger got out.
    DroppedOff,
    /// The clock ran out.
    TimeUp,
}

/// The whole moving city.
pub struct Sim {
    /// The car you are driving.
    pub taxi: Car,
    /// Everyone else's cars.
    pub traffic: Vec<Car>,
    /// Steering for each of them.
    traffic_ctl: Vec<Controls>,
    /// How fast each of them would like to be going with a clear road.
    /// Varied per car, so a street is not a convoy at one speed.
    traffic_cruise: Vec<Fx>,
    /// Where in its own half of the road each car likes to sit, -1 against
    /// the crown to +1 against the kerb.  See [`road::lane_biased`].
    traffic_bias: Vec<Fx>,
    /// Ticks each of them has left to spend reversing out of a shunt.
    traffic_backing: Vec<u32>,
    /// Whether the cab was on the brakes last tick, for its brake lights.
    braking: bool,
    /// The street furniture.
    pub props: Vec<Prop>,
    /// The pedestrians.
    pub peds: Vec<Ped>,
    /// The current job, if any.
    pub fare: Option<Fare>,
    /// Money taken this shift, after petrol.
    pub money: i32,
    /// What the petrol has cost so far, for the scoreboard.
    pub spent: u32,
    /// Consecutive things hit without stopping.
    pub combo: u32,
    /// Ticks left on the clock, at [`drive::HZ`].
    pub ticks_left: i32,
    /// Frames since the shift began.
    pub tick: u32,
    /// Whether the shift is over.
    ///
    /// Never set by the clock any more, and kept because the field is what a
    /// front end asks.  Running out of time used to freeze the whole
    /// simulation on the tick it happened: the cab stopped mid-corner, the
    /// traffic stopped around it, and the only thing left to do was quit.
    /// That is a scoreboard, not an ending.
    ///
    /// The clock now runs past zero into the negative and the fare stays on
    /// the meter, so the shift you have overrun is a shift you are working
    /// at a loss - which is a state you can drive your way out of, because
    /// fares still pay.  See [`Sim::seconds_left`].
    pub over: bool,
    rng: Rng,
    /// Scratch for the billboard sort, so a frame does not allocate.
    order: Vec<(Fx, usize)>,
    /// Scratch for the billboards handed to the sprite pass.
    pub(crate) boards: Vec<Billboard>,
}

impl Sim {
    /// Start a shift in a generated city.
    pub fn new(city: &City, seed: u32) -> Sim {
        let start = Camera::spawn(city, SIZE as i32 / 2, SIZE as i32 / 2);
        let mut sim = Sim {
            taxi: Car::new(CarKind::Taxi, start.x, start.y, 0, palette::H_YELLOW),
            traffic: Vec::with_capacity(TRAFFIC),
            traffic_ctl: vec![Controls::default(); TRAFFIC],
            traffic_cruise: vec![0; TRAFFIC],
            traffic_bias: vec![0; TRAFFIC],
            traffic_backing: vec![0; TRAFFIC],
            braking: false,
            props: Vec::new(),
            peds: Vec::with_capacity(PEDS),
            fare: None,
            money: 0,
            spent: 0,
            combo: 0,
            ticks_left: START_TIME * drive::HZ,
            tick: 0,
            over: false,
            rng: Rng::new(seed.wrapping_add(0x0000_5EED)),
            order: Vec::new(),
            boards: Vec::new(),
        };
        sim.furnish(city);
        // On a road before anything else happens.  The camera spawns on the
        // pavement now - that is where a person stands - and everything
        // below is placed relative to the cab, so a cab left on the paving
        // would seed the traffic and the fare off the road as well.
        sim.park_near(city, fixed::floor(start.x), fixed::floor(start.y));
        for i in 0..TRAFFIC {
            let (c, cruise, bias) = sim.spawn_car(city);
            sim.traffic.push(c);
            sim.traffic_cruise[i] = cruise;
            sim.traffic_bias[i] = bias;
        }
        for _ in 0..PEDS {
            let p = sim.spawn_ped(city);
            sim.peds.push(p);
        }
        sim.hail(city);
        sim
    }

    /// Put furniture on every sidewalk that wants some.
    ///
    /// Deterministic from the cell coordinates rather than from the running
    /// generator, so the same city always has the same lamp posts however
    /// many times the shift is restarted.
    fn furnish(&mut self, city: &City) {
        for y in 0..SIZE as i32 {
            for x in 0..SIZE as i32 {
                if city.at(x, y).kind != Kind::Sidewalk {
                    continue;
                }
                let h = hash3(x as u32, y as u32, 0x000F_0111);
                let stamp = match h % 24 {
                    0 | 1 => Stamp::LampPost,
                    2 => Stamp::Hydrant,
                    3 => Stamp::Mailbox,
                    4 | 5 => Stamp::Tree,
                    6 => Stamp::Meter,
                    7 => Stamp::Bollard,
                    8 if on_corner(city, x, y) => Stamp::Signal,
                    _ => continue,
                };
                let (w, ht, hue) = match stamp {
                    // Three times what it was.  A street light at two and a
                    // quarter cells is thirteen metres, which is correct for
                    // a residential street and invisible beside a
                    // fifty-cell tower; the arterials here are ninety-six
                    // metres wide and carry masts to match.
                    Stamp::LampPost => (fixed::ratio(2, 5), fixed::ratio(27, 4), palette::H_WHITE),
                    // Narrower than it was, so that a crown centred in the
                    // verge clears the kerb.  It may overhang the paving -
                    // that is what a street tree does - but not the road.
                    Stamp::Tree => (fixed::ratio(13, 20), fixed::ratio(2, 1), palette::H_GREEN),
                    Stamp::Signal => (fixed::ratio(1, 3), fixed::ratio(2, 1), palette::H_WHITE),
                    Stamp::Mailbox => (fixed::ratio(2, 5), fixed::ratio(3, 5), palette::H_BLUE),
                    Stamp::Hydrant => (fixed::ratio(1, 4), fixed::ratio(2, 5), palette::H_RED),
                    Stamp::Meter => (fixed::ratio(1, 5), fixed::ratio(4, 5), palette::H_WHITE),
                    _ => (fixed::ratio(1, 5), fixed::ratio(2, 5), palette::H_YELLOW),
                };
                // Where across the pavement this stands.  Street lighting
                // goes to the kerb, planting into the verge behind it, and
                // everything else onto the paving.  Measured from the kerb
                // in cells, matching the bands the renderer draws.
                let across = if stamp.kerbside() {
                    fixed::ratio(1, 6)
                } else if stamp.planted() {
                    fixed::ratio(2, 5)
                } else {
                    fixed::ratio(7, 10)
                };
                let (ox, oy, along_x) = kerb_offset(city, x, y, across);

                // Off-centre by a stable amount *along* the kerb, so a row
                // of lamp posts is not a row of identically placed lamp
                // posts.  Only along it: across it, the band is the point.
                //
                // The play is bounded by the half-width and a margin,
                // because a prop whose edge lands exactly on a cell boundary
                // is read as being in the next cell.
                let play = (fixed::HALF - w / 2 - fixed::ratio(1, 16)).max(0);
                let wobble = fixed::mul(fixed::ratio(((h >> 8) % 5) as i32 - 2, 2), play);
                let (jx, jy) = if along_x {
                    (wobble + ox, oy)
                } else {
                    (ox, wobble + oy)
                };
                self.props.push(Prop {
                    board: Billboard::upright(
                        stamp,
                        fixed::from_int(x) + fixed::HALF + jx,
                        fixed::from_int(y) + fixed::HALF + jy,
                        w,
                        ht,
                        hue,
                    ),
                    vx: 0,
                    vy: 0,
                    standing: true,
                });
            }
        }
    }

    /// A road cell somewhere near the taxi but not on top of it.
    fn road_near(&mut self, city: &City, min: i32, max: i32) -> Option<(i32, i32)> {
        let (tx, ty) = (fixed::floor(self.taxi.x), fixed::floor(self.taxi.y));
        for _ in 0..200 {
            let r = self.rng.range(min, max);
            let a = self.rng.below(65536) as Ang;
            let x = tx + fixed::floor(fixed::mul(trig::cos(a), fixed::from_int(r)));
            let y = ty + fixed::floor(fixed::mul(trig::sin(a), fixed::from_int(r)));
            if x < 1 || y < 1 || x >= SIZE as i32 - 1 || y >= SIZE as i32 - 1 {
                continue;
            }
            if city.at(x, y).kind == Kind::Road {
                return Some((x, y));
            }
        }
        None
    }

    /// A carriageway cell near the taxi that has a side of the road to be
    /// on, and the direction the traffic on that side is going.
    ///
    /// Junctions, alleys and the middle cell of an odd-width avenue all have
    /// no answer - see [`road::flow`] - and a car put down on one of them
    /// has to guess which way to face, which is what a street full of cars
    /// driving at each other is made of.
    fn lane_near(&mut self, city: &City, min: i32, max: i32) -> Option<((i32, i32), (i32, i32))> {
        for _ in 0..24 {
            let (x, y) = self.road_near(city, min, max)?;
            if city.plan.is_junction(x, y) || !city.drivable(x, y) {
                continue;
            }
            if let Some(dir) = road::flow(city, x, y) {
                return Some(((x, y), dir));
            }
        }
        None
    }

    /// A place on foot near the taxi where somebody could be standing, and
    /// the kerb a cab pulls up at to reach them.
    ///
    /// The pedestrian network, so it is pavement, plaza or park and never
    /// the carriageway - a passenger waits *beside* the road, and a marker
    /// painted in the middle of an avenue asks the player to park in the
    /// traffic.  Crossings are on the walking network too and are excluded
    /// on purpose: they are road.
    ///
    /// The kerb comes back with it because a car cannot go where the marker
    /// is.  Two cells of reach, which covers a passenger a step back from
    /// the kerb without allowing one in the middle of a park a block away
    /// from any road.
    fn kerb_near(
        &mut self,
        city: &City,
        min: i32,
        max: i32,
    ) -> Option<((i32, i32), (i32, i32))> {
        let (tx, ty) = (fixed::floor(self.taxi.x), fixed::floor(self.taxi.y));
        for _ in 0..400 {
            let r = self.rng.range(min, max);
            let a = self.rng.below(65536) as Ang;
            let x = tx + fixed::floor(fixed::mul(trig::cos(a), fixed::from_int(r)));
            let y = ty + fixed::floor(fixed::mul(trig::sin(a), fixed::from_int(r)));
            if x < 1 || y < 1 || x >= SIZE as i32 - 1 || y >= SIZE as i32 - 1 {
                continue;
            }
            // Snap to the nearest place a person could stand rather than
            // insisting the dart landed on one: a random point in a city is
            // usually inside a building, and 400 darts that each have to hit
            // a pavement outright do miss - measured, often enough that the
            // fallback below was placing dropoffs in the middle of avenues.
            let Some((px, py)) = city.walk.nearest(x, y, 6) else { continue };
            // The snap can pull the spot back towards the cab - up to six
            // cells of it - so the range has to be checked *after* it and
            // not before.  Two fares in sixty landed inside the minimum
            // that way, which is the one place a player would notice: the
            // circle for the next job on the pavement they are stopped at.
            if (px - tx).abs() + (py - ty).abs() < min {
                continue;
            }
            // On the walking network *and* off the carriageway.  The two are
            // not the same test: an alley is road with no pavement beside
            // it, so people walk down the middle of one and it is marked
            // walkable - which makes it exactly the wrong place to stand
            // waiting for a cab.
            if city.walk.at(px, py) != Foot::Path || city.at(px, py).kind == Kind::Road {
                continue;
            }
            if let Some(stop) = kerb_beside(city, px, py) {
                return Some(((px, py), stop));
            }
        }
        None
    }

    /// Put a car down in the traffic, facing the way that side of the road
    /// goes.
    ///
    /// The direction comes from the lane rather than from a coin.  A car
    /// used to be dropped on a road cell and pointed along it either way,
    /// so half the traffic on any given side of the paint was oncoming, two
    /// cars a lane apart drove head-on at each other, and there was no
    /// stream for anyone - the player included - to join.  Reading the side
    /// of the crown first and taking the heading from that is the whole
    /// difference between a road and a car park with lines on it.
    fn spawn_car(&mut self, city: &City) -> (Car, Fx, Fx) {
        const HUES: [u8; 6] = [
            palette::H_WHITE,
            palette::H_RED,
            palette::H_BLUE,
            palette::H_GREEN,
            palette::H_ORANGE,
            palette::H_PURPLE,
        ];
        let hue = HUES[self.rng.below(6) as usize];
        let kind = if self.rng.chance(1, 8) { CarKind::Bus } else { CarKind::Traffic };
        // A cruising speed per car, so the traffic has to overtake and give
        // way to itself rather than moving as one block.
        let cruise = fixed::ratio(self.rng.range(18, 32), 10);
        // And a place in its own half of the road, for the same reason.  A
        // bus takes the middle of its half because it does not fit anywhere
        // else; everything smaller picks a line and keeps it.
        let bias = if kind == CarKind::Bus {
            0
        } else {
            fixed::ratio(self.rng.range(-8, 9), 10)
        };
        // If no lane turned up, put the car on top of the taxi rather than
        // at the origin: it will be recycled on the next tick, whereas a
        // fallback in the corner of the map is a car that is instantly and
        // permanently a straggler.
        let Some(((x, y), dir)) = self.lane_near(city, SPAWN_NEAR, RECYCLE) else {
            let c = Car::new(kind, self.taxi.x, self.taxi.y, self.taxi.yaw, hue);
            return (c, cruise, bias);
        };
        let yaw = road::heading(dir.0, dir.1);
        // On the lane line, not merely on the cell: a car that starts
        // straddling the paint spends its first second steering back onto
        // its own side, in front of whoever is behind it.
        let (lx, ly) = road::lane_biased(city, x, y, dir, bias);
        let mut c = Car::new(kind, lx, ly, yaw, hue);
        c.vx = fixed::mul(trig::cos(yaw), cruise);
        c.vy = fixed::mul(trig::sin(yaw), cruise);
        (c, cruise, bias)
    }

    /// Somewhere a person could plausibly be walking to, near the taxi.
    fn walk_goal(&mut self, city: &City, from: (i32, i32)) -> (i32, i32) {
        for _ in 0..40 {
            let x = from.0 + self.rng.range(-18, 18);
            let y = from.1 + self.rng.range(-18, 18);
            if city.walk.passable(x, y) {
                return (x, y);
            }
        }
        from
    }

    fn spawn_ped(&mut self, city: &City) -> Ped {
        let (tx, ty) = (fixed::floor(self.taxi.x), fixed::floor(self.taxi.y));
        for _ in 0..200 {
            let x = tx + self.rng.range(-RECYCLE, RECYCLE);
            let y = ty + self.rng.range(-RECYCLE, RECYCLE);
            // Placed on the *pedestrian* network, not merely on ground that
            // is not built on.  The two are different maps and conflating
            // them is what used to put people in the middle of the avenue.
            if city.walk.at(x, y) != Foot::Path {
                continue;
            }
            let goal = self.walk_goal(city, (x, y));
            return Ped {
                x: fixed::from_int(x) + fixed::HALF,
                y: fixed::from_int(y) + fixed::HALF,
                dir: (self.rng.below(4) as Ang).wrapping_mul(trig::QUARTER),
                goal,
                phase: self.rng.below(2) as u8,
                hue: [palette::H_PINK, palette::H_CYAN, palette::H_WHITE, palette::H_ORANGE]
                    [self.rng.below(4) as usize],
            };
        }
        let here = city.walk.nearest(tx, ty, 30).unwrap_or((tx, ty));
        Ped {
            x: fixed::from_int(here.0) + fixed::HALF,
            y: fixed::from_int(here.1) + fixed::HALF,
            dir: 0,
            goal: here,
            phase: 0,
            hue: palette::H_WHITE,
        }
    }

    /// Put the taxi on the nearest road to a point, facing along it.
    ///
    /// So that a shift starts with the cab where you are rather than
    /// wherever the middle of the map happened to be - you should be able to
    /// see it from where you are standing and walk over to it.
    pub fn park_near(&mut self, city: &City, x: i32, y: i32) {
        let mut best: Option<(i32, i32, i32)> = None;
        for r in 0..24i32 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dy.abs()) != r {
                        continue;
                    }
                    let (px, py) = (x + dx, y + dy);
                    if city.at(px, py).kind != Kind::Road || !city.open(px, py) {
                        continue;
                    }
                    // Not in a junction and not on an alley: a cab waits at a
                    // kerb on an ordinary street.
                    if city.plan.is_junction(px, py) {
                        continue;
                    }
                    if !city.plan.cols.at(px).class.is_street()
                        && !city.plan.rows.at(py).class.is_street()
                    {
                        continue;
                    }
                    best = Some((px, py, r));
                    break;
                }
                if best.is_some() {
                    break;
                }
            }
            if best.is_some() {
                break;
            }
        }
        let Some((px, py, _)) = best else { return };
        self.taxi.vx = 0;
        self.taxi.vy = 0;
        self.taxi.spin = 0;
        // Facing the way this side of the road goes, and sitting in its
        // lane.  A cab parked against the flow is a cab whose first move is
        // a U-turn, and the shift now starts with the autopilot driving:
        // whatever the car is pointing at is the first thing anybody sees.
        match road::flow(city, px, py) {
            Some(dir) => {
                let (lx, ly) = road::lane(city, px, py, dir);
                self.taxi.x = lx;
                self.taxi.y = ly;
                self.taxi.yaw = road::heading(dir.0, dir.1);
            }
            None => {
                self.taxi.x = fixed::from_int(px) + fixed::HALF;
                self.taxi.y = fixed::from_int(py) + fixed::HALF;
                self.taxi.yaw = if city.plan.cols.at(px).class.is_street() {
                    trig::QUARTER
                } else {
                    0
                };
            }
        }
    }

    /// Find a new fare and string coins along the way to it.
    ///
    /// Both ends are on the pavement - or a plaza, or a park - and never on
    /// the carriageway.  A marker in the middle of the road asks the player
    /// to park in the traffic to earn it, and asks the autopilot to stop
    /// dead on an avenue, which is where most of a shift used to be spent
    /// being rear-ended.  Somebody hailing a cab stands at the kerb.
    ///
    /// The kerb beside each end comes back with it, because that is where a
    /// car can actually be; the coins are strung between those two, since
    /// they are collected by driving over them.
    pub fn hail(&mut self, city: &City) {
        let (from, from_stop) = match self.kerb_near(city, HAIL_NEAR, HAIL_FAR) {
            Some(v) => v,
            // Nowhere to stand within reach: rather than leave the shift
            // with no job at all, take any road cell.  The next tick tries
            // again from wherever the cab has got to.
            None => match self.road_near(city, 4, 14) {
                Some(c) => (c, c),
                None => return,
            },
        };
        // The drop-off, at least a block from the pickup as well as from the
        // cab.  Several tries, because the constraint is on a pair and the
        // dart only knows about one of them; the last candidate is kept if
        // none of them is far enough, since a short fare is better than no
        // fare.
        let mut far = None;
        for _ in 0..8 {
            let Some(v) = self.kerb_near(city, HAIL_NEAR * 2, RECYCLE) else { break };
            let d = (v.0 .0 - from.0).abs() + (v.0 .1 - from.1).abs();
            far = Some(v);
            if d >= FARE_MIN {
                break;
            }
        }
        let (to, to_stop) = match far {
            Some(v) => v,
            None => match self.road_near(city, HAIL_NEAR * 2, RECYCLE) {
                Some(c) => (c, c),
                None => return,
            },
        };
        let coins = coin_trail(city, from_stop, to_stop);
        let dist = (from.0 - to.0).abs() + (from.1 - to.1).abs();
        self.fare = Some(Fare {
            from: road::centre(from.0, from.1),
            to: road::centre(to.0, to.1),
            from_stop: road::centre(from_stop.0, from_stop.1),
            to_stop: road::centre(to_stop.0, to_stop.1),
            aboard: false,
            coins,
            value: (dist as u32) * 3 + 10,
        });
    }

    /// Advance the whole simulation one tick, and report what happened.
    pub fn step(&mut self, city: &City, c: &Controls, hz: i32, out: &mut Vec<Event>) {
        out.clear();
        if self.over {
            return;
        }
        self.tick = self.tick.wrapping_add(1);

        let before = self.taxi.damage;
        // On the brakes, rather than merely not on the throttle: the lamps
        // come on when the driver asks for them.
        self.braking = c.throttle < 0;
        self.taxi.step(c, city, hz);
        if self.taxi.damage > before.saturating_add(3) {
            out.push(Event::Crunched);
            self.combo = 0;
        }

        self.step_traffic(city, hz, out);
        self.step_props(hz, out);
        self.step_peds(city, hz);
        self.step_fare(city, out);

        // The meter runs on petrol as well as on time, and it does not stop
        // at nothing: a shift can be in the red, which is what makes the
        // clock running out a thing that costs you rather than a thing that
        // ends you.
        if self.tick.is_multiple_of(FUEL_TICKS) {
            self.money -= 1;
            self.spent = self.spent.saturating_add(1);
        }

        // Past zero, not stopped at it.  The event fires once, on the tick
        // the clock crosses, and then the shift carries on into overtime.
        let was = self.ticks_left;
        self.ticks_left -= 1;
        if was > 0 && self.ticks_left <= 0 {
            out.push(Event::TimeUp);
        }
    }

    /// Drive the traffic, and let it bump into itself.
    ///
    /// Each car is given the same three controls the player gets, worked out
    /// from what is in front of it, and then everything is collided against
    /// everything else.  It is not a route - the traffic still goes
    /// wherever the street it is on goes - but it keeps its lane and it
    /// lifts off for what is ahead, which is the difference between traffic
    /// and a hazard that happens to be car-shaped.
    fn step_traffic(&mut self, city: &City, hz: i32, out: &mut Vec<Event>) {
        for i in 0..self.traffic.len() {
            // Recycle anything that has fallen too far behind.
            let far = fixed::abs(self.traffic[i].x - self.taxi.x)
                + fixed::abs(self.traffic[i].y - self.taxi.y);
            if far > fixed::from_int(RECYCLE + 12) {
                let (c, cruise, bias) = self.spawn_car(city);
                self.traffic[i] = c;
                self.traffic_cruise[i] = cruise;
                self.traffic_bias[i] = bias;
                self.traffic_backing[i] = 0;
                continue;
            }
            let ctl = self.traffic_controls(city, i, hz);
            self.traffic_ctl[i] = ctl;
            self.traffic[i].step(&ctl, city, hz);

            // Against the taxi.
            let (mut a, mut b) = (self.taxi, self.traffic[i]);
            if let Some(sev) = drive::collide(&mut a, &mut b, city) {
                self.taxi = a;
                self.traffic[i] = b;
                self.traffic_backing[i] = BACKING as u32;
                if sev > ONE {
                    self.combo += 1;
                    self.money += 2 * self.combo as i32;
                    out.push(Event::Rammed);
                }
            }
        }
        // And against each other.  Traffic used to pass clean through
        // itself, which is invisible until you are following a queue and two
        // of them occupy the same six metres of road; giving way is also
        // only half a rule if failing to give way costs nothing.  No event:
        // the scoreboard is for what *you* hit.
        for i in 0..self.traffic.len() {
            for j in i + 1..self.traffic.len() {
                let (mut a, mut b) = (self.traffic[i], self.traffic[j]);
                if drive::collide(&mut a, &mut b, city).is_some() {
                    self.traffic[i] = a;
                    self.traffic[j] = b;
                    self.traffic_backing[i] = BACKING as u32;
                    self.traffic_backing[j] = BACKING as u32;
                }
            }
        }
    }

    /// What the driver of traffic car `i` does this tick.
    ///
    /// Two jobs, and they are independent: keep the lane, and do not run
    /// into anything.  Neither of them is a route - this car has no idea
    /// where it is going and does not need one.
    fn traffic_controls(&mut self, city: &City, i: usize, hz: i32) -> Controls {
        // Backing out of a shunt.  A driver who has just been hit does not
        // carry on as though nothing happened: they reverse, with the wheel
        // over, until they are clear - and on a street the player is still
        // on, they are recycled somewhere ahead before they finish, which is
        // how a wreck stops being a permanent obstacle.
        if self.traffic_backing[i] > 0 {
            self.traffic_backing[i] -= 1;
            // ...but only until it is clear of whatever it hit.  The quarter
            // of a shift is a *limit*, not a duration: a car that reverses
            // for fifteen seconds after every touch spends its life going
            // backwards, and a street of them is a farce rather than a
            // shunt.  Two car lengths of clear road in front and the driver
            // gets on with it.
            if !self.crowded(i, CLEAR) {
                self.traffic_backing[i] = 0;
            } else {
                // The lock alternates by index, so a pile-up does not
                // reverse in formation.  It is applied through a wheel that
                // works backwards in reverse - see `Car::step` - so this is
                // the direction the *tail* goes, which is the end that has
                // to find the gap.
                let lock = if i.is_multiple_of(2) { ONE } else { -ONE };
                return Controls {
                    throttle: -ONE,
                    steer: fixed::mul(lock, fixed::HALF),
                    handbrake: false,
                };
            }
        }
        let c = self.traffic[i];
        let (fx, fy) = (trig::cos(c.yaw), trig::sin(c.yaw));
        // The car's own right-hand side, which is the axis every offset
        // below is measured on.
        let (rx, ry) = (-fy, fx);
        let vf = fixed::mul(c.vx, fx) + fixed::mul(c.vy, fy);

        let dir = intent(city, &c);
        let (cx, cy) = (fixed::floor(c.x), fixed::floor(c.y));
        // The line to hold: the middle of the lane it is already in, unless
        // that lane belongs to the traffic going the other way, in which
        // case the first lane on the correct side of the crown.  Holding
        // `road::lane` unconditionally would file every car on a
        // fourteen-cell arterial into one lane and leave the rest of it
        // empty.
        let (lx, ly) = match road::flow(city, cx, cy) {
            Some(f) if f == dir => road::centre(cx, cy),
            _ => road::lane_biased(city, cx, cy, dir, self.traffic_bias[i]),
        };
        // How far to the right of that line the car is, and how far its nose
        // is off the way the street runs.
        let off = fixed::mul(c.x - lx, rx) + fixed::mul(c.y - ly, ry);
        let psi = road::heading(dir.0, dir.1).wrapping_sub(c.yaw) as i16 as i32;
        // The same steering law the autopilot uses: an angle term to point
        // it down the street and a cross-track term to walk it onto the
        // line, the second divided by speed so that a correction that suits
        // a crawl does not snake the car at speed.
        let angle = fixed::ratio(psi, LANE_LOCK);
        let pull = fixed::clamp(fixed::div(off, c.speed() + ONE), -LANE_CROSS, LANE_CROSS);
        // The derivative, the same one the autopilot has: lock back in
        // proportion to how fast the car is already coming round.  Without
        // it a car nudged off its line answers with full lock, arrives
        // pointing across the road, and answers that - which is a car
        // weaving down a straight street, and twelve of them weaving is a
        // street nobody can get down.
        let damp = fixed::mul(LANE_DAMP, c.turn_rate(hz));
        let steer = fixed::clamp(angle - pull - damp, -ONE, ONE);

        Controls { throttle: pace(vf, self.give_way(i)), steer, handbrake: false }
    }

    /// Whether anything is within `reach` of the front of car `i`.
    ///
    /// Used to decide that a car which has been shunted is clear again.  The
    /// taxi counts: backing away from the thing that hit you is the point.
    fn crowded(&self, i: usize, reach: Fx) -> bool {
        let c = self.traffic[i];
        let (fx, fy) = (trig::cos(c.yaw), trig::sin(c.yaw));
        let (rx, ry) = (-fy, fx);
        let others = self
            .traffic
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, o)| o)
            .chain(std::iter::once(&self.taxi));
        for o in others {
            let (dx, dy) = (o.x - c.x, o.y - c.y);
            let lon = fixed::mul(dx, fx) + fixed::mul(dy, fy);
            let lat = fixed::mul(dx, rx) + fixed::mul(dy, ry);
            if lon > -c.half_len() && lon < reach && fixed::abs(lat) < CORRIDOR {
                return true;
            }
        }
        false
    }

    /// The fastest car `i` should be going, given what is in front of it.
    ///
    /// Two rules, which is as much traffic law as a car with no route can
    /// use.  Do not close on the vehicle ahead in your own lane: the speed
    /// falls off with the gap, so a queue settles rather than concertinas.
    /// And give way to anything crossing from your right, which is the
    /// junction rule everywhere that drives on the right, and is enough to
    /// keep two cars arriving at the same crossroads from arriving in the
    /// same place.
    fn give_way(&self, i: usize) -> Fx {
        let c = self.traffic[i];
        let (fx, fy) = (trig::cos(c.yaw), trig::sin(c.yaw));
        let (rx, ry) = (-fy, fx);
        let mut want = self.traffic_cruise[i];

        let others = self
            .traffic
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, o)| o)
            .chain(std::iter::once(&self.taxi));
        for o in others {
            let (dx, dy) = (o.x - c.x, o.y - c.y);
            // Where it is in this car's own frame: how far up the road, and
            // how far to the side.
            let lon = fixed::mul(dx, fx) + fixed::mul(dy, fy);
            if lon <= 0 {
                continue;
            }
            let lat = fixed::mul(dx, rx) + fixed::mul(dy, ry);
            // Bumper to bumper, so the gap is between the cars rather than
            // between their middles - a bus is four cells long and closing
            // to two of them is already a collision.
            let gap = lon - c.half_len() - o.half_len();
            if fixed::abs(lat) < CORRIDOR && lon < LOOK {
                want = want.min(follow(gap));
            } else if lat > 0
                && lat < JUNCTION
                && lon < JUNCTION
                && crossing(c.yaw, o.yaw)
                && o.speed() > ROLLING
            {
                // Somebody coming from the right, across the nose: wait.
                want = 0;
            }
        }
        want
    }

    fn step_props(&mut self, hz: i32, out: &mut Vec<Event>) {
        let inv = fixed::div(ONE, fixed::from_int(hz.max(1)));
        let speed = self.taxi.speed();
        for p in self.props.iter_mut() {
            if p.standing {
                let dx = p.board.x - self.taxi.x;
                let dy = p.board.y - self.taxi.y;
                let reach = self.taxi.half_len() + p.board.w;
                if fixed::abs(dx) < reach && fixed::abs(dy) < reach && speed > ONE {
                    // Over it goes, in the direction the car was travelling,
                    // and the car does not slow down at all - which is the
                    // whole appeal.
                    p.standing = false;
                    p.vx = fixed::mul(self.taxi.vx, fixed::ratio(3, 5));
                    p.vy = fixed::mul(self.taxi.vy, fixed::ratio(3, 5));
                    self.combo += 1;
                    self.money += self.combo as i32;
                    out.push(Event::Flattened);
                }
            } else if p.board.lean < 8 {
                p.board.x += fixed::mul(p.vx, inv);
                p.board.y += fixed::mul(p.vy, inv);
                p.vx = fixed::mul(p.vx, fixed::ratio(93, 100));
                p.vy = fixed::mul(p.vy, fixed::ratio(93, 100));
                if self.tick.is_multiple_of(4) {
                    p.board.lean += 1;
                }
            }
        }
    }

    /// Walk the pedestrians along the pedestrian network.
    ///
    /// Each one has somewhere to be and follows the cheap greedy stepper
    /// towards it, which keeps them on the pavements and sends them over the
    /// crossings.  When one arrives - or gets stuck, which the greedy
    /// stepper is explicitly allowed to do - it picks somewhere else.
    fn step_peds(&mut self, city: &City, hz: i32) {
        let inv = fixed::div(ONE, fixed::from_int(hz.max(1)));
        let pace = fixed::ratio(2, 5);
        let mut regoal: Vec<usize> = Vec::new();
        for (i, p) in self.peds.iter_mut().enumerate() {
            let at = (fixed::floor(p.x), fixed::floor(p.y));
            match city.walk.step_toward(at, p.goal) {
                Some(a) => p.dir = a,
                None => regoal.push(i),
            }
            let nx = p.x + fixed::mul(fixed::mul(trig::cos(p.dir), pace), inv);
            let ny = p.y + fixed::mul(fixed::mul(trig::sin(p.dir), pace), inv);
            // Never step off the network, whatever the heading says.
            if city.walk.passable(fixed::floor(nx), fixed::floor(p.y)) {
                p.x = nx;
            }
            if city.walk.passable(fixed::floor(p.x), fixed::floor(ny)) {
                p.y = ny;
            }
            if self.tick.wrapping_add(i as u32).is_multiple_of(24) {
                p.phase ^= 1;
            }
        }
        // Anybody who arrived or wedged gets a new errand.  Done after the
        // loop because picking one needs the generator, which the loop is
        // borrowing.
        for i in regoal {
            let at = (fixed::floor(self.peds[i].x), fixed::floor(self.peds[i].y));
            let goal = self.walk_goal(city, at);
            self.peds[i].goal = goal;
        }
        // And a slow trickle of new errands, so that a crowd does not settle.
        if self.tick.is_multiple_of(37) && !self.peds.is_empty() {
            let i = (self.tick as usize / 37) % self.peds.len();
            let at = (fixed::floor(self.peds[i].x), fixed::floor(self.peds[i].y));
            let goal = self.walk_goal(city, at);
            self.peds[i].goal = goal;
        }
    }

    fn step_fare(&mut self, city: &City, out: &mut Vec<Event>) {
        let Some(fare) = self.fare.as_mut() else {
            self.hail(city);
            return;
        };
        let speed = self.taxi.speed();

        for c in fare.coins.iter_mut() {
            if c.taken {
                continue;
            }
            if fixed::abs(c.x - self.taxi.x) < ONE && fixed::abs(c.y - self.taxi.y) < ONE {
                c.taken = true;
                self.ticks_left += COIN_TIME * drive::HZ;
                self.money += 1;
                self.taxi.boost = self.taxi.boost.max(COIN_BOOST);
                out.push(Event::Coin);
            }
        }

        if !fare.at_stop(self.taxi.x, self.taxi.y) || speed > STOP_SPEED {
            return;
        }
        if fare.aboard {
            self.money += fare.value as i32;
            self.ticks_left += PICKUP_TIME * drive::HZ;
            out.push(Event::DroppedOff);
            self.fare = None;
            self.hail(city);
        } else {
            fare.aboard = true;
            self.ticks_left += PICKUP_TIME * drive::HZ;
            out.push(Event::PickedUp);
        }
    }

    /// Seconds left on the clock, which goes negative once it has run out.
    ///
    /// Rounded *away* from zero on the negative side, so the first tick of
    /// overtime reads -1 rather than 0: a clock that sits on zero for a
    /// second before going negative is a clock that looks stuck, and looking
    /// stuck is the thing this was changed to avoid.
    pub fn seconds_left(&self) -> i32 {
        if self.ticks_left >= 0 {
            self.ticks_left / drive::HZ
        } else {
            -((-self.ticks_left + drive::HZ - 1) / drive::HZ)
        }
    }

    /// Where the passenger or the destination is, for the compass and the
    /// marker.  On the pavement, which is where people are.
    pub fn target(&self) -> Option<(Fx, Fx)> {
        self.fare.as_ref().map(Fare::marker)
    }

    /// Where a car goes to reach that: the kerb beside the marker.
    ///
    /// What the autopilot drives at.  Aiming a car at the marker itself
    /// would aim it over the kerb and onto the paving, which is both wrong
    /// and slower - the last cell is the passenger's to walk.
    pub fn drive_target(&self) -> Option<(Fx, Fx)> {
        self.fare.as_ref().map(Fare::stop)
    }

    /// Bearing from the taxi to the target, relative to where it is pointing.
    /// Zero is dead ahead.
    pub fn target_bearing(&self) -> Option<i32> {
        let (tx, ty) = self.target()?;
        let (dx, dy) = (tx - self.taxi.x, ty - self.taxi.y);
        Some(atan2_approx(dy, dx).wrapping_sub(self.taxi.yaw) as i16 as i32)
    }

    /// Collect every billboard worth drawing this frame, and draw them.
    pub fn draw(&mut self, f: &mut Frame, depth: &[Fx], cam: &Camera, atmos: &Atmos, p: &Proj) {
        self.boards.clear();
        let cull = fixed::from_int(crate::atmos::draw_distance(atmos.haze));
        let near = |x: Fx, y: Fx| fixed::abs(x - cam.x) + fixed::abs(y - cam.y) < cull;

        // The circle, painted on the pavement where the passenger is
        // standing, before anything is put in front of it.  On the pavement
        // and not on the road: it is a person's spot, not a parking space,
        // and a circle painted across a carriageway reads as somewhere to
        // stop *in*.  A cab that pulls up at the kerb beside it is within
        // `REACH` of the person, which is the rule the handover tests.
        if let Some(fare) = &self.fare {
            let (mx, my) = fare.marker();
            if near(mx, my) {
                crate::decal::ring(
                    f,
                    depth,
                    cam,
                    atmos,
                    p,
                    mx,
                    my,
                    STOP_RADIUS,
                    fixed::ratio(1, 8),
                    crate::catalog::shade(6),
                    if fare.aboard { palette::H_CYAN } else { palette::H_GREEN },
                    7,
                );
            }
        }

        for pr in &self.props {
            if near(pr.board.x, pr.board.y) {
                let mut b = pr.board;
                b.phase = ((self.tick / 90) % 3) as u8;
                self.boards.push(b);
            }
        }
        // The cab itself, first, so a chase camera has something to chase.
        // Leaving it out is easy to do - you are notionally inside it - and
        // the result is a camera following an invisible car.
        if near(self.taxi.x, self.taxi.y) {
            let (len, wid, h) = self.taxi.hull();
            let view = crate::sprite::aspect(
                len,
                wid,
                self.taxi.yaw,
                self.taxi.x - cam.x,
                self.taxi.y - cam.y,
            );
            let w = view.width;
            // The cab keeps its own *body* whichever way it is pointing -
            // the chequer band and the roof sign are what make the thing you
            // are chasing recognisable - but it is seen from the same angles
            // as everything else, and from behind and slightly to one side
            // is most of them.
            let mut b = Billboard::upright(
                Stamp::Taxi,
                self.taxi.x,
                self.taxi.y,
                w,
                h,
                palette::H_YELLOW,
            );
            b.view = view;
            // The same two bits every car uses: which end you are looking
            // at, and whether it is on the brakes.  You are usually behind
            // your own cab, but not after a spin.
            let (tfx, tfy) = (trig::cos(self.taxi.yaw), trig::sin(self.taxi.yaw));
            let toward = fixed::mul(cam.x - self.taxi.x, tfx) + fixed::mul(cam.y - self.taxi.y, tfy);
            b.phase = u8::from(toward > 0) | (u8::from(self.braking) << 1);
            self.boards.push(b);
        }
        for (i, c) in self.traffic.iter().enumerate() {
            if !near(c.x, c.y) {
                continue;
            }
            let (len, wid, h) = c.hull();
            let view = crate::sprite::aspect(len, wid, c.yaw, c.x - cam.x, c.y - cam.y);
            // Which picture.  No longer "and from which side": the side is
            // continuous now - see [`crate::sprite::aspect`] - so a stamp
            // names the *body* and nothing else, and the body is a property
            // of the vehicle rather than of the moment.
            let stamp = match (c.kind, c.damage, c.style) {
                (CarKind::Bus, _, _) => Stamp::Bus,
                (_, d, _) if d > 60 => Stamp::Wreck,
                (_, _, drive::Style::Jeep) => Stamp::Jeep,
                (_, _, drive::Style::Boat) => Stamp::Boat,
                (_, _, drive::Style::Sedan) => Stamp::Car,
            };
            let mut b = Billboard::upright(stamp, c.x, c.y, view.width, h, c.hue);
            b.view = view;
            // Which end of it you are looking at, for the lights: the
            // camera is in front of the car when the car's own heading
            // points away from it.
            let (cfx, cfy) = (trig::cos(c.yaw), trig::sin(c.yaw));
            let toward = fixed::mul(cam.x - c.x, cfx) + fixed::mul(cam.y - c.y, cfy);
            // Bit 0: you are looking at its front.  Bit 1: it is braking,
            // which is only worth showing on the end the lamps are on.
            let braking = self.traffic_ctl[i].throttle < 0;
            b.phase = u8::from(toward > 0) | (u8::from(braking && toward <= 0) << 1);
            self.boards.push(b);
        }
        for pd in &self.peds {
            if near(pd.x, pd.y) {
                // Twice as tall as it was, and the same width.
                //
                // Three fifths of a cell is a metre and a half against a
                // car's length of four, which is a person seen from a
                // helicopter and a bollard seen from the pavement.  Six
                // fifths reads as somebody standing there.
                let mut b = Billboard::upright(
                    Stamp::Ped,
                    pd.x,
                    pd.y,
                    fixed::ratio(1, 3),
                    fixed::ratio(6, 5),
                    pd.hue,
                );
                b.phase = pd.phase;
                self.boards.push(b);
            }
        }
        if let Some(fare) = &self.fare {
            for c in &fare.coins {
                if c.taken || !near(c.x, c.y) {
                    continue;
                }
                // The spin is a width that pulses, which is what a spinning
                // disc does and costs one table lookup.
                let s = trig::sin((self.tick.wrapping_mul(2400)) as Ang);
                let w = fixed::ratio(1, 4) + fixed::mul(fixed::abs(s), fixed::ratio(1, 4));
                let b = Billboard::upright(
                    Stamp::Coin,
                    c.x,
                    c.y,
                    w,
                    fixed::ratio(3, 5),
                    palette::H_YELLOW,
                );
                // On the road, not hovering over it.  It was a third of a
                // cell up - two metres - which reads as a coin floating at
                // windscreen height, and which made it impossible to tell
                // whether you were going to drive over one or under it.
                self.boards.push(b);
            }
            let (mx, my) = fare.marker();
            if near(mx, my) {
                let mut b = Billboard::upright(
                    if fare.aboard { Stamp::Dropoff } else { Stamp::Pickup },
                    mx,
                    my,
                    ONE,
                    fixed::ratio(4, 5),
                    if fare.aboard { palette::H_CYAN } else { palette::H_GREEN },
                );
                // Bob, so it is findable in a street full of lit windows.
                b.base = fixed::ratio(3, 2)
                    + fixed::mul(trig::sin((self.tick.wrapping_mul(900)) as Ang), fixed::ratio(1, 5));
                self.boards.push(b);
            }
        }
        crate::sprite::draw_all(f, depth, cam, atmos, p, &mut self.boards, &mut self.order);
    }
}

/// The kerb a car would pull up at to reach a spot on foot.
///
/// The nearest carriageway cell that a cab can actually wait on: a street
/// rather than a service alley, and outside the junction box.  A stop in a
/// junction is a stop in the middle of a crossroads, and on the crossing of
/// two arterials the box is fourteen cells square, so "the nearest road
/// cell" can be a long way from any kerb.
///
/// Two cells of reach.  One covers a passenger on the pavement, which is
/// most of them; two covers one a step back from it, on a plaza or the edge
/// of a park.  Further than that and the marker stops being *beside* a road.
fn kerb_beside(city: &City, x: i32, y: i32) -> Option<(i32, i32)> {
    for r in 1..=2i32 {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs().max(dy.abs()) != r {
                    continue;
                }
                let (px, py) = (x + dx, y + dy);
                if !city.drivable(px, py) || city.plan.is_junction(px, py) {
                    continue;
                }
                if city.plan.cols.at(px).class.is_street()
                    || city.plan.rows.at(py).class.is_street()
                {
                    return Some((px, py));
                }
            }
        }
    }
    None
}

/// Which way along the street a car in the traffic is trying to go.
///
/// Its own direction of travel, resolved onto the axis the street runs on -
/// not the flow of the cell it is sitting in.  The distinction matters after
/// a shunt: a car knocked into the oncoming lane is still going the way it
/// was going, and what it needs is to cross back over, not to turn round.
/// Only when it has been left pointing across the street entirely does the
/// road get to say which way it should go.
fn intent(city: &City, c: &Car) -> (i32, i32) {
    let (cx, cy) = (fixed::floor(c.x), fixed::floor(c.y));
    let (fx, fy) = (trig::cos(c.yaw), trig::sin(c.yaw));
    let along = |v: Fx, f: Fx| if fixed::abs(v) > fixed::HALF { v } else { f };
    match road::street_axis(city, cx, cy) {
        Some(true) => {
            let d = along(c.vx, fx);
            if fixed::abs(d) > fixed::ratio(1, 8) {
                return (if d > 0 { 1 } else { -1 }, 0);
            }
        }
        Some(false) => {
            let d = along(c.vy, fy);
            if fixed::abs(d) > fixed::ratio(1, 8) {
                return (0, if d > 0 { 1 } else { -1 });
            }
        }
        // A junction, or not a street: carry straight on through it.
        None => {}
    }
    if let Some(f) = road::flow(city, cx, cy) {
        return f;
    }
    if fixed::abs(fx) >= fixed::abs(fy) {
        (if fx > 0 { 1 } else { -1 }, 0)
    } else {
        (0, if fy > 0 { 1 } else { -1 })
    }
}

/// The speed that keeps a given gap to the car in front.
///
/// Straight-line in the gap, so it is a following distance rather than a
/// switch: the driver is off the throttle a good way back and only stopped
/// when the gap is gone.
fn follow(gap: Fx) -> Fx {
    if gap <= 0 {
        return 0;
    }
    fixed::mul(CRUISE_THROTTLE * 8, fixed::div(gap, LOOK).min(ONE))
}

/// Whether two headings are crossing rather than sharing a road.
///
/// Within a quarter turn of perpendicular, which on a grid means the other
/// car is on the cross street.
fn crossing(a: Ang, b: Ang) -> bool {
    let d = (b.wrapping_sub(a) as i16 as i32).abs();
    (d - trig::QUARTER as i32).abs() < trig::QUARTER as i32 / 2
}

/// Throttle or brake to hold a speed.
///
/// A dead band around the target, because without one the pedal alternates
/// between flat out and hard on the brake every tick.  And nothing below
/// zero at a standstill: a negative throttle on a stopped car is reverse,
/// and a car that gives way by reversing into the one behind it has not
/// given way.
fn pace(vf: Fx, want: Fx) -> Fx {
    if vf > want + SLACK {
        if vf > ROLLING { -ONE } else { 0 }
    } else if vf < want - SLACK {
        CRUISE_THROTTLE
    } else {
        0
    }
}

/// The offset from a pavement cell's centre that puts a prop `across` cells
/// from the kerb.
///
/// Returns a zero offset when no road adjoins, which leaves the prop
/// centred, and a flag saying which axis the kerb runs along.
fn kerb_offset(city: &City, x: i32, y: i32, across: Fx) -> (Fx, Fx, bool) {
    let road = |dx: i32, dy: i32| city.at(x + dx, y + dy).kind == Kind::Road;
    // Distance from the cell centre to the point `across` from that kerb,
    // and which axis the kerb runs along - the one a prop may wobble on.
    let d = across - fixed::HALF;
    if road(-1, 0) {
        (d, 0, false)
    } else if road(1, 0) {
        (-d, 0, false)
    } else if road(0, -1) {
        (0, d, true)
    } else if road(0, 1) {
        (0, -d, true)
    } else {
        (0, 0, true)
    }
}

/// Whether a sidewalk cell is at a corner - which is where signals go.
fn on_corner(city: &City, x: i32, y: i32) -> bool {
    let road = |dx: i32, dy: i32| city.at(x + dx, y + dy).kind == Kind::Road;
    (road(-1, 0) || road(1, 0)) && (road(0, -1) || road(0, 1))
}

/// String coins along the road between two points.
///
/// Down the route a car would actually take, one every few cells.  It used
/// to be a Manhattan L - across in x, then along in y - which reads well at
/// speed but is only a road by coincidence: it was drawn between two points
/// on the carriageway and kept whichever of its cells happened to land on
/// one, so a trail between two ends of a bent street could be three coins,
/// or none at all.  The route search is the same one the autopilot plans
/// with, so the coins are on the road, in order, and joined up - and a
/// player following them is being shown the way rather than a bearing.
///
/// Every third cell: closer together and the trail is a solid line of gold
/// with no shape to it; further apart and it stops reading as a trail.
fn coin_trail(city: &City, from: (i32, i32), to: (i32, i32)) -> Vec<Coin> {
    let mut coins = Vec::new();
    let Some(route) = city.drive_route(from, to, COIN_BUDGET) else {
        return coins;
    };
    // In the lane, not down the middle of the cell.
    //
    // The route is a breadth-first path and has no opinion about which side
    // of the road it is on, so coins laid on its cell centres are as often
    // in the oncoming lane as in yours - and a car that drives at them,
    // which is the whole point of them, is being paid to drive on the wrong
    // side.  Measured: with the cab collecting, centred coins took it from
    // 79 per cent on the correct side to 55.  Put in the lane, the trail and
    // the lane rule ask for the same thing.
    let n = route.len();
    for (i, &(x, y)) in route.iter().enumerate().step_by(3) {
        // The direction over a few cells, for the same reason the autopilot
        // takes its heading over a few: a route across a wide road
        // staircases, and one step of it is as often across as along.
        let a = route[i.saturating_sub(2)];
        let b = route[(i + 2).min(n - 1)];
        let dir = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
        let (cx, cy) = road::lane(city, x, y, dir);
        coins.push(Coin { x: cx, y: cy, taken: false });
    }
    coins
}

/// A cheap arctangent, accurate to about a fifth of a degree.
///
/// Only ever used for the compass arrow, where a degree is a fifth of a
/// character.  Two shifts, three multiplies and no table beyond the octant
/// fold, all in fixed point, so it goes to the Plus/4 unchanged.
///
/// The kernel is the standard cubic fit on `0..=1`:
///
/// ```text
///     atan(r) ~ (pi/4) r - r (r - 1) (0.2447 + 0.0663 r)
/// ```
///
/// evaluated in angle units, where a quarter turn is 16384 and therefore
/// `pi/4` is 8192 and one radian is 10430.
pub fn atan2_approx(y: Fx, x: Fx) -> Ang {
    if x == 0 && y == 0 {
        return 0;
    }
    let (ax, ay) = (fixed::abs(x), fixed::abs(y));

    // Fold into the first octant so the fit is only ever asked about `0..=1`.
    let (num, den, folded) = if ax >= ay { (ay, ax, false) } else { (ax, ay, true) };
    let r = fixed::div(num, den);
    let t = fixed::mul(r, r - ONE); // negative over the whole range
    let k = fixed::ratio(2447, 10000) + fixed::mul(fixed::ratio(663, 10000), r);
    let corr = fixed::mul(t, k);
    let atan = (((8192i64 * r as i64) >> 16) - ((10430i64 * corr as i64) >> 16)) as i32;

    let mut a = if folded { trig::QUARTER as i32 - atan } else { atan };
    if x < 0 {
        a = trig::HALF as i32 - a;
    }
    if y < 0 {
        a = -a;
    }
    (a & 0xffff) as Ang
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::City;

    /// Straight-line distance between two cars, near enough.
    fn dist2(a: &Car, b: &Car) -> Fx {
        let (dx, dy) = (fixed::abs(a.x - b.x), fixed::abs(a.y - b.y));
        let (hi, lo) = if dx > dy { (dx, dy) } else { (dy, dx) };
        hi + fixed::mul(lo, fixed::ratio(3, 8))
    }

    fn shift() -> (City, Sim) {
        let city = City::generate(404);
        let sim = Sim::new(&city, 404);
        (city, sim)
    }

    #[test]
    fn a_shift_starts_with_furniture_traffic_people_and_a_fare() {
        let (_c, sim) = shift();
        assert!(sim.props.len() > 100, "only {} props", sim.props.len());
        assert_eq!(sim.traffic.len(), TRAFFIC);
        assert_eq!(sim.peds.len(), PEDS);
        assert!(sim.fare.is_some(), "nobody hailed the cab");
        assert_eq!(sim.seconds_left(), START_TIME);
    }

    #[test]
    fn the_taxi_starts_on_a_road() {
        let (city, sim) = shift();
        assert!(city.open(fixed::floor(sim.taxi.x), fixed::floor(sim.taxi.y)));
    }

    #[test]
    fn props_only_stand_on_sidewalks() {
        let (city, sim) = shift();
        for p in &sim.props {
            let k = city.at(fixed::floor(p.board.x), fixed::floor(p.board.y)).kind;
            assert_eq!(k, Kind::Sidewalk, "a {:?} is standing in the {:?}", p.board.stamp, k);
        }
    }

    #[test]
    fn nothing_planted_overhangs_the_road() {
        // The claim is about the *sprite*, not the anchor: a tree is most of
        // a cell across and its crown reached over the kerb when it was
        // merely centred on a pavement cell.
        //
        // Restricted to vegetation on purpose.  Street lighting is placed at
        // the kerb, which is where it is of use and where it necessarily
        // overhangs a little; asserting that nothing at all crosses the kerb
        // line would forbid the arrangement that was asked for.
        let (city, sim) = shift();
        for p in &sim.props {
            let b = &p.board;
            if !b.stamp.planted() {
                continue;
            }
            let half = b.w / 2;
            for (dx, dy) in [(-half, 0), (half, 0), (0, -half), (0, half)] {
                let (x, y) = (fixed::floor(b.x + dx), fixed::floor(b.y + dy));
                assert_ne!(
                    city.at(x, y).kind,
                    Kind::Road,
                    "a {:?} at {},{} reaches over the carriageway",
                    b.stamp,
                    fixed::to_f32(b.x),
                    fixed::to_f32(b.y)
                );
            }
        }
    }

    #[test]
    fn street_lighting_stands_nearer_the_road_than_the_trees_do() {
        // The arrangement, kerb outwards: lighting, verge with the planting
        // in it, then paving.  Checked as an ordering rather than as two
        // absolute positions, so retuning the bands cannot silently swap
        // them.
        let (city, sim) = shift();
        let mean_from_kerb = |want: fn(crate::sprite::Stamp) -> bool| -> f32 {
            let mut total = 0.0f32;
            let mut n = 0.0f32;
            for p in &sim.props {
                if !want(p.board.stamp) {
                    continue;
                }
                let (gx, gy) = (fixed::floor(p.board.x), fixed::floor(p.board.y));
                let (fx, fy) = (fixed::frac(p.board.x), fixed::frac(p.board.y));
                let mut d = ONE;
                for (dx, dy, edge) in [
                    (-1, 0, fx),
                    (1, 0, ONE - fx),
                    (0, -1, fy),
                    (0, 1, ONE - fy),
                ] {
                    if city.at(gx + dx, gy + dy).kind == Kind::Road {
                        d = d.min(edge);
                    }
                }
                total += fixed::to_f32(d);
                n += 1.0;
            }
            total / n.max(1.0)
        };
        let lamps = mean_from_kerb(|s| s == crate::sprite::Stamp::LampPost);
        let trees = mean_from_kerb(|s| s.planted());
        assert!(
            lamps < trees,
            "lighting sits {lamps:.2} cells from the kerb and planting {trees:.2}"
        );
    }

    #[test]
    fn the_pavement_stands_above_the_carriageway() {
        let (city, _sim) = shift();
        let mut checked = 0;
        for y in 0..SIZE as i32 {
            for x in 0..SIZE as i32 {
                if city.at(x, y).kind != Kind::Sidewalk {
                    continue;
                }
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    if city.at(x + dx, y + dy).kind != Kind::Road {
                        continue;
                    }
                    checked += 1;
                    assert!(
                        city.ground(x, y) > city.ground(x + dx, y + dy),
                        "the pavement at {x},{y} is {} steps and the road beside it {}",
                        city.elev.ground_steps(x, y),
                        city.elev.ground_steps(x + dx, y + dy),
                    );
                }
            }
        }
        assert!(checked > 500, "only {checked} kerb edges examined");
    }

    #[test]
    fn the_cab_is_drawn_so_there_is_something_to_follow() {
        let (city, mut sim) = shift();
        let cam = crate::camera::Camera::spawn(&city, fixed::floor(sim.taxi.x), fixed::floor(sim.taxi.y));
        let atmos = crate::atmos::Atmos::default();
        let mut f = crate::frame::Frame::new(80, 24);
        let mut depth = Vec::new();
        crate::raycast::render_to(&city, &cam, &atmos, &mut f, &mut depth);
        let proj = crate::raycast::projection(&city, &cam, &f);
        sim.draw(&mut f, &depth, &cam, &atmos, &proj);
        assert!(
            sim.boards.iter().any(|b| b.stamp == crate::sprite::Stamp::Taxi),
            "the cab was not among the billboards"
        );
    }

    /// The brake lights come on when you ask for the brakes.
    ///
    /// The chase camera looks at the back of the cab, so the rear lamps are
    /// the part of the car you spend the game looking at: they are the only
    /// feedback in the frame that says the car heard the key.
    #[test]
    fn the_cab_lights_up_when_it_brakes() {
        let city = City::generate(99);
        let mut sim = Sim::new(&city, 99);
        let mut ev = Vec::new();
        // A camera behind the cab, looking at it.
        let mut cam = crate::camera::Camera::spawn(&city, 117, 117);
        cam.yaw = sim.taxi.yaw;
        let (dx, dy) = cam.dir();
        cam.x = sim.taxi.x - fixed::mul(dx, fixed::from_int(4));
        cam.y = sim.taxi.y - fixed::mul(dy, fixed::from_int(4));
        cam.z = city.ground(fixed::floor(cam.x), fixed::floor(cam.y)) + fixed::ratio(4, 5);

        let mut f = Frame::new(80, 30);
        let mut depth = Vec::new();
        let atmos = Atmos { ..Default::default() };
        let lamps = |sim: &mut Sim, f: &mut Frame, depth: &mut Vec<Fx>| -> usize {
            crate::raycast::render_to(&city, &cam, &atmos, f, depth);
            let p = crate::raycast::projection(&city, &cam, f);
            sim.draw(f, depth, &cam, &atmos, &p);
            f.cels
                .iter()
                .filter(|c| {
                    palette::hue_of(c.color) == palette::H_RED && palette::luma_of(c.color) >= 6
                })
                .count()
        };

        sim.step(&city, &Controls { throttle: ONE, ..Default::default() }, drive::HZ, &mut ev);
        let rolling = lamps(&mut sim, &mut f, &mut depth);
        sim.step(&city, &Controls { throttle: -ONE, ..Default::default() }, drive::HZ, &mut ev);
        let braking = lamps(&mut sim, &mut f, &mut depth);
        assert!(
            braking > rolling,
            "the brake lights did not come on: {braking} lit cells against {rolling}"
        );
    }

    #[test]
    fn the_clock_runs_down_and_the_shift_ends() {
        let (city, mut sim) = shift();
        let mut ev = Vec::new();
        sim.ticks_left = 3;
        let mut called = 0;
        for _ in 0..drive::HZ * 3 {
            sim.step(&city, &Controls::default(), drive::HZ, &mut ev);
            called += ev.iter().filter(|e| matches!(e, Event::TimeUp)).count();
        }
        // Once, and then the shift carries on into the red.
        assert_eq!(called, 1, "the clock called time {called} times");
        assert!(!sim.over, "running out of time froze the shift");
        assert!(sim.seconds_left() < 0, "the clock stopped at zero");
        assert!(sim.money < 0, "the meter stopped at nothing");
    }

    #[test]
    fn driving_into_a_lamp_post_flattens_it_and_does_not_slow_the_car() {
        let (_city, mut sim) = shift();
        let mut ev = Vec::new();
        // Park the taxi on top of a standing prop, moving.
        let p = sim.props[0].board;
        sim.taxi.x = p.x;
        sim.taxi.y = p.y;
        sim.taxi.vx = fixed::from_int(4);
        let before = sim.taxi.speed();
        sim.step_props(drive::HZ, &mut ev);
        assert!(ev.contains(&Event::Flattened), "the post survived");
        assert!(!sim.props[0].standing);
        assert_eq!(sim.taxi.speed(), before, "the post slowed the car down");
        assert!(sim.combo >= 1);
    }

    #[test]
    fn a_flattened_prop_stays_flattened() {
        let (_c, mut sim) = shift();
        let mut ev = Vec::new();
        sim.props[0].standing = false;
        sim.taxi.x = sim.props[0].board.x;
        sim.taxi.y = sim.props[0].board.y;
        sim.taxi.vx = fixed::from_int(5);
        for _ in 0..100 {
            sim.step_props(drive::HZ, &mut ev);
        }
        assert!(!sim.props[0].standing);
        assert_eq!(sim.props[0].board.lean, 8, "it should have come to rest flat");
    }

    #[test]
    fn a_slow_car_does_not_flatten_anything() {
        let (_c, mut sim) = shift();
        let mut ev = Vec::new();
        sim.taxi.x = sim.props[0].board.x;
        sim.taxi.y = sim.props[0].board.y;
        sim.taxi.vx = fixed::ratio(1, 4);
        sim.step_props(drive::HZ, &mut ev);
        assert!(sim.props[0].standing, "it fell over at walking pace");
    }

    #[test]
    fn stopping_at_the_marker_picks_the_fare_up_and_adds_time() {
        let (city, mut sim) = shift();
        let mut ev = Vec::new();
        let from = sim.fare.as_ref().unwrap().from;
        sim.taxi.x = from.0;
        sim.taxi.y = from.1;
        sim.taxi.vx = 0;
        sim.taxi.vy = 0;
        let clock = sim.ticks_left;
        sim.step_fare(&city, &mut ev);
        assert!(ev.contains(&Event::PickedUp), "the passenger did not get in");
        assert!(sim.fare.as_ref().unwrap().aboard);
        assert!(sim.ticks_left > clock, "picking up did not buy any time");
    }

    #[test]
    fn arriving_too_fast_does_not_pick_anyone_up() {
        let (city, mut sim) = shift();
        let mut ev = Vec::new();
        let from = sim.fare.as_ref().unwrap().from;
        sim.taxi.x = from.0;
        sim.taxi.y = from.1;
        sim.taxi.vx = fixed::from_int(6);
        sim.step_fare(&city, &mut ev);
        assert!(ev.is_empty(), "picked up a passenger at 90 mph");
    }

    #[test]
    fn delivering_a_fare_pays_and_finds_another() {
        let (city, mut sim) = shift();
        let mut ev = Vec::new();
        let f = sim.fare.as_ref().unwrap().clone();
        sim.fare.as_mut().unwrap().aboard = true;
        sim.taxi.x = f.to.0;
        sim.taxi.y = f.to.1;
        sim.taxi.vx = 0;
        sim.taxi.vy = 0;
        sim.step_fare(&city, &mut ev);
        assert!(ev.contains(&Event::DroppedOff));
        assert!(sim.money >= f.value as i32, "the fare was not paid");
        assert!(sim.fare.is_some(), "no new fare after a drop-off");
    }

    #[test]
    fn coins_are_on_the_road_and_can_be_collected() {
        let (city, mut sim) = shift();
        let mut ev = Vec::new();
        let coins = sim.fare.as_ref().unwrap().coins.clone();
        assert!(!coins.is_empty(), "no coins on the route");
        for c in &coins {
            assert_eq!(city.at(fixed::floor(c.x), fixed::floor(c.y)).kind, Kind::Road);
        }
        sim.taxi.x = coins[0].x;
        sim.taxi.y = coins[0].y;
        let clock = sim.ticks_left;
        sim.step_fare(&city, &mut ev);
        assert!(ev.contains(&Event::Coin));
        assert!(sim.ticks_left > clock, "a coin bought no time");
    }

    #[test]
    fn a_coin_can_only_be_collected_once() {
        let (city, mut sim) = shift();
        let mut ev = Vec::new();
        let c0 = sim.fare.as_ref().unwrap().coins[0];
        sim.taxi.x = c0.x;
        sim.taxi.y = c0.y;
        sim.step_fare(&city, &mut ev);
        let clock = sim.ticks_left;
        sim.step_fare(&city, &mut ev);
        assert_eq!(sim.ticks_left, clock, "the same coin paid twice");
    }

    #[test]
    fn a_long_shift_never_panics_and_never_leaves_the_map() {
        let city = City::generate(88);
        let mut sim = Sim::new(&city, 88);
        let mut ev = Vec::new();
        sim.ticks_left = i32::MAX / 2;
        let mut steer = ONE;
        for i in 0..12_000 {
            if i % 53 == 0 {
                steer = -steer;
            }
            sim.step(
                &city,
                &Controls { throttle: ONE, steer, handbrake: i % 300 < 25 },
                drive::HZ,
                &mut ev,
            );
            assert!(city.open(fixed::floor(sim.taxi.x), fixed::floor(sim.taxi.y)));
        }
        assert!(!sim.over);
    }

    #[test]
    fn traffic_stays_near_the_player() {
        let city = City::generate(9);
        let mut sim = Sim::new(&city, 9);
        let mut ev = Vec::new();
        for _ in 0..3000 {
            sim.step(&city, &Controls { throttle: ONE, ..Default::default() }, drive::HZ, &mut ev);
        }
        let stragglers = sim
            .traffic
            .iter()
            .filter(|c| {
                fixed::abs(c.x - sim.taxi.x) + fixed::abs(c.y - sim.taxi.y)
                    > fixed::from_int(RECYCLE + 16)
            })
            .count();
        assert_eq!(stragglers, 0, "{stragglers} cars were left behind");
    }

    /// Which side of the crown of the road a car is on, for its direction of
    /// travel, or `None` where the question has no answer - inside a
    /// junction, on a road too narrow to have sides, or for a car that is
    /// travelling across the street rather than along it.
    ///
    /// Positive is the correct side for driving on the right.
    fn side_of_road(city: &City, c: &Car) -> Option<Fx> {
        let (x, y) = (fixed::floor(c.x), fixed::floor(c.y));
        let along_x = road::street_axis(city, x, y)?;
        let cell = if along_x { city.plan.rows.at(y) } else { city.plan.cols.at(x) };
        if cell.width < 2 {
            return None;
        }
        let v = if along_x { c.vx } else { c.vy };
        if fixed::abs(v) < ONE {
            return None;
        }
        let kerb = fixed::from_int(if along_x { y } else { x } - cell.across as i32);
        let mid = kerb + fixed::from_int(cell.width as i32) / 2;
        let off = if along_x { c.y - mid } else { c.x - mid };
        Some(if along_x == (v > 0) { off } else { -off })
    }

    /// Traffic keeps right, and it does so from the moment it is put down.
    ///
    /// The direction each car faces is taken from the half of the road it is
    /// on, so this is really a test that the two are read the same way by
    /// the thing that places cars and the thing that steers them.  It used
    /// to be a coin toss by construction: a car was dropped on a road cell
    /// and pointed along it either way.
    #[test]
    fn traffic_keeps_to_the_right_hand_lane() {
        for seed in [1u32, 7, 99, 4242] {
            let city = City::generate(seed);
            let mut sim = Sim::new(&city, seed);
            let mut ev = Vec::new();
            let (mut right, mut wrong) = (0u32, 0u32);
            for _ in 0..1200 {
                sim.step(&city, &Controls::default(), drive::HZ, &mut ev);
                for c in &sim.traffic {
                    match side_of_road(&city, c) {
                        Some(s) if s > 0 => right += 1,
                        Some(s) if s < 0 => wrong += 1,
                        _ => {}
                    }
                }
            }
            println!("seed {seed}: {right} right {wrong} wrong - {}%", right * 100 / (right + wrong).max(1));
            assert!(right + wrong > 1000, "seed {seed}: only {} ticks with a side", right + wrong);
            // Nine to one, measured at 98 to 100 per cent.  What is left is
            // cars crossing back after a shunt, which is the correct thing
            // for them to be doing.
            assert!(
                right > wrong * 9,
                "seed {seed}: {right} car-ticks on the right, {wrong} on the wrong side"
            );
        }
    }

    /// Traffic gives way instead of driving through itself.
    ///
    /// Two things are being asserted at once and they are the same thing:
    /// cars leave a gap, and when they fail to, the collision is resolved.
    /// Before, traffic was invisible to itself - two cars would occupy the
    /// same six metres of road and drive on together.
    #[test]
    fn traffic_gives_way_to_traffic() {
        let city = City::generate(11);
        let mut sim = Sim::new(&city, 11);
        let mut ev = Vec::new();
        let mut overlapping = 0;
        for _ in 0..1800 {
            sim.step(&city, &Controls::default(), drive::HZ, &mut ev);
            for i in 0..sim.traffic.len() {
                for j in i + 1..sim.traffic.len() {
                    let (a, b) = (&sim.traffic[i], &sim.traffic[j]);
                    let reach = a.half_len() + b.half_len();
                    // Well inside each other, not merely touching: a
                    // collision is resolved over a tick or two and touching
                    // during it is the point.
                    if dist2(a, b) < fixed::mul(reach, fixed::HALF) {
                        overlapping += 1;
                    }
                }
            }
        }
        // One overlap in a thousand pair-ticks.  Not zero: a shunt is
        // resolved over a tick or two and being inside each other for part
        // of one is what a collision *is*.  Measured at 33 in 66,000 with
        // the drivers giving way and 366 with them ignoring each other,
        // which is the difference the rule makes.
        //
        // Counted against the *pairs* sampled rather than as a flat number,
        // because the pool went from twelve cars to sixty-four and the pairs
        // with it - from 66 to 2,016 - so a bar written as a count would
        // have tightened thirtyfold without anybody deciding to tighten it.
        let pairs = sim.traffic.len() * (sim.traffic.len() - 1) / 2;
        let ticks = pairs as u32 * 1_000;
        assert!(
            overlapping * 1_000 < ticks,
            "{overlapping} car-ticks spent inside another car, of {ticks}"
        );
    }

    /// A car with something stopped in its lane slows down for it.
    #[test]
    fn a_driver_lifts_off_for_the_car_in_front() {
        let city = City::generate(3);
        let mut sim = Sim::new(&city, 3);
        // Two cars and nobody else.  The pool is sixty-four deep and spread
        // over four blocks, so a third car happening to sit in front of the
        // one being measured makes both halves of the comparison come back
        // the same and the test says nothing.
        let lead = sim.traffic[0];
        for k in 2..sim.traffic.len() {
            sim.traffic[k].x = fixed::from_int(SIZE as i32 - 2);
            sim.traffic[k].y = fixed::from_int(SIZE as i32 - 2);
        }
        let (fx, fy) = (trig::cos(lead.yaw), trig::sin(lead.yaw));
        let back = fixed::from_int(4);
        sim.traffic[0].vx = 0;
        sim.traffic[0].vy = 0;
        sim.traffic[1] = Car::new(
            CarKind::Traffic,
            lead.x - fixed::mul(fx, back),
            lead.y - fixed::mul(fy, back),
            lead.yaw,
            palette::H_RED,
        );
        sim.traffic_cruise[1] = fixed::from_int(3);
        let free = sim.give_way(1);
        // And the same car with the road ahead of it cleared.
        sim.traffic[0].x = sim.taxi.x;
        sim.traffic[0].y = sim.taxi.y;
        let blocked_by_nothing = sim.give_way(1);
        assert!(
            free < blocked_by_nothing,
            "a car four cells behind a parked one wanted {} against {} on a clear road",
            fixed::to_f32(free),
            fixed::to_f32(blocked_by_nothing)
        );
    }

    /// Nobody is put down in the roadway who does not belong there.
    ///
    /// The fare's two ends and the pedestrians are all on the walking
    /// network; the cab and the traffic are the only things on the
    /// carriageway.
    #[test]
    fn people_are_put_down_beside_the_road_and_not_on_it() {
        for seed in [1u32, 7, 99, 4242] {
            let city = City::generate(seed);
            let mut sim = Sim::new(&city, seed);
            for _ in 0..40 {
                sim.hail(&city);
                let fare = sim.fare.as_ref().expect("no fare");
                for (name, (mx, my)) in [("pickup", fare.from), ("dropoff", fare.to)] {
                    let (x, y) = (fixed::floor(mx), fixed::floor(my));
                    assert_ne!(
                        city.at(x, y).kind,
                        Kind::Road,
                        "seed {seed}: the {name} is in the carriageway at {x},{y}"
                    );
                    assert_eq!(
                        city.walk.at(x, y),
                        Foot::Path,
                        "seed {seed}: the {name} is not somewhere a person can stand"
                    );
                }
                for (name, (sx, sy)) in
                    [("pickup", fare.from_stop), ("dropoff", fare.to_stop)]
                {
                    let (x, y) = (fixed::floor(sx), fixed::floor(sy));
                    assert!(
                        city.drivable(x, y),
                        "seed {seed}: the {name} kerb at {x},{y} is not road"
                    );
                }
                // And a car standing on the kerb is close enough to hand the
                // passenger over, which is the whole point of having two
                // places instead of one.
                assert!(
                    fare.at_stop(fare.from_stop.0, fare.from_stop.1),
                    "seed {seed}: a cab at the kerb cannot reach the passenger"
                );
            }
        }
    }

    /// A fare is somewhere you have to drive to.
    ///
    /// The pickup is at least a block from where the cab is standing, and
    /// the drop-off at least a block from the pickup - the second measured
    /// from the *pickup*, because that is the distance the passenger is
    /// paying for and two places can both be far from the cab and next door
    /// to each other.
    #[test]
    fn the_next_fare_is_at_least_a_block_away() {
        for seed in [1u32, 7, 99, 4242] {
            let city = City::generate(seed);
            let mut sim = Sim::new(&city, seed);
            let (mut near_cab, mut short_fare) = (0, 0);
            for _ in 0..60 {
                sim.hail(&city);
                let fare = sim.fare.as_ref().expect("no fare");
                let cab = (fixed::floor(sim.taxi.x), fixed::floor(sim.taxi.y));
                let at = (fixed::floor(fare.from.0), fixed::floor(fare.from.1));
                let to = (fixed::floor(fare.to.0), fixed::floor(fare.to.1));
                if (at.0 - cab.0).abs() + (at.1 - cab.1).abs() < HAIL_NEAR {
                    near_cab += 1;
                }
                if (to.0 - at.0).abs() + (to.1 - at.1).abs() < FARE_MIN {
                    short_fare += 1;
                }
            }
            // The pickup is a hard floor - it is what the dart is thrown at.
            assert_eq!(near_cab, 0, "seed {seed}: {near_cab} fares within a block of the cab");
            // The pair is a preference: eight tries at it, and a short fare
            // is better than none.  Measured at zero on all four cities.
            assert!(short_fare < 4, "seed {seed}: {short_fare} of 60 fares were shorter than a block");
        }
    }

    /// The coins are on the road and joined up, because they are the route.
    #[test]
    fn the_coin_trail_follows_the_road() {
        for seed in [1u32, 7, 99] {
            let city = City::generate(seed);
            let mut sim = Sim::new(&city, seed);
            for _ in 0..12 {
                sim.hail(&city);
                let fare = sim.fare.as_ref().expect("no fare");
                assert!(!fare.coins.is_empty(), "seed {seed}: a fare with no coins");
                for c in &fare.coins {
                    let (x, y) = (fixed::floor(c.x), fixed::floor(c.y));
                    assert!(city.drivable(x, y), "seed {seed}: a coin off the road at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn the_compass_points_the_right_way() {
        let (_c, mut sim) = shift();
        sim.taxi.x = fixed::from_int(20);
        sim.taxi.y = fixed::from_int(20);
        sim.taxi.yaw = 0; // facing +x
        let f = sim.fare.as_mut().unwrap();
        f.aboard = false;
        f.from = (fixed::from_int(30), fixed::from_int(20)); // dead ahead
        let b = sim.target_bearing().unwrap();
        assert!(b.abs() < 2000, "dead ahead read as {b}");

        let f = sim.fare.as_mut().unwrap();
        f.from = (fixed::from_int(20), fixed::from_int(30)); // to the right
        let b = sim.target_bearing().unwrap();
        assert!((b - trig::QUARTER as i32).abs() < 2500, "90 degrees right read as {b}");
    }

    #[test]
    fn atan2_agrees_with_the_sine_table() {
        for deg in (0..360).step_by(7) {
            let a = trig::from_degrees(deg as f64);
            let got = atan2_approx(trig::sin(a), trig::cos(a));
            let err = (got.wrapping_sub(a)) as i16 as i32;
            assert!(err.abs() < 700, "{deg} degrees came back {err} units out");
        }
    }
}

//! Arcade car physics.
//!
//! This is not a simulation.  There are no tyres, no weight transfer, no
//! suspension and no engine curve, and adding any of them would make the car
//! worse.  What is modelled is the *feel* the reference asks for, which is
//! four properties and nothing else:
//!
//! 1. **The car wants to go forwards, like a boat.**  Velocity is split into
//!    a longitudinal component and a lateral one; the lateral one bleeds
//!    away every tick.  How fast it bleeds is the entire handling model.
//! 2. **Turning the wheel does not turn the velocity.**  The heading is
//!    rotated *after* the velocity is recombined, so the car's momentum
//!    carries on in the old direction for a few ticks.  That gap between
//!    where the car points and where it is going is the drift, and it is a
//!    consequence of the update order rather than a special case.
//! 3. **It is a car up to a point and a boat past it.**  Grip is quoted as
//!    the fraction of the slide that survives a tick, and it is held near
//!    the parked figure through the whole ordinary speed range and then
//!    let go over the last of it: town driving tracks the nose, and only
//!    flat out does the tail come round.  The handbrake removes grip at
//!    any speed, which is how you get the car sideways on purpose.
//! 4. **The wheel stops working as the speed rises.**  Yaw rate climbs
//!    with speed while the steering lock is what limits it, and falls away
//!    as `1 / speed` once the front tyres are, which is to say the car has
//!    a fixed cornering *force* rather than a fixed cornering angle.  The
//!    radius of the tightest corner available therefore grows with the
//!    square of the speed: 5 m at 28 km/h, 18 m at 65, 87 m flat out.
//!    This is the one piece of real vehicle behaviour in here, and it is
//!    present because without it a car with grip pivots on the spot at
//!    150 km/h, which is what a tank does.
//! 5. **Buildings are rigid and everything else is not.**  A wall stops the
//!    car and costs it speed and paint - and then turns it to run along
//!    itself, because a city this tight is mostly alleys and a car that
//!    rebounds off one wall is a car aimed at the other.  A lamp post does
//!    none of that.
//!
//! Reality is not a goal.  Pace is.

use crate::fixed::{self, Fx, ONE};
use crate::trig::{self, Ang};
use crate::world::City;

/// Ticks per second the physics is written for.  The step function takes the
/// real rate and scales, but the constants below are quoted at this one.
pub const HZ: i32 = 60;

/// Engine force at a standstill, in units per second per second.
///
/// Peak force, and the car does not have it at a standstill: it is scaled
/// down at the bottom of the range by [`LAUNCH_BITE`] and tapered away at
/// the top by [`ENGINE_CEILING`], so the most the engine pushes is somewhere
/// in the middle.  A constant force is what this was, and it is why the car
/// had no acceleration to speak of: at twenty-six units per second per
/// second against a top speed of seven, the car was at the clamp in a
/// quarter of a second, from any speed, in any gear it does not have.  There
/// was nothing to hold the throttle *down* for, which is most of what
/// driving one of these is.
const ACCEL: Fx = fixed::ratio(10, 1);
/// The speed at which the engine has nothing left to give, in units per
/// second.
///
/// Force falls off linearly from [`ACCEL`] at the top of the engine's bite
/// to nothing here, which is the upper half of a torque curve through a
/// gearbox and, more to the point, the half that makes the last of the speed
/// something the car has to *hold* the throttle for.  The approach is
/// exponential, so what this really sets is a time constant.
///
/// A quarter above [`VMAX`] rather than equal to it, because the engine has
/// to out-pull the drag at the top of the range or the car never reaches
/// the speed it is supposed to have.  With the ceiling at the top speed,
/// force and drag balance a little under it and the clamp never binds.
const ENGINE_CEILING: Fx = fixed::ratio(35, 4);
/// The speed by which the engine is pulling with everything it has, in
/// units per second.
///
/// See [`LAUNCH_BITE`].  About 90 mph, which is a whole town speed range
/// spent building rather than a moment spent arriving.
const LAUNCH: Fx = fixed::ratio(3, 1);
/// What fraction of its force the engine has at a standstill.
///
/// This is the missing half of the curve, and it is the half you feel most,
/// because it is the only half that happens while you are looking at the
/// road rather than at a blur.  [`ACCEL`] against [`ENGINE_CEILING`] is a
/// force that is *largest* at a standstill and only ever falls: the car left
/// the line harder than it did anything else afterwards, reached 60 mph in a
/// third of a second and the base top speed inside one, so pulling away was
/// a cut rather than a launch and the throttle had no bottom half.
///
/// A real drivetrain is the other way up at the bottom: a stationary engine
/// is off its torque, the clutch is slipping, and the force arrives as the
/// car starts moving.  So the force is scaled from this fraction at rest up
/// to all of it by [`LAUNCH`], and the product of a rising bite and a
/// falling taper is a hump - which is what a torque curve through a gearbox
/// is, and what makes the middle of the range the part that shoves.
///
/// A fifth.  Measured: 60 mph in about a second and a half rather than a
/// third of one, and the top speeds are untouched, because above [`LAUNCH`]
/// this term is one and the engine is exactly the engine it was.
const LAUNCH_BITE: Fx = fixed::ratio(1, 3);
/// Braking is stronger than the engine, as it is on every car.
const BRAKE: Fx = fixed::ratio(44, 1);
/// Reverse is weak, as it is on every car.
const REVERSE: Fx = fixed::ratio(9, 1);
/// Rolling drag, as a per-second fraction of speed retained.
///
/// Linear, which is what a tyre and a bearing are: the same fraction of
/// whatever you are doing, so it barely shows at the top of the range and is
/// most of what stops the car at the bottom of it.
const DRAG: Fx = fixed::ratio(88, 100);
/// Air drag, per second per unit of speed squared.
///
/// The other half of the curve, and the half that was missing.  With only a
/// linear drag the engine's own taper is all that limits the car, so the
/// speed goes wherever the top-speed clamp is put and the clamp is the whole
/// feel: the throttle wind-up read as a number going up rather than as a car
/// getting faster, because every step accelerated exactly as hard as the
/// last one.
///
/// Squared drag makes the last few miles an hour cost what they cost.  It
/// sets a *natural* top speed - where the engine and the air balance - so
/// each step of the wind-up buys progressively less, which is the shape of
/// every accelerating thing there is.  Measured: the car settles at about
/// 150 mph unwound, 270 held down, and 310 with a coin in it, without any of
/// those numbers being written down anywhere.
const DRAG_AIR: Fx = fixed::ratio(19, 100);
/// Top speed, in units per second.  A unit is six metres, so this is about
/// 150 km/h - fast enough that the grid goes past in a blur.
const VMAX: Fx = fixed::ratio(7, 1);
/// Lateral grip, quoted as the fraction of the slide that survives *one
/// tick* at [`HZ`].
///
/// Per tick, not per second, and the distinction is the whole reason the
/// car used to swim.  A per-second figure has to be linearised to be spent
/// a tick at a time, and the linear form cannot remove more than one
/// tick's worth of anything: with the bleed written as `(1 - keep) / 60`
/// per tick, even a grip of zero leaves `(1 - 1/60)^60`, or 37%, of the
/// slide alive after a full second.  Every corner was a boat because no
/// setting of the old constants could make one that was not.
///
/// Parked, under three quarters of the slide survives a tick and nothing
/// measurable survives a second: the car goes where it points.
const GRIP_LOW_SPEED: Fx = fixed::ratio(72, 100);
/// Lateral grip at top speed.
///
/// It was 0.985, which leaves half the slide alive three quarters of a
/// second later - a boat.  Measured against the corner table, that read as
/// 0.29 of slip through a full-lock quarter turn at 150 km/h with nothing
/// touching the handbrake, and a car that drifts without being asked to is a
/// car you cannot place between two rows of buildings six metres apart.  At
/// 0.95 the same corner slips 0.15 and the handbrake still slides 0.84, so
/// the slide is something you ask for.
const GRIP_HIGH_SPEED: Fx = fixed::ratio(950, 1000);
/// Lateral grip with the handbrake pulled: the slide outlives the corner.
const GRIP_HANDBRAKE: Fx = fixed::ratio(995, 1000);
/// How the grip between those two is found, as the power the speed fraction
/// is raised to before interpolating.
///
/// Cubed, so grip is still nearly the parked figure at half the top speed
/// and only lets go over the last quarter of the range.  Interpolating
/// straight down the speed - which is what this did - makes the middle of
/// the range, where the car actually spends its life, half a boat.
const GRIP_CURVE: u32 = 3;
/// Peak yaw rate, in angle units per second: about 130 degrees a second.
const TURN_RATE: i32 = 24_000;
/// How far down the throttle has to be to count as pinned.
const PIN_DOWN: Fx = fixed::ratio(3, 4);
/// How long one step of the wind-up takes, in ticks at [`HZ`].
///
/// A second.  It was half of one, which was quicker than the car itself: the
/// cap had finished tripling before the engine had got the car to a third of
/// the first cap, so all three steps landed on a car that was nowhere near
/// any of them and the whole wind-up read as one long pull.  A second apiece
/// gives the car time to arrive at each ceiling and sit on it, which is what
/// makes the next step a step.
const PIN_STEP: u32 = HZ as u32;
/// How many steps there are.
///
/// Three, a second apart.  Each one raises the ceiling and arrives as a
/// shove - see [`SURGE`] - so the build is something you feel three times
/// rather than a number going up: measured from a standstill, 83 mph at one
/// second, 171 by two, 242 by three, settling at 311 a second after that.
/// Each of those is a plateau the car reaches and holds before the next step
/// lands, which is what makes the build four moments rather than one ramp.
///
/// It is worth about twice the unwound top speed rather than the three times
/// the multiplier says, and that is the air drag rather than a bug.  Drag
/// rises with the square of the speed, so three times the speed wants nine
/// times the force *and* an engine that is still pulling there; what the
/// steps actually buy is a car that goes from a town speed to a frightening
/// one and then stops gaining, which is the shape that was wanted.
const PIN_STEPS: u32 = 3;
/// What the top speed is multiplied by at the last step, and what a coin
/// makes it.
///
/// Three, and four with a coin in you.  The coin is worth a step past
/// anything holding the pedal can do on its own, which is what makes one
/// worth going slightly out of the way for even when the clock is not the
/// thing you are short of.
const PIN_MAX: Fx = fixed::ratio(3, 1);
const PIN_BOOST: Fx = fixed::ratio(4, 1);

/// The shove each step of the wind-up arrives with, in units a second.
///
/// Two thirds of a unit - about fifteen miles an hour - which is enough to
/// be felt through the seat and not enough to be a teleport.
const SURGE: Fx = fixed::ratio(2, 3);

/// How far over the wheel has to be to count as held.
const WIND_LOCK: Fx = fixed::ratio(3, 4);
/// How long it has to be held there before the lock starts winding on, in
/// ticks at [`HZ`].
///
/// A second.  Anything shorter and an ordinary junction - which is a second
/// of lock at town speed - starts to tighten on its own, and a corner that
/// tightens without being asked is a corner you cannot place.
const WIND_AFTER: u32 = HZ as u32;
/// And how long from there to all of it.
const WIND_OVER: u32 = HZ as u32;
/// How much more lock a fully wound wheel is worth.
///
/// A quarter again.  It is what turns a held turn into a tightening one:
/// the first second is the corner you asked for, and past that the car keeps
/// turning in, which is how you get round something you have misjudged.
///
/// It was half again, and half again is too much for anything that holds
/// lock as a matter of course.  The autopilot does - its steering latch pins
/// the wheel through every junction - so it wound itself onto the pavement:
/// 26 per cent of one city's travelling ticks off the carriageway, against
/// 2.  A quarter is a wind-on you can feel and not one that drives for you.
const WIND_MAX: Fx = fixed::ratio(1, 4);
/// Speed at which the car turns its hardest, in units per second - about
/// 32 km/h.
///
/// Below it the steering lock is the limit, the yaw rate rises with speed,
/// and the car turns inside about five metres however slowly it is going.
/// Above it the grip is the limit: the yaw rate falls as `TURN_REF / speed`,
/// which is a corner of constant force and so of a radius that grows with
/// the square of the speed.  The alternative - the flat plateau this had -
/// lets the car spin on its own axis at any speed you like.
const TURN_REF: Fx = fixed::ratio(3, 2);
/// What is left of the speed *into* a wall after hitting it.
///
/// Nothing.  It was -0.35, which is a bounce: the car came off the wall
/// backwards, and in an alley - two walls a car and a half apart - the
/// bounce off one is the run-up to the other, so a single clip became a
/// pinball rally that ended with the cab facing the way it came.  A
/// building is not a bumper.  It takes the speed you drove into it and
/// leaves you the speed you had along it, which is the difference between
/// hitting a wall and scraping one.
///
/// The speed along the wall is untouched here, and that is the point: it is
/// what [`WALL_ALIGN`] then has something to steer.
const WALL_BOUNCE: Fx = fixed::ratio(-5, 100);
/// How fast a wall turns the car to run along it, in angle units per second.
///
/// A wall you are still touching is a wall you are still scraping, so this
/// is applied every tick of contact and stops of its own accord the moment
/// the car is parallel - which makes it a *lean*, not a snap: hit a wall
/// square and it does nothing, because square has no wall to point along;
/// clip one at twenty degrees and the car is straight again in a third of a
/// second, still moving, still yours.
///
/// About seventy degrees a second, which is a little over half of what the
/// car's own wheel can do.  Faster and the wall drives for you; slower and
/// an alley eats the whole fare.  What it replaces is a flat 120 units of
/// spin per hit in whichever direction the impact happened to be, which
/// knocked the car crooked - fine on an open street, fatal between two
/// buildings, where crooked is how you hit the next one.
const WALL_ALIGN: i32 = 13_000;
/// How much of the car's body a wall claims per impact, per unit of speed.
const WALL_DAMAGE: i32 = 9;
/// How much of a car's own length is its bumper, for deciding that two of
/// them have touched.
///
/// Three quarters.  The bodies are drawn as boxes and collided as circles,
/// and a circle round a box that is twice as long as it is wide is a great
/// deal of empty air at the corners: at the full half-length two cars
/// passing in opposite lanes of a two-cell street clipped each other, which
/// is not a near miss, it is a phantom.  Three quarters lets them pass and
/// still stops a rear-ending from being a drive-through.
const CONTACT: Fx = fixed::ratio(3, 4);

/// How far past merely-not-touching the car that gives ground is pushed.
///
/// An eighth of a cell - three quarters of a metre.  Enough that a car the
/// cab has hit is seen to go backwards rather than to stop overlapping, and
/// small enough that it is not a teleport.
const BOUNCE: Fx = fixed::ratio(1, 8);

/// What the driver is doing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    /// -1 hard on the brake, +1 flat out.
    pub throttle: Fx,
    /// -1 full left, +1 full right.
    pub steer: Fx,
    /// Handbrake.
    pub handbrake: bool,
}

/// What kind of thing is being driven or shoved around.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CarKind {
    /// The one you are in.
    Taxi,
    /// Everyone else.
    Traffic,
    /// Heavier, and it does not move much when you hit it.
    Bus,
}

impl CarKind {
    /// Mass, in arbitrary units, for the impulse exchange when two cars meet.
    pub fn mass(self) -> i32 {
        match self {
            CarKind::Taxi => 10,
            CarKind::Traffic => 9,
            CarKind::Bus => 40,
        }
    }

    /// What it weighs while it is doing the hitting.
    ///
    /// The cab weighs three times as much in a collision as it does when
    /// something collides with it, and that is not physics, it is the game.
    /// A taxi that loses half its speed to every saloon it clips is a taxi
    /// that cannot cross town, and the fare is on a clock: the thing you are
    /// driving has to plough.  So the impulse is worked out with the cab
    /// heavy, which leaves it most of its momentum and gives the other car
    /// half again as much of a shove as an even exchange would.
    ///
    /// A bus is still forty and still wins, because the point of there being
    /// a bus is that there is something you do not simply drive through.
    pub fn impact_mass(self) -> i32 {
        match self {
            // The cab ploughs.
            CarKind::Taxi => self.mass() * 3,
            // And the bus is the thing it cannot plough through, which only
            // stays true if the bus keeps its lead: at its ordinary forty
            // against a cab weighing thirty, a saloon and a bus went nearly
            // the same distance when hit, and the point of there being a bus
            // is that they do not.
            CarKind::Bus => self.mass() * 3,
            CarKind::Traffic => self.mass(),
        }
    }

    /// Half-length of the body, in world units.
    ///
    /// A cell is about six metres, so these are: a saloon at 9.6 m, the cab
    /// at 12 m, a bus at 24 m.  Long, deliberately - these are American cars
    /// of the period the rest of the thing is dressed as, and at forty
    /// columns a vehicle that is honestly six metres long is three
    /// characters and reads as a crate.
    ///
    /// They were all 3 m at one point, which is a smart car, and it showed:
    /// the collision reach is the sum of two half-lengths and cars were
    /// passing through each other's boots.
    pub fn half_len(self) -> Fx {
        match self {
            // Cut by a quarter twice.  An eight-cell bus - forty-eight
            // metres of it - was longer than most of the buildings it drove
            // past were wide and filled a two-cell street end to end; at
            // nine eighths of a cell it is a thirteen-metre single-decker,
            // which is a bus.
            CarKind::Bus => fixed::ratio(9, 8),
            CarKind::Taxi => fixed::from_int(1),
            CarKind::Traffic => fixed::ratio(4, 5),
        }
    }

    /// The body as a box: length, width and height, in world units.
    ///
    /// A car is drawn as a rectangle standing on the road, and a rectangle
    /// has two horizontal dimensions rather than one.  Which of them you see
    /// depends entirely on where you are looking from - a saloon is two
    /// metres across the back and ten metres down the side - so a single
    /// "width" cannot draw one.  See [`crate::sprite::silhouette`].
    ///
    /// The cab is deliberately the squattest of the three, and has been cut
    /// twice.  The chase camera sits behind it and looks over it, so its
    /// roof is the bottom of what you can see of the road ahead: at seven
    /// fifths of a cell its roofline was two rows above the middle of a
    /// forty-row frame and the horizon was behind it.  Six fifths put the
    /// roof two character rows lower, and nine tenths, which is a quarter
    /// off again, puts it two more.  It is the one dimension here chosen for
    /// the camera rather than for the car.
    ///
    /// The bus lost a quarter of all three at the same time, for the
    /// opposite reason: it was not too tall, it was simply enormous.
    pub fn hull(self) -> (Fx, Fx, Fx) {
        match self {
            CarKind::Bus => (fixed::ratio(9, 4), fixed::from_int(1), fixed::ratio(27, 20)),
            CarKind::Taxi => (fixed::from_int(2), fixed::ratio(6, 5), fixed::ratio(9, 10)),
            CarKind::Traffic => (fixed::ratio(8, 5), fixed::from_int(1), fixed::ratio(13, 10)),
        }
    }

    /// How wide and how tall the billboard for one of these is, in world
    /// units, seen end-on.
    ///
    /// Kept for callers that only want a rough size; anything drawing a car
    /// wants [`CarKind::hull`] and the view angle.
    pub fn body(self) -> (Fx, Fx) {
        let (_, w, h) = self.hull();
        (w, h)
    }
}

/// A vehicle.
#[derive(Clone, Copy, Debug)]
pub struct Car {
    /// Position.
    pub x: Fx,
    /// Position.
    pub y: Fx,
    /// Velocity in world space.  Not along the heading - that is the point.
    pub vx: Fx,
    /// Velocity in world space.
    pub vy: Fx,
    /// Which way the body is pointing.
    pub yaw: Ang,
    /// Angular velocity, in angle units per tick, for the spin after a hit.
    pub spin: i32,
    /// Accumulated dents, 0 (showroom) to 255 (sculpture).  Cosmetic: the
    /// car never stops working, because a car that stops working ends the
    /// run and the run is the point.
    pub damage: u8,
    /// Body colour.
    pub hue: u8,
    /// What it is.
    pub kind: CarKind,
    /// Which step of the wind-up has already been paid out as a shove.
    pub stepped: u32,
    /// Ticks the throttle has been held down, for the speed that winds up.
    ///
    /// The same idea as [`Car::wound`] and for the same reason: the pedal
    /// only goes to one place, so how long it has been there is the only
    /// thing left to say how much you meant it.
    pub pinned: u32,
    /// Ticks the wheel has been held hard over, for the lock that winds on.
    ///
    /// A car park manoeuvre and a corner are the same input here - the wheel
    /// only goes to one place - so the *time* it has been there is the only
    /// thing left to tell them apart, and it is what a driver actually does:
    /// turn in, and keep turning in if it is not enough.
    pub wound: u32,
    /// Ticks of boost left: twice the engine and twice the top speed.
    ///
    /// It is on the car rather than in the controls because it outlives a
    /// frame - you pick a coin up once and spend it over the next few
    /// seconds - and because the autopilot and the player earn it the same
    /// way and neither has to remember it.
    pub boost: u32,
}

impl Car {
    /// A car sitting still at a place, pointing somewhere.
    pub fn new(kind: CarKind, x: Fx, y: Fx, yaw: Ang, hue: u8) -> Car {
        Car { x, y, vx: 0, vy: 0, yaw, spin: 0, damage: 0, hue, kind, pinned: 0, stepped: 0, wound: 0, boost: 0 }
    }

    /// Speed, in units per second.
    #[inline]
    pub fn speed(&self) -> Fx {
        // Octagonal approximation of a hypotenuse: max + 3/8 min.  Two
        // comparisons and a shift, no square root, and within 4% - which is
        // well inside what a speedometer in a game like this is for.
        let (a, b) = (fixed::abs(self.vx), fixed::abs(self.vy));
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        hi + (lo * 3 / 8)
    }

    /// How fast the car is turning, as a signed fraction of the fastest it
    /// can: +1 is a full-lock right-hander, -1 the same to the left.
    ///
    /// This is the derivative every steering controller in the game wants
    /// and none of them had.  A controller with only position terms - how
    /// far off the line, how far off parallel - cannot tell a car that is
    /// heading back to the lane from one that is sitting still beside it,
    /// so it asks for the same lock in both cases and arrives at the line
    /// with the wheel still over.  That is the saw.
    ///
    /// Reported as a fraction rather than in angle units so the gain that
    /// uses it means something: a damping gain of a half takes half the
    /// wheel away from a car that is already turning as hard as it can.
    ///
    /// Clamped before it is scaled, because [`Car::spin`] is also where a
    /// wall impact goes and an impact is not a steering input.
    pub fn turn_rate(&self, hz: i32) -> Fx {
        let per_s = (self.spin as i64 * hz.max(1) as i64).clamp(-32_767, 32_767) as i32;
        fixed::div(fixed::from_int(per_s), fixed::from_int(TURN_RATE))
    }

    /// Speed along the car's own nose, signed: positive going forwards,
    /// negative in reverse.
    ///
    /// [`Car::speed`] is the magnitude and cannot tell the two apart, which
    /// is fine for a speedometer and no use at all to a control that has to
    /// know whether it is slowing the car down or backing it up.
    pub fn forward(&self) -> Fx {
        let (fx, fy) = (trig::cos(self.yaw), trig::sin(self.yaw));
        fixed::mul(self.vx, fx) + fixed::mul(self.vy, fy)
    }

    /// How sideways the car is, 0 (tracking straight) to 1 (fully broadside).
    /// This is what a scoring layer would read to award a drift.
    pub fn slip(&self) -> Fx {
        let s = self.speed();
        if s < fixed::ratio(1, 2) {
            return 0;
        }
        let (fx, fy) = (trig::cos(self.yaw), trig::sin(self.yaw));
        let lat = fixed::abs(fixed::mul(self.vx, -fy) + fixed::mul(self.vy, fx));
        fixed::div(lat, s).min(ONE)
    }

    /// Advance one tick at `hz` ticks per second.
    pub fn step(&mut self, c: &Controls, city: &City, hz: i32) {
        let hz = hz.max(1);
        let inv = fixed::div(ONE, fixed::from_int(hz));

        let (fx, fy) = (trig::cos(self.yaw), trig::sin(self.yaw));
        let (rx, ry) = (-fy, fx);

        // Split the world-space velocity along the body's own axes.
        let mut vf = fixed::mul(self.vx, fx) + fixed::mul(self.vy, fy);
        let mut vl = fixed::mul(self.vx, rx) + fixed::mul(self.vy, ry);

        // Engine, brake and reverse are three different forces, because
        // pressing "back" while rolling forwards must brake rather than
        // engage reverse - otherwise the car is undriveable.
        // Boost, while it lasts and while you are on the throttle.  Lifting
        // off does not spend it, which means a coin taken into a corner is
        // still worth something coming out of it.
        let boosting = self.boost > 0 && c.throttle > 0;
        if boosting {
            self.boost -= 1;
        }

        // The wind-up.  Hold the throttle and the top speed steps up every
        // half second - three times over - and a coin is worth a step past
        // where holding it can get you.  Let go and it falls twice as fast
        // as it climbed, so this is a thing you commit to a straight for and
        // not a thing you have.
        let step = (PIN_STEP * hz as u32 / HZ as u32).max(1);
        if c.throttle > PIN_DOWN {
            self.pinned = self.pinned.saturating_add(1).min(step * PIN_STEPS);
        } else {
            self.pinned = self.pinned.saturating_sub(2);
        }
        // Each step lands as a shove as well as a raised ceiling.  Without
        // it the wind-up is invisible: the car is far below the new cap when
        // it arrives, so nothing about the moment feels different, and a
        // mechanic you cannot feel is a number.
        let step_now = (self.pinned / step).min(PIN_STEPS);
        if step_now > self.stepped {
            self.stepped = step_now;
            vf += fixed::mul(SURGE, fixed::from_int(if vf < 0 { -1 } else { 1 }));
        } else if step_now < self.stepped {
            self.stepped = step_now;
        }
        let steps = fixed::from_int(step_now as i32);
        let wound_up = ONE
            + fixed::mul(
                fixed::div(steps, fixed::from_int(PIN_STEPS as i32)),
                PIN_MAX - ONE,
            );
        let mult = if boosting { PIN_BOOST } else { wound_up };
        // The engine has to keep pulling to wherever the top speed has got
        // to, or the car is quick to a speed it can no longer reach - and
        // with the air drag in, that takes the *square* of the multiple.
        // Drag rises with the square of the speed, so twice the speed is
        // four times the force and three times is nine.  Scaling the force
        // linearly gave a car whose steps bought less and less speed each
        // time, which is a fine curve and is not what was asked for: the
        // steps are meant to be worth three times the speed, not three times
        // the push.
        let accel = fixed::mul(ACCEL, fixed::mul(mult, mult));
        let vmax = fixed::mul(VMAX, mult);
        let ceiling = fixed::mul(ENGINE_CEILING, mult);

        let t = c.throttle;
        let force = if t > 0 {
            // The engine, on its curve: pulling harder the further up the
            // rev range it gets, and tapering to nothing at
            // `ENGINE_CEILING`.  Never negative - past the ceiling the
            // engine simply stops pushing, it does not start braking.
            let left = (ONE - fixed::div(vf.max(0), ceiling)).max(0);
            let bite = LAUNCH_BITE
                + fixed::mul(
                    ONE - LAUNCH_BITE,
                    fixed::div(vf.max(0), LAUNCH).min(ONE),
                );
            fixed::mul(fixed::mul(fixed::mul(t, accel), left), bite)
        } else if vf > fixed::ratio(1, 4) {
            fixed::mul(t, BRAKE)
        } else {
            fixed::mul(t, REVERSE)
        };
        vf += fixed::mul(force, inv);

        // Drag, applied per tick as a fraction of the per-second figure:
        // rolling drag in proportion to the speed, air drag in proportion to
        // its square.
        vf -= fixed::mul(fixed::mul(vf, ONE - DRAG), inv);
        // The squared term is applied as a division rather than a
        // subtraction - `v / (1 + k |v| dt)` rather than `v - k v |v| dt` -
        // which is the same thing to first order and behaves at any tick
        // rate.  Subtracted, the error grows with the square of the speed
        // and with the size of the step: at 300 mph, thirty ticks a second
        // settled four per cent faster than sixty, which is a car that
        // handles differently on a slower machine.
        vf = fixed::div(
            vf,
            ONE + fixed::mul(fixed::mul(DRAG_AIR, fixed::abs(vf)), inv),
        );
        // The clamp is a backstop rather than the top speed.  What actually
        // stops the car is the air: the clamp is here so that a collision or
        // a bad tick cannot put a car into orbit.
        vf = vf.clamp(-fixed::mul(VMAX, fixed::HALF), vmax);

        // Grip.  Interpolated between the parked figure and the flat-out
        // one along a cubic, so the car keeps the nose through the speeds it
        // is driven at and gets loose only near the top - and gets there
        // without a threshold anyone can feel as a switch.
        let frac = fixed::div(fixed::abs(vf), VMAX).min(ONE);
        let mut f = frac;
        for _ in 1..GRIP_CURVE {
            f = fixed::mul(f, frac);
        }
        let keep = if c.handbrake {
            GRIP_HANDBRAKE
        } else {
            fixed::lerp(GRIP_LOW_SPEED, GRIP_HIGH_SPEED, f)
        };
        // `keep` is per tick at `HZ`; at any other rate the bleed is scaled
        // to match, and clamped because a slow enough tick would otherwise
        // ask to remove more sideways motion than there is.
        let per_tick = fixed::div(fixed::from_int(HZ), fixed::from_int(hz));
        let bleed = fixed::mul(ONE - keep, per_tick).min(ONE);
        vl = fixed::mul(vl, ONE - bleed);

        // Recombine *before* the heading changes.  See the module note: this
        // ordering is the drift.
        self.vx = fixed::mul(fx, vf) + fixed::mul(rx, vl);
        self.vy = fixed::mul(fy, vf) + fixed::mul(ry, vl);

        // Steering authority rises with speed, peaks where the lock stops
        // being the limit, and falls away again above it.  The falling half
        // is what keeps a car with grip from turning like a tank at 150
        // km/h: past `TURN_REF` the wheel buys a corner of roughly constant
        // force, so going faster means going a great deal wider, and the way
        // to get the nose round a junction at speed is to slow down or to
        // hang the tail out - which is the boat.
        let sp = fixed::abs(vf);
        let auth = if sp <= TURN_REF {
            fixed::div(sp, TURN_REF)
        } else {
            fixed::div(TURN_REF, sp)
        };
        let dir = if vf < 0 { -ONE } else { ONE };

        // Winding the lock on.  Hold the wheel hard over and the turn keeps
        // tightening for the next second, up to half again as much lock; let
        // it come back and that unwinds twice as fast as it wound.  It is
        // the one input in the game with a memory, and it is what makes a
        // held turn a *decision* rather than a state.
        // In ticks of whatever rate this is, not of `HZ`: the constants are
        // quoted at sixty and the counter runs at `hz`, and a car that winds
        // on twice as fast at sixty frames a second as at thirty is a car
        // that handles differently on a faster machine.
        let after = WIND_AFTER * hz as u32 / HZ as u32;
        let over = (WIND_OVER * hz as u32 / HZ as u32).max(1);
        if fixed::abs(c.steer) > WIND_LOCK {
            self.wound = self.wound.saturating_add(1).min(after + over);
        } else {
            self.wound = self.wound.saturating_sub(2);
        }
        let wound = fixed::div(
            fixed::from_int(self.wound.saturating_sub(after) as i32),
            fixed::from_int(over as i32),
        )
        .clamp(0, ONE);
        let wind = ONE + fixed::mul(wound, WIND_MAX);

        let rate = fixed::mul(fixed::mul(fixed::mul(c.steer, auth), dir), wind);
        let turn = ((rate as i64 * TURN_RATE as i64) >> 16) / hz as i64;
        // A low-pass filter on the yaw rate, so the wheel has weight and a
        // knock from a wall spins the car and then washes out.  It has to be
        // written as a weighted average that *converges on* `turn`:
        // `spin = spin * 7/8 + turn` looks like smoothing but is a filter
        // with a gain of eight, and it makes the car spin like a top at any
        // steering input at all.
        self.spin = (self.spin * 3 + turn as i32) / 4;
        self.yaw = self.yaw.wrapping_add(self.spin as Ang);

        self.integrate(city, inv);
    }

    /// Move, and bounce off anything rigid.
    ///
    /// The two axes are resolved separately.  Clipping a corner therefore
    /// scrubs speed off one axis and lets the other carry on, which is what
    /// makes glancing a building feel like glancing a building rather than
    /// like hitting a full stop.
    fn integrate(&mut self, city: &City, inv: Fx) {
        let nose = self.kind.half_len();
        let (fx, fy) = (trig::cos(self.yaw), trig::sin(self.yaw));

        let dx = fixed::mul(self.vx, inv);
        let dy = fixed::mul(self.vy, inv);

        // Which end is going first.  Forwards it is the nose; in reverse it
        // is the tail, and probing the nose while reversing is the same as
        // not probing at all - the car backs its whole rear half into a wall
        // before its centre notices, and then stops dead from a standing
        // overlap.  That is what "the brakes come on when you back into
        // something" is: not braking, arriving late.
        let vf = fixed::mul(self.vx, fx) + fixed::mul(self.vy, fy);
        let lead = if vf < 0 { -nose } else { nose };
        let probe = |x: Fx, y: Fx| -> bool {
            // Probe at the leading end as well as the centre, so a car does
            // not bury half its length in a wall before anything notices.
            city.open(fixed::floor(x), fixed::floor(y))
                && city.open(
                    fixed::floor(x + fixed::mul(fx, lead)),
                    fixed::floor(y + fixed::mul(fy, lead)),
                )
        };

        if probe(self.x + dx, self.y) {
            self.x += dx;
        } else {
            self.hit(true, inv);
        }
        if probe(self.x, self.y + dy) {
            self.y += dy;
        } else {
            self.hit(false, inv);
        }
    }

    /// Take a wall impact on one axis.
    ///
    /// The city is a grid, so the wall that stopped this axis runs along the
    /// other one and there is no normal to work out: blocked going east, the
    /// wall runs north-south.  That is what makes the alignment below
    /// cheap enough to do every tick of contact.
    fn hit(&mut self, on_x: bool, inv: Fx) {
        let v = if on_x { self.vx } else { self.vy };
        let sev = fixed::floor(fixed::abs(v)) * WALL_DAMAGE;
        self.damage = self.damage.saturating_add(sev.clamp(0, 40) as u8);
        if on_x {
            self.vx = fixed::mul(self.vx, WALL_BOUNCE);
        } else {
            self.vy = fixed::mul(self.vy, WALL_BOUNCE);
        }
        // And point the car along the wall rather than into it.  There are
        // two ways to run along any wall and the nearer of the two is the
        // one taken, which is what makes this a car being turned *away* from
        // what it hit rather than a car being turned round: whichever end
        // was leading stays leading.
        let along = if on_x { trig::QUARTER as Ang } else { 0 };
        let mut off = along.wrapping_sub(self.yaw) as i16 as i32;
        // Half a turn from the nearer heading is the other one, and it is
        // never further than a quarter turn away.
        if off.abs() > trig::QUARTER as i32 {
            off -= off.signum() * trig::HALF as i32;
        }
        let most = fixed::floor(fixed::mul(fixed::from_int(WALL_ALIGN), inv));
        self.yaw = self.yaw.wrapping_add(off.clamp(-most, most) as Ang);
    }

    /// Take an impulse - from another car, or from something you flattened.
    pub fn shove(&mut self, ix: Fx, iy: Fx, spin: i32) {
        self.shove_as(ix, iy, spin, self.kind.mass());
    }

    /// Take an impulse as though this car weighed `mass`.
    ///
    /// The collision solver works the impulse out from
    /// [`CarKind::impact_mass`] and has to spend it against the same number,
    /// or the two halves disagree and the pair gains energy - which looks
    /// like two cars that touch and fire apart.
    pub fn shove_as(&mut self, ix: Fx, iy: Fx, spin: i32, mass: i32) {
        let m = fixed::from_int(mass.max(1));
        self.vx += fixed::div(ix, m);
        self.vy += fixed::div(iy, m);
        self.spin += spin;
    }
}

/// Resolve a collision between two cars.
///
/// Momentum is exchanged along the line between their centres, scaled by
/// mass, which is the whole of it.  A taxi at speed sends a parked saloon
/// spinning; a bus barely notices either.
pub fn collide(a: &mut Car, b: &mut Car, city: &City) -> Option<Fx> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let reach = fixed::mul(a.kind.half_len() + b.kind.half_len(), CONTACT);
    let (nx, ny, dist) = normalise(dx, dy);
    if dist > reach {
        return None;
    }

    // Closing speed along the contact normal.  The normal has to be a *unit*
    // vector before this dot product means anything: taking it against the
    // raw separation instead scales the whole impulse by how far apart the
    // cars happen to be, which makes a hard contact - where they are closest
    // - the gentlest one, and lets the same pair collide over and over
    // because neither ever gets pushed hard enough to separate.
    let (rvx, rvy) = (b.vx - a.vx, b.vy - a.vy);
    let closing = fixed::mul(rvx, nx) + fixed::mul(rvy, ny);
    if closing > 0 {
        return None; // already separating
    }

    let (ka, kb) = (a.kind.impact_mass(), b.kind.impact_mass());
    let ma = fixed::from_int(ka);
    let mb = fixed::from_int(kb);

    // The textbook impulse for two bodies:
    //
    //     j = -(1 + e) * closing / (1/ma + 1/mb)
    //
    // and then each car's velocity changes by j/m.  Both halves of that
    // matter.  Splitting the impulse by mass *and* dividing by mass inside
    // `shove` applies it twice, which makes every collision a nudge - a
    // taxi at 40 mph moved a parked car about a foot.  `shove` divides, so
    // this must not.
    //
    // The restitution is 0.7, which is far too bouncy for two tonnes of
    // steel and exactly right for what this is: cars should go over like
    // skittles.
    let reduced = fixed::div(fixed::mul(ma, mb), ma + mb);
    let j = fixed::mul(fixed::mul(closing, fixed::ratio(-17, 10)), reduced);
    let ix = fixed::mul(j, nx);
    let iy = fixed::mul(j, ny);

    a.shove_as(-ix, -iy, -260, ka);
    b.shove_as(ix, iy, 260, kb);
    let sev = fixed::abs(closing);
    a.damage = a.damage.saturating_add(fixed::floor(sev).clamp(0, 12) as u8);
    b.damage = b.damage.saturating_add(fixed::floor(sev).clamp(0, 12) as u8);

    // Push them apart so they do not stick together and re-collide forever.
    //
    // The push has to respect walls.  It is a *teleport* - it moves a car
    // without going through `integrate` - so on a narrow street two cars
    // meeting near a kerb will happily be shoved through the building
    // behind it, and the car then spends the rest of the run inside the
    // lobby.  Each half is applied only if it lands somewhere you could
    // drive; if it does not, the other car takes the whole correction, and
    // if neither can move they simply stay touching for another tick, which
    // is harmless.
    // Who gives ground is not an even split when one of them is the cab.
    // It ploughs, so the *other* car is bounced back the whole way and a
    // little further - `BOUNCE`, so it visibly recoils rather than merely
    // stopping touching - and the cab only gives ground if the other car
    // has nowhere to go, which on a narrow street is a wall behind it.
    let gap = (reach - dist).max(0);
    let (fx, fy) = (fixed::mul(nx, gap + BOUNCE), fixed::mul(ny, gap + BOUNCE));
    let half = (fixed::mul(nx, gap / 2 + 1), fixed::mul(ny, gap / 2 + 1));
    let ploughs = |k: CarKind| k == CarKind::Taxi;
    if ploughs(a.kind) != ploughs(b.kind) {
        // The one that does not plough moves first, and the one that does
        // moves only if that failed.
        let moved = if ploughs(a.kind) {
            nudge(b, fx, fy, city)
        } else {
            nudge(a, -fx, -fy, city)
        };
        if !moved {
            let _ = if ploughs(a.kind) {
                nudge(a, -fx, -fy, city)
            } else {
                nudge(b, fx, fy, city)
            };
        }
    } else {
        // Two of a kind: half each, and whichever half will not fit is
        // taken by the other one.
        let a_ok = nudge(a, -half.0, -half.1, city);
        let b_ok = nudge(b, half.0, half.1, city);
        if !a_ok {
            nudge(b, half.0, half.1, city);
        }
        if !b_ok {
            nudge(a, -half.0, -half.1, city);
        }
    }
    Some(sev)
}

/// Move a car by a delta if that leaves it somewhere it could have driven.
fn nudge(c: &mut Car, dx: Fx, dy: Fx, city: &City) -> bool {
    let (nx, ny) = (c.x + dx, c.y + dy);
    if city.open(fixed::floor(nx), fixed::floor(ny)) {
        c.x = nx;
        c.y = ny;
        true
    } else {
        false
    }
}

/// A unit vector in the direction of `(x, y)`, and its length.
///
/// The length is the same octagonal approximation [`Car::speed`] uses - max
/// plus three eighths of min - so that "how far apart are they" and "how
/// fast is it going" are measured with the same ruler, and neither needs a
/// square root.
fn normalise(x: Fx, y: Fx) -> (Fx, Fx, Fx) {
    let (a, b) = (fixed::abs(x), fixed::abs(y));
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    let len = hi + (lo * 3 / 8);
    if len == 0 {
        (ONE, 0, 0)
    } else {
        (fixed::div(x, len), fixed::div(y, len), len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::City;

    /// A taxi in the middle of an avenue, pointing along it.
    ///
    /// Handling tests need somewhere to actually handle.  Dropping the car
    /// on whatever road cell happens to be nearest the middle of the map
    /// means half of them start it eight feet from a wall, where every run
    /// ends in a crash before anything can be measured.
    fn on_the_road() -> (City, Car) {
        let city = City::generate(21);
        // Whichever column the plan happened to lay a wide road down.  It
        // used to be hard-coded as "x % 14 < 3", which stopped being true
        // the moment the street system became something generated rather
        // than something computed.
        let col = (0..crate::world::SIZE as i32)
            .find(|x| {
                let r = city.plan.cols.at(*x);
                r.class >= crate::world::RoadClass::Avenue && r.across == 1
            })
            .expect("the plan laid no avenue at all");
        let x = fixed::from_int(col) + fixed::HALF;
        let y = fixed::from_int(6) + fixed::HALF;
        assert!(city.open(col, 6), "the test avenue is not clear");
        (city, Car::new(CarKind::Taxi, x, y, trig::QUARTER, 7))
    }

    /// A city whose origin corner is open road, for collision tests that
    /// place cars at the origin and care about nothing else.
    fn open_ground() -> City {
        let mut c = City::generate(21);
        for y in 0..64i32 {
            for x in 0..64i32 {
                c.elev.build(x, y, 0);
                c.cells[y as usize * crate::world::SIZE + x as usize].kind =
                    crate::world::Kind::Road;
            }
        }
        c
    }

    fn flat_out(car: &mut Car, city: &City, ticks: u32) {
        let c = Controls { throttle: ONE, ..Default::default() };
        for _ in 0..ticks {
            car.step(&c, city, HZ);
        }
    }

    /// Take a corner at a held speed on open ground, and report the worst
    /// slip and how tight the corner was.
    ///
    /// Held, because the interesting question is what the car does at *a*
    /// speed: measured with the throttle simply pinned, every corner is
    /// taken at the top speed the car reached on the way round and the
    /// speeds cannot be told apart.
    ///
    /// The radius comes from the chord: a quarter turn of radius r joins
    /// its ends with a chord of r * sqrt(2).
    fn corner(speed: Fx, handbrake: bool) -> (Fx, f32) {
        let city = open_ground();
        let mut car = Car::new(CarKind::Taxi, fixed::HALF, fixed::HALF, 0, 7);
        let go = Controls { throttle: ONE, ..Default::default() };
        for _ in 0..600 {
            if car.speed() >= speed {
                break;
            }
            car.step(&go, &city, HZ);
        }
        let (x0, y0, yaw0) = (car.x, car.y, car.yaw);
        let mut peak = 0;
        for _ in 0..900 {
            let throttle = if car.speed() < speed { ONE } else { 0 };
            car.step(&Controls { throttle, steer: ONE, handbrake }, &city, HZ);
            peak = peak.max(car.slip());
            if (car.yaw.wrapping_sub(yaw0) as i16 as i32).abs() >= trig::QUARTER as i32 {
                break;
            }
        }
        let (dx, dy) = (fixed::to_f32(car.x - x0), fixed::to_f32(car.y - y0));
        (peak, (dx * dx + dy * dy).sqrt() / std::f32::consts::SQRT_2)
    }

    #[test]
    fn the_throttle_makes_it_go() {
        let (city, mut car) = on_the_road();
        flat_out(&mut car, &city, 30);
        assert!(car.speed() > ONE, "half a second flat out and it is not moving");
    }

    /// Speed is something the car builds, and the build tapers.
    ///
    /// Measured once the wind-up has finished, because there are two builds
    /// on top of each other and this is about the other one: for the first
    /// three seconds the *cap* is still rising - see
    /// `holding_the_throttle_winds_the_speed_up` - so the car re-accelerates
    /// every second and nothing tapers at all.  Past that the cap is
    /// fixed and what is left is the engine's own curve, which is the thing
    /// that makes the last few miles an hour cost more than the first.
    #[test]
    fn the_engine_has_a_curve_rather_than_a_switch() {
        let city = open_ground();
        let mut car = Car::new(CarKind::Taxi, fixed::HALF, fixed::HALF, 0, 7);
        // Get the wind-up out of the way.
        flat_out(&mut car, &city, PIN_STEP * PIN_STEPS);
        let mut gains = Vec::new();
        let mut last = car.speed();
        for _ in 0..6 {
            flat_out(&mut car, &city, HZ as u32 / 4);
            gains.push(car.speed() - last);
            last = car.speed();
        }
        assert!(
            gains[0] > gains[5] * 3,
            "no taper: {:?}",
            gains.iter().map(|g| fixed::to_f32(*g)).collect::<Vec<_>>()
        );
    }

    /// It does not exceed the top speed it has wound up to.
    ///
    /// Which is three times the base one after a second and a half of held
    /// throttle - see `PIN_STEPS` - and four times with a coin in it.  The
    /// number that must not run away is the multiple, not the base.
    /// It settles at a speed rather than climbing until something stops it.
    ///
    /// The top speed is *found*, not set: the engine's taper and the air
    /// drag balance somewhere, and where they balance is the top speed.  The
    /// clamp is a backstop for a bad tick and should never be what is
    /// holding the car back.  Measured on open ground: about 150 mph
    /// unwound, 306 with the throttle held down, 405 with a coin.
    #[test]
    fn it_settles_at_a_top_speed_it_finds_for_itself() {
        let city = open_ground();
        let hold = |boost: u32| {
            let mut car = Car::new(CarKind::Taxi, fixed::HALF, fixed::HALF, 0, 7);
            car.boost = boost;
            flat_out(&mut car, &city, 2000);
            let settled = car.speed();
            flat_out(&mut car, &city, 200);
            assert!(
                fixed::abs(car.speed() - settled) < fixed::ratio(1, 10),
                "it never settled: {} then {}",
                fixed::to_f32(settled),
                fixed::to_f32(car.speed())
            );
            settled
        };
        let held = hold(0);
        let coined = hold(100_000);
        assert!(held > VMAX, "holding the throttle bought nothing: {}", fixed::to_f32(held));
        assert!(coined > held, "the coin bought nothing: {} against {}", fixed::to_f32(coined), fixed::to_f32(held));
        assert!(
            coined < fixed::mul(VMAX, PIN_BOOST) + ONE,
            "it went past its own backstop: {}",
            fixed::to_f32(coined)
        );
    }

    /// The top speed steps up while the throttle is held, and falls back
    /// when it is let go.
    #[test]
    fn holding_the_throttle_winds_the_speed_up() {
        let city = open_ground();
        let mut car = Car::new(CarKind::Taxi, fixed::HALF, fixed::HALF, 0, 7);
        let mut at = Vec::new();
        for _ in 0..4 {
            flat_out(&mut car, &city, PIN_STEP);
            at.push(car.speed());
        }
        // Every half second is faster than the one before it, and the last
        // is past the base top speed by a good margin.
        for w in at.windows(2) {
            assert!(w[1] > w[0], "it stopped gaining: {:?}", at.iter().map(|s| fixed::to_f32(*s)).collect::<Vec<_>>());
        }
        assert!(
            at[3] > fixed::mul(VMAX, fixed::ratio(3, 2)),
            "a full wind-up only reached {}",
            fixed::to_f32(at[3])
        );

        // Let go and the wind-up unwinds, so it is not a thing you keep.
        for _ in 0..HZ * 2 {
            car.step(&Controls::default(), &city, HZ);
        }
        assert_eq!(car.pinned, 0, "the throttle stayed wound up after being let go");
    }

    #[test]
    fn the_brake_stops_it_before_reverse_engages() {
        let (city, mut car) = on_the_road();
        flat_out(&mut car, &city, 60);
        let fast = car.speed();
        let c = Controls { throttle: -ONE, ..Default::default() };
        for _ in 0..10 {
            car.step(&c, &city, HZ);
        }
        assert!(car.speed() < fast, "the brake did not slow it");
        let (fx, fy) = (trig::cos(car.yaw), trig::sin(car.yaw));
        assert!(
            fixed::mul(car.vx, fx) + fixed::mul(car.vy, fy) > -ONE,
            "it went straight into reverse from speed"
        );
    }

    /// The corner table in the documentation, printed so that it can be
    /// kept true, and asserted so that it cannot quietly stop being.
    #[test]
    fn the_corner_table() {
        for (kmh, speed) in [
            (28, ONE),
            (65, fixed::ratio(3, 1)),
            (100, fixed::ratio(46, 10)),
            (150, VMAX),
        ] {
            let (slip, r) = corner(speed, false);
            let (hslip, hr) = corner(speed, true);
            println!(
                "{kmh:3} km/h  radius {:5.1} m  slip {:.2}   handbrake: radius {:5.1} m slip {:.2}",
                r * 6.0,
                fixed::to_f32(slip),
                hr * 6.0,
                fixed::to_f32(hslip)
            );
            assert!(slip < fixed::ratio(1, 5), "{kmh} km/h drifts on its own");
            assert!(hslip > fixed::ratio(3, 5), "{kmh} km/h will not slide even on the handbrake");
        }
    }

    /// Flat out and off the brakes, it still goes where it points.
    ///
    /// This used to assert the opposite - that a corner at the top of the
    /// range slid - because it did, at 0.29 of slip.  A car that drifts
    /// without being asked to is a car you cannot place, and placing it is
    /// the whole of driving between two rows of buildings six metres apart.
    /// The slide is now something you ask for, with the handbrake, and this
    /// asserts both halves of that.
    #[test]
    fn it_tracks_its_nose_even_flat_out() {
        let (loose, _) = corner(VMAX, false);
        let (slid, _) = corner(VMAX, true);
        assert!(loose < fixed::ratio(1, 5), "it drifted on its own: slip {}", fixed::to_f32(loose));
        assert!(
            slid > loose * 4,
            "the handbrake did nothing: {} against {}",
            fixed::to_f32(slid),
            fixed::to_f32(loose)
        );
    }

    /// The point of the whole exercise: at the speeds the car is actually
    /// driven at, it goes where it points.
    ///
    /// Measured at about 0.10 at 65 km/h and 0.29 flat out, against 0.92 and
    /// 0.93 for the version this replaced - which drifted the same amount at
    /// every speed and so was a boat at all of them.
    #[test]
    fn it_tracks_its_nose_at_town_speed() {
        let (peak, _) = corner(fixed::from_int(3), false);
        assert!(
            peak < fixed::ratio(1, 5),
            "it swims through an ordinary corner: slip {}",
            fixed::to_f32(peak)
        );
        let (fast, _) = corner(VMAX, false);
        assert!(fast > peak, "going faster made no difference to how much it slides");
    }

    /// Going faster means going wider, which is the difference between a car
    /// and a tank.  Measured: 18 m at 65 km/h, 41 m at 100, 87 m flat out.
    #[test]
    fn the_faster_it_goes_the_wider_it_turns() {
        let (_, town) = corner(fixed::from_int(3), false);
        let (_, quick) = corner(fixed::ratio(9, 2), false);
        let (_, flat) = corner(VMAX, false);
        assert!(quick > town * 1.5, "no wider at 100 km/h: {town} then {quick}");
        assert!(flat > quick * 1.5, "no wider flat out: {quick} then {flat}");
    }

    /// And the handbrake is how you get sideways on purpose - at any speed,
    /// not only at the top of the range where the car is loose anyway.
    ///
    /// Measured: 0.90 against 0.09 at 100 km/h.  It was 1.00 against 0.92
    /// before, which is to say it did nothing you could see.
    /// Holding a turn tightens it.
    ///
    /// The first second is the corner you asked for; past that the wheel
    /// keeps winding on.  Measured as the yaw in the second second of a held
    /// turn against the yaw in the first, at a speed the car actually
    /// corners at.
    #[test]
    fn a_held_turn_tightens() {
        let city = open_ground();
        let mut car = Car::new(CarKind::Taxi, fixed::from_int(8), fixed::from_int(8), 0, 7);
        car.vx = fixed::from_int(3);
        let hold = Controls { throttle: 0, steer: ONE, ..Default::default() };
        let turn = |car: &mut Car, ticks: i32| {
            let was = car.yaw;
            for _ in 0..ticks {
                // Hold the speed, so this is about the wheel and not about
                // the throttle.
                let want = fixed::from_int(3);
                let vf = fixed::mul(car.vx, trig::cos(car.yaw)) + fixed::mul(car.vy, trig::sin(car.yaw));
                let c = Controls { throttle: if vf < want { ONE } else { 0 }, ..hold };
                car.step(&c, &city, HZ);
            }
            (car.yaw.wrapping_sub(was) as i16 as i32).abs()
        };
        let first = turn(&mut car, HZ);
        let second = turn(&mut car, HZ);
        assert!(
            second > first * 11 / 10,
            "the wheel did not wind on: {first} units in the first second, {second} in the next"
        );

        // And it unwinds when you straighten up.
        let straight = Controls { throttle: 0, steer: 0, ..Default::default() };
        for _ in 0..HZ {
            car.step(&straight, &city, HZ);
        }
        assert_eq!(car.wound, 0, "the lock stayed wound on after the wheel came back");
    }

    #[test]
    fn the_handbrake_makes_it_slide_more() {
        for v in [fixed::from_int(3), fixed::ratio(9, 2), VMAX] {
            let (hb, _) = corner(v, true);
            let (no, _) = corner(v, false);
            assert!(
                hb > no + fixed::ratio(1, 4),
                "the handbrake did nothing at {}: {} against {}",
                fixed::to_f32(v),
                fixed::to_f32(hb),
                fixed::to_f32(no)
            );
        }
    }

    /// The same manoeuvre at half the tick rate puts the car in the same
    /// place.
    ///
    /// Grip and drag are quoted per tick and per second respectively, and
    /// both have to be scaled to the rate they are actually spent at.  The
    /// version this replaced scaled neither: at 30 Hz - which is what the
    /// autopilot and the Plus/4 timings run at - the car kept twice as much
    /// of every slide, so the machine that could least afford a loose car
    /// got the loosest one.
    #[test]
    fn the_tick_rate_does_not_change_how_it_drives() {
        let drive = |hz: i32| {
            let city = open_ground();
            let mut car = Car::new(CarKind::Taxi, fixed::HALF, fixed::HALF, 0, 7);
            for i in 0..(4 * hz) {
                let steer = if i > hz { ONE } else { 0 };
                car.step(&Controls { throttle: ONE, steer, handbrake: false }, &city, hz);
            }
            (fixed::to_f32(car.x), fixed::to_f32(car.y), fixed::to_f32(car.speed()))
        };
        let (x60, y60, s60) = drive(60);
        let (x30, y30, s30) = drive(30);
        // Within a car's length over four seconds of full-lock cornering.
        assert!(
            (x60 - x30).abs() < 1.0 && (y60 - y30).abs() < 1.0,
            "30 Hz went somewhere else: {x60},{y60} against {x30},{y30}"
        );
        assert!((s60 - s30).abs() < 0.5, "30 Hz ended at a different speed: {s60} against {s30}");
    }

    /// The wheel works like a wheel: right is right going forward, and the
    /// other way round in reverse.
    ///
    /// A car reversing with the wheel turned right pushes its *back* to the
    /// right, so the nose swings left and the car's heading turns left.
    /// Anybody who has reversed a car knows this in their hands and nobody
    /// can explain it at the keyboard, which is why it is worth a test
    /// rather than a comment.
    #[test]
    fn the_wheel_works_backwards_in_reverse() {
        let city = open_ground();
        let right = Controls { throttle: 0, steer: ONE, ..Default::default() };

        let mut fwd = Car::new(CarKind::Taxi, fixed::from_int(8), fixed::from_int(8), 0, 7);
        fwd.vx = fixed::from_int(3);
        let was = fwd.yaw;
        for _ in 0..HZ / 2 {
            fwd.step(&right, &city, HZ);
        }
        let turned = fwd.yaw.wrapping_sub(was) as i16 as i32;
        assert!(turned > 0, "the wheel to the right did not turn it right going forwards");

        let mut back = Car::new(CarKind::Taxi, fixed::from_int(8), fixed::from_int(8), 0, 7);
        back.vx = -fixed::from_int(3);
        let was = back.yaw;
        for _ in 0..HZ / 2 {
            back.step(&right, &city, HZ);
        }
        let turned = back.yaw.wrapping_sub(was) as i16 as i32;
        assert!(turned < 0, "the wheel to the right turned it right in reverse as well");
    }

    #[test]
    fn a_parked_car_cannot_be_steered() {
        let (city, mut car) = on_the_road();
        let before = car.yaw;
        let c = Controls { steer: ONE, ..Default::default() };
        for _ in 0..60 {
            car.step(&c, &city, HZ);
        }
        assert_eq!(car.yaw, before, "it turned on the spot like a tank");
    }

    /// Backing into a wall stops at the wall, not half a car past it.
    ///
    /// The probe used to look at the nose whatever direction the car was
    /// going, which in reverse is the end that is moving *away* from
    /// whatever it is about to hit.  The car backed its whole rear half into
    /// the building before its centre reached the wall and then stopped from
    /// a standing overlap, which reads as the brakes coming on by
    /// themselves.
    #[test]
    fn backing_into_a_wall_stops_at_the_wall() {
        // Open ground with one wall in it, so the geometry is the point
        // rather than whatever the generator happened to lay down.
        let mut city = open_ground();
        let (x, y) = (20i32, 20i32);
        city.elev.build(x - 1, y, 12);
        assert!(!city.open(x - 1, y), "the test wall is not a wall");
        // Facing east, two cells clear of it, so reversing takes it west
        // into that wall from a standing start with room to get going.
        let mut car = Car::new(
            CarKind::Taxi,
            fixed::from_int(x + 2) + fixed::HALF,
            fixed::from_int(y) + fixed::HALF,
            0,
            7,
        );
        let start = car.x;
        for _ in 0..HZ * 2 {
            car.step(&Controls { throttle: -ONE, ..Default::default() }, &city, HZ);
            // The invariant, every tick and whatever the car has rotated to:
            // the back bumper is not inside a building.
            let (fx, fy) = (trig::cos(car.yaw), trig::sin(car.yaw));
            let half = car.kind.half_len();
            let (bx, by) = (car.x - fixed::mul(fx, half), car.y - fixed::mul(fy, half));
            assert!(
                city.open(fixed::floor(bx), fixed::floor(by)),
                "the back bumper is inside the building at {},{}",
                fixed::to_f32(bx),
                fixed::to_f32(by)
            );
        }
        // ...and it did reverse rather than being stuck from the first tick.
        assert!(car.x < start, "it never moved: {}", fixed::to_f32(car.x));
    }

    #[test]
    fn it_never_ends_up_inside_a_building() {
        let (city, mut car) = on_the_road();
        let mut steer = ONE;
        for i in 0..6000 {
            if i % 37 == 0 {
                steer = -steer;
            }
            car.step(&Controls { throttle: ONE, steer, handbrake: i % 200 < 20 }, &city, HZ);
            assert!(
                city.open(fixed::floor(car.x), fixed::floor(car.y)),
                "drove into a building at tick {i}: {},{}",
                fixed::to_f32(car.x),
                fixed::to_f32(car.y)
            );
        }
    }

    /// A wall points the car along itself instead of throwing it back.
    ///
    /// The alley case, which is the one that matters: two buildings a couple
    /// of cells apart and a car that arrives at an angle.  What used to
    /// happen is the rebound - a third of the impact speed straight back
    /// across the alley, into the far wall, and out of that one into the
    /// first, until the cab was pointing the way it came with the fare still
    /// running.
    #[test]
    fn a_wall_turns_the_car_to_run_along_it() {
        // An alley: a solid column of building either side of a clear one,
        // running north.
        let mut city = open_ground();
        let lane = 20i32;
        for y in 0..40 {
            city.elev.build(lane - 1, y, 12);
            city.elev.build(lane + 1, y, 12);
        }
        // In the middle of it, doing 20 degrees off the alley - pointing at
        // the right-hand wall and travelling that way too.
        let head = trig::QUARTER.wrapping_sub(trig::from_degrees(20.0));
        let mut car = Car::new(
            CarKind::Taxi,
            fixed::from_int(lane) + fixed::HALF,
            fixed::from_int(4) + fixed::HALF,
            head,
            7,
        );
        let speed = fixed::from_int(3);
        car.vx = fixed::mul(trig::cos(head), speed);
        car.vy = fixed::mul(trig::sin(head), speed);
        let hands = Controls { throttle: ONE, ..Default::default() };
        let mut worst_back = 0;
        for _ in 0..HZ * 2 {
            car.step(&hands, &city, HZ);
            // It never gets sent back down the alley.
            worst_back = worst_back.min(car.vy);
        }
        assert!(worst_back >= 0, "the wall sent it back down the alley");
        // And it comes out of the alley pointing along it, within a few
        // degrees, rather than crabbed across it.
        let off = (car.yaw.wrapping_sub(trig::QUARTER) as i16 as i32).abs();
        assert!(
            off < trig::from_degrees(10.0) as i32,
            "it is still {} degrees off the alley",
            off as f64 * 360.0 / 65536.0
        );
        // ...having actually gone somewhere, rather than stopping dead
        // against the wall it clipped.
        assert!(car.y > fixed::from_int(12), "it never got up the alley: {}", fixed::to_f32(car.y));
    }

    #[test]
    fn hitting_a_wall_costs_paint_and_speed() {
        let (city, mut car) = on_the_road();
        let mut dented = false;
        for deg in (0..360).step_by(15) {
            car.yaw = trig::from_degrees(deg as f64);
            car.vx = 0;
            car.vy = 0;
            car.damage = 0;
            flat_out(&mut car, &city, 120);
            if car.damage > 0 {
                dented = true;
                break;
            }
        }
        assert!(dented, "drove into walls in every direction without a scratch");
    }

    #[test]
    fn a_taxi_scatters_a_parked_car_and_barely_moves_a_bus() {
        let hit = |kind: CarKind| {
            let mut a = Car::new(CarKind::Taxi, 0, 0, 0, 7);
            a.vx = fixed::from_int(6);
            let mut b = Car::new(kind, fixed::ratio(1, 3), 0, 0, 2);
            collide(&mut a, &mut b, &open_ground()).expect("no contact");
            b.speed()
        };
        let saloon = hit(CarKind::Traffic);
        let bus = hit(CarKind::Bus);
        assert!(saloon > ONE, "the parked car hardly moved: {}", fixed::to_f32(saloon));
        assert!(bus < saloon / 2, "the bus went as far as the saloon");
    }

    #[test]
    fn cars_that_are_separating_are_left_alone() {
        let mut a = Car::new(CarKind::Taxi, 0, 0, 0, 7);
        a.vx = -fixed::from_int(4);
        let mut b = Car::new(CarKind::Traffic, fixed::ratio(1, 3), 0, 0, 2);
        b.vx = fixed::from_int(4);
        assert!(collide(&mut a, &mut b, &open_ground()).is_none());
    }

    #[test]
    fn cars_far_apart_do_not_collide() {
        let mut a = Car::new(CarKind::Taxi, 0, 0, 0, 7);
        let mut b = Car::new(CarKind::Traffic, fixed::from_int(3), 0, 0, 2);
        assert!(collide(&mut a, &mut b, &open_ground()).is_none());
    }

    #[test]
    fn a_collision_separates_them_so_it_does_not_repeat_forever() {
        let mut a = Car::new(CarKind::Taxi, 0, 0, 0, 7);
        a.vx = fixed::from_int(5);
        let mut b = Car::new(CarKind::Traffic, fixed::ratio(1, 8), 0, 0, 2);
        let ground = open_ground();
        assert!(collide(&mut a, &mut b, &ground).is_some());
        // Immediately after, they must be either apart or separating.
        assert!(collide(&mut a, &mut b, &ground).is_none(), "the same collision fired twice");
    }
}






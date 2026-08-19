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
//!    car and costs it speed and paint.  A lamp post does not.
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
/// It is not the force at every speed - see [`ENGINE_CEILING`].  A constant
/// force is what this was, and it is why the car had no acceleration to
/// speak of: at twenty-six units per second per second against a top speed
/// of seven, the car was at the clamp in a quarter of a second, from any
/// speed, in any gear it does not have.  There was nothing to hold the
/// throttle *down* for, which is most of what driving one of these is.
const ACCEL: Fx = fixed::ratio(10, 1);
/// The speed at which the engine has nothing left to give, in units per
/// second.
///
/// Force falls off linearly from [`ACCEL`] at a standstill to nothing here,
/// which is the shape of a torque curve through a gearbox and, more to the
/// point, the shape that makes speed something the car *builds*.  The
/// approach is exponential, so what this really sets is a time constant:
/// half of top speed in about half a second, top speed itself in about one
/// and three quarters.
///
/// A quarter above [`VMAX`] rather than equal to it, because the engine has
/// to out-pull the drag at the top of the range or the car never reaches
/// the speed it is supposed to have.  With the ceiling at the top speed,
/// force and drag balance a little under it and the clamp never binds.
const ENGINE_CEILING: Fx = fixed::ratio(35, 4);
/// Braking is stronger than the engine, as it is on every car.
const BRAKE: Fx = fixed::ratio(44, 1);
/// Reverse is weak, as it is on every car.
const REVERSE: Fx = fixed::ratio(9, 1);
/// Rolling and air drag, as a per-second fraction of speed retained.
const DRAG: Fx = fixed::ratio(88, 100);
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
/// Parked, four fifths of the slide survives a tick and a thousandth of it
/// survives a second: the car goes where it points.
const GRIP_LOW_SPEED: Fx = fixed::ratio(80, 100);
/// Lateral grip at top speed.  Half the slide is still there three quarters
/// of a second later, which is the boat.
const GRIP_HIGH_SPEED: Fx = fixed::ratio(985, 1000);
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
/// How much speed a wall takes.
const WALL_BOUNCE: Fx = fixed::ratio(-35, 100);
/// How much of the car's body a wall claims per impact, per unit of speed.
const WALL_DAMAGE: i32 = 9;

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
            CarKind::Bus => fixed::from_int(2),
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
    pub fn hull(self) -> (Fx, Fx, Fx) {
        match self {
            CarKind::Bus => (fixed::from_int(4), fixed::ratio(9, 5), fixed::ratio(12, 5)),
            CarKind::Taxi => (fixed::from_int(2), fixed::ratio(6, 5), fixed::ratio(7, 5)),
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
}

impl Car {
    /// A car sitting still at a place, pointing somewhere.
    pub fn new(kind: CarKind, x: Fx, y: Fx, yaw: Ang, hue: u8) -> Car {
        Car { x, y, vx: 0, vy: 0, yaw, spin: 0, damage: 0, hue, kind }
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
        let t = c.throttle;
        let force = if t > 0 {
            // The engine, on its curve: everything it has from a standstill,
            // tapering to nothing at `ENGINE_CEILING`.  Never negative -
            // past the ceiling the engine simply stops pushing, it does not
            // start braking.
            let left = (ONE - fixed::div(vf.max(0), ENGINE_CEILING)).max(0);
            fixed::mul(fixed::mul(t, ACCEL), left)
        } else if vf > fixed::ratio(1, 4) {
            fixed::mul(t, BRAKE)
        } else {
            fixed::mul(t, REVERSE)
        };
        vf += fixed::mul(force, inv);

        // Drag, applied per tick as a fraction of the per-second figure.
        vf -= fixed::mul(fixed::mul(vf, ONE - DRAG), inv);
        vf = vf.clamp(-fixed::mul(VMAX, fixed::HALF), VMAX);

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
        let rate = fixed::mul(fixed::mul(c.steer, auth), dir);
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

        let probe = |x: Fx, y: Fx| -> bool {
            // Probe at the nose as well as the centre, so a car does not
            // bury half its length in a wall before anything notices.
            city.open(fixed::floor(x), fixed::floor(y))
                && city.open(
                    fixed::floor(x + fixed::mul(fx, nose)),
                    fixed::floor(y + fixed::mul(fy, nose)),
                )
        };

        if probe(self.x + dx, self.y) {
            self.x += dx;
        } else {
            self.hit(true);
        }
        if probe(self.x, self.y + dy) {
            self.y += dy;
        } else {
            self.hit(false);
        }
    }

    /// Take a wall impact on one axis.
    fn hit(&mut self, on_x: bool) {
        let v = if on_x { self.vx } else { self.vy };
        let sev = fixed::floor(fixed::abs(v)) * WALL_DAMAGE;
        self.damage = self.damage.saturating_add(sev.clamp(0, 40) as u8);
        // A wall also knocks the car crooked, which is most of why hitting
        // one is exciting rather than merely a stop.
        self.spin += if v > 0 { 120 } else { -120 };
        if on_x {
            self.vx = fixed::mul(self.vx, WALL_BOUNCE);
        } else {
            self.vy = fixed::mul(self.vy, WALL_BOUNCE);
        }
    }

    /// Take an impulse - from another car, or from something you flattened.
    pub fn shove(&mut self, ix: Fx, iy: Fx, spin: i32) {
        let m = fixed::from_int(self.kind.mass());
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
    let reach = a.kind.half_len() + b.kind.half_len();
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

    let ma = fixed::from_int(a.kind.mass());
    let mb = fixed::from_int(b.kind.mass());

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

    a.shove(-ix, -iy, -260);
    b.shove(ix, iy, 260);
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
    let overlap = (reach - dist).max(0) / 2 + 1;
    let a_ok = nudge(a, -fixed::mul(nx, overlap), -fixed::mul(ny, overlap), city);
    let b_ok = nudge(b, fixed::mul(nx, overlap), fixed::mul(ny, overlap), city);
    if !a_ok {
        nudge(b, fixed::mul(nx, overlap), fixed::mul(ny, overlap), city);
    }
    if !b_ok {
        nudge(a, -fixed::mul(nx, overlap), -fixed::mul(ny, overlap), city);
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
    /// Measured from a standstill on open ground, flat out: 47 mph after a
    /// quarter of a second, 82 after half, 125 after a second, and the full
    /// 154 at one and three quarters.  The version this replaced was at the
    /// clamp in 0.27 s and every one of those figures was 154.
    #[test]
    fn the_engine_has_a_curve_rather_than_a_switch() {
        let city = open_ground();
        // Quarter-second samples of the speed, from a standstill, flat out.
        let mut car = Car::new(CarKind::Taxi, fixed::HALF, fixed::HALF, 0, 7);
        let mut speeds = Vec::new();
        for _ in 0..8 {
            for _ in 0..HZ / 4 {
                car.step(&Controls { throttle: ONE, ..Default::default() }, &city, HZ);
            }
            speeds.push(car.speed());
        }
        // The first quarter second is worth more than the fourth, which is
        // the whole of what a curve means.
        let first = speeds[0];
        let fourth = speeds[3] - speeds[2];
        // Two and a half to one; measured at 2.16 units against 0.83, which
        // is 2.6.  The bar is under the measurement because what is being
        // defended is that the curve exists, not its exact shape.
        assert!(
            first * 2 > fourth * 5,
            "no taper: {} in the first quarter second against {} in the fourth",
            fixed::to_f32(first),
            fixed::to_f32(fourth)
        );
        // And it still gets there: full speed within two seconds.
        assert!(
            speeds[7] >= VMAX - fixed::ratio(1, 20),
            "two seconds flat out and it is only doing {}",
            fixed::to_f32(speeds[7])
        );
    }

    #[test]
    fn it_does_not_exceed_its_top_speed() {
        let (city, mut car) = on_the_road();
        flat_out(&mut car, &city, 2000);
        assert!(car.speed() <= VMAX + ONE, "speed ran away: {}", fixed::to_f32(car.speed()));
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

    #[test]
    fn turning_at_speed_makes_it_slide() {
        let (peak, _) = corner(VMAX, false);
        assert!(peak > fixed::ratio(1, 5), "it turned on rails: slip {}", fixed::to_f32(peak));
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


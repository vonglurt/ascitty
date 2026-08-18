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
//! 3. **Grip falls off with speed, and the handbrake removes it.**  Slow
//!    corners are on rails.  Fast ones are not.  Pulling the handbrake
//!    drops grip to almost nothing, which is how you get the car sideways
//!    on purpose.
//! 4. **Buildings are rigid and everything else is not.**  A wall stops the
//!    car and costs it speed and paint.  A lamp post does not.
//!
//! Reality is not a goal.  Pace is.

use crate::fixed::{self, Fx, ONE};
use crate::trig::{self, Ang};
use crate::world::City;

/// Ticks per second the physics is written for.  The step function takes the
/// real rate and scales, but the constants below are quoted at this one.
pub const HZ: i32 = 60;

/// Engine force, in units per second per second.
const ACCEL: Fx = fixed::ratio(26, 1);
/// Braking is stronger than the engine, as it is on every car.
const BRAKE: Fx = fixed::ratio(44, 1);
/// Reverse is weak, as it is on every car.
const REVERSE: Fx = fixed::ratio(9, 1);
/// Rolling and air drag, as a per-second fraction of speed retained.
const DRAG: Fx = fixed::ratio(88, 100);
/// Top speed, in units per second.  A unit is six metres, so this is about
/// 150 km/h - fast enough that the grid goes past in a blur.
const VMAX: Fx = fixed::ratio(7, 1);
/// Lateral grip at rest: almost all sideways motion is killed each second.
const GRIP_LOW_SPEED: Fx = fixed::ratio(2, 100);
/// Lateral grip at top speed: much more of the slide survives.
const GRIP_HIGH_SPEED: Fx = fixed::ratio(46, 100);
/// Lateral grip with the handbrake pulled.
const GRIP_HANDBRAKE: Fx = fixed::ratio(88, 100);
/// Peak yaw rate, in angle units per second.
const TURN_RATE: i32 = 40_000;
/// Speed at which the car turns its hardest.  Below this the wheels have
/// nothing to work against; above it the turn is limited on purpose so that
/// flat-out cornering has to be done sideways.
const TURN_REF: Fx = fixed::ratio(5, 2);
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
    pub fn half_len(self) -> Fx {
        match self {
            CarKind::Bus => fixed::ratio(1, 2),
            _ => fixed::ratio(1, 4),
        }
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
            fixed::mul(t, ACCEL)
        } else if vf > fixed::ratio(1, 4) {
            fixed::mul(t, BRAKE)
        } else {
            fixed::mul(t, REVERSE)
        };
        vf += fixed::mul(force, inv);

        // Drag, applied per tick as a fraction of the per-second figure.
        vf -= fixed::mul(fixed::mul(vf, ONE - DRAG), inv * hz / 60);
        vf = vf.clamp(-fixed::mul(VMAX, fixed::HALF), VMAX);

        // Grip.  Interpolated between the parked figure and the flat-out
        // one, so the car gets loose as it gets fast without a threshold
        // anyone can feel as a switch.
        let f = fixed::div(fixed::abs(vf), VMAX).min(ONE);
        let keep = if c.handbrake {
            GRIP_HANDBRAKE
        } else {
            fixed::lerp(GRIP_LOW_SPEED, GRIP_HIGH_SPEED, f)
        };
        // `keep` is per second; per tick it is the same fraction scaled.
        vl = fixed::mul(vl, ONE - fixed::mul(ONE - keep, inv * hz / 60));

        // Recombine *before* the heading changes.  See the module note: this
        // ordering is the drift.
        self.vx = fixed::mul(fx, vf) + fixed::mul(rx, vl);
        self.vy = fixed::mul(fy, vf) + fixed::mul(ry, vl);

        // Steering authority rises with speed and then plateaus.
        let auth = fixed::div(fixed::abs(vf), TURN_REF).min(ONE);
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
            city.walkable(fixed::floor(x), fixed::floor(y))
                && city.walkable(
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
pub fn collide(a: &mut Car, b: &mut Car) -> Option<Fx> {
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
    let overlap = (reach - dist).max(0) / 2 + 1;
    a.x -= fixed::mul(nx, overlap);
    a.y -= fixed::mul(ny, overlap);
    b.x += fixed::mul(nx, overlap);
    b.y += fixed::mul(ny, overlap);
    Some(sev)
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
        let x = fixed::from_int(1) + fixed::HALF; // avenues sit at x % 14 < 3
        let y = fixed::from_int(20) + fixed::HALF;
        assert!(city.walkable(1, 20), "the test avenue is not clear");
        (city, Car::new(CarKind::Taxi, x, y, trig::QUARTER, 7))
    }

    fn flat_out(car: &mut Car, city: &City, ticks: u32) {
        let c = Controls { throttle: ONE, ..Default::default() };
        for _ in 0..ticks {
            car.step(&c, city, HZ);
        }
    }

    #[test]
    fn the_throttle_makes_it_go() {
        let (city, mut car) = on_the_road();
        flat_out(&mut car, &city, 30);
        assert!(car.speed() > ONE, "half a second flat out and it is not moving");
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
        let (city, mut car) = on_the_road();
        flat_out(&mut car, &city, 90);
        let c = Controls { throttle: ONE, steer: ONE, ..Default::default() };
        let mut peak = 0;
        for _ in 0..40 {
            car.step(&c, &city, HZ);
            peak = peak.max(car.slip());
        }
        assert!(peak > fixed::ratio(1, 10), "it turned on rails: slip {}", fixed::to_f32(peak));
    }

    #[test]
    fn the_handbrake_makes_it_slide_more() {
        let run = |hb: bool| {
            let (city, mut car) = on_the_road();
            flat_out(&mut car, &city, 90);
            let c = Controls { throttle: ONE, steer: ONE, handbrake: hb };
            let mut peak = 0;
            for _ in 0..40 {
                car.step(&c, &city, HZ);
                peak = peak.max(car.slip());
            }
            peak
        };
        let (hb, no) = (run(true), run(false));
        assert!(hb > no, "the handbrake did nothing: {} vs {}", fixed::to_f32(hb), fixed::to_f32(no));
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
                city.walkable(fixed::floor(car.x), fixed::floor(car.y)),
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
            collide(&mut a, &mut b).expect("no contact");
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
        assert!(collide(&mut a, &mut b).is_none());
    }

    #[test]
    fn cars_far_apart_do_not_collide() {
        let mut a = Car::new(CarKind::Taxi, 0, 0, 0, 7);
        let mut b = Car::new(CarKind::Traffic, fixed::from_int(3), 0, 0, 2);
        assert!(collide(&mut a, &mut b).is_none());
    }

    #[test]
    fn a_collision_separates_them_so_it_does_not_repeat_forever() {
        let mut a = Car::new(CarKind::Taxi, 0, 0, 0, 7);
        a.vx = fixed::from_int(5);
        let mut b = Car::new(CarKind::Traffic, fixed::ratio(1, 8), 0, 0, 2);
        assert!(collide(&mut a, &mut b).is_some());
        // Immediately after, they must be either apart or separating.
        assert!(collide(&mut a, &mut b).is_none(), "the same collision fired twice");
    }
}

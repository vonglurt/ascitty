//! The city that moves: street furniture, traffic, pedestrians, and the
//! fare.
//!
//! # What the rules are
//!
//! - **Buildings are rigid.** Nothing you can do moves one.
//! - **Everything else on the pavement is not.**  Lamp posts, mailboxes,
//!   hydrants, meters and signals go over when you hit them, take a
//!   velocity and a lean, and stay down.  None of it stops the car.
//! - **Traffic is skittles.**  Other cars take a full impulse exchange and
//!   go spinning.  A bus does not.
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
use crate::sprite::{Billboard, Stamp};
use crate::trig::{self, Ang};
use crate::walk::Foot;
use crate::world::{City, Kind, SIZE};

/// How many other vehicles are in the pool.
pub const TRAFFIC: usize = 36;
/// How many pedestrians are in the pool.
pub const PEDS: usize = 48;
/// Beyond this many cells, a pooled actor is recycled somewhere nearer.
pub const RECYCLE: i32 = 34;
/// Seconds on the clock at the start of a shift.
pub const START_TIME: i32 = 60;
/// Seconds a coin is worth.
pub const COIN_TIME: i32 = 2;
/// Seconds picking up a fare is worth.
pub const PICKUP_TIME: i32 = 12;
/// How close, and how slow, you have to be to pick up or drop off.
pub const STOP_RADIUS: Fx = fixed::ratio(3, 4);
/// Above this speed the passenger will not get in or out.
pub const STOP_SPEED: Fx = fixed::ratio(3, 2);

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
#[derive(Clone, Debug)]
pub struct Fare {
    /// Where the passenger is waiting.
    pub from: (Fx, Fx),
    /// Where they are going.
    pub to: (Fx, Fx),
    /// Whether they are in the car.
    pub aboard: bool,
    /// The coins between here and there.
    pub coins: Vec<Coin>,
    /// What the fare is worth, in whole units of money.
    pub value: u32,
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
    /// The street furniture.
    pub props: Vec<Prop>,
    /// The pedestrians.
    pub peds: Vec<Ped>,
    /// The current job, if any.
    pub fare: Option<Fare>,
    /// Money taken this shift.
    pub money: u32,
    /// Consecutive things hit without stopping.
    pub combo: u32,
    /// Ticks left on the clock, at [`drive::HZ`].
    pub ticks_left: i32,
    /// Frames since the shift began.
    pub tick: u32,
    /// Whether the shift is over.
    pub over: bool,
    rng: Rng,
    /// Scratch for the billboard sort, so a frame does not allocate.
    order: Vec<(Fx, usize)>,
    /// Scratch for the billboards handed to the sprite pass.
    boards: Vec<Billboard>,
}

impl Sim {
    /// Start a shift in a generated city.
    pub fn new(city: &City, seed: u32) -> Sim {
        let start = Camera::spawn(city, SIZE as i32 / 2, SIZE as i32 / 2);
        let mut sim = Sim {
            taxi: Car::new(CarKind::Taxi, start.x, start.y, 0, palette::H_YELLOW),
            traffic: Vec::with_capacity(TRAFFIC),
            traffic_ctl: vec![Controls::default(); TRAFFIC],
            props: Vec::new(),
            peds: Vec::with_capacity(PEDS),
            fare: None,
            money: 0,
            combo: 0,
            ticks_left: START_TIME * drive::HZ,
            tick: 0,
            over: false,
            rng: Rng::new(seed.wrapping_add(0x0000_5EED)),
            order: Vec::new(),
            boards: Vec::new(),
        };
        sim.furnish(city);
        for _ in 0..TRAFFIC {
            let c = sim.spawn_car(city);
            sim.traffic.push(c);
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
                    Stamp::LampPost => (fixed::ratio(1, 3), fixed::ratio(9, 4), palette::H_WHITE),
                    Stamp::Tree => (fixed::ratio(4, 5), fixed::ratio(9, 5), palette::H_GREEN),
                    Stamp::Signal => (fixed::ratio(1, 3), fixed::ratio(2, 1), palette::H_WHITE),
                    Stamp::Mailbox => (fixed::ratio(2, 5), fixed::ratio(3, 5), palette::H_BLUE),
                    Stamp::Hydrant => (fixed::ratio(1, 4), fixed::ratio(2, 5), palette::H_RED),
                    Stamp::Meter => (fixed::ratio(1, 5), fixed::ratio(4, 5), palette::H_WHITE),
                    _ => (fixed::ratio(1, 5), fixed::ratio(2, 5), palette::H_YELLOW),
                };
                // Off-centre by a stable amount, so a street of lamp posts
                // is not a row of identically placed lamp posts.
                let jx = fixed::ratio(((h >> 8) % 5) as i32 - 2, 12);
                let jy = fixed::ratio(((h >> 12) % 5) as i32 - 2, 12);
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

    fn spawn_car(&mut self, city: &City) -> Car {
        const HUES: [u8; 6] = [
            palette::H_WHITE,
            palette::H_RED,
            palette::H_BLUE,
            palette::H_GREEN,
            palette::H_ORANGE,
            palette::H_PURPLE,
        ];
        // If no road turned up, put the car on top of the taxi rather than
        // at the origin: it will be recycled on the next tick, whereas a
        // fallback in the corner of the map is a car that is instantly and
        // permanently a straggler.
        let (x, y) = self.road_near(city, 8, RECYCLE).unwrap_or((
            fixed::floor(self.taxi.x),
            fixed::floor(self.taxi.y),
        ));
        let hue = HUES[self.rng.below(6) as usize];
        let kind = if self.rng.chance(1, 8) { CarKind::Bus } else { CarKind::Traffic };
        // Traffic drives along whichever axis its road runs on.
        let along_x = city.at(x + 1, y).kind == Kind::Road && city.at(x - 1, y).kind == Kind::Road;
        let yaw = if along_x {
            if self.rng.chance(1, 2) { 0 } else { trig::HALF }
        } else if self.rng.chance(1, 2) {
            trig::QUARTER
        } else {
            trig::QUARTER.wrapping_add(trig::HALF)
        };
        let mut c = Car::new(
            kind,
            fixed::from_int(x) + fixed::HALF,
            fixed::from_int(y) + fixed::HALF,
            yaw,
            hue,
        );
        let cruise = fixed::ratio(self.rng.range(15, 30), 10);
        c.vx = fixed::mul(trig::cos(yaw), cruise);
        c.vy = fixed::mul(trig::sin(yaw), cruise);
        c
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
                    if city.at(px, py).kind != Kind::Road || !city.walkable(px, py) {
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
        self.taxi.x = fixed::from_int(px) + fixed::HALF;
        self.taxi.y = fixed::from_int(py) + fixed::HALF;
        self.taxi.vx = 0;
        self.taxi.vy = 0;
        self.taxi.spin = 0;
        // Point it down the road it is parked on.
        self.taxi.yaw = if city.plan.cols.at(px).class.is_street() {
            trig::QUARTER
        } else {
            0
        };
    }

    /// Find a new fare and string coins along the way to it.
    pub fn hail(&mut self, city: &City) {
        let Some(from) = self.road_near(city, 4, 14) else { return };
        let Some(to) = self.road_near(city, 18, RECYCLE) else { return };
        let coins = coin_trail(city, from, to);
        let dist = (from.0 - to.0).abs() + (from.1 - to.1).abs();
        self.fare = Some(Fare {
            from: (fixed::from_int(from.0) + fixed::HALF, fixed::from_int(from.1) + fixed::HALF),
            to: (fixed::from_int(to.0) + fixed::HALF, fixed::from_int(to.1) + fixed::HALF),
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
        self.taxi.step(c, city, hz);
        if self.taxi.damage > before.saturating_add(3) {
            out.push(Event::Crunched);
            self.combo = 0;
        }

        self.step_traffic(city, hz, out);
        self.step_props(hz, out);
        self.step_peds(city, hz);
        self.step_fare(city, out);

        self.ticks_left -= 1;
        if self.ticks_left <= 0 {
            self.ticks_left = 0;
            self.over = true;
            out.push(Event::TimeUp);
        }
    }

    fn step_traffic(&mut self, city: &City, hz: i32, out: &mut Vec<Event>) {
        for i in 0..self.traffic.len() {
            // Recycle anything that has fallen too far behind.
            let far = fixed::abs(self.traffic[i].x - self.taxi.x)
                + fixed::abs(self.traffic[i].y - self.taxi.y);
            if far > fixed::from_int(RECYCLE + 12) {
                self.traffic[i] = self.spawn_car(city);
                continue;
            }
            // Traffic drives at a steady throttle and does not steer.  It is
            // scenery with momentum, and giving it a route would only make
            // it harder to hit.
            self.traffic_ctl[i].throttle = fixed::ratio(2, 5);
            let ctl = self.traffic_ctl[i];
            self.traffic[i].step(&ctl, city, hz);

            // Against the taxi.
            let (mut a, mut b) = (self.taxi, self.traffic[i]);
            if let Some(sev) = drive::collide(&mut a, &mut b, city) {
                self.taxi = a;
                self.traffic[i] = b;
                if sev > ONE {
                    self.combo += 1;
                    self.money += 2 * self.combo;
                    out.push(Event::Rammed);
                }
            }
        }
    }

    fn step_props(&mut self, hz: i32, out: &mut Vec<Event>) {
        let inv = fixed::div(ONE, fixed::from_int(hz.max(1)));
        let speed = self.taxi.speed();
        for p in self.props.iter_mut() {
            if p.standing {
                let dx = p.board.x - self.taxi.x;
                let dy = p.board.y - self.taxi.y;
                let reach = self.taxi.kind.half_len() + p.board.w;
                if fixed::abs(dx) < reach && fixed::abs(dy) < reach && speed > ONE {
                    // Over it goes, in the direction the car was travelling,
                    // and the car does not slow down at all - which is the
                    // whole appeal.
                    p.standing = false;
                    p.vx = fixed::mul(self.taxi.vx, fixed::ratio(3, 5));
                    p.vy = fixed::mul(self.taxi.vy, fixed::ratio(3, 5));
                    self.combo += 1;
                    self.money += self.combo;
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
                out.push(Event::Coin);
            }
        }

        let target = if fare.aboard { fare.to } else { fare.from };
        let close = fixed::abs(target.0 - self.taxi.x) < STOP_RADIUS
            && fixed::abs(target.1 - self.taxi.y) < STOP_RADIUS;
        if !close || speed > STOP_SPEED {
            return;
        }
        if fare.aboard {
            self.money += fare.value;
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

    /// Seconds left on the clock.
    pub fn seconds_left(&self) -> i32 {
        self.ticks_left / drive::HZ
    }

    /// Where the passenger or the destination is, for the compass.
    pub fn target(&self) -> Option<(Fx, Fx)> {
        self.fare.as_ref().map(|f| if f.aboard { f.to } else { f.from })
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

        for pr in &self.props {
            if near(pr.board.x, pr.board.y) {
                let mut b = pr.board;
                b.phase = ((self.tick / 90) % 3) as u8;
                self.boards.push(b);
            }
        }
        for c in &self.traffic {
            if !near(c.x, c.y) {
                continue;
            }
            let stamp = match (c.kind, c.damage) {
                (CarKind::Bus, _) => Stamp::Bus,
                (_, d) if d > 60 => Stamp::Wreck,
                _ => Stamp::Car,
            };
            self.boards.push(Billboard::upright(
                stamp,
                c.x,
                c.y,
                if c.kind == CarKind::Bus { fixed::ratio(6, 5) } else { fixed::ratio(4, 5) },
                if c.kind == CarKind::Bus { fixed::ratio(11, 10) } else { fixed::ratio(3, 5) },
                c.hue,
            ));
        }
        for pd in &self.peds {
            if near(pd.x, pd.y) {
                let mut b = Billboard::upright(
                    Stamp::Ped,
                    pd.x,
                    pd.y,
                    fixed::ratio(1, 3),
                    fixed::ratio(3, 5),
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
                let mut b = Billboard::upright(
                    Stamp::Coin,
                    c.x,
                    c.y,
                    w,
                    fixed::ratio(2, 5),
                    palette::H_YELLOW,
                );
                b.base = fixed::ratio(1, 3);
                self.boards.push(b);
            }
            let (mx, my) = if fare.aboard { fare.to } else { fare.from };
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

/// Whether a sidewalk cell is at a corner - which is where signals go.
fn on_corner(city: &City, x: i32, y: i32) -> bool {
    let road = |dx: i32, dy: i32| city.at(x + dx, y + dy).kind == Kind::Road;
    (road(-1, 0) || road(1, 0)) && (road(0, -1) || road(0, 1))
}

/// String coins along a Manhattan path between two points, on roads only.
///
/// The path is x first and then y, which is not the shortest route by road
/// but is the one a player can *read* at a glance: a line of coins that goes
/// straight and then turns once.  A cleverer path would be harder to follow
/// at 90 mph, which would make it a worse path.
fn coin_trail(city: &City, from: (i32, i32), to: (i32, i32)) -> Vec<Coin> {
    let mut coins = Vec::new();
    let mut push = |x: i32, y: i32| {
        if city.at(x, y).kind == Kind::Road {
            coins.push(Coin {
                x: fixed::from_int(x) + fixed::HALF,
                y: fixed::from_int(y) + fixed::HALF,
                taken: false,
            });
        }
    };
    let step = 2;
    let sx = if to.0 > from.0 { step } else { -step };
    let mut x = from.0;
    while (to.0 - x).abs() >= step {
        x += sx;
        push(x, from.1);
    }
    let sy = if to.1 > from.1 { step } else { -step };
    let mut y = from.1;
    while (to.1 - y).abs() >= step {
        y += sy;
        push(to.0, y);
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
        assert!(city.walkable(fixed::floor(sim.taxi.x), fixed::floor(sim.taxi.y)));
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
    fn the_clock_runs_down_and_the_shift_ends() {
        let (city, mut sim) = shift();
        let mut ev = Vec::new();
        sim.ticks_left = 3;
        for _ in 0..10 {
            sim.step(&city, &Controls::default(), drive::HZ, &mut ev);
        }
        assert!(sim.over, "the clock ran out and the shift went on");
        assert_eq!(sim.seconds_left(), 0);
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
        assert!(sim.money >= f.value, "the fare was not paid");
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
            assert!(city.walkable(fixed::floor(sim.taxi.x), fixed::floor(sim.taxi.y)));
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

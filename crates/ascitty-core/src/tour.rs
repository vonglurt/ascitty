//! The autopilot: a camera that walks the streets and looks around.
//!
//! Written because "show me what it looks like" should not require somebody
//! to hold `w` down, and because a scripted path baked for one city is
//! wrong for every other seed. This one *reads* the city instead: it walks
//! until something is in the way, turns at the junction, and stops to look
//! up at whatever is tallest nearby.
//!
//! # Heading and gaze are separate
//!
//! The single thing that makes this look like a person rather than a
//! dolly. `heading` is where the feet are going; the camera's `yaw` is
//! `heading + gaze`, and gaze wanders. So the autopilot can turn its head to
//! watch a tower go past without veering into it, and the movement never
//! stops to let the look happen.
//!
//! # It is deterministic
//!
//! Same city and same seed, same walk, every time - which is what makes a
//! recorded animation reproducible and what lets the tests assert that it
//! never walks into a building.

use crate::camera::Camera;
use crate::fixed::{self, Fx};
use crate::rng::Rng;
use crate::trig::{self, Ang};
use crate::world::{City, Kind};

/// What the walker is doing at the moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Doing {
    /// Walking, with the head drifting slowly from side to side.
    Strolling,
    /// Stopped, or nearly, looking up at something tall.
    Admiring,
    /// Turning at a junction.
    Turning,
    /// Standing still, looking down the street.
    Waiting,
}

/// How far ahead the walker looks before deciding the way is blocked.
///
/// Three and a half cells, and it is not a "bigger is safer" dial.  Too
/// short and the walker turns with its nose already against a forty-storey
/// facade, so the frame is one wall and nothing else.  Too long and it
/// starts reacting to buildings it was never going to reach, which in a
/// narrow street means turning, turning back, and oscillating into the
/// kerb - measured at four and a half cells, four times as many frames were
/// pressed against a wall as at three and a half.
const PROBE: Fx = fixed::ratio(7, 2);

/// How far it steps between probe samples.
const PROBE_STEP: Fx = fixed::ratio(1, 4);

/// Walking speed, in cell units per second.
pub const PACE: Fx = fixed::ratio(9, 4);

/// How hard the walker is pulled towards the middle of the street, in cell
/// units per second at full lopsidedness.
///
/// Two thirds of the walking pace.  The pull only reaches full strength with
/// one shoulder against a wall and eight clear cells on the other side; at
/// anything less it is a nudge.  It was two and a half for a while, which
/// was compensating for the sign of the bias being wrong - once that was
/// fixed the stronger pull just made the walk wobble.
const CENTRING: Fx = fixed::ratio(3, 2);

/// How far to one side of the crown of the road the walker settles, in
/// quarter-cell probe steps.
///
/// Dead centre is the obvious target and the wrong one: it puts the camera
/// directly on top of the double yellow, so the nearest few rows of every
/// frame are a wall of centre line.  Half a cell over is the middle of a
/// lane, which is where a vehicle would be and where the line converges
/// away from you down the street instead of out from under you.
const LANE_BIAS: i32 = 4;

/// The nearest a building may be for the walker to stop and look up at it,
/// in cells.
const MIN_ADMIRE: i32 = 4;

/// The most the camera will tilt up, in screen rows.
///
/// The pitch is a shear of the horizon, so a large value pushes the ground
/// off the bottom of the screen entirely.  Eight rows leaves the street in
/// shot on any frame taller than about twenty rows, and the front end
/// clamps it again against the real height.
const MAX_TILT: i32 = 8;

/// How fast the head turns towards where it wants to look, as a fraction of
/// the remaining angle per tick at [`REFERENCE_HZ`].  A sixth is unhurried
/// without being slack.
const HEAD_TRACK: i32 = 6;

/// The tick rate the smoothing fractions above are quoted at.
const REFERENCE_HZ: i32 = 30;

/// Whether a cell is part of the street corridor - carriageway or pavement.
///
/// The tour follows *streets*, not merely open ground.  A park or a plaza is
/// walkable and is the wrong place for a camera: it is a clearing in the
/// middle of a block, surrounded on all sides, and a camera that wanders
/// into one spends the next minute looking at the backs of buildings.
#[inline]
fn on_street(city: &City, x: i32, y: i32) -> bool {
    match city.at(x, y).kind {
        // An alley is one cell wide with buildings hard against both sides.
        // It is a street by any reasonable definition and a terrible place
        // to put a camera: there is a wall against each shoulder for its
        // whole length, so the walker is boxed in the entire time it is in
        // one.
        Kind::Road => {
            city.plan.cols.at(x).class != crate::world::RoadClass::Alley
                && city.plan.rows.at(y).class != crate::world::RoadClass::Alley
        }
        Kind::Sidewalk => true,
        _ => false,
    }
}

/// The autopilot.
pub struct Tour {
    /// The camera it is driving. Read this; the app renders from it.
    pub cam: Camera,
    /// Where the feet are going.
    heading: Ang,
    /// Where the feet are going *next* - the walker turns towards this.
    heading_target: Ang,
    /// Offset from the heading that the head is turned by.
    gaze: i32,
    /// Where the head is turning to.
    gaze_target: i32,
    /// Where the pitch is heading, in screen rows.
    pitch_target: i32,
    /// What it is doing.
    pub doing: Doing,
    /// Ticks left in the current behaviour.
    dwell: u32,
    /// Ticks since the tour started.
    pub tick: u32,
    rng: Rng,
}

impl Tour {
    /// Start a tour of a city, from wherever [`Camera::spawn`] puts you.
    pub fn new(city: &City, seed: u32) -> Tour {
        let size = crate::world::SIZE as i32;
        let mut cam = Camera::spawn(city, size / 2, size / 2);
        // Out into the carriageway.  The walker settles half a cell off the
        // crown of the road - see `LANE_BIAS` - so the road is where this
        // tour lives, and starting it on the pavement it was spawned on only
        // means its first few seconds are spent stepping off a kerb.
        if let Some((rx, ry)) = city.nearest_road(fixed::floor(cam.x), fixed::floor(cam.y), 48) {
            cam.x = fixed::from_int(rx) + fixed::HALF;
            cam.y = fixed::from_int(ry) + fixed::HALF;
            cam.stand(city);
        }
        // If that landed in an alley or a plaza, walk out to a street
        // corridor before starting.
        if !on_street(city, fixed::floor(cam.x), fixed::floor(cam.y)) {
            'out: for r in 1..40i32 {
                for dy in -r..=r {
                    for dx in -r..=r {
                        let (x, y) = (fixed::floor(cam.x) + dx, fixed::floor(cam.y) + dy);
                        if on_street(city, x, y) && city.open(x, y) {
                            cam.x = fixed::from_int(x) + fixed::HALF;
                            cam.y = fixed::from_int(y) + fixed::HALF;
                            break 'out;
                        }
                    }
                }
            }
            cam.stand(city);
        }
        let mut t = Tour {
            cam,
            heading: 0,
            heading_target: 0,
            gaze: 0,
            gaze_target: 0,
            pitch_target: 0,
            doing: Doing::Strolling,
            dwell: 1,
            tick: 0,
            rng: Rng::new(seed ^ 0x_7005_0000),
        };
        // Face whichever way the street actually runs, rather than east and
        // straight into a wall.
        t.heading = t.best_heading(city, 0);
        t.heading_target = t.heading;
        t.cam.yaw = t.heading;
        t
    }

    /// Advance one tick at `hz` ticks per second.
    pub fn step(&mut self, city: &City, hz: i32) {
        let hz = hz.max(1);
        self.tick = self.tick.wrapping_add(1);

        if self.dwell == 0 {
            self.choose(city, hz);
        }
        self.dwell -= 1;

        // Turn the feet towards the target heading, and the head towards
        // where it wants to look. Both are exponential approaches, which is
        // what stops either from arriving with a jolt.
        //
        // The fraction is scaled by the tick rate. A fixed "an eighth of the
        // way there per tick" turns twice as fast at sixty hertz as at
        // thirty, which makes the whole walk frame-rate dependent - and a
        // recorded tour would then depend on the rate it was recorded at.
        let rate = |divisor: i32| (divisor * hz / REFERENCE_HZ).max(1);
        let dh = self.heading_target.wrapping_sub(self.heading) as i16 as i32;
        self.heading = self.heading.wrapping_add((dh / rate(8)) as Ang);

        let dg = self.gaze_target - self.gaze;
        self.gaze += dg / rate(HEAD_TRACK);
        self.cam.yaw = self.heading.wrapping_add(self.gaze as Ang);

        let dp = self.pitch_target - self.cam.pitch;
        if dp != 0 {
            self.cam.pitch += if dp.abs() < rate(4) { dp.signum() } else { dp / rate(4) };
        }

        // Keep to the middle of the street.
        //
        // Without this the walker ends up hugging whichever wall it drifted
        // towards, and a camera at eye height pressed against a forty-storey
        // building sees forty storeys of window and nothing else.  Probing
        // both flanks and easing towards the open side puts it in the middle
        // of the widest space available, which on this street grid is the
        // centre of the avenue - and the centre of the avenue is the shot.
        let lateral = {
            let left = self.heading.wrapping_sub(trig::QUARTER);
            let right = self.heading.wrapping_add(trig::QUARTER);
            // `bias` is positive when the walker should move to its right.
            // The sign matters and was wrong for a while: with the terms the
            // other way round the walker steers *towards* the closed side,
            // which is invisible on a wide symmetric avenue and pins it to
            // the kerb everywhere else.  The centring strength had been
            // tuned upwards to compensate, which made it worse.
            let bias =
                self.clearance(city, right) - self.clearance(city, left) + LANE_BIAS;
            fixed::mul(fixed::ratio(bias.clamp(-8, 8), 8), CENTRING)
        };

        // Walk along the heading, not along the look direction.
        let speed = match self.doing {
            Doing::Admiring => fixed::mul(PACE, fixed::ratio(1, 4)),
            Doing::Waiting => 0,
            Doing::Turning => fixed::mul(PACE, fixed::ratio(1, 2)),
            Doing::Strolling => PACE,
        };
        let d = fixed::div(speed, fixed::from_int(hz));
        let side = fixed::div(lateral, fixed::from_int(hz));
        let (fx, fy) = (trig::cos(self.heading), trig::sin(self.heading));
        self.cam.slide_where(
            fixed::mul(fx, d) + fixed::mul(-fy, side),
            fixed::mul(fy, d) + fixed::mul(fx, side),
            |x, y| on_street(city, x, y) && city.open(x, y),
        );
        self.cam.stand(city);

        // Blocked? Turn, whatever else was going on. Checked every tick
        // rather than only when a behaviour ends, because a behaviour that
        // outlasts the pavement walks you into a wall and leaves you there
        // grinding against it for the rest of the take.
        if !self.clear(city, self.heading, PROBE) {
            self.heading_target = self.best_heading(city, self.heading);
            if self.doing != Doing::Turning {
                self.doing = Doing::Turning;
                self.dwell = (hz as u32 * 3) / 4;
                self.gaze_target = 0;
                self.pitch_target = 0;
            }
        }
    }

    /// Pick the next thing to do.
    fn choose(&mut self, city: &City, hz: i32) {
        let hz = hz as u32;
        // At a junction, a walker has a real choice; mid-block it does not.
        let at_junction = self.junction(city);

        let roll = self.rng.below(100);
        if at_junction && roll < 45 {
            self.doing = Doing::Turning;
            self.dwell = (hz * 3) / 4;
            self.heading_target = self.pick_turn(city);
            self.gaze_target = 0;
            self.pitch_target = 0;
            return;
        }
        if roll < 62 {
            // Look up at whatever is tallest nearby. This is the shot the
            // whole renderer exists for, so it gets a third of the time.
            if let Some((ang, pitch)) = self.tallest_nearby(city) {
                self.doing = Doing::Admiring;
                self.dwell = hz * 2 + self.rng.below(hz);
                self.gaze_target = (ang.wrapping_sub(self.heading) as i16 as i32).clamp(-9000, 9000);
                self.pitch_target = pitch;
                return;
            }
        }
        if roll < 70 {
            self.doing = Doing::Waiting;
            self.dwell = hz + self.rng.below(hz);
            self.gaze_target = 0;
            self.pitch_target = 0;
            return;
        }

        self.doing = Doing::Strolling;
        self.dwell = hz + self.rng.below(hz * 2);
        // A slow drift of the head, left or right, while walking.
        self.gaze_target = self.rng.range(-5200, 5200);
        self.pitch_target = self.rng.range(-2, 3);
    }

    /// Whether the way is clear along `ang` for `dist`.
    fn clear(&self, city: &City, ang: Ang, dist: Fx) -> bool {
        let (dx, dy) = (trig::cos(ang), trig::sin(ang));
        let mut t = PROBE_STEP;
        while t <= dist {
            let x = self.cam.x + fixed::mul(dx, t);
            let y = self.cam.y + fixed::mul(dy, t);
            if !on_street(city, fixed::floor(x), fixed::floor(y)) {
                return false;
            }
            t += PROBE_STEP;
        }
        true
    }

    /// How far the way is clear along `ang`, up to eight cells.
    fn clearance(&self, city: &City, ang: Ang) -> i32 {
        let (dx, dy) = (trig::cos(ang), trig::sin(ang));
        let mut n = 0;
        while n < 32 {
            let t = PROBE_STEP * (n + 1);
            let x = self.cam.x + fixed::mul(dx, t);
            let y = self.cam.y + fixed::mul(dy, t);
            if !on_street(city, fixed::floor(x), fixed::floor(y)) {
                break;
            }
            n += 1;
        }
        n
    }

    /// The clearest of the four cardinal directions relative to `from`,
    /// preferring straight on, then a turn, and only reversing at a dead end.
    fn best_heading(&self, city: &City, from: Ang) -> Ang {
        let mut best = from;
        let mut best_score = i32::MIN;
        for (i, quarter) in [0i32, 1, 3, 2].into_iter().enumerate() {
            let ang = from.wrapping_add((quarter as u16).wrapping_mul(trig::QUARTER));
            // Preference falls off with each option tried, so a tie goes to
            // straight on and a reversal is the last resort.
            let score = self.clearance(city, ang) * 4 - i as i32;
            if score > best_score {
                best_score = score;
                best = ang;
            }
        }
        best
    }

    /// A turn at a junction: left or right if either is clear, else straight.
    fn pick_turn(&mut self, city: &City) -> Ang {
        let left = self.heading.wrapping_sub(trig::QUARTER);
        let right = self.heading.wrapping_add(trig::QUARTER);
        let (lc, rc) = (self.clearance(city, left), self.clearance(city, right));
        if lc >= 6 && rc >= 6 {
            if self.rng.chance(1, 2) {
                left
            } else {
                right
            }
        } else if lc >= 6 {
            left
        } else if rc >= 6 {
            right
        } else {
            self.heading
        }
    }

    /// Whether the cell underfoot is where an avenue meets a cross street.
    fn junction(&self, city: &City) -> bool {
        let (x, y) = (fixed::floor(self.cam.x), fixed::floor(self.cam.y));
        if city.at(x, y).kind != Kind::Road {
            return false;
        }
        city.plan.is_junction(x, y)
    }

    /// The tallest building within a few blocks, as a bearing and the pitch
    /// needed to take its top in.
    ///
    /// Pitch is in screen rows and the renderer's projection is not known
    /// here, so this is a proportion of height to distance scaled to
    /// something that looks right rather than a derived figure. A camera
    /// that tilts by exactly the right amount to frame a roofline is a
    /// camera that snaps; one that tilts roughly is one that looks.
    fn tallest_nearby(&self, city: &City) -> Option<(Ang, i32)> {
        let (cx, cy) = (fixed::floor(self.cam.x), fixed::floor(self.cam.y));
        let mut best: Option<(i32, Ang, i32)> = None;
        for dy in -10..=10i32 {
            for dx in -10..=10i32 {
                let h = city.height(cx + dx, cy + dy) as i32;
                if h < 12 {
                    continue;
                }
                let dist = dx.abs().max(dy.abs());
                // Nothing under four cells.  A tower nine metres away is not
                // something you crane your neck at, it is something you are
                // about to walk into, and framing it fills the screen with a
                // wall of windows and no sky.  The shot is the tower down
                // the block.
                if dist < MIN_ADMIRE {
                    continue;
                }
                // Tall and close beats tall and far - within reason.
                let score = h * 8 / dist;
                if best.is_none_or(|(b, _, _)| score > b) {
                    let ang = crate::sim::atan2_approx(
                        fixed::from_int(dy),
                        fixed::from_int(dx),
                    );
                    let pitch = ((h * 3) / (dist + 2)).clamp(2, MAX_TILT);
                    best = Some((score, ang, pitch));
                }
            }
        }
        best.map(|(_, a, p)| (a, p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::City;

    const HZ: i32 = 30;

    fn walk(seed: u32, ticks: u32) -> (City, Tour) {
        let city = City::generate(seed);
        let mut t = Tour::new(&city, seed);
        for _ in 0..ticks {
            t.step(&city, HZ);
        }
        (city, t)
    }

    #[test]
    fn it_never_walks_into_a_building() {
        for seed in [1u32, 7, 99, 31337] {
            let city = City::generate(seed);
            let mut t = Tour::new(&city, seed);
            for i in 0..9000 {
                t.step(&city, HZ);
                assert!(
                    city.open(fixed::floor(t.cam.x), fixed::floor(t.cam.y)),
                    "seed {seed} walked into a building at tick {i}: {},{}",
                    fixed::to_f32(t.cam.x),
                    fixed::to_f32(t.cam.y)
                );
            }
        }
    }

    #[test]
    fn it_actually_goes_somewhere() {
        let city = City::generate(5);
        let mut t = Tour::new(&city, 5);
        let (x0, y0) = (t.cam.x, t.cam.y);
        let mut furthest = 0;
        for _ in 0..3000 {
            t.step(&city, HZ);
            let d = fixed::abs(t.cam.x - x0) + fixed::abs(t.cam.y - y0);
            furthest = furthest.max(fixed::floor(d));
        }
        assert!(furthest > 12, "it only got {furthest} cells from where it started");
    }

    #[test]
    fn it_does_not_get_stuck_grinding_against_a_wall() {
        // Progress has to keep happening, not just happen once. Measured in
        // windows, because a walker that covers ground for ten seconds and
        // then wedges in a doorway passes a total-distance test.
        let city = City::generate(23);
        let mut t = Tour::new(&city, 23);
        for window in 0..12 {
            let (x0, y0) = (t.cam.x, t.cam.y);
            for _ in 0..(HZ * 8) {
                t.step(&city, HZ);
            }
            let moved = fixed::abs(t.cam.x - x0) + fixed::abs(t.cam.y - y0);
            assert!(
                fixed::floor(moved) >= 2,
                "window {window}: moved only {} cells in eight seconds",
                fixed::to_f32(moved)
            );
        }
    }

    #[test]
    fn it_looks_around_while_it_walks() {
        let (_c, _t) = walk(11, 1);
        let city = City::generate(11);
        let mut t = Tour::new(&city, 11);
        let mut gazes = std::collections::HashSet::new();
        let mut pitches = std::collections::HashSet::new();
        for _ in 0..2400 {
            t.step(&city, HZ);
            gazes.insert(t.cam.yaw.wrapping_sub(t.heading) as i16 / 512);
            pitches.insert(t.cam.pitch);
        }
        assert!(gazes.len() > 4, "the head never turned ({} positions)", gazes.len());
        assert!(pitches.len() > 2, "it never looked up or down");
    }

    #[test]
    fn it_looks_up_at_something_at_some_point() {
        let city = City::generate(3);
        let mut t = Tour::new(&city, 3);
        let mut admired = false;
        let mut high = 0;
        for _ in 0..6000 {
            t.step(&city, HZ);
            admired |= t.doing == Doing::Admiring;
            high = high.max(t.cam.pitch);
        }
        assert!(admired, "it never stopped to look at anything");
        assert!(high >= 2, "it never tilted up (best {high})");
    }

    #[test]
    fn it_keeps_off_the_walls() {
        // Not "never touches one" - it has to pass close to turn a corner -
        // but it must not spend the walk grinding along one, because that is
        // a camera pointed at a single facade for the whole take.
        let city = City::generate(99);
        let mut t = Tour::new(&city, 99);
        let mut hugging = 0;
        let total = 3000;
        for _ in 0..total {
            t.step(&city, HZ);
            let left = t.clearance(&city, t.heading.wrapping_sub(trig::QUARTER));
            let right = t.clearance(&city, t.heading.wrapping_add(trig::QUARTER));
            if left.min(right) == 0 {
                hugging += 1;
            }
        }
        assert!(
            hugging * 4 < total,
            "spent {hugging} of {total} ticks with a wall against one shoulder"
        );
    }

    #[test]
    fn it_never_tilts_the_ground_off_the_screen() {
        let city = City::generate(3);
        let mut t = Tour::new(&city, 3);
        for _ in 0..6000 {
            t.step(&city, HZ);
            assert!(
                t.cam.pitch.abs() <= MAX_TILT,
                "tilted to {} rows, past the {MAX_TILT} the front end expects",
                t.cam.pitch
            );
        }
    }

    #[test]
    fn the_walk_mostly_has_somewhere_to_look() {
        // The complaint this exists for: the walker turning too late and
        // spending frames with its nose against a facade, so the whole
        // screen is one building.
        //
        // Measured as the distance to the nearest wall dead ahead, which the
        // renderer already reports.  Counting blank cells instead looks like
        // the same question and is not: a frame looking slightly downwards
        // along a street is mostly road, has almost no sky in it, and is a
        // perfectly good frame.
        use crate::atmos::Atmos;
        use crate::frame::Frame;

        let city = City::generate(99);
        let mut t = Tour::new(&city, 99);
        let atmos = Atmos { haze: 2, ..Default::default() };
        let mut f = Frame::new(80, 24);
        let mut boxed_in = 0;
        let samples = 40;
        for _ in 0..samples {
            for _ in 0..30 {
                t.step(&city, HZ);
            }
            let st = crate::raycast::render(&city, &t.cam, &atmos, &mut f);
            if st.nearest < 1.5 {
                boxed_in += 1;
            }
        }
        assert!(
            boxed_in * 4 < samples,
            "{boxed_in} of {samples} frames had a wall inside a cell and a half"
        );
    }

    #[test]
    fn the_walk_is_reproducible() {
        let a = walk(42, 900);
        let b = walk(42, 900);
        assert_eq!(a.1.cam.x, b.1.cam.x);
        assert_eq!(a.1.cam.y, b.1.cam.y);
        assert_eq!(a.1.cam.yaw, b.1.cam.yaw);
        assert_eq!(a.1.cam.pitch, b.1.cam.pitch);
    }

    #[test]
    fn different_seeds_take_different_walks() {
        let a = walk(1, 1200);
        let b = walk(2, 1200);
        assert!(
            a.1.cam.x != b.1.cam.x || a.1.cam.y != b.1.cam.y,
            "two seeds produced the same walk"
        );
    }

    #[test]
    fn the_frame_rate_does_not_change_how_far_it_gets() {
        // Same wall-clock, different tick rates.
        //
        // The endpoint is *not* the thing to compare.  The walk turns at
        // junctions and picks behaviours from a generator, so two runs that
        // differ by one tick's rounding end up in different streets - it is
        // chaotic, and asserting they finish in the same place is asserting
        // something untrue that happens to hold on some seeds.
        //
        // What must hold is that the walker covers the same *ground* per
        // second, whatever the tick rate.  If it does not, something is
        // integrating per tick that should be integrating per second.
        let city = City::generate(6);
        let distance = |hz: i32| {
            let mut t = Tour::new(&city, 6);
            let mut total = 0i64;
            let (mut px, mut py) = (t.cam.x, t.cam.y);
            for _ in 0..(20 * hz) {
                t.step(&city, hz);
                total += (fixed::abs(t.cam.x - px) + fixed::abs(t.cam.y - py)) as i64;
                px = t.cam.x;
                py = t.cam.y;
            }
            total
        };
        let (slow, fast) = (distance(30), distance(60));
        let ratio = fast * 100 / slow.max(1);
        assert!(
            (60..=165).contains(&ratio),
            "60 Hz covered {ratio}% of the ground 30 Hz did over the same twenty seconds"
        );
    }

    #[test]
    fn it_starts_facing_down_a_street_rather_than_at_a_wall() {
        for seed in [1u32, 2, 3, 4, 5, 6, 7, 8] {
            let city = City::generate(seed);
            let t = Tour::new(&city, seed);
            assert!(
                t.clear(&city, t.heading, fixed::ONE),
                "seed {seed} started facing a wall"
            );
        }
    }
}


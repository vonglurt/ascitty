//! ASCITTY on a colour terminal.
//!
//! Renders the city at whatever size the terminal is, in ASCII or in block
//! elements, at 24-bit colour or 16 or none.  `--shot` renders one frame and
//! prints it as plain text, which is how the pictures in the documentation
//! are made and how the build checks that the renderer still works without
//! needing a terminal at all.

mod cast;
mod gif;
mod hud;
mod image;
mod paint;
mod png;
mod term;

use ascitty_core::atmos::Atmos;
use ascitty_core::camera::{Camera, TURN_SPEED, WALK_SPEED};
use ascitty_core::cabbie::Cabbie;
use ascitty_core::drive::Controls;
use ascitty_core::fixed::{self, Fx, ONE};
use ascitty_core::frame::Frame;
use ascitty_core::glyph::Mode;
use ascitty_core::raycast;
use ascitty_core::sim::{Event, Sim};
use ascitty_core::tour::Tour;
use ascitty_core::trig::{self, Ang};

use ascitty_core::world::{City, SIZE};
use paint::Depth;
use term::{Edge, Key, Keys, Term};

/// How the camera is being driven.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    /// On foot, eye height, cannot leave the pavement.
    Walk,
    /// Free flight above the city, looking down.
    Copter,
    /// Third person, behind the taxi.
    Drive,
}

impl View {
    fn name(self) -> &'static str {
        match self {
            View::Walk => "WALK",
            View::Copter => "COPTER",
            View::Drive => "DRIVE",
        }
    }

}

/// How fast the head tilts while a look key is held, in screen rows a
/// second.  Twenty is the whole useful range of tilt in about half a second.
const LOOK_ROWS: Fx = fixed::ratio(20, 1);

/// One control, as an analogue axis rather than a button.
///
/// # Why a level and not a flag
///
/// Two reasons, and they are independent.
///
/// The first is that a control that goes from nothing to everything in one
/// frame is not a pedal.  A throttle that ramps up while it is held, and
/// steering that winds on lock the longer you hold it, is how an arcade car
/// has always worked, and it is what makes holding a key feel like doing
/// something rather than like setting a flag.
///
/// The second is that on a terminal that cannot report a key coming up, the
/// level *is* the release detection.  A press tops the level up, it stays up
/// for [`Hands::GRACE`] without being renewed, and then it bleeds away - so
/// letting go costs about two thirds of a second of coasting, which is a
/// great deal better than either stopping dead between two autorepeats or
/// never stopping at all.  On a terminal that does report releases the level
/// still ramps, because of the first reason.
#[derive(Clone, Copy, Default)]
struct Axis {
    /// How far in it is, from 0 to [`ONE`].
    level: Fx,
    /// Whether the terminal has said it is down and not yet said otherwise.
    held: bool,
    /// Frames since the last byte for it, for terminals that cannot say.
    idle: u8,
}

impl Axis {
    /// Down: from a key press, or from a repeat of one.
    fn press(&mut self) {
        self.held = true;
        self.idle = 0;
    }

    /// Up.  Only a terminal that speaks the progressive protocol ever says
    /// this, and when one does it is the truth and the grace period is not
    /// consulted.
    fn release(&mut self) {
        self.held = false;
        self.idle = u8::MAX;
    }

    /// Move the level towards where the key is, one frame's worth.
    ///
    /// `up` and `down` are per-frame steps, worked out from the frame rate
    /// by [`Hands::tick`], so the car reaches full throttle in the same
    /// fraction of a second at any frame rate.
    fn tick(&mut self, up: Fx, down: Fx, grace: u8, trust_release: bool) {
        let down_now = if trust_release { self.held } else { self.idle < grace };
        self.idle = self.idle.saturating_add(1);
        if down_now {
            self.level = (self.level + up).min(ONE);
        } else {
            self.level = (self.level - down).max(0);
            if self.level == 0 {
                self.held = false;
            }
        }
    }
}

/// One control each, named for what the hands are doing rather than for
/// which key does it - the keys that reach them differ between walking and
/// driving, and both pairs of them reach the same two axes when driving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ctl {
    /// Throttle, or forwards.
    Gas,
    /// Brake, or backwards.
    Brake,
    /// Steer or turn.
    Left,
    /// Steer or turn.
    Right,
    /// Sideways.  Steering as well, when driving: a car cannot strafe, so
    /// the keys that would do it are the second pair of steering keys.
    StrafeLeft,
    /// Sideways.
    StrafeRight,
    /// Up, and the handbrake when there is one to pull.
    Up,
    /// Down.
    Down,
    /// Look.
    LookUp,
    /// Look.
    LookDown,
    /// Swing the camera round the car without steering it.
    PanLeft,
    /// Swing the camera round the car without steering it.
    PanRight,
}

/// How many there are.
const CTLS: usize = 12;

/// Everything the driver is holding this frame.
#[derive(Default)]
struct Hands {
    axes: [Axis; CTLS],
    /// Whether key releases are reported, in which case a held key is held
    /// and the grace period is not used.
    trust_release: bool,
}

impl Hands {
    /// Full throttle from nothing, in seconds.  Fast enough not to feel like
    /// lag on a control that is already a ramp in the car - see
    /// [`ascitty_core::drive`] - and slow enough to be a wind-on rather than
    /// a switch.
    const RISE: Fx = fixed::ratio(1, 5);
    /// And back to nothing, which is quicker: lifting off should be lifting
    /// off.
    const FALL: Fx = fixed::ratio(1, 8);
    /// How long a press stays live without being renewed, in seconds, on a
    /// terminal that cannot report a key coming up.
    ///
    /// Half a second, which is the *initial* delay before a keyboard starts
    /// autorepeating: shorter, and the first half second of holding a key is
    /// a dip, because the terminal has sent one byte and is not yet sending
    /// any more.  Measured against an emulated terminal at the system
    /// defaults - 500 ms to the first repeat, then one every 33 ms - a
    /// quarter-second grace read 43 and 52 mph at the two moments where half
    /// a second read 58 and 84.
    ///
    /// It is also the whole of what makes two keys at once work here.  A
    /// terminal autorepeats the most recently pressed key *only*, so holding
    /// `w` and then pressing `q` stops `w` arriving at all, and the grace is
    /// what keeps the throttle on while you turn.  The price is that a tap
    /// lingers for half a second, and the fix for that is not a shorter
    /// grace - it is a terminal that reports releases, where none of this is
    /// consulted.  See [`term::Term::holds_keys`].
    const GRACE: Fx = fixed::ratio(1, 2);

    fn at(&self, c: Ctl) -> Fx {
        self.axes[c as usize].level
    }

    fn press(&mut self, c: Ctl) {
        self.axes[c as usize].press();
    }

    fn release(&mut self, c: Ctl) {
        self.axes[c as usize].release();
    }

    /// Let go of everything.  Used when the view changes: the release for a
    /// key held across a change would be delivered to whatever that key does
    /// in the *new* view, and the old one would stay down forever.
    fn open(&mut self) {
        self.axes = [Axis::default(); CTLS];
    }

    /// One frame of ramp, at whatever rate the program is running.
    fn tick(&mut self, hz: i32) {
        let hz = fixed::from_int(hz.max(1));
        let up = fixed::div(ONE, fixed::mul(Hands::RISE, hz)).max(1);
        let down = fixed::div(ONE, fixed::mul(Hands::FALL, hz)).max(1);
        let grace = fixed::floor(fixed::mul(Hands::GRACE, hz)).clamp(1, 254) as u8;
        for a in self.axes.iter_mut() {
            a.tick(up, down, grace, self.trust_release);
        }
    }

    /// What the car is being asked to do.
    ///
    /// Both pairs of steering keys add, and the sum is clamped, so holding
    /// `q` and `Left` is not two lots of lock.
    fn controls(&self) -> Controls {
        let left = self.at(Ctl::Left) + self.at(Ctl::StrafeLeft);
        let right = self.at(Ctl::Right) + self.at(Ctl::StrafeRight);
        Controls {
            throttle: fixed::clamp(self.at(Ctl::Gas) - self.at(Ctl::Brake), -ONE, ONE),
            steer: fixed::clamp(right - left, -ONE, ONE),
            handbrake: self.at(Ctl::Up) > fixed::HALF,
        }
    }
}

/// Which control a key works, in a given view.
///
/// `wasd` is the vehicle in every mode - forward, back, and left and right
/// meaning whatever left and right mean to the thing you are in: the wheel
/// in the cab, a step sideways on foot and in the air.  It never changes,
/// which is the point of it.
///
/// The arrows are the *view*, and what a view is differs by mode.  Behind
/// the cab they swing the camera round the car - the driver looking about
/// rather than the car turning - while up and down stay on the pedals,
/// because the chase camera sets its own pitch every frame and there is
/// nothing there for them to look at.  On foot and in the helicopter,
/// looking about is what `q` and `e` already do, so left and right go to
/// the other useful thing instead and move you sideways.
fn control_for(k: Key, view: View) -> Option<Ctl> {
    let driving = view == View::Drive;
    Some(match k {
        Key::Char('w') => Ctl::Gas,
        Key::Char('s') => Ctl::Brake,
        Key::Char('a') => Ctl::StrafeLeft,
        Key::Char('d') => Ctl::StrafeRight,
        Key::Char('q') => Ctl::Left,
        Key::Char('e') => Ctl::Right,
        Key::Char(' ') => Ctl::Up,
        Key::Char('z') => Ctl::Down,
        Key::Left if driving => Ctl::PanLeft,
        Key::Right if driving => Ctl::PanRight,
        Key::Up if driving => Ctl::Gas,
        Key::Down if driving => Ctl::Brake,
        Key::Left => Ctl::StrafeLeft,
        Key::Right => Ctl::StrafeRight,
        Key::Up => Ctl::LookUp,
        Key::Down => Ctl::LookDown,
        _ => return None,
    })
}

/// The driver's head, which does not accelerate with the car.
///
/// # What it models
///
/// A head is on a neck, and a neck is a spring.  Get on the throttle and the
/// head is left behind - the chin comes up, and you see more sky than road.
/// Stand on the brake and it is thrown forward, and you see more road than
/// sky.  Hold a speed, any speed, and it sits where it always sat: the lean
/// answers to *acceleration*, never to how fast the car is going, which is
/// why a hundred and fifty miles an hour down a straight looks exactly like
/// standing still and why the moment you lift off is the moment you feel.
///
/// # Why it is a spring and not a number
///
/// Reading the acceleration straight onto the camera gives a horizon that
/// jumps the instant the throttle does, which reads as the *picture*
/// twitching rather than as the driver moving.  A second-order response has
/// somewhere to be and takes time to get there, so a stab of throttle sends
/// the head back, past where it settles, and down again over about half a
/// second.  That overshoot is the whole effect: it is the difference between
/// a camera that is told about the acceleration and a head that is subject
/// to it.
///
/// Deliberately underdamped, at about three quarters of critical.  Damped
/// harder there is no bob, only a lean; damped less it wobbles for a second
/// after every gearchange the car does not have.
#[derive(Default)]
struct Head {
    /// Where the head is, from -1 flat back to +1 hard forward.
    lean: Fx,
    /// How fast it is going there.
    rate: Fx,
    /// Last frame's forward speed, for working out the acceleration.
    /// There is no other way to get it: the car integrates a force it does
    /// not keep.
    last: Fx,
    /// Whether `last` means anything yet.
    started: bool,
}

impl Head {
    /// Acceleration, in units per second per second, that leans the head all
    /// the way over.
    ///
    /// Fourteen, against an engine that pulls ten from a standstill and
    /// brakes that pull forty-four.  So flat out from rest is about seven
    /// tenths of the travel and standing on the brakes is all of it, which is
    /// the right way round: a car dives under braking far harder than it
    /// squats under power, and so does the person in it.
    const FULL: Fx = fixed::ratio(14, 1);
    /// Spring rate, per second squared.
    const STIFF: Fx = fixed::ratio(60, 1);
    /// Damping, per second.  Three quarters of critical for this stiffness,
    /// which is a single overshoot and no second one.
    const DAMP: Fx = fixed::ratio(6, 1);
    /// How far the horizon moves at full lean, in screen rows, and the least
    /// it may be.
    ///
    /// A ninth of the frame, floored at four rows.  It is a fraction rather
    /// than a count because a fixed number of rows is a different amount of
    /// *picture* in a twenty-four row window than in a sixty row one, and it
    /// has a floor because below about three rows the horizon does not move
    /// so much as flicker.
    ///
    /// What that works out to is the figure worth checking: flat out from
    /// rest the head settles at about seven tenths of its travel, which on a
    /// forty-row frame is three rows of sky, and standing on the brakes uses
    /// all of it, which is four.  Braking bigger than accelerating is not an
    /// accident - it is what the numbers in [`Head::FULL`] say and what a car
    /// does.
    const ROWS_MIN: i32 = 4;
    /// The fraction of the frame the lean is worth, as a divisor.
    const ROWS_OF_FRAME: i32 = 9;

    /// Advance the head one tick, and say how far over it is.
    fn step(&mut self, car: &ascitty_core::drive::Car, hz: i32) -> Fx {
        let inv = fixed::div(ONE, fixed::from_int(hz.max(1)));
        // Forward speed, along the car's own nose: the acceleration that
        // moves a head is the one it is strapped in line with, and a car
        // that is sliding sideways is not accelerating its driver forwards.
        let (fx, fy) = (ascitty_core::trig::cos(car.yaw), ascitty_core::trig::sin(car.yaw));
        let vf = fixed::mul(car.vx, fx) + fixed::mul(car.vy, fy);
        let accel = if self.started {
            fixed::div(vf - self.last, inv)
        } else {
            self.started = true;
            0
        };
        self.last = vf;

        // Where the head wants to be.  Backwards under power, forwards under
        // braking, and clamped, because a collision is an acceleration of a
        // hundred and the neck only has so far to go.
        let want = fixed::clamp(-fixed::div(accel, Head::FULL), -ONE, ONE);
        self.rate += fixed::mul(fixed::mul(want - self.lean, Head::STIFF), inv);
        self.rate -= fixed::mul(fixed::mul(self.rate, Head::DAMP), inv);
        self.lean += fixed::mul(self.rate, inv);
        self.lean = fixed::clamp(self.lean, -ONE, ONE);
        self.lean
    }

    /// The lean as whole screen rows of horizon, for a frame this tall.
    ///
    /// Rounded rather than truncated, because the renderer can only shear
    /// the horizon by whole rows and truncation is not symmetric about zero:
    /// two and a bit rows of lean became three rows of sky one way and two
    /// of road the other, so the car appeared to dive less than it squatted.
    fn rows(&self, rows: i32) -> i32 {
        let span = (rows / Head::ROWS_OF_FRAME).max(Head::ROWS_MIN);
        fixed::floor(fixed::mul(self.lean, fixed::from_int(span)) + fixed::HALF)
    }
}

/// Put the camera behind the taxi.
///
/// Two things make this feel like a driving camera rather than a camera
/// bolted to a car.  The heading *lags* the car's, so a flick of the wheel
/// swings the view a moment later and a drift is watched from the outside
/// rather than from inside the spin.  And the boom is shortened until it is
/// out of a building, because a chase camera that clips through a wall shows
/// you the inside of the wall at exactly the moment you most need to see the
/// road.
/// How far round the car the camera swings at full deflection.
///
/// A quarter turn, which is enough to look down the cross street you are
/// arriving at and not so far that you lose which way the car is pointing.
/// It is applied to the heading the camera is *chasing* rather than to the
/// camera, so the existing lag pans it round smoothly and returns it to
/// centre when the key comes up, and the boom swings with it - the camera
/// orbits the cab rather than turning its back on it.
const PAN: i32 = trig::QUARTER as i32;

fn chase(cam: &mut Camera, sim: &Sim, city: &City, rows: i32, head: &mut Head, hz: i32, pan: Fx) {
    let target = sim
        .taxi
        .yaw
        .wrapping_add((((pan as i64) * (PAN as i64)) >> 16) as Ang);
    let delta = target.wrapping_sub(cam.yaw) as i16 as i32;
    cam.yaw = cam.yaw.wrapping_add((delta / 6) as u16);

    let (dx, dy) = cam.dir();
    // Measured from the car rather than fixed, because the car changed
    // length and this did not: at two and a quarter cells the boom was
    // barely longer than the cab's own half-length once the vehicles were
    // doubled, so the camera sat inside the boot and the frame was a
    // yellow wall.
    //
    // Halved since - one and a quarter car-lengths rather than two and a
    // half.  A long boom puts the cab in the middle distance and the game in
    // the middle of a wide shot; a short one puts you behind the car.
    let want = fixed::mul(sim.taxi.kind.half_len(), fixed::ratio(5, 2)) + fixed::ratio(1, 4);
    let mut boom = want;
    while boom > fixed::ratio(1, 4) {
        let x = sim.taxi.x - fixed::mul(dx, boom);
        let y = sim.taxi.y - fixed::mul(dy, boom);
        if city.open(fixed::floor(x), fixed::floor(y)) {
            cam.x = x;
            cam.y = y;
            break;
        }
        boom -= fixed::ratio(1, 4);
    }
    if boom <= fixed::ratio(1, 4) {
        cam.x = sim.taxi.x;
        cam.y = sim.taxi.y;
    }
    // Head height for a car, above whatever the ground is doing here.
    //
    // A quarter higher than it was, over a boom half as long.  Both changes
    // point the same way: from further back and lower down you are a camera
    // following a car, and from nearer and higher up you are looking over
    // its roof at the road, which is the shot this game is.
    //
    // The two do fight over one thing, and the number is a compromise
    // between them.  The car's foot lands `eye x scale / boom` rows below
    // the horizon, so halving the boom doubles that and raising the eye
    // adds to it again: at six fifths of a cell the cab's whole lower half
    // was off the bottom of a forty-row frame.  At one cell it keeps about
    // four fifths of the car and gives the rest of the frame to the street,
    // which is what the height was raised for.
    cam.z = city.ground(fixed::floor(cam.x), fixed::floor(cam.y)) + ONE;
    // Looking slightly down at the road, plus wherever the driver's head has
    // got to.  Positive pitch is down, so a head thrown back by the throttle
    // subtracts and you see sky.
    head.step(&sim.taxi, hz);
    cam.pitch = rows / 10 + head.rows(rows);
}

struct Opts {
    seed: u32,
    mode: Mode,
    depth: Depth,
    size: Option<(usize, usize)>,
    fps: u32,
    atmos: Atmos,
    shot: Option<u32>,
    /// Where to write the shot as a picture instead of as text.
    png: Option<std::path::PathBuf>,
    bench: bool,
    view: View,
    fov: f64,
    tour: bool,
    /// Whether the demonstration walks rather than drives.
    walk_demo: bool,
    /// Whether the user asked for a demonstration in so many words.
    demo_asked: bool,
    anim: bool,
    frames: u32,
    record: Option<std::path::PathBuf>,
    /// Where to write the demonstration as an animated GIF.
    gif: Option<std::path::PathBuf>,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            seed: ascitty_core::DEFAULT_SEED,
            mode: Mode::Unicode,
            depth: Depth::detect(),
            size: None,
            fps: 30,
            atmos: Atmos::default(),
            shot: None,
            png: None,
            bench: false,
            // The game, driving, with the cab driving itself until you
            // touch a key.  This is what the thing is: starting somebody on
            // a pavement with a static view of a street and leaving them to
            // find the `t` key was a demonstration of the renderer, not of
            // the city.
            view: View::Drive,
            fov: 67.0,
            tour: true,
            walk_demo: false,
            demo_asked: false,
            anim: false,
            frames: 900,
            record: None,
            gif: None,
        }
    }
}

const USAGE: &str = "\
ascitty - a raytraced ASCII city

USAGE: ascitty [options]

  --seed N          city to generate (default: a fixed one, so runs match)
  --mode M          ascii | unicode           (default: unicode)
  --color D         true | 16 | none          (default: from $COLORTERM)
  --size WxH        fix the frame size; without it the frame is the window
                    and follows it when you resize
  --fps N           frame rate cap            (default: 30)
  --fov DEGREES     horizontal field of view  (default: 67)
  --rain N          0 dry .. 8 torrential     (default: 0, dry)
  --haze N          0 clear .. 8 soup         (default: 3)
  --stars N         0 .. 8                    (default: 4)
  --no-moon         moonless night
  --day TICKS       length of one turn of the sky   (default: 7200, 4 min)
                    0 holds it still
  --sky N           start at phase N of 12: 0 night, 3 sunrise, 5 noon,
                    8 sunset - see --sky list
  --walk            get out and walk instead of driving
  --copter          start above the city instead of behind the wheel
  --drive           behind the wheel of the taxi          (the default)
  --play            take the wheel from the start, with no autopilot
  --shot [N]        render N frames, print the last as plain text, exit
  --png FILE        write that shot as a picture instead of printing it
  --bench           render 200 frames as fast as possible and report
  -V, --version     which build this is
  -h, --help        this

DRIVING ITSELF
  This is how it starts.  The cab takes fares on its own - it picks one,
  plans a route, drives the right-hand lane to it and pulls up at the
  circle - and the moment you touch a key you are driving instead.  `\\`
  hands it back.  `--play` starts you at the wheel; `--tour` and `--demo`
  ask for the autopilot explicitly, which is what it does anyway.

  --anim            play the demonstration and exit; --frames says how long
  --record FILE     write the demonstration to an asciinema .cast file
  --gif FILE        write it as an animated GIF as well, or instead
  --frames N        frames for --anim and --record   (default: 900, 30 s)

CONTROLS
  Any of these takes the car off the autopilot.  You are the driver from
  the first key, and the clock does not wait for you to be ready.

  `wasd` is the vehicle, wherever you are: forward, back, and left and
  right meaning whatever they mean to the thing you are in.  The arrows are
  the view.

  DRIVING
  w  or up        throttle           s  or down      brake, then reverse
  a  or q         steer left         d  or e         steer right
  left  right     swing the camera round the cab, and let go to centre it
  space           handbrake          t               get out and walk

  Hold them.  The throttle winds on while it is down and the engine pulls
  hardest low down, so the top of the range takes about a second and three
  quarters to reach - and `a` and `w` together is a left-hander taken under
  power, which is what holding two keys at once is for.

  ON FOOT AND IN THE AIR
  w s             forward, back      q e             turn
  a d, left right step sideways      up down         look
  space  z        up, down           c               walk / copter
  t               get in the taxi

  ANY TIME
  g          ascii / unicode glyphs   m          moon
  1-9 0      rain, 0 dry              h          haze
  \\          back to the autopilot    esc        quit

  Two keys at once wants a terminal that reports key releases - kitty,
  ghostty, WezTerm, foot and others do, and this asks yours at startup.
  Where it is not on offer, a press is held for half a second and topped up
  by the terminal's own autorepeat, which is close but is not the same
  thing.

  A terminal cannot see a bare Shift - it sends no bytes at all - so `z`
  descends where you might expect Shift to.
";

fn parse_args() -> Result<Opts, String> {
    let mut o = Opts::default();
    // Applied at the end, because it depends on `--day` and the two may
    // arrive in either order.
    let mut sky: Option<u32> = None;
    let mut args = std::env::args().skip(1).peekable();
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            // Which build this is, in the form the pictures in the README
            // are captioned with, so a frame can be tied to the code that
            // drew it.
            "-V" | "--version" => {
                println!("ascitty {} (seed {:#010x})", ascitty_core::VERSION, ascitty_core::DEFAULT_SEED);
                std::process::exit(0);
            }
            "--seed" => o.seed = val()?.parse().map_err(|_| "bad --seed".to_string())?,
            "--mode" => {
                let v = val()?;
                o.mode = Mode::parse(&v).ok_or(format!("unknown mode {v}"))?;
            }
            "--color" | "--colour" => {
                let v = val()?;
                o.depth = Depth::parse(&v).ok_or(format!("unknown colour depth {v}"))?;
            }
            "--size" => {
                let v = val()?;
                let (w, h) = v.split_once('x').ok_or("--size wants WxH")?;
                o.size = Some((
                    w.parse().map_err(|_| "bad width")?,
                    h.parse().map_err(|_| "bad height")?,
                ));
            }
            "--fps" => o.fps = val()?.parse().map_err(|_| "bad --fps".to_string())?,
            "--fov" => o.fov = val()?.parse().map_err(|_| "bad --fov".to_string())?,
            "--rain" => o.atmos.rain = val()?.parse::<u8>().map_err(|_| "bad --rain")?.min(8),
            "--haze" => o.atmos.haze = val()?.parse::<u8>().map_err(|_| "bad --haze")?.min(8),
            "--stars" => o.atmos.stars = val()?.parse::<u8>().map_err(|_| "bad --stars")?.min(8),
            "--no-moon" => o.atmos.moon = false,
            "--day" => o.atmos.day = val()?.parse().map_err(|_| "bad --day".to_string())?,
            "--sky" => {
                let v = val()?;
                if v == "list" {
                    for (i, p) in ascitty_core::atmos::DAY.iter().enumerate() {
                        println!("{i:2}  {}", p.name);
                    }
                    std::process::exit(0);
                }
                // Wind the clock forward to the start of that phase.  The
                // tick counter drives the sky and nothing else cares where
                // it starts, so there is no state to add for this.
                let n: u32 = v.parse().map_err(|_| "bad --sky".to_string())?;
                sky = Some(n);
            }
            "--copter" => o.view = View::Copter,
            "--drive" => o.view = View::Drive,
            "--walk" => {
                o.walk_demo = true;
                o.view = View::Walk;
            }
            // Skip the attract mode and start driving.
            "--play" | "--no-demo" => o.tour = false,
            // Both words, because both are in use: the Makefile target and
            // the Plus/4 build call it a demo, this flag called it a tour,
            // and a user should not have to know which half of the project
            // they are talking to.
            "--tour" | "--demo" => {
                o.tour = true;
                o.demo_asked = true;
            }
            "--anim" => {
                o.tour = true;
                o.demo_asked = true;
                o.anim = true;
            }
            "--frames" => o.frames = val()?.parse().map_err(|_| "bad --frames".to_string())?,
            "--record" => {
                o.record = Some(std::path::PathBuf::from(val()?));
                o.tour = true;
                o.demo_asked = true;
            }
            "--gif" => {
                o.gif = Some(std::path::PathBuf::from(val()?));
                o.tour = true;
                o.demo_asked = true;
            }
            "--png" => {
                o.png = Some(std::path::PathBuf::from(val()?));
                o.shot = o.shot.or(Some(1));
            }
            "--bench" => o.bench = true,
            "--shot" => {
                let n = match args.peek() {
                    Some(s) if s.parse::<u32>().is_ok() => args.next().unwrap().parse().unwrap(),
                    _ => 1,
                };
                o.shot = Some(n);
            }
            _ => return Err(format!("unknown option {a}\n\n{USAGE}")),
        }
    }
    if let Some(n) = sky {
        o.atmos.sky_offset = Atmos::phase_offset(n, o.atmos.day);
    }
    // The helicopter is a thing you fly, not a thing that flies itself:
    // there is no demonstration of it, and the walking tour would take the
    // camera straight back down to the pavement.  So asking to be in the air
    // turns the autopilot off, unless a demonstration was asked for in so
    // many words as well.
    if o.view == View::Copter && !o.demo_asked {
        o.tour = false;
    }
    Ok(o)
}

fn main() {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("ascitty: {e}");
            std::process::exit(2);
        }
    };
    if let Err(e) = run(opts) {
        term::restore();
        eprintln!("ascitty: {e}");
        std::process::exit(1);
    }
}

/// Above every roof, which is where the copter starts and the lowest it is
/// allowed to fly - the point of the mode is to look *down*.
/// Point the camera at the city below it, for a frame this size.
///
/// The copter is the one view where the horizon is not the subject.  What
/// the pitch has to be depends on how high the camera is, how wide the frame
/// is - the lens is fixed but a wider frame is more rows per world unit -
/// and how far the haze lets you see, so it is worked out rather than
/// guessed.  See [`raycast::pitch_down`].
fn aim_at_city(cam: &mut Camera, city: &City, atmos: &Atmos, w: usize, h: usize) {
    let eye = cam.z - city.ground(fixed::floor(cam.x), fixed::floor(cam.y));
    cam.pitch = raycast::pitch_down(
        w as i32,
        h as i32,
        cam.fov,
        eye,
        ascitty_core::atmos::draw_distance(atmos.haze),
    );
}

/// How far down the camera may be tilted in this view.
///
/// Walking and driving keep the horizon on the screen, because a view of
/// nothing but pavement is not a view.  The copter is the opposite case: it
/// is *looking at the ground*, its horizon is off the top of the frame on
/// purpose, and clamping it to the walking rule was enough on its own to
/// point it back at the empty sky.  Past the aim there is nothing further to
/// see - only the same ground, stretched - so one frame beyond it is as far
/// as the limit needs to go.
fn tilt_limit(view: View, cam: &Camera, city: &City, atmos: &Atmos, w: usize, h: usize) -> i32 {
    match view {
        View::Copter => {
            let mut aimed = *cam;
            aim_at_city(&mut aimed, city, atmos, w, h);
            aimed.pitch.abs() + h as i32
        }
        _ => h as i32 / 3,
    }
}

fn ceiling_of(city: &City) -> Fx {
    let tallest = city.lots.iter().map(|l| l.height).max().unwrap_or(20);
    fixed::from_int(tallest as i32 + 6)
}

/// The HUD layer over a driving frame: the arrow on the road.
///
/// All three paths that draw the game - the interactive one, `--shot` and
/// the recorder - call this, because a picture of the game without the thing
/// you steer by is a picture of a different program.
fn hud_layer(f: &mut Frame, sim: &Sim, cam: &Camera, atmos: &Atmos, p: &raycast::Proj) {
    if let Some((tx, ty)) = sim.target() {
        let want = ascitty_core::sim::atan2_approx(ty - cam.y, tx - cam.x);
        let rel = want.wrapping_sub(cam.yaw) as i16 as i32;
        hud::arrow_on_the_road(f, p, cam.fov, rel, atmos.tick);
    }
}

fn run(mut o: Opts) -> Result<(), String> {
    let city = City::generate(o.seed);
    let mut cam = Camera::spawn(&city, SIZE as i32 / 2, SIZE as i32 / 2);
    // One lens, applied to every camera in the program.  Set on the tour's
    // camera as well as this one: the autopilot owns its own, and a --fov
    // that quietly does nothing on a recorded tour is worse than no --fov.
    let lens = ascitty_core::camera::fov_for_degrees(o.fov);
    cam.fov = lens;
    let mut sim = Sim::new(&city, o.seed);
    // The cab waits where you start, not where the middle of the map is.
    sim.park_near(&city, fixed::floor(cam.x), fixed::floor(cam.y));
    let mut view = o.view;
    match view {
        View::Copter => {
            // Height here, aim once the frame size is known: the pitch that
            // looks at the city depends on how many rows there are.
            cam.z = ceiling_of(&city);
        }
        View::Drive => {
            // The cab is already parked at the kerb nearest where you came
            // in - see `park_near` - and the camera goes to *it*, not the
            // other way round.  Moving the car to the camera used to be
            // harmless because both spawned on the carriageway; now that a
            // person spawns on the pavement it would park the taxi on the
            // paving and start the shift with two wheels on a kerb.
            cam.yaw = sim.taxi.yaw;
            cam.x = sim.taxi.x;
            cam.y = sim.taxi.y;
        }
        View::Walk => {}
    }


    // Headless paths first: neither needs a terminal, which is what makes
    // them usable from a Makefile and from CI.
    if let Some(n) = o.shot {
        let (w, h) = o.size.unwrap_or((100, 34));
        if view == View::Copter {
            aim_at_city(&mut cam, &city, &o.atmos, w, h);
        }
        let mut f = Frame::new(w, h);
        let mut depth = Vec::new();
        let mut events = Vec::new();
        let mut tour = Tour::new(&city, o.seed);
        tour.cam.fov = lens;
        let mut cabbie = Cabbie::new();
        let mut head = Head::default();
        let hz = o.fps.max(1) as i32;
        for _ in 0..n {
            o.atmos.step();
            if o.tour && view == View::Drive {
                let c = cabbie.drive(&city, &sim, hz);
                sim.step(&city, &c, hz, &mut events);
                chase(&mut cam, &sim, &city, h as i32, &mut head, hz, 0);
            } else if o.tour {
                tour.step(&city, hz);
                cam = tour.cam;
                cam.pitch = cam.pitch.clamp(-(h as i32 / 3), h as i32 / 3);
            } else if view == View::Drive {
                sim.step(&city, &Controls { throttle: ONE, ..Default::default() }, hz, &mut events);
                chase(&mut cam, &sim, &city, h as i32, &mut head, hz, 0);
            }
            raycast::render_to(&city, &cam, &o.atmos, &mut f, &mut depth);
            let proj = raycast::projection(&city, &cam, &f);
            sim.draw(&mut f, &depth, &cam, &o.atmos, &proj);
            o.atmos.rain_over(&mut f, &cam);
            if view == View::Drive {
                hud_layer(&mut f, &sim, &cam, &o.atmos, &proj);
            }
        }
        if let Some(path) = &o.png {
            std::fs::write(path, png::encode(&f)).map_err(|e| format!("{}: {e}", path.display()))?;
            eprintln!("{}", path.display());
        } else {
            print!("{}", paint::plain(&f, o.mode));
        }
        return Ok(());
    }
    if o.record.is_some() || o.gif.is_some() {
        // One loop, two sinks.  A .cast and a .gif are the same
        // demonstration in two containers, and running the city twice to
        // get them would let the two recordings differ.
        let (w, h) = o.size.unwrap_or((120, 36));
        if view == View::Copter {
            aim_at_city(&mut cam, &city, &o.atmos, w, h);
        }
        let mut f = Frame::new(w, h);
        let mut buf = String::new();
        let mut depth: Vec<Fx> = Vec::new();
        let mut tour = Tour::new(&city, o.seed);
        tour.cam.fov = lens;
        let mut cabbie = Cabbie::new();
        let mut head = Head::default();
        let mut events: Vec<Event> = Vec::new();
        let hz = o.fps.max(1) as i32;
        let mut rec = match &o.record {
            Some(path) => Some(
                cast::Recorder::create(path, w, h, o.fps)
                    .map_err(|e| format!("cannot write {}: {e}", path.display()))?,
            ),
            None => None,
        };
        let mut anim = o.gif.as_ref().map(|_| gif::Gif::new(o.fps));
        // A clear and a cursor home first, so a player starting mid-stream
        // does not inherit whatever was on the terminal before.
        if let Some(rec) = rec.as_mut() {
            rec.frame("\x1b[2J\x1b[H").map_err(|e| e.to_string())?;
        }
        for _ in 0..o.frames {
            o.atmos.step();
            // Whichever demonstration was asked for drives the camera; the
            // rest of the loop does not care which.
            if view == View::Drive {
                let c = cabbie.drive(&city, &sim, hz);
                sim.step(&city, &c, hz, &mut events);
                chase(&mut cam, &sim, &city, h as i32, &mut head, hz, 0);
            } else {
                tour.step(&city, hz);
                cam = tour.cam;
                cam.pitch = cam.pitch.clamp(-(h as i32 / 3), h as i32 / 3);
            }
            raycast::render_to(&city, &cam, &o.atmos, &mut f, &mut depth);
            let proj = raycast::projection(&city, &cam, &f);
            sim.draw(&mut f, &depth, &cam, &o.atmos, &proj);
            o.atmos.rain_over(&mut f, &cam);
            if view == View::Drive {
                hud_layer(&mut f, &sim, &cam, &o.atmos, &proj);
            }
            if let Some(rec) = rec.as_mut() {
                paint::paint(&f, o.mode, o.depth, &mut buf);
                rec.frame(&buf).map_err(|e| e.to_string())?;
            }
            if let Some(anim) = anim.as_mut() {
                anim.push(&f);
            }
        }
        if let (Some(rec), Some(path)) = (rec, &o.record) {
            let (n, secs) = rec.finish().map_err(|e| e.to_string())?;
            let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            eprintln!(
                "{}  {n} frames  {secs:.1}s  {} KB\n  play it:  asciinema play {}",
                path.display(),
                bytes / 1024,
                path.display()
            );
        }
        if let (Some(anim), Some(path)) = (anim, &o.gif) {
            let n = anim.frames();
            let bytes = anim.finish();
            let kb = bytes.len() / 1024;
            std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            eprintln!(
                "{}  {n} frames  {:.1}s  {kb} KB",
                path.display(),
                n as f32 / o.fps.max(1) as f32
            );
        }
        return Ok(());
    }
    if o.bench {
        let (w, h) = o.size.unwrap_or((160, 48));
        let mut f = Frame::new(w, h);
        let mut buf = String::new();
        let t0 = std::time::Instant::now();
        const N: u32 = 200;
        for i in 0..N {
            o.atmos.step();
            cam.turn(200);
            let st = raycast::render(&city, &cam, &o.atmos, &mut f);
            paint::paint(&f, o.mode, o.depth, &mut buf);
            if i == 0 {
                println!("{w}x{h}  {} cells  {} steps/frame", w * h, st.steps);
            }
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0 / N as f64;
        println!("{ms:.2} ms/frame  =  {:.0} fps", 1000.0 / ms);
        return Ok(());
    }

    let term = Term::enter().map_err(|e| format!("cannot set up the terminal: {e}"))?;
    let keys = Keys::start();
    let (mut w, mut h) = o.size.unwrap_or_else(Term::size);
    let mut f = Frame::new(w, h.saturating_sub(1).max(1));
    let mut buf = String::new();

    let dt = std::time::Duration::from_micros(1_000_000 / o.fps.clamp(1, 240) as u64);
    let step = fixed::div(WALK_SPEED, fixed::from_int(o.fps as i32));
    let turn = TURN_SPEED / o.fps as i32;
    let mut stats = raycast::Stats::default();
    let mut fps_ms = 0.0f64;
    let mut quit = false;
    let mut depth: Vec<Fx> = Vec::new();
    let mut hands = Hands { trust_release: term.holds_keys, ..Default::default() };
    // The fraction of a row of tilt left over from the last frame.
    let mut tilt_carry: Fx = 0;
    let mut tour = Tour::new(&city, o.seed);
    tour.cam.fov = lens;
    let mut cabbie = Cabbie::new();
    let mut head = Head::default();
    let mut autopilot = o.tour;
    // The walking demonstration starts where the walker starts; the driving
    // one starts in the cab, which is already parked next to you.
    if autopilot && view != View::Drive {
        cam = tour.cam;
    }
    // --anim plays a fixed length and exits; otherwise it runs until you
    // press escape.
    let mut left = if o.anim { o.frames } else { u32::MAX };
    let mut events: Vec<Event> = Vec::new();
    let mut flash: Option<(&'static str, i32)> = None;
    // Frames since the start, for things that happen on a timer rather than
    // every frame, and the size the terminal last said it was.
    let mut frame: u32 = 0;
    let mut resize: Option<(usize, usize)> = None;

    while !quit {
        let t0 = std::time::Instant::now();

        // Input.  Every control is an axis that winds on while its key is
        // down and bleeds away when it is not - see `Axis` - so holding two
        // of them at once is two controls held, and the throttle is
        // something you lean on rather than something you switch.
        hands.tick(o.fps.max(1) as i32);
        for st in keys.drain() {
            let down = st.edge != Edge::Release;
            // Any movement key takes the camera back off the autopilot -
            // there is no mode to leave, you just start driving.  So does
            // changing the view: asking for the helicopter and getting a
            // camera that carries on walking is a key that appears to do
            // nothing.
            let takes_over = control_for(st.key, view).is_some()
                || matches!(st.key, Key::Char('t' | 'c'));
            if autopilot && down && takes_over {
                autopilot = false;
            }
            if let Some(c) = control_for(st.key, view) {
                if down {
                    hands.press(c);
                } else {
                    hands.release(c);
                }
                continue;
            }
            // Everything else is a switch, and a switch is thrown on the way
            // down only: acting on the release as well turns every one of
            // them into two presses on a terminal that reports releases.
            if !down || st.edge == Edge::Repeat {
                continue;
            }
            match st.key {
                Key::Quit => quit = true,
                // Not a key: the terminal answering how big it is.
                Key::Size(nw, nh) => resize = Some((nw, nh)),
                Key::Char('\\') => {
                    // ...and this hands it back, from wherever you are now.
                    // Back to the same demonstration you were watching: a
                    // key that silently changes what you are looking at is
                    // worse than one that does nothing.
                    if view != View::Drive {
                        tour = Tour::new(&city, o.seed);
                        tour.cam = cam;
                        view = View::Walk;
                    }
                    cabbie = Cabbie::new();
                    autopilot = true;
                    hands.open();
                }
                // Get in the cab, or get out of it.  Its own key rather
                // than a position in a cycle: driving is a mode you enter
                // deliberately, and having to pass through the helicopter
                // to reach it is silly.
                Key::Char('t') => {
                    if view == View::Drive {
                        view = View::Walk;
                        cam.x = sim.taxi.x;
                        cam.y = sim.taxi.y;
                        cam.yaw = sim.taxi.yaw;
                        cam.pitch = 0;
                        cam.stand(&city);
                    } else {
                        view = View::Drive;
                        // The cab is where it is parked; you are put in it
                        // rather than it being brought to you.
                        cam.yaw = sim.taxi.yaw;
                    }
                    hands.open();
                }
                // Walk and fly.  Driving is not in this cycle.
                Key::Char('c') => {
                    view = if view == View::Copter {
                        cam.pitch = 0;
                        View::Walk
                    } else {
                        cam.z = ceiling_of(&city);
                        aim_at_city(&mut cam, &city, &o.atmos, f.w, f.h);
                        View::Copter
                    };
                    hands.open();
                }
                Key::Char('m') => o.atmos.moon = !o.atmos.moon,
                Key::Char('h') => o.atmos.haze = (o.atmos.haze + 1) % 9,
                Key::Char('g') => {
                    o.mode = match o.mode {
                        Mode::Ascii => Mode::Unicode,
                        Mode::Unicode => Mode::Ascii,
                    }
                }
                Key::Char(c) if c.is_ascii_digit() => {
                    o.atmos.rain = c.to_digit(10).unwrap() as u8;
                }
                _ => {}
            }
        }

        // The axes, as movement.  Walking and flying steer the camera
        // directly; driving hands the same three controls to the car.
        let fwd = fixed::mul(hands.at(Ctl::Gas) - hands.at(Ctl::Brake), step);
        let side = fixed::mul(hands.at(Ctl::StrafeRight) - hands.at(Ctl::StrafeLeft), step);
        let rise = fixed::mul(hands.at(Ctl::Up) - hands.at(Ctl::Down), step);
        if view != View::Drive && !autopilot {
            let spin = hands.at(Ctl::Right) - hands.at(Ctl::Left);
            cam.turn(((turn as i64 * spin as i64) >> 16) as i32);
            // The head tilts at a rate, and rows are whole numbers, so the
            // fraction is carried rather than thrown away - at thirty frames
            // a second a rate of twenty rows a second is two thirds of a row
            // per frame, and dropping that is a camera that never looks up.
            let look = hands.at(Ctl::LookDown) - hands.at(Ctl::LookUp);
            tilt_carry += fixed::div(fixed::mul(look, LOOK_ROWS), fixed::from_int(o.fps.max(1) as i32));
            let whole = fixed::floor(tilt_carry);
            if whole != 0 {
                tilt_carry -= fixed::from_int(whole);
                let l = tilt_limit(view, &cam, &city, &o.atmos, f.w, f.h);
                cam.look(whole, l);
            }
        }

        if autopilot && view != View::Drive {
            tour.step(&city, o.fps.max(1) as i32);
            cam = tour.cam;
            // The autopilot does not know how tall the frame is, so its tilt
            // is clamped again here against the real one.
            cam.pitch = cam.pitch.clamp(-(f.h as i32 / 3), f.h as i32 / 3);
        }
        let tilt = tilt_limit(view, &cam, &city, &o.atmos, f.w, f.h);
        cam.pitch = cam.pitch.clamp(-tilt, tilt);
        match view {
            View::Walk if autopilot => {}
            View::Walk => {
                cam.walk(&city, fwd, side);
                cam.stand(&city);
            }
            View::Copter => {
                // Flight ignores buildings horizontally - you are above them
                // - but not the floor of the mode, which keeps the camera
                // over the roofline where the view is worth having.
                let (dx, dy) = cam.dir();
                cam.x += fixed::mul(dx, fwd * 3) + fixed::mul(-dy, side * 3);
                cam.y += fixed::mul(dy, fwd * 3) + fixed::mul(dx, side * 3);
                cam.z = (cam.z + rise * 4).clamp(fixed::from_int(4), fixed::from_int(160));
            }
            View::Drive => {
                // The driver is either you or the cabbie, and the rest of
                // the mode cannot tell: both hand over the same three
                // controls, so nothing below here is duplicated for the
                // demonstration.
                let hz = o.fps.max(1) as i32;
                let ctl = if autopilot {
                    cabbie.drive(&city, &sim, hz)
                } else {
                    hands.controls()
                };
                sim.step(&city, &ctl, hz, &mut events);
                for e in &events {
                    flash = match e {
                        Event::Flattened => Some(("CRUNCH", 20)),
                        Event::Rammed => Some(("SMASH", 20)),
                        Event::Crunched => Some(("OW", 14)),
                        Event::Coin => Some(("+2s", 10)),
                        Event::PickedUp => Some(("GO GO GO", 45)),
                        Event::DroppedOff => Some(("FARE PAID", 45)),
                        Event::TimeUp => Some(("TIME UP", 200)),
                    };
                }
                chase(&mut cam, &sim, &city, f.h as i32, &mut head, hz, hands.at(Ctl::PanRight) - hands.at(Ctl::PanLeft));
            }
        }

        // Follow the window.  The frame is whatever size the terminal is,
        // and there is no signal to wait for without a libc, so it is asked
        // - twice a second, not thirty times.
        //
        // Which way it is asked depends on what the terminal admitted to at
        // startup.  One that answers `CSI 18 t` answers on the stream we are
        // already reading and the reply arrives as `Key::Size` a frame or
        // two later; one that does not gets `stty size`, which is a fork and
        // an exec, and which this used to do *every frame*.
        if o.size.is_none() && frame.is_multiple_of((o.fps.max(1) / 2).max(1)) {
            if term.reports_size {
                term::ask_size();
            } else {
                let (nw, nh) = Term::size();
                resize = Some((nw, nh));
            }
        }
        if let Some((nw, nh)) = resize.take() {
            if (nw, nh) != (w, h) && nw > 2 && nh > 2 {
                w = nw;
                h = nh;
                f.resize(w, h.saturating_sub(1).max(1));
                buf.clear();
                print!("\x1b[2J");
            }
        }
        frame = frame.wrapping_add(1);

        o.atmos.step();
        stats = raycast::render_to(&city, &cam, &o.atmos, &mut f, &mut depth);
        let proj = raycast::projection(&city, &cam, &f);
        sim.draw(&mut f, &depth, &cam, &o.atmos, &proj);
        o.atmos.rain_over(&mut f, &cam);
        // The HUD layer: over the city, over the cab, over the weather.
        if view == View::Drive {
            hud_layer(&mut f, &sim, &cam, &o.atmos, &proj);
        }
        paint::paint(&f, o.mode, o.depth, &mut buf);
        hud::append(&mut buf, &hud::Status {
            view: match (autopilot, view) {
                (true, View::Drive) => "AUTOCAB",
                (true, _) => "TOUR",
                (false, v) => v.name(),
            },
            mode: o.mode,
            depth: o.depth,
            cam: &cam,
            atmos: &o.atmos,
            stats,
            ms: fps_ms,
            seed: o.seed,
            sim: if view == View::Drive { Some(&sim) } else { None },
            flash: flash.and_then(|(t, n)| if n > 0 { Some(t) } else { None }),
        });
        if let Some((_, n)) = flash.as_mut() {
            *n -= 1;
            if *n == 0 {
                flash = None;
            }
        }
        term::present(&buf).map_err(|e| e.to_string())?;

        if left != u32::MAX {
            left -= 1;
            if left == 0 {
                quit = true;
            }
        }

        let spent = t0.elapsed();
        fps_ms = fps_ms * 0.9 + spent.as_secs_f64() * 1000.0 * 0.1;
        if spent < dt {
            std::thread::sleep(dt - spent);
        }
    }
    let _ = stats;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thirty frames a second, which is what the program runs at.
    const HZ: i32 = 30;

    fn held(h: &mut Hands, c: Ctl, frames: u32) {
        h.press(c);
        for _ in 0..frames {
            h.tick(HZ);
        }
    }

    /// The throttle winds on rather than switching on.
    #[test]
    fn a_held_key_ramps_up() {
        let mut h = Hands { trust_release: true, ..Default::default() };
        h.press(Ctl::Gas);
        h.tick(HZ);
        let first = h.at(Ctl::Gas);
        assert!(first > 0 && first < ONE, "one frame gave {}", fixed::to_f32(first));
        for _ in 0..HZ {
            h.tick(HZ);
        }
        assert_eq!(h.at(Ctl::Gas), ONE, "a second of holding is not full throttle");
    }

    /// And winds off when the key comes up.
    #[test]
    fn a_released_key_ramps_down() {
        let mut h = Hands { trust_release: true, ..Default::default() };
        held(&mut h, Ctl::Gas, HZ as u32);
        h.release(Ctl::Gas);
        for _ in 0..HZ {
            h.tick(HZ);
        }
        assert_eq!(h.at(Ctl::Gas), 0, "it is still on the throttle");
    }

    /// The one thing the old edge-triggered input could not do: accelerate
    /// through a corner.
    #[test]
    fn two_keys_can_be_held_at_once() {
        let mut h = Hands { trust_release: true, ..Default::default() };
        h.press(Ctl::Gas);
        h.press(Ctl::Left);
        for _ in 0..HZ {
            h.tick(HZ);
        }
        let c = h.controls();
        assert_eq!(c.throttle, ONE, "not accelerating");
        assert_eq!(c.steer, -ONE, "not turning left");
    }

    /// On a terminal that cannot report releases, a press stays live for a
    /// few frames so that autorepeat reads as a held pedal - and then dies,
    /// so that letting go stops the car.
    /// On a terminal that cannot report releases, a press stays live long
    /// enough to bridge the delay before autorepeat starts - and then dies,
    /// so that letting go stops the car.
    #[test]
    fn without_release_events_a_press_outlives_the_autorepeat_delay() {
        let mut h = Hands::default();
        h.press(Ctl::Gas);
        // Half a second is the system default before the first repeat.
        for _ in 0..HZ / 2 - 1 {
            h.tick(HZ);
        }
        assert_eq!(h.at(Ctl::Gas), ONE, "it dipped waiting for the first repeat");
        for _ in 0..HZ {
            h.tick(HZ);
        }
        assert_eq!(h.at(Ctl::Gas), 0, "it never lets go");
    }

    /// Both pairs of steering keys reach the wheel, and holding both is not
    /// two lots of lock.
    ///
    /// `a` and `d` are the wheel, and `q` and `e` are the same wheel: the
    /// second pair is there because it is where a walker's turn keys are,
    /// and getting into the cab should not move your hand.
    #[test]
    fn both_pairs_of_steering_keys_reach_the_wheel() {
        assert_eq!(control_for(Key::Char('q'), View::Drive), Some(Ctl::Left));
        assert_eq!(control_for(Key::Char('a'), View::Drive), Some(Ctl::StrafeLeft));
        let mut h = Hands { trust_release: true, ..Default::default() };
        h.press(Ctl::Left);
        h.press(Ctl::StrafeLeft);
        for _ in 0..HZ {
            h.tick(HZ);
        }
        assert_eq!(h.controls().steer, -ONE);
    }

    /// `wasd` is the vehicle in every mode; the arrows are the view.
    #[test]
    fn the_arrows_are_the_view_and_wasd_is_the_vehicle() {
        for view in [View::Drive, View::Walk, View::Copter] {
            assert_eq!(control_for(Key::Char('w'), view), Some(Ctl::Gas));
            assert_eq!(control_for(Key::Char('s'), view), Some(Ctl::Brake));
            assert_eq!(control_for(Key::Char('a'), view), Some(Ctl::StrafeLeft));
            assert_eq!(control_for(Key::Char('d'), view), Some(Ctl::StrafeRight));
        }
        // Driving, the arrows swing the camera and work the pedals.
        assert_eq!(control_for(Key::Left, View::Drive), Some(Ctl::PanLeft));
        assert_eq!(control_for(Key::Right, View::Drive), Some(Ctl::PanRight));
        assert_eq!(control_for(Key::Up, View::Drive), Some(Ctl::Gas));
        assert_eq!(control_for(Key::Down, View::Drive), Some(Ctl::Brake));
        // On foot and in the air they step sideways and tilt the head.
        assert_eq!(control_for(Key::Left, View::Walk), Some(Ctl::StrafeLeft));
        assert_eq!(control_for(Key::Right, View::Copter), Some(Ctl::StrafeRight));
        assert_eq!(control_for(Key::Up, View::Walk), Some(Ctl::LookUp));
        assert_eq!(control_for(Key::Down, View::Copter), Some(Ctl::LookDown));
    }

    /// Panning swings the camera round the car, and lets go of it again.
    #[test]
    fn the_camera_swings_round_the_cab_and_comes_back() {
        use ascitty_core::sim::Sim;
        use ascitty_core::world::City;
        let city = City::generate(99);
        let sim = Sim::new(&city, 99);
        let mut cam = ascitty_core::camera::Camera::spawn(&city, 117, 117);
        let mut head = Head::default();
        for _ in 0..HZ {
            chase(&mut cam, &sim, &city, 40, &mut head, HZ, 0);
        }
        let straight = cam.yaw;
        for _ in 0..HZ {
            chase(&mut cam, &sim, &city, 40, &mut head, HZ, ONE);
        }
        let swung = (cam.yaw.wrapping_sub(straight) as i16 as i32).abs();
        assert!(
            swung > trig::QUARTER as i32 * 3 / 4,
            "a second of full pan moved the camera {swung} units of {}",
            trig::QUARTER
        );
        for _ in 0..HZ {
            chase(&mut cam, &sim, &city, 40, &mut head, HZ, 0);
        }
        let back = (cam.yaw.wrapping_sub(straight) as i16 as i32).abs();
        assert!(back < trig::QUARTER as i32 / 8, "it did not come back: {back} units off");
    }

    /// A key held across a change of view does not stay held.
    #[test]
    fn changing_view_lets_go_of_everything() {
        let mut h = Hands { trust_release: true, ..Default::default() };
        held(&mut h, Ctl::Gas, HZ as u32);
        h.open();
        h.tick(HZ);
        assert_eq!(h.at(Ctl::Gas), 0, "still on the throttle in the new view");
    }

    /// Run the head against a car accelerating at a given rate for a given
    /// number of frames, and report the lean and the rows of horizon each
    /// frame.
    fn head_run(rates: &[(Fx, i32)]) -> Vec<(Fx, i32)> {
        use ascitty_core::drive::{Car, CarKind};
        let mut car = Car::new(CarKind::Taxi, 0, 0, 0, 0);
        let mut head = Head::default();
        let mut out = Vec::new();
        for &(accel, frames) in rates {
            for _ in 0..frames {
                car.vx += fixed::div(accel, fixed::from_int(HZ));
                head.step(&car, HZ);
                out.push((head.lean, head.rows(40)));
            }
        }
        out
    }

    /// Under power the head goes back and you see sky; hard on the brakes it
    /// is thrown forward and you see road.  Positive pitch is down.
    #[test]
    fn the_head_leans_back_under_power_and_forward_under_braking() {
        let up = head_run(&[(fixed::from_int(10), HZ)]);
        let back = up.iter().map(|&(_, r)| r).min().unwrap();
        assert!(back <= -3, "flat out moved the horizon {back} rows");

        let down = head_run(&[(fixed::from_int(10), HZ), (fixed::from_int(-40), HZ / 2)]);
        let forward = down.iter().map(|&(_, r)| r).max().unwrap();
        assert!(forward >= 3, "standing on the brakes moved the horizon {forward} rows");
    }

    /// At a constant speed - any constant speed - it sits still.  The lean
    /// answers to acceleration and to nothing else, which is why a hundred
    /// and fifty miles an hour down a straight looks like standing still.
    #[test]
    fn at_a_constant_speed_the_head_is_level() {
        let r = head_run(&[(fixed::from_int(10), HZ), (0, HZ * 2)]);
        let settled = &r[r.len() - HZ as usize / 2..];
        for &(lean, rows) in settled {
            assert_eq!(rows, 0, "still leaning {} at a constant speed", fixed::to_f32(lean));
        }
    }

    /// And it does not simply slide back: it swings past level and returns,
    /// which is the difference between a head on a neck and a number on a
    /// camera.
    #[test]
    fn the_head_overshoots_and_settles() {
        let r = head_run(&[(fixed::from_int(10), HZ), (0, HZ * 2)]);
        let after: Vec<Fx> = r[HZ as usize..].iter().map(|&(l, _)| l).collect();
        let past = after.iter().copied().max().unwrap();
        assert!(
            past > fixed::ratio(1, 10),
            "it crept back to level without a bob: {} past it",
            fixed::to_f32(past)
        );
        // ...and then stops.  A neck is not a spring alone.
        let end = after[after.len() - 1];
        assert!(fixed::abs(end) < fixed::ratio(1, 50), "still ringing: {}", fixed::to_f32(end));
    }

    /// Three rows of horizon on the smallest frame anybody drives on.
    #[test]
    fn the_bob_is_visible_on_a_small_frame() {
        use ascitty_core::drive::{Car, CarKind};
        let mut car = Car::new(CarKind::Taxi, 0, 0, 0, 0);
        let mut head = Head::default();
        let mut most = 0;
        for _ in 0..HZ {
            car.vx += fixed::div(fixed::from_int(10), fixed::from_int(HZ));
            head.step(&car, HZ);
            most = most.max(-head.rows(24));
        }
        assert!(most >= 3, "only {most} rows of horizon on a 24-row frame");
    }

    /// The whole thing, through the chase camera and a real car.
    #[test]
    fn the_chase_camera_pitches_with_the_car() {
        use ascitty_core::sim::Sim;
        use ascitty_core::world::City;
        let city = City::generate(99);
        let mut sim = Sim::new(&city, 99);
        let mut cam = ascitty_core::camera::Camera::spawn(&city, 117, 117);
        let mut head = Head::default();
        let mut ev = Vec::new();
        let rows = 40;
        let mut sky = i32::MAX;
        for _ in 0..HZ {
            sim.step(&city, &Controls { throttle: ONE, ..Default::default() }, HZ, &mut ev);
            chase(&mut cam, &sim, &city, rows, &mut head, HZ, 0);
            sky = sky.min(cam.pitch);
        }
        let mut road = i32::MIN;
        for _ in 0..HZ / 2 {
            sim.step(&city, &Controls { throttle: -ONE, ..Default::default() }, HZ, &mut ev);
            chase(&mut cam, &sim, &city, rows, &mut head, HZ, 0);
            road = road.max(cam.pitch);
        }
        assert!(
            road - sky >= 6,
            "the horizon moved {} rows between flat out and hard on the brakes",
            road - sky
        );
    }

    /// The ramp is in seconds, not in frames.    /// The ramp is in seconds, not in frames.
    #[test]
    fn the_frame_rate_does_not_change_how_fast_the_pedal_goes_down() {
        let mut slow = Hands { trust_release: true, ..Default::default() };
        let mut fast = Hands { trust_release: true, ..Default::default() };
        slow.press(Ctl::Gas);
        fast.press(Ctl::Gas);
        for _ in 0..15 {
            slow.tick(30);
        }
        for _ in 0..30 {
            fast.tick(60);
        }
        let d = fixed::abs(slow.at(Ctl::Gas) - fast.at(Ctl::Gas));
        assert!(
            d < fixed::ratio(1, 20),
            "half a second in: {} at 30 Hz against {} at 60",
            fixed::to_f32(slow.at(Ctl::Gas)),
            fixed::to_f32(fast.at(Ctl::Gas))
        );
    }
}

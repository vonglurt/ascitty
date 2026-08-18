//! ASCITTY on a colour terminal.
//!
//! Renders the city at whatever size the terminal is, in ASCII or in block
//! elements, at 24-bit colour or 16 or none.  `--shot` renders one frame and
//! prints it as plain text, which is how the pictures in the documentation
//! are made and how the build checks that the renderer still works without
//! needing a terminal at all.

mod hud;
mod paint;
mod term;

use ascitty_core::atmos::Atmos;
use ascitty_core::camera::{Camera, TURN_SPEED, WALK_SPEED};
use ascitty_core::drive::Controls;
use ascitty_core::fixed::{self, Fx, ONE};
use ascitty_core::frame::Frame;
use ascitty_core::glyph::Mode;
use ascitty_core::raycast;
use ascitty_core::sim::{Event, Sim};

use ascitty_core::world::City;
use paint::Depth;
use term::{Key, Keys, Term};

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

    /// The next mode in the cycle.
    fn next(self) -> View {
        match self {
            View::Walk => View::Drive,
            View::Drive => View::Copter,
            View::Copter => View::Walk,
        }
    }
}

/// A key that has to feel held down on a device that never reports a key
/// being released.
///
/// A terminal sends a byte when a key goes down and nothing at all when it
/// comes up, so "is the accelerator pressed" is not a question the input
/// stream can answer.  What it can answer is "was it pressed recently", and
/// a short decay turns the terminal's own autorepeat into something that
/// reads as a held pedal.  Longer than the autorepeat interval and the car
/// stutters; much longer and it will not stop.
#[derive(Clone, Copy, Default)]
struct Held(u8);

impl Held {
    /// Number of frames a press stays live.
    const LINGER: u8 = 5;

    fn press(&mut self) {
        self.0 = Held::LINGER;
    }

    fn decay(&mut self) {
        self.0 = self.0.saturating_sub(1);
    }

    /// How hard, from 1.0 just pressed down to 0 released.
    fn amount(self) -> Fx {
        fixed::div(fixed::from_int(self.0 as i32), fixed::from_int(Held::LINGER as i32))
    }

    fn down(self) -> bool {
        self.0 > 0
    }
}

/// Everything the driver is holding this frame.
#[derive(Default)]
struct Pedals {
    gas: Held,
    brake: Held,
    left: Held,
    right: Held,
    hand: Held,
}

impl Pedals {
    fn decay(&mut self) {
        for h in [&mut self.gas, &mut self.brake, &mut self.left, &mut self.right, &mut self.hand] {
            h.decay();
        }
    }

    fn controls(&self) -> Controls {
        Controls {
            throttle: self.gas.amount() - self.brake.amount(),
            steer: self.right.amount() - self.left.amount(),
            handbrake: self.hand.down(),
        }
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
fn chase(cam: &mut Camera, sim: &Sim, city: &City, rows: i32) {
    let target = sim.taxi.yaw;
    let delta = target.wrapping_sub(cam.yaw) as i16 as i32;
    cam.yaw = cam.yaw.wrapping_add((delta / 6) as u16);

    let (dx, dy) = cam.dir();
    let want = fixed::ratio(9, 4);
    let mut boom = want;
    while boom > fixed::ratio(1, 4) {
        let x = sim.taxi.x - fixed::mul(dx, boom);
        let y = sim.taxi.y - fixed::mul(dy, boom);
        if city.walkable(fixed::floor(x), fixed::floor(y)) {
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
    cam.z = fixed::ratio(4, 5);
    cam.pitch = rows / 10;
}

struct Opts {
    seed: u32,
    mode: Mode,
    depth: Depth,
    size: Option<(usize, usize)>,
    fps: u32,
    atmos: Atmos,
    shot: Option<u32>,
    bench: bool,
    view: View,
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
            bench: false,
            view: View::Walk,
        }
    }
}

const USAGE: &str = "\
ascitty - a raytraced ASCII city

USAGE: ascitty [options]

  --seed N          city to generate (default: a fixed one, so runs match)
  --mode M          ascii | unicode           (default: unicode)
  --color D         true | 16 | none          (default: from $COLORTERM)
  --size WxH        override the terminal size
  --fps N           frame rate cap            (default: 30)
  --rain N          0 dry .. 8 torrential     (default: 3)
  --haze N          0 clear .. 8 soup         (default: 3)
  --stars N         0 .. 8                    (default: 4)
  --no-moon         moonless night
  --copter          start above the city instead of on the pavement
  --drive           start behind the wheel of the taxi
  --shot [N]        render N frames, print the last as plain text, exit
  --bench           render 200 frames as fast as possible and report
  -h, --help        this

CONTROLS
  w s        forward, back            a d        turn
  q e        strafe                   arrows     turn and look
  shift+w    run
  r f        rise, descend (copter)   c          walk / drive / copter
  space      handbrake (drive)        w a s d    throttle, steer, brake
  1-9 0      rain                     h          haze
  m          moon                     t          ascii / unicode
  esc        quit
";

fn parse_args() -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut args = std::env::args().skip(1).peekable();
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
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
            "--rain" => o.atmos.rain = val()?.parse::<u8>().map_err(|_| "bad --rain")?.min(8),
            "--haze" => o.atmos.haze = val()?.parse::<u8>().map_err(|_| "bad --haze")?.min(8),
            "--stars" => o.atmos.stars = val()?.parse::<u8>().map_err(|_| "bad --stars")?.min(8),
            "--no-moon" => o.atmos.moon = false,
            "--copter" => o.view = View::Copter,
            "--drive" => o.view = View::Drive,
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
fn ceiling_of(city: &City) -> Fx {
    let tallest = city.lots.iter().map(|l| l.height).max().unwrap_or(20);
    fixed::from_int(tallest as i32 + 6)
}

fn run(mut o: Opts) -> Result<(), String> {
    let city = City::generate(o.seed);
    let mut cam = Camera::spawn(&city, 48, 48);
    let mut sim = Sim::new(&city, o.seed);
    let mut view = o.view;
    match view {
        View::Copter => {
            cam.z = ceiling_of(&city);
            cam.pitch = -8;
        }
        View::Drive => {
            sim.taxi.x = cam.x;
            sim.taxi.y = cam.y;
        }
        View::Walk => {}
    }


    // Headless paths first: neither needs a terminal, which is what makes
    // them usable from a Makefile and from CI.
    if let Some(n) = o.shot {
        let (w, h) = o.size.unwrap_or((100, 34));
        let mut f = Frame::new(w, h);
        let mut depth = Vec::new();
        let mut events = Vec::new();
        if o.view == View::Drive {
            cam.z = fixed::ratio(4, 5);
        }
        for _ in 0..n {
            o.atmos.step();
            if o.view == View::Drive {
                sim.step(&city, &Controls { throttle: ONE, ..Default::default() }, 60, &mut events);
                chase(&mut cam, &sim, &city, h as i32);
            }
            raycast::render_to(&city, &cam, &o.atmos, &mut f, &mut depth);
            let proj = raycast::projection(&cam, &f);
            sim.draw(&mut f, &depth, &cam, &o.atmos, &proj);
            o.atmos.rain_over(&mut f, &cam);
        }
        print!("{}", paint::plain(&f, o.mode));
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

    let _term = Term::enter().map_err(|e| format!("cannot set up the terminal: {e}"))?;
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
    let mut pedals = Pedals::default();
    let mut events: Vec<Event> = Vec::new();
    let mut flash: Option<(&'static str, i32)> = None;

    while !quit {
        let t0 = std::time::Instant::now();

        // Input.  Keys are edge-triggered here rather than held, because a
        // terminal does not report key release - so movement is one step per
        // press, and holding a key repeats it at the terminal's autorepeat
        // rate, which is close enough to feel continuous.
        let mut fwd: Fx = 0;
        let mut side: Fx = 0;
        let mut rise: Fx = 0;
        pedals.decay();
        for k in keys.drain() {
            match k {
                Key::Quit => quit = true,
                Key::Char('w') => {
                    fwd += step;
                    pedals.gas.press();
                }
                Key::Char('s') => {
                    fwd -= step;
                    pedals.brake.press();
                }
                Key::Char(' ') => pedals.hand.press(),
                Key::Char('q') => side -= step,
                Key::Char('e') => side += step,
                Key::Char('a') | Key::Left => {
                    cam.turn(-turn);
                    pedals.left.press();
                }
                Key::Char('d') | Key::Right => {
                    cam.turn(turn);
                    pedals.right.press();
                }
                Key::Up => cam.look(-1, (f.h / 3) as i32),
                Key::Down => cam.look(1, (f.h / 3) as i32),
                Key::Char('r') => rise += step,
                Key::Char('f') => rise -= step,
                Key::Char('c') => {
                    view = view.next();
                    match view {
                        View::Copter => {
                            cam.z = ceiling_of(&city);
                            cam.pitch = -(f.h as i32 / 6);
                        }
                        View::Walk => {
                            cam.z = ascitty_core::camera::EYE;
                            cam.pitch = 0;
                        }
                        View::Drive => {
                            // Start the shift from wherever you were
                            // standing, so switching in feels like getting
                            // into a cab rather than teleporting.
                            sim.taxi.x = cam.x;
                            sim.taxi.y = cam.y;
                            sim.taxi.vx = 0;
                            sim.taxi.vy = 0;
                            sim.taxi.yaw = cam.yaw;
                        }
                    }
                }
                Key::Char('m') => o.atmos.moon = !o.atmos.moon,
                Key::Char('h') => o.atmos.haze = (o.atmos.haze + 1) % 9,
                Key::Char('t') => {
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

        match view {
            View::Walk => {
                cam.z = ascitty_core::camera::EYE;
                cam.walk(&city, fwd, side);
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
                sim.step(&city, &pedals.controls(), o.fps.max(1) as i32, &mut events);
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
                chase(&mut cam, &sim, &city, f.h as i32);
            }
        }

        if o.size.is_none() {
            let (nw, nh) = Term::size();
            if (nw, nh) != (w, h) {
                w = nw;
                h = nh;
                f.resize(w, h.saturating_sub(1).max(1));
                buf.clear();
                print!("\x1b[2J");
            }
        }

        o.atmos.step();
        stats = raycast::render_to(&city, &cam, &o.atmos, &mut f, &mut depth);
        let proj = raycast::projection(&cam, &f);
        sim.draw(&mut f, &depth, &cam, &o.atmos, &proj);
        o.atmos.rain_over(&mut f, &cam);
        paint::paint(&f, o.mode, o.depth, &mut buf);
        hud::append(&mut buf, &hud::Status {
            view: view.name(),
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

        let spent = t0.elapsed();
        fps_ms = fps_ms * 0.9 + spent.as_secs_f64() * 1000.0 * 0.1;
        if spent < dt {
            std::thread::sleep(dt - spent);
        }
    }
    let _ = stats;
    Ok(())
}

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
use ascitty_core::fixed::{self, Fx};
use ascitty_core::frame::Frame;
use ascitty_core::glyph::Mode;
use ascitty_core::raycast;
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
}

impl View {
    fn name(self) -> &'static str {
        match self {
            View::Walk => "WALK",
            View::Copter => "COPTER",
        }
    }
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
            seed: 0x_A5C1_77_1E,
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
  --shot [N]        render N frames, print the last as plain text, exit
  --bench           render 200 frames as fast as possible and report
  -h, --help        this

CONTROLS
  w s        forward, back            a d        turn
  q e        strafe                   arrows     turn and look
  shift+w    run
  r f        rise, descend (copter)   c          switch walk / copter
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
    let mut view = o.view;
    if view == View::Copter {
        cam.z = ceiling_of(&city);
        cam.pitch = -8;
    }

    // Headless paths first: neither needs a terminal, which is what makes
    // them usable from a Makefile and from CI.
    if let Some(n) = o.shot {
        let (w, h) = o.size.unwrap_or((100, 34));
        let mut f = Frame::new(w, h);
        for _ in 0..n {
            o.atmos.step();
            raycast::render(&city, &cam, &o.atmos, &mut f);
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

    while !quit {
        let t0 = std::time::Instant::now();

        // Input.  Keys are edge-triggered here rather than held, because a
        // terminal does not report key release - so movement is one step per
        // press, and holding a key repeats it at the terminal's autorepeat
        // rate, which is close enough to feel continuous.
        let mut fwd: Fx = 0;
        let mut side: Fx = 0;
        let mut rise: Fx = 0;
        for k in keys.drain() {
            match k {
                Key::Quit => quit = true,
                Key::Char('w') => fwd += step,
                Key::Char('s') => fwd -= step,
                Key::Char('q') => side -= step,
                Key::Char('e') => side += step,
                Key::Char('a') | Key::Left => cam.turn(-turn),
                Key::Char('d') | Key::Right => cam.turn(turn),
                Key::Up => cam.look(-1, (f.h / 3) as i32),
                Key::Down => cam.look(1, (f.h / 3) as i32),
                Key::Char('r') => rise += step,
                Key::Char('f') => rise -= step,
                Key::Char('c') => {
                    view = match view {
                        View::Walk => {
                            cam.z = ceiling_of(&city);
                            cam.pitch = -(f.h as i32 / 6);
                            View::Copter
                        }
                        View::Copter => {
                            cam.z = ascitty_core::camera::EYE;
                            cam.pitch = 0;
                            View::Walk
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
        stats = raycast::render(&city, &cam, &o.atmos, &mut f);
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
        });
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

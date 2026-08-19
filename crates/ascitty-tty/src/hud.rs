//! The status line.
//!
//! One row, at the bottom, outside the rendered frame - so the renderer
//! never has to know it exists and the frame is always exactly the size the
//! city was drawn at.

use crate::paint::Depth;
use ascitty_core::atmos::Atmos;
use ascitty_core::catalog;
use ascitty_core::fixed::{self, Fx, ONE};
use ascitty_core::frame::{Cel, Frame};
use ascitty_core::palette;
use ascitty_core::trig::Ang;
use ascitty_core::camera::Camera;
use ascitty_core::glyph::Mode;
use ascitty_core::raycast::{Proj, Stats};
use ascitty_core::sim::Sim;
use ascitty_core::trig;

/// Everything the status line reports.
pub struct Status<'a> {
    /// Camera mode name.
    pub view: &'static str,
    /// Glyph mode.
    pub mode: Mode,
    /// Colour depth.
    pub depth: Depth,
    /// Where the camera is.
    pub cam: &'a Camera,
    /// The weather.
    pub atmos: &'a Atmos,
    /// Last frame's cost.
    pub stats: Stats,
    /// Smoothed frame time.
    pub ms: f64,
    /// The city's seed.
    pub seed: u32,
    /// The shift, when one is being driven.
    pub sim: Option<&'a Sim>,
    /// A word to shout, if anything just happened.
    pub flash: Option<&'static str>,
}

/// Where the middle of the arrow sits, in cells of road in front of the
/// camera.
///
/// Four cells.  The chase camera sits five and a half behind the cab, and
/// the bottom row of the frame is about two and a half cells of road in
/// front of it, so an arrow centred here runs from the bottom edge of the
/// screen to about the middle of the taxi - which is where it was asked to
/// be, and which is the only part of the road you can see all of.
const ARROW_AHEAD: Fx = fixed::ratio(4, 1);
/// Half its length, in cells of road.
const ARROW_LONG: Fx = fixed::ratio(8, 5);
/// Half the width of its shaft, in cells of road.
const ARROW_SHAFT: Fx = fixed::ratio(7, 20);
/// Half the width of the head where it is widest.
const ARROW_BARB: Fx = fixed::ratio(11, 10);
/// Where along the arrow the head starts, from the middle.
const ARROW_NECK: Fx = fixed::ratio(2, 5);
/// How thick the black outline is, in cells of road.
const ARROW_EDGE: Fx = fixed::ratio(1, 6);
/// How far back from the point the tip is a different colour, in cells of
/// road.
///
/// Half a cell.  An arrow is symmetrical enough at a glance that the head
/// and the tail can be read the wrong way round in the corner of your eye,
/// and the whole job of this thing is to be read in the corner of your eye.
/// One end being a different colour settles it without needing a second
/// look.
const ARROW_TIP: Fx = fixed::ratio(3, 5);

/// Whether a point in the arrow's own coordinates is inside it.
///
/// `u` runs along the arrow, positive towards the point; `v` runs across.
/// `grow` inflates the whole shape, which is how the outline is drawn: the
/// same test, a little bigger, in black, underneath.
fn in_arrow(u: Fx, v: Fx, grow: Fx) -> bool {
    let long = ARROW_LONG + grow;
    if u < -long || u > long {
        return false;
    }
    if u <= ARROW_NECK {
        fixed::abs(v) <= ARROW_SHAFT + grow
    } else {
        let along = fixed::div(long - u, long - ARROW_NECK).clamp(0, ONE);
        fixed::abs(v) <= fixed::mul(ARROW_BARB + grow, along)
    }
}

/// Paint the fare arrow onto the road, over everything else.
///
/// # What it is
///
/// A yellow arrow **lying on the ground plane**, a few cells in front of the
/// camera, pointing at whichever end of the fare is current. Not a shape
/// rotated on the screen: a shape rotated on the *road*, projected through
/// the same arithmetic the ground itself is drawn with, so it converges with
/// the street it is lying on. The near end is wider than the far end, and
/// swinging it round the compass sweeps it across the road the way a needle
/// laid flat would rather than spinning it like a dial.
///
/// That is the whole difference between this and the version before it,
/// which was rotated in screen x and y and squashed by a constant: that one
/// reads as a card held up in front of the car, and no amount of squashing
/// fixes it, because a card has no perspective in it.
///
/// # How it is drawn
///
/// Backwards, which is the cheap way. Every cell below the horizon is turned
/// back into the piece of road it is a picture of - `eye x scale / rows
/// below the horizon` is the distance, and the column gives the offset
/// across at that distance - and that point is rotated into the arrow's
/// frame and tested against a shaft and a triangle. The same test, inflated,
/// paints a black outline underneath, which is what keeps it legible over a
/// yellow cab on a yellow-lit street.
///
/// It is drawn last, so it is over the cab and over the road. A decal in the
/// world would disappear under the car at exactly the moment the car is what
/// you are looking at.
///
/// `bearing` is relative to the way the *camera* is looking, not the way the
/// car is pointing. The two differ by however much the chase camera is
/// lagging a turn, and the arrow belongs to the screen.
pub fn arrow_on_the_road(f: &mut Frame, p: &Proj, fov: Fx, bearing: i32, tick: u32) {
    let (sin, cos) = (trig::sin(bearing as Ang), trig::cos(bearing as Ang));

    // A slow pulse, so it reads as an instrument rather than as scenery.
    let luma = if (tick >> 4) & 3 == 0 { 6 } else { 7 };
    let body = Cel { glyph: catalog::G_SOLID, color: palette::rgb_index(palette::H_YELLOW, luma) };
    // A step down the ramp from the body rather than up it.  This palette
    // scales chroma with luminance, so at the top of it every hue is nearly
    // white and an orange tip on a yellow arrow is a cream tip on a cream
    // arrow - measured, (255,245,193) against (255,255,157).  One step down
    // is (232,192,151), which is orange.
    let tip = Cel { glyph: catalog::G_SOLID, color: palette::rgb_index(palette::H_ORANGE, 6) };
    let outline = Cel { glyph: catalog::G_SOLID, color: palette::rgb_index(palette::H_BLACK, 0) };

    let half = fixed::from_int(p.w / 2);
    for y in (p.horizon + 1).max(0)..p.h.min(f.h as i32) {
        // How far up the road this row is.  The same expression the floor
        // pass uses, from the same numbers, so the arrow lands on the road
        // rather than near it.
        let below = fixed::from_int(y - p.horizon);
        let d = fixed::div(fixed::mul(p.eye, p.proj), below);
        if d <= 0 || d > fixed::from_int(64) {
            continue;
        }
        // Where the arrow is, relative to this row: along the road, and
        // then rotated into the arrow's own frame.
        let ahead = d - ARROW_AHEAD;
        for x in 0..f.w as i32 {
            // How far across the road this column is, at that distance.
            let camx = fixed::div(fixed::from_int(x) + fixed::HALF - half, half);
            let across = fixed::mul(fixed::mul(d, fov), camx);
            let u = fixed::mul(ahead, cos) + fixed::mul(across, sin);
            let v = fixed::mul(across, cos) - fixed::mul(ahead, sin);
            if in_arrow(u, v, 0) {
                // The point itself, and only the point.
                let end = u > ARROW_LONG - ARROW_TIP;
                f.put(x, y, if end { tip } else { body });
            } else if in_arrow(u, v, ARROW_EDGE) {
                f.put(x, y, outline);
            }
        }
    }
}

/// An eight-point compass arrow for a bearing relative to straight ahead.
///
/// Eight points and not sixteen: at a glance, while sliding sideways through
/// a junction, the only question is which way to throw the wheel, and more
/// resolution than that is more reading than there is time for.
fn arrow(bearing: i32) -> char {
    // Rotate by half a sector so that each sector is centred on its arrow.
    let sector = (((bearing + trig::QUARTER as i32 / 4) as u32 >> 13) & 7) as usize;
    ['^', '/', '>', '\\', 'v', '/', '<', '\\'][sector]
}

/// Append the status line to a painted frame.
pub fn append(out: &mut String, s: &Status) {
    out.push_str("\r\n\x1b[0m\x1b[7m ");
    if let Some(sim) = s.sim {
        // Driving replaces the diagnostics with the only four numbers that
        // matter while the clock is running.
        let bearing = sim.target_bearing().unwrap_or(0);
        let line = format!(
            "TIME {:3}s   ${:<6}  {} mph  {}{}  {}{}",
            sim.seconds_left(),
            sim.money,
            fixed::floor(fixed::mul(sim.taxi.speed(), fixed::from_int(22))),
            arrow(bearing),
            if sim.fare.as_ref().is_some_and(|f| f.aboard) { " DROP" } else { " FARE" },
            if sim.combo > 1 { format!("x{} COMBO  ", sim.combo) } else { String::new() },
            s.flash.unwrap_or(""),
        );
        out.push_str(&line);
        out.push_str("\x1b[K\x1b[0m");
        return;
    }
    let line = format!(
        "{}  {},{}  {}  {}  haze {}  {}  #{:08x}  {:.1}ms {:.0}fps  {} steps  [t]axi [c]opter [g]lyphs esc",
        s.view,
        fixed::floor(s.cam.x),
        fixed::floor(s.cam.y),
        match s.mode {
            Mode::Ascii => "ascii",
            Mode::Unicode => "blocks",
        },
        // What the sky is doing, which is the one thing on this line that
        // changes on its own.
        s.atmos.phase_name(),
        s.atmos.haze,
        match s.depth {
            Depth::True => "24bit",
            Depth::Ansi16 => "16col",
            Depth::Mono => "mono",
        },
        s.seed,
        s.ms,
        if s.ms > 0.01 { 1000.0 / s.ms } else { 0.0 },
        s.stats.steps,
    );
    out.push_str(&line);
    out.push_str("\x1b[K\x1b[0m");
}

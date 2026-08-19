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
use ascitty_core::raycast::Stats;
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

/// Half the length of the arrow, in cells.
const ARROW_LONG: Fx = fixed::ratio(12, 1);
/// Half the width of its shaft.
const ARROW_SHAFT: Fx = fixed::ratio(3, 1);
/// Half the width of the head where it is widest.
const ARROW_BARB: Fx = fixed::ratio(15, 2);
/// Where along the arrow the head starts, from the middle.
const ARROW_NECK: Fx = fixed::ratio(2, 1);
/// How thick the black outline is, in cells.
const ARROW_EDGE: Fx = fixed::ratio(1, 1);
/// How much the road plane squashes a vertical distance.
///
/// A ground plane seen from a camera a car's height up compresses about two
/// to one over the few cells in front of the bumper, so the arrow is drawn
/// twice as wide as it is deep and reads as lying on the road rather than
/// as being painted on the screen.  It is a constant rather than the real
/// projection because the arrow is on the HUD, not in the world: it has to
/// read at any pitch, including the ones where the road under the car is not
/// on screen at all.
const ARROW_SQUASH: Fx = fixed::ratio(45, 100);
/// How far up from the bottom of the frame its middle sits.
const ARROW_ROW: i32 = 8;

/// Whether a point in the arrow's own coordinates is inside it.
///
/// `u` runs along the arrow, positive towards the point; `v` runs across.
/// `grow` inflates the whole shape, which is how the outline is drawn: the
/// same test, a little bigger, in black, underneath.
fn in_arrow(u: Fx, v: Fx, grow: Fx) -> bool {
    let long = ARROW_LONG + grow;
    let neck = ARROW_NECK;
    if u < -long || u > long {
        return false;
    }
    if u <= neck {
        // The shaft.
        fixed::abs(v) <= ARROW_SHAFT + grow
    } else {
        // The head: a triangle from the barbs to the point.
        let along = fixed::div(long - u, long - neck).clamp(0, ONE);
        fixed::abs(v) <= fixed::mul(ARROW_BARB + grow, along)
    }
}

/// Paint the fare arrow into the frame, over everything else.
///
/// # What it is
///
/// A yellow arrow lying in the plane of the road, under the car and over it,
/// at the bottom middle of the screen. It points at wherever the fare is -
/// the passenger, or where they are going once they are aboard - and it is
/// the one piece of the interface that is *in* the picture rather than on a
/// status line, because "which way now" is a question you ask at ninety
/// miles an hour and reading a word is not an answer you have time for.
///
/// It is deliberately enormous: twenty-four cells long and fifteen across
/// in the road's own plane, which on a forty-row frame is most of the width
/// of the street. A small one is a
/// decoration that the eye has to go and find; this one is the first thing
/// in the frame, and the car is behind it.
///
/// # How it is drawn
///
/// As a shape rather than as glyphs. Every cell in the bounding box is
/// transformed into the arrow's own coordinates - rotate by the bearing,
/// undo the road plane's squash - and tested against a shaft and a triangle.
/// The same test, inflated, paints a black outline underneath, which is what
/// keeps it legible over a yellow cab on a yellow-lit street: an arrow the
/// same colour as what it is over is not an arrow.
///
/// It is drawn last, so it is over the cab and over the road. A decal would
/// disappear under the car at exactly the moment the car is what you are
/// looking at.
///
/// `bearing` is relative to the way the *camera* is looking, not the way the
/// car is pointing. The two differ by however much the chase camera is
/// lagging a turn, and the arrow belongs to the screen.
pub fn arrow_on_the_road(f: &mut Frame, bearing: i32, tick: u32) {
    let (sin, cos) = (trig::sin(bearing as Ang), trig::cos(bearing as Ang));
    let (ax, ay) = (
        fixed::from_int(f.w as i32 / 2),
        fixed::from_int(f.h as i32 - ARROW_ROW),
    );

    // A slow pulse, so it reads as an instrument rather than as scenery.
    let luma = if (tick >> 4) & 3 == 0 { 6 } else { 7 };
    let body = Cel { glyph: catalog::G_SOLID, color: palette::rgb_index(palette::H_YELLOW, luma) };
    let edge = Cel { glyph: catalog::G_SOLID, color: palette::rgb_index(palette::H_BLACK, 0) };

    // The bounding box, generous enough for the outline and for the squash.
    let reach = fixed::floor(ARROW_LONG + ARROW_EDGE) + 2;
    let (cx, cy) = (f.w as i32 / 2, f.h as i32 - ARROW_ROW);
    for y in (cy - reach)..=(cy + reach) {
        for x in (cx - reach * 2)..=(cx + reach * 2) {
            if x < 0 || y < 0 || x >= f.w as i32 || y >= f.h as i32 {
                continue;
            }
            // Into the arrow's frame: undo the squash, then the rotation.
            let px = fixed::from_int(x) + fixed::HALF - ax;
            let py = fixed::div(fixed::from_int(y) + fixed::HALF - ay, ARROW_SQUASH);
            let u = fixed::mul(px, sin) - fixed::mul(py, cos);
            let v = fixed::mul(px, cos) + fixed::mul(py, sin);
            if in_arrow(u, v, 0) {
                f.put(x, y, body);
            } else if in_arrow(u, v, ARROW_EDGE) {
                f.put(x, y, edge);
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

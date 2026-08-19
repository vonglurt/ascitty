//! The status line.
//!
//! One row, at the bottom, outside the rendered frame - so the renderer
//! never has to know it exists and the frame is always exactly the size the
//! city was drawn at.

use crate::paint::Depth;
use ascitty_core::atmos::Atmos;
use ascitty_core::camera::Camera;
use ascitty_core::fixed;
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

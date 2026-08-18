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
}

/// Append the status line to a painted frame.
pub fn append(out: &mut String, s: &Status) {
    out.push_str("\r\n\x1b[0m\x1b[7m ");
    let line = format!(
        "{}  {},{}  {}  rain {}  haze {}  {}  #{:08x}  {:.1}ms {:.0}fps  {} steps  [c]opter [t]ext [1-9]rain [h]aze esc",
        s.view,
        fixed::floor(s.cam.x),
        fixed::floor(s.cam.y),
        match s.mode {
            Mode::Ascii => "ascii",
            Mode::Unicode => "blocks",
        },
        s.atmos.rain,
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

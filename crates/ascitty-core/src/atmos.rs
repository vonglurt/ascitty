//! Weather and sky: rain, the moon, stars, and the haze that eats distance.
//!
//! All four are cheap on purpose.  Rain is not particles - it is a glyph
//! family and a scrolling hash, so a downpour costs one table lookup per wet
//! cell and nothing at all per drop.  Stars are a hash of the direction you
//! are facing, so they stay put as you turn without being stored.  The moon
//! is four glyphs.  Every one of these has to survive on a 1.76 MHz machine,
//! and none of them may allocate.

use crate::arch::{self, Face};
use crate::camera::Camera;
use crate::catalog;
use crate::fixed::{self, Fx, ONE};
use crate::frame::{Cel, Frame};
use crate::palette::{self, rgb_index, Color};
use crate::rng::hash3;
use crate::trig::{self, Ang};

/// One phase of the day: what colour the sky is, and how it is graded.
///
/// Two luminances rather than one, because a sky is not a flat colour.  The
/// light comes from the horizon, so that is where it is palest - a hue at
/// the top of this palette's ramp is a washed, almost white version of
/// itself, which is exactly what the bottom of a sky looks like - and it
/// darkens towards the zenith.  Getting that the wrong way round produces a
/// ceiling rather than a sky.
#[derive(Clone, Copy, Debug)]
pub struct Phase {
    /// What to call it, for the status line.
    pub name: &'static str,
    /// The hue of the whole sky in this phase.
    pub hue: u8,
    /// Luminance at the zenith.
    pub top: u8,
    /// Luminance at the horizon, which is where the light is.
    pub bottom: u8,
    /// How long this phase lasts, in shares of the day.
    ///
    /// Not every phase is worth the same amount of time.  A dust storm is a
    /// thing that happens to an afternoon; the afternoon is the afternoon.
    /// Weights rather than durations so that the length of a day stays one
    /// number - see [`Atmos::day`] - and changing what a phase is worth
    /// cannot change how long a day is.
    pub hold: u8,
}

/// The day, as twelve phases in the order they happen.
///
/// It is a *cycle*, so the last one runs into the first, and the interesting
/// property is that no two adjacent phases share a hue: the sky always
/// visibly moves.  The colours are not meteorology.  They are the twelve
/// skies a city like this one has in the sort of film it comes from, and
/// they are chosen to be told apart at sixteen hues.
pub const DAY: [Phase; 12] = [
    Phase { name: "NIGHT", hue: palette::H_DARK_BLUE, top: 0, bottom: 2, hold: 2 },
    Phase { name: "MORNING", hue: palette::H_WHITE, top: 2, bottom: 4, hold: 2 },
    Phase { name: "AWAKENING", hue: palette::H_LIGHT_BLUE, top: 3, bottom: 5, hold: 2 },
    Phase { name: "SUNRISE", hue: palette::H_ORANGE, top: 3, bottom: 6, hold: 1 },
    Phase { name: "DUST", hue: palette::H_RED, top: 3, bottom: 5, hold: 1 },
    Phase { name: "NOON", hue: palette::H_YELLOW, top: 5, bottom: 7, hold: 2 },
    Phase { name: "AFTERNOON", hue: palette::H_BLUE, top: 4, bottom: 6, hold: 6 },
    Phase { name: "OVERCAST", hue: palette::H_WHITE, top: 4, bottom: 6, hold: 2 },
    Phase { name: "SUNSET", hue: palette::H_GREEN, top: 3, bottom: 5, hold: 2 },
    Phase { name: "AFTERGLOW", hue: palette::H_PINK, top: 2, bottom: 5, hold: 1 },
    Phase { name: "GLOAMING", hue: palette::H_GREEN, top: 1, bottom: 3, hold: 1 },
    Phase { name: "DEEP NIGHT", hue: palette::H_BLUE, top: 0, bottom: 2, hold: 2 },
];

/// The shares of a day, added up.
pub const DAY_SHARES: u32 = 24;

/// Ticks in a full cycle of [`DAY`], at the rate the program steps the
/// atmosphere - one per frame.
///
/// Seven thousand two hundred is four minutes at thirty frames a second, so
/// a phase is twenty seconds and a shift on the clock - sixty seconds, if
/// you are not earning - spans three of them.  Long enough that the sky is
/// not a strobe, short enough that a single run is not all one colour.
pub const DAY_TICKS: u32 = 7200;

/// The fewest rows the sky gradient is ever spread over.
///
/// The gradient normally spans the whole sky, so it reaches the top of the
/// frame whatever size the frame is.  This is the floor: a camera pointed at
/// the ground has three rows of sky above the buildings, and a whole ramp
/// squeezed into three rows is a stripe rather than a sky.
pub const MIN_SKY_SPAN: i32 = 12;

/// The state of the sky.
#[derive(Clone, Copy, Debug)]
pub struct Atmos {
    /// Rain, 0 (dry) to 8 (torrential).
    pub rain: u8,
    /// Whether the moon is up.
    pub moon: bool,
    /// The moon's compass bearing.
    pub moon_az: Ang,
    /// The moon's height above the horizon, in screen rows.
    pub moon_alt: i32,
    /// How fast distance fades to black, 0 (clear) to 8 (soup).
    pub haze: u8,
    /// Star density, 0 to 8.  Zero in a real city; this is not a real city.
    pub stars: u8,
    /// Ticks in a full cycle of the sky.  Zero holds it at `sky_offset`,
    /// which is what one picture of one sky wants.
    pub day: u32,
    /// Where in the cycle the sky starts, in ticks.
    ///
    /// A separate number from the tick counter rather than a head start on
    /// it, because the tick counter also drives the rain, the twinkle and
    /// everything else that moves, and freezing the sky must not freeze
    /// those.
    pub sky_offset: u32,
    /// Frame counter, driving everything that moves.
    pub tick: u32,
}

impl Default for Atmos {
    fn default() -> Self {
        Atmos {
            // Dry.  Rain is still here and `--rain 1..8` still asks for it,
            // but it is no longer what you get without asking: a character
            // cell is a large pixel, so a raindrop is a large raindrop, and
            // a frame with a couple of hundred of them leaning across it is
            // reading the weather rather than the city.  The city is the
            // thing being drawn.
            rain: 0,
            moon: true,
            // The same bearing the shadow sweep uses, so that the
            // shadows and the thing casting them agree.
            moon_az: crate::shadow::DEFAULT_AZ,
            moon_alt: 9,
            haze: 3,
            stars: 4,
            day: DAY_TICKS,
            sky_offset: 0,
            tick: 0,
        }
    }
}

/// How much of a cell the sky fills, for a given luminance.
///
/// The two darkest levels come from the haze family, which is sparser than
/// the lightest dither - a first-light sky should be a suggestion, not a
/// texture - and the rest climb the dither ramp to solid.  In ASCII that is
/// the difference between a blank sky at night and `. : - = +` grading up
/// towards the horizon at noon, which is the only way a sky can have a
/// gradient in a mode with no colour at all.
///
/// It stops short of a solid fill even at the top.  A cell is one colour, so
/// a solid sky is a flat wash - and in ASCII it is a wall of `@`, which is
/// the brightest thing the mode has and reads as a building rather than as
/// air.  Six eighths is bright enough to be a noon sky in colour and light
/// enough to still be sky without it.
#[inline]
fn sky_fill(luma: u8) -> catalog::GlyphId {
    match luma {
        0 => catalog::G_BLANK,
        1 => catalog::G_HAZE + 1,
        2 => catalog::G_HAZE + 2,
        3 => catalog::G_HAZE + 3,
        4 => catalog::shade(2),
        5 => catalog::shade(3),
        6 => catalog::shade(4),
        _ => catalog::shade(6),
    }
}

/// Distance at which everything has faded to black, in world units, as a
/// function of haze.
pub fn draw_distance(haze: u8) -> i32 {
    // A block further than it used to be, at every setting.
    //
    // The city grew a suburb, a ring of fields and a coast around it, and
    // the reason to see further is the same reason those are there: from
    // the outer rings the towers have to be *visible*, so that the way back
    // to the middle is something you can see rather than something you have
    // to remember.  A block is thirteen cells - see `zone::BLOCK_PITCH`.
    const BLOCK: i32 = crate::zone::BLOCK_PITCH as i32;
    BLOCK
        + match haze {
            0 => 200,
            1 => 150,
            2 => 110,
            3 => 80,
            4 => 60,
            5 => 45,
            6 => 34,
            7 => 26,
            _ => 20,
        }
}

impl Atmos {
    /// Advance one frame.
    pub fn step(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// How many luminance steps to drop at a given distance.
    ///
    /// This is the depth cue, and it is deliberately a *step count* rather
    /// than a blend: the Plus/4 shades by subtracting from a luminance
    /// nibble, so the host does the same subtraction and the two pictures
    /// darken identically.
    #[inline(always)]
    pub fn fade(&self, dist: Fx) -> u8 {
        let d = fixed::floor(dist).max(0);
        let full = draw_distance(self.haze);
        ((d * 8) / full.max(1)).clamp(0, 8) as u8
    }

    /// Apply the depth cue to a colour.
    #[inline(always)]
    pub fn shade(&self, hue: u8, luma: u8, dist: Fx) -> Color {
        let f = self.fade(dist);
        let l = luma.saturating_sub(f);
        rgb_index(hue, l)
    }

    /// Which phase the sky is in, and how far through it, from 0 to
    /// [`ONE`].
    ///
    /// With `day` at zero the sky holds wherever the tick counter left it,
    /// which is what `--day 0` is for: a picture of one sky.
    pub fn phase(&self) -> (usize, Fx) {
        let cycle = if self.day == 0 { DAY_TICKS } else { self.day }.max(DAY_SHARES);
        let t = if self.day == 0 {
            self.sky_offset % cycle
        } else {
            self.tick.wrapping_add(self.sky_offset) % cycle
        };
        // Walk the shares.  Twelve of them, once per call, on a cheap
        // machine as well as this one: it is an add and a compare a phase.
        let mut start = 0u32;
        for (i, p) in DAY.iter().enumerate() {
            let len = cycle * p.hold as u32 / DAY_SHARES;
            if t < start + len || i == DAY.len() - 1 {
                let within = if len == 0 {
                    0
                } else {
                    fixed::div(fixed::from_int((t - start) as i32), fixed::from_int(len as i32))
                };
                return (i, within.clamp(0, ONE));
            }
            start += len;
        }
        (0, 0)
    }

    /// Where the light is, as a compass bearing.
    ///
    /// One turn a day, arranged so that the middle of `SUNRISE` is due east
    /// and the middle of `SUNSET` is due west - which the shares in [`DAY`]
    /// are set up to make exactly half a turn apart, so this is the honest
    /// sun rather than an approximation of one.
    ///
    /// East is angle zero here because east is `+x`, which is the same
    /// convention the driving and the shadow sweep use.  Nothing else in the
    /// program has an opinion about where the sun is, so this is the only
    /// place it has to be true.
    pub fn sun_az(&self) -> Ang {
        let cycle = if self.day == 0 { DAY_TICKS } else { self.day }.max(DAY_SHARES);
        let t = if self.day == 0 {
            self.sky_offset % cycle
        } else {
            self.tick.wrapping_add(self.sky_offset) % cycle
        };
        // Where sunrise is, in ticks, plus half of it: the middle of the
        // phase is the moment the sun is on the horizon.
        let mut rise = 0u32;
        for p in DAY.iter().take(3) {
            rise += cycle * p.hold as u32 / DAY_SHARES;
        }
        rise += cycle * DAY[3].hold as u32 / DAY_SHARES / 2;
        let round = ((t as i64 - rise as i64).rem_euclid(cycle as i64) * 65536 / cycle as i64) as i32;
        round as Ang
    }

    /// The tick offset that starts the cycle at phase `n`.
    pub fn phase_offset(n: u32, day: u32) -> u32 {
        let cycle = if day == 0 { DAY_TICKS } else { day }.max(DAY_SHARES);
        let n = (n as usize) % DAY.len();
        let mut start = 0u32;
        for p in DAY.iter().take(n) {
            start += cycle * p.hold as u32 / DAY_SHARES;
        }
        // Half a phase in, so `--sky 5` is noon rather than the moment noon
        // is still sweeping up over the morning.
        start + cycle * DAY[n].hold as u32 / DAY_SHARES / 2
    }

    /// How much light the sky is throwing on the city, in luminance steps.
    ///
    /// Read off the phase's own brightness at the horizon, so it cannot
    /// disagree with what the sky looks like: a night sky lights nothing, a
    /// sunrise lights a little, noon lights everything by two steps.  It is
    /// added to every surface whichever way it faces, because that is what
    /// ambient means and because a directional daylight would want a second
    /// shadow sweep - which is a real thing to want and is in the backlog.
    pub fn daylight(&self) -> i8 {
        // Read off the *zenith*, not the horizon.  The horizon is bright at
        // sunrise because the sun is on it, and a city at sunrise is not lit
        // like a city at noon; how high the light has got is what the top of
        // the sky says.  Over the twelve phases this runs 0,0,1,1,1,2,2,2,
        // 1,1,0,0, which is a day.
        match DAY[self.phase().0].top {
            0..=1 => 0,
            2..=3 => 1,
            _ => 2,
        }
    }

    /// The colour of the sky, for things that reflect it.
    ///
    /// The phase's hue at its horizon brightness, dropped a step: a window is
    /// a dark mirror, not a hole in the roof.  Glass that takes this is glass
    /// that is blue in the afternoon and gold at sunrise without anything
    /// having to be told what time it is.
    pub fn sky_colour(&self) -> (u8, u8) {
        let p = DAY[self.phase().0];
        (p.hue, p.bottom.saturating_sub(1).max(1))
    }

    /// What to call the sky right now, for the status line.
    pub fn phase_name(&self) -> &'static str {
        DAY[self.phase().0].name
    }

    /// The hue and luminance of the sky this many rows above the horizon.
    ///
    /// # The sweep
    ///
    /// A phase change is not a cross-fade.  Two hues cannot be mixed in a
    /// palette that gives a cell one colour, and dithering them together
    /// costs a colour change per cell across the whole sky - which on a
    /// terminal is a colour escape per cell, and the sky is half the frame.
    ///
    /// So the new sky *rises*, which is what a sky does anyway: for the
    /// first half of a phase the incoming colour climbs from the horizon to
    /// the zenith, and for the second half it holds.  Every row is one
    /// colour, so a row is still one escape and a hundred and forty
    /// characters, and the change reads as weather moving rather than as a
    /// palette being swapped.
    ///
    /// The boundary itself is dithered by a row: without it the sweep is a
    /// ruled line across the sky, which reads as a rendering artefact
    /// because it is one.
    pub fn sky_at(&self, rows_above: i32, sky_rows: i32, bearing: Ang, jitter: u32) -> (u8, u8) {
        let (i, t) = self.phase();
        let now = DAY[i];
        let before = DAY[(i + DAY.len() - 1) % DAY.len()];

        // The gradient runs over the whole sky rather than over a fixed
        // number of rows.  Anchoring it to rows keeps it fixed to the world
        // when the camera nods, which is the honest thing, and it also means
        // that on any frame taller than the anchor the top half of the sky
        // is one flat colour - and a flat top half is not a sky.  So the
        // span is what is actually above the horizon, floored so that a
        // sliver of sky under a camera pointed at the ground is not a whole
        // ramp squeezed into three rows.
        let span = sky_rows.max(MIN_SKY_SPAN);

        // How far the new sky has climbed, over the first half of the phase.
        let swept = fixed::floor(fixed::mul(
            fixed::mul(t, fixed::from_int(2)).min(ONE),
            fixed::from_int(span),
        ));
        // One row of noise at the edge, so the boundary is a weather front
        // and not a ruler.
        let here = rows_above + (jitter & 1) as i32;
        let p = if here <= swept { now } else { before };

        // The gradient: palest at the horizon, darkening to the zenith.
        let up = fixed::div(fixed::from_int(rows_above.clamp(0, span)), fixed::from_int(span));
        let mut luma = fixed::lerp(
            fixed::from_int(p.bottom as i32),
            fixed::from_int(p.top as i32),
            up,
        );

        // And the glow, which is what makes the day turn.
        //
        // The light is a bearing that goes once round in a day - due east at
        // sunrise, due west at sunset - and the sky nearest it is brighter.
        // It is the only thing in the frame that says which way you are
        // facing, and it is why a sunrise looks different from a sunset
        // rather than merely being a different colour.
        //
        // Strongest at the horizon and gone by the zenith, because that is
        // where a low sun puts it, and scaled by how much light the phase
        // has to give: a midnight sky does not glow in the east.
        let strength = match p.bottom {
            0..=2 => 0,
            3..=4 => 1,
            _ => 2,
        };
        if strength > 0 {
            let off = (bearing.wrapping_sub(self.sun_az()) as i16 as i32).abs();
            let quarter = trig::QUARTER as i32;
            if off < quarter {
                // One at the edge of the quarter, none at right angles to it.
                let near = fixed::div(fixed::from_int(quarter - off), fixed::from_int(quarter));
                let low = ONE - up;
                luma += fixed::mul(fixed::mul(fixed::from_int(strength), near), low);
            }
        }

        (p.hue, fixed::floor(luma + fixed::HALF).clamp(0, 7) as u8)
    }

    /// What is in the sky along a given ray, at a given row.
    ///
    /// `col_ang` is the compass bearing of the ray, so stars are fixed to
    /// the world rather than to the screen: turn around and the same stars
    /// come back.
    pub fn sky(&self, col_ang: Ang, row_above_horizon: i32, sky_rows: i32) -> Cel {
        if row_above_horizon <= 0 {
            return Cel::EMPTY;
        }
        // Bucket the bearing finely enough that stars do not visibly snap
        // between columns, coarsely enough that they do not swarm.
        let bucket = (col_ang >> 5) as u32;
        let h = hash3(bucket, row_above_horizon as u32, 0x5747_2A25);
        let (hue, luma) = self.sky_at(row_above_horizon, sky_rows, col_ang, h >> 5);

        // Stars, but only where it is dark enough to see one.  Nothing
        // switches them off at dawn: the sky gets brighter and they stop
        // being drawn, which is what happens.
        let dark = 2u32.saturating_sub(luma as u32);
        if self.stars > 0 && (h & 255) < self.stars as u32 * dark {
            // A few stars twinkle; most do not, because all of them
            // twinkling reads as static rather than as sky.
            let twinkle = (h >> 17) & 7 == 0;
            let star = if twinkle {
                2 + ((self.tick >> 3) as u8 ^ (h >> 9) as u8) % 3
            } else {
                1 + (h >> 11) as u8 % 3
            };
            return Cel {
                glyph: catalog::G_STAR + (h >> 24) as u8 % 8,
                color: rgb_index(palette::H_WHITE, star),
            };
        }

        // The sky itself, as coverage rather than as a flat wash.  A cell
        // is one colour, so brightness has to come from how much of the
        // cell is filled as well as from the luminance - which is the same
        // thing the ground does, and it is what lets an ASCII sky have a
        // gradient at all.  It also keeps the night empty: at the bottom of
        // the ramp the glyph is a blank and the sky is the black it always
        // was.
        if luma == 0 {
            return Cel::EMPTY;
        }
        Cel { glyph: sky_fill(luma), color: rgb_index(hue, luma) }
    }

    /// Where the moon lands on screen, if it is visible at all.
    ///
    /// Returns the column and row of its top-left cell; it occupies a 2x2
    /// block from there.
    pub fn moon_at(&self, cam: &Camera, w: usize, h: usize, horizon: i32) -> Option<(i32, i32)> {
        if !self.moon {
            return None;
        }
        // Bearing of the moon relative to where the camera is looking,
        // as a signed angle.
        let rel = self.moon_az.wrapping_sub(cam.yaw) as i16 as i32;
        let quarter = trig::QUARTER as i32;
        if rel.abs() >= quarter * 8 / 10 {
            return None; // behind you, or so far off-axis the projection blows up
        }
        // Project through the same camera plane the rays use: a direction at
        // angle `rel` meets the plane at tan(rel) / fov.
        let a = (rel as i64 * 65536 / quarter as i64) as i32; // -1..1 in Q16 of a quarter turn
        let s = trig::sin(rel as i16 as u16);
        let c = trig::cos(rel as i16 as u16);
        let _ = a;
        if c <= 0 {
            return None;
        }
        let camx = fixed::div(fixed::div(s, c), cam.fov);
        if camx.abs() > fixed::from_int(1) {
            return None;
        }
        let x = fixed::floor(fixed::mul(camx + fixed::ONE, fixed::from_int(w as i32 / 2)));
        let y = horizon - self.moon_alt;
        if y < -1 || y > h as i32 || x < -1 || x > w as i32 {
            return None;
        }
        Some((x, y))
    }

    /// Draw the moon and its halo.
    pub fn draw_moon(&self, f: &mut Frame, cam: &Camera, horizon: i32) {
        let Some((mx, my)) = self.moon_at(cam, f.w, f.h, horizon) else {
            return;
        };
        // Halo first, so the disc sits on top of it.
        let halo = rgb_index(palette::H_WHITE, if self.haze >= 4 { 3 } else { 2 });
        for dy in -1..=2i32 {
            for dx in -1..=2i32 {
                if (0..2).contains(&dx) && (0..2).contains(&dy) {
                    continue;
                }
                let q = ((dy.clamp(0, 1) as u8) << 1) | dx.clamp(0, 1) as u8;
                f.put(mx + dx, my + dy, Cel { glyph: catalog::G_HALO + q, color: halo });
            }
        }
        let disc = rgb_index(palette::H_WHITE, 7);
        for dy in 0..2i32 {
            for dx in 0..2i32 {
                let q = ((dy as u8) << 1) | dx as u8;
                f.put(mx + dx, my + dy, Cel { glyph: catalog::G_MOON + q, color: disc });
            }
        }
    }

    /// Lay rain over the finished frame.
    ///
    /// Rain goes on last, because it is in front of everything - but it is
    /// *not* laid on evenly.
    ///
    /// Against the night sky a streak is the only thing in the cell and
    /// reads immediately. Against a facade it is competing with the window
    /// grid that carries the whole picture, and at any density that shows up
    /// there it stops looking like weather and starts looking like the
    /// screen needs cleaning. So rain falls on the sky and on nothing else:
    /// it is weather in the distance, over the horizon and down the gaps
    /// between the towers, and the street you are driving on is dry.
    ///
    /// "Sky" is a blank glyph or a colour that has faded to black, so the
    /// distant buildings the haze has taken count as sky, which is what they
    /// look like - the curtain of rain therefore begins about where the city
    /// stops being legible, which is where weather belongs.
    ///
    /// It falls straight down. It used to lean with the camera's heading,
    /// which is a nice idea and reads as the whole screen being dragged
    /// sideways when you turn the wheel, and the streaks used to scroll
    /// *upwards*: the glyph's phase shifts the pattern up as it increases,
    /// so adding the tick to it made the rain rise.
    pub fn rain_over(&self, f: &mut Frame, cam: &Camera) {
        let _ = cam;
        if self.rain == 0 {
            return;
        }
        let sky_density = self.rain as u32 * 3;
        let scroll = (self.tick as i32 * 3) as u32;
        for y in 0..f.h as i32 {
            for x in 0..f.w as i32 {
                let behind = f.get(x, y);
                let on_sky =
                    behind.glyph == catalog::G_BLANK || palette::luma_of(behind.color) == 0;
                if !on_sky {
                    continue;
                }
                let h = hash3(x as u32, (y as u32).wrapping_add(scroll / 2), 0x_4241_4E00);
                if (h & 255) >= sky_density {
                    continue;
                }
                let luma = 3;
                let phase = ((y as u32).wrapping_sub(scroll) & 7) as u8;
                f.put(
                    x,
                    y,
                    Cel {
                        glyph: catalog::G_RAIN + phase,
                        color: rgb_index(palette::H_LIGHT_BLUE, luma),
                    },
                );
            }
        }
    }

    /// The diffuse light each of the five possible normals receives, as a
    /// luminance offset.
    ///
    /// This is the whole of the lighting model, and it is five numbers.
    ///
    /// A height field of axis-aligned cells presents exactly five normals -
    /// four walls and a roof - and the renderer already knows which one a
    /// ray hit, because that is [`crate::arch::Face`].  The moon is a
    /// *directional* source, so `L` is the same everywhere in the scene.
    /// Therefore `L·N` is five numbers, and they only change when the moon
    /// moves.
    ///
    /// Per frame, not per pixel and not per hit.  A textbook renderer
    /// evaluates a dot product per fragment; here the whole term collapses
    /// to a five-entry table and one addition at the point of use, because
    /// luminance is a three-bit nibble and adding an offset to it is the
    /// same operation as scaling it.
    ///
    /// Indexed by `Face as usize`, with [`crate::arch::ROOF`] last.
    pub fn lambert(&self) -> [i8; arch::NORMALS] {
        if !self.moon {
            return [0; arch::NORMALS];
        }
        // The moon's ground bearing, foreshortened by how high it is.  The
        // altitude is carried as screen rows rather than as an angle - see
        // `Camera::pitch` for why - so it is turned into a horizontal
        // fraction here rather than pretending to be a real elevation.
        let tilt = fixed::clamp(
            fixed::div(fixed::from_int(self.moon_alt), fixed::from_int(24)),
            0,
            ONE,
        );
        let flat = ONE - fixed::mul(tilt, fixed::ratio(3, 4));
        let lx = fixed::mul(trig::cos(self.moon_az), flat);
        let ly = fixed::mul(trig::sin(self.moon_az), flat);

        // N·L for each wall, which for an axis-aligned normal is just the
        // matching component of L with the matching sign.  No dot product
        // is actually evaluated - there is nothing left of one.
        let step = |nl: Fx| -> i8 {
            // Q16.16 in -1..1 to a luminance offset.  Asymmetric on
            // purpose: an eight-level ramp has far more room below a
            // building's base brightness than above it, so a face turned
            // away loses more than a face turned towards gains.
            let v = fixed::floor(fixed::mul(nl, fixed::from_int(3)));
            v.clamp(-2, 1) as i8
        };
        let mut t = [0i8; arch::NORMALS];
        t[Face::North as usize] = step(-ly);
        t[Face::East as usize] = step(lx);
        t[Face::South as usize] = step(ly);
        t[Face::West as usize] = step(-lx);
        // The roof faces up, and the moon is above the horizon whenever it
        // is up at all.
        t[arch::ROOF] = step(tilt.max(fixed::ratio(1, 3)));
        t
    }

    /// Whether the ground should be wet, which the ground pass uses to put
    /// puddles and reflections down.
    pub fn wet(&self) -> bool {
        self.rain >= 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sky twenty rows tall, which is about what a terminal gives.
    const SPAN: i32 = 20;

    /// The day goes all the way round and comes back.
    #[test]
    fn the_sky_visits_every_phase_and_returns() {
        let mut a = Atmos { day: 1200, ..Default::default() };
        let mut seen = vec![false; DAY.len()];
        let mut order = Vec::new();
        for t in 0..1200u32 {
            a.tick = t;
            let (i, _) = a.phase();
            seen[i] = true;
            if order.last() != Some(&i) {
                order.push(i);
            }
        }
        assert!(seen.iter().all(|&s| s), "some phase never happened: {seen:?}");
        assert_eq!(order, (0..DAY.len()).collect::<Vec<_>>(), "out of order");
        // ...and wraps.
        a.tick = 1200;
        assert_eq!(a.phase().0, 0, "the day did not come round again");
    }

    /// A sky is palest where the light is, which is the horizon.
    #[test]
    fn the_sky_is_a_gradient_from_the_horizon_up() {
        for n in 0..DAY.len() as u32 {
            let a = Atmos {
                day: 0,
                sky_offset: Atmos::phase_offset(n, 0),
                ..Default::default()
            };
            let (_, low) = a.sky_at(1, SPAN, 0, 0);
            let (_, high) = a.sky_at(SPAN, SPAN, 0, 0);
            assert!(
                low >= high,
                "{}: the zenith at {high} is brighter than the horizon at {low}",
                DAY[n as usize].name
            );
        }
    }

    /// A phase change climbs out of the horizon rather than being swapped
    /// in everywhere at once.
    #[test]
    fn a_new_sky_rises() {
        let each = DAY_TICKS / DAY.len() as u32;
        // A quarter of the way into a phase: the new hue is down at the
        // horizon and the old one is still overhead.
        let a = Atmos { day: DAY_TICKS, sky_offset: each / 4, ..Default::default() };
        let (low, _) = a.sky_at(1, SPAN, 0, 0);
        let (high, _) = a.sky_at(SPAN, SPAN, 0, 0);
        assert_eq!(low, DAY[0].hue, "the new sky is not at the horizon");
        assert_eq!(high, DAY[DAY.len() - 1].hue, "the old sky has already gone");
        // And by the half-way point it has taken the whole sky.
        let a = Atmos { day: DAY_TICKS, sky_offset: each * 3 / 4, ..Default::default() };
        assert_eq!(a.sky_at(SPAN, SPAN, 0, 0).0, DAY[0].hue, "it never finished rising");
    }

    /// Stars are only drawn where the sky is dark enough to have any.
    #[test]
    fn the_stars_go_out_at_dawn() {
        let night = Atmos {
            day: 0,
            sky_offset: Atmos::phase_offset(0, 0),
            stars: 8,
            ..Default::default()
        };
        let noon = Atmos { sky_offset: Atmos::phase_offset(5, 0), ..night };
        let count = |a: &Atmos| {
            (0..2000)
                .filter(|i| {
                    let c = a.sky((i * 31) as Ang, 1 + i % SPAN, SPAN);
                    catalog::is_star(c.glyph)
                })
                .count()
        };
        let dark = count(&night);
        assert!(dark > 20, "a clear night sky had {dark} stars in it");
        assert_eq!(count(&noon), 0, "there are stars out at noon");
    }

    /// The city is lit by the sky it is under.
    #[test]
    fn daylight_follows_the_sky() {
        let night = Atmos { day: 0, sky_offset: Atmos::phase_offset(0, 0), ..Default::default() };
        let noon = Atmos { sky_offset: Atmos::phase_offset(5, 0), ..night };
        let dawn = Atmos { sky_offset: Atmos::phase_offset(3, 0), ..night };
        assert_eq!(night.daylight(), 0, "the night is lighting the city");
        assert_eq!(noon.daylight(), 2, "noon is not");
        assert!(dawn.daylight() > night.daylight() && dawn.daylight() < noon.daylight());
    }

    /// Holding the sky holds it, and does not stop the rain.
    #[test]
    fn day_zero_holds_the_sky_where_it_was_put() {
        let mut a = Atmos {
            day: 0,
            sky_offset: Atmos::phase_offset(5, 0),
            ..Default::default()
        };
        let was = a.sky_at(4, SPAN, 0, 0);
        for _ in 0..DAY_TICKS * 2 {
            a.step();
        }
        assert_eq!(a.sky_at(4, SPAN, 0, 0), was, "it moved anyway");
        assert_eq!(a.phase_name(), "NOON");
    }
    use crate::arch::{self, Face};
use crate::camera::Camera;

    #[test]
    fn fade_reaches_black_at_the_draw_distance() {
        let a = Atmos { haze: 3, ..Default::default() };
        assert_eq!(a.fade(fixed::from_int(0)), 0);
        assert_eq!(a.fade(fixed::from_int(draw_distance(3))), 8);
        assert_eq!(a.fade(fixed::from_int(10_000)), 8);
    }

    #[test]
    fn fade_is_monotonic() {
        let a = Atmos::default();
        let mut last = 0;
        for d in 0..200 {
            let f = a.fade(fixed::from_int(d));
            assert!(f >= last, "fade went backwards at {d}");
            last = f;
        }
    }

    #[test]
    fn shading_keeps_the_hue_and_loses_the_light() {
        let a = Atmos::default();
        let near = a.shade(palette::H_BLUE, 7, fixed::from_int(1));
        let far = a.shade(palette::H_BLUE, 7, fixed::from_int(60));
        assert_eq!(palette::hue_of(near), palette::H_BLUE);
        assert_eq!(palette::hue_of(far), palette::H_BLUE);
        assert!(palette::luma_of(far) < palette::luma_of(near));
    }

    #[test]
    fn the_moon_is_in_front_of_you_or_it_is_not_drawn() {
        let a = Atmos { moon_az: trig::from_degrees(0.0), ..Default::default() };
        let facing = Camera { yaw: trig::from_degrees(0.0), ..Default::default() };
        let away = Camera { yaw: trig::from_degrees(180.0), ..Default::default() };
        assert!(a.moon_at(&facing, 80, 24, 12).is_some(), "the moon is dead ahead and missing");
        assert!(a.moon_at(&away, 80, 24, 12).is_none(), "the moon is behind you and drawn anyway");
    }

    #[test]
    fn the_moon_tracks_the_camera_the_right_way_round() {
        let a = Atmos { moon_az: trig::from_degrees(0.0), ..Default::default() };
        let left = Camera { yaw: trig::from_degrees(-12.0), ..Default::default() };
        let right = Camera { yaw: trig::from_degrees(12.0), ..Default::default() };
        let (lx, _) = a.moon_at(&left, 80, 24, 12).unwrap();
        let (rx, _) = a.moon_at(&right, 80, 24, 12).unwrap();
        assert!(rx < lx, "turning right must move the moon left, got {rx} vs {lx}");
    }

    #[test]
    fn stars_are_fixed_to_the_world_not_to_the_screen() {
        let a = Atmos::default();
        let ang = trig::from_degrees(41.0);
        assert_eq!(a.sky(ang, 5, SPAN).glyph, a.sky(ang, 5, SPAN).glyph);
    }

    #[test]
    fn a_face_turned_towards_the_moon_is_lit_and_one_turned_away_is_not() {
        let a = Atmos { moon: true, moon_az: trig::from_degrees(0.0), moon_alt: 6, ..Default::default() };
        let t = a.lambert();
        // Bearing zero is +x, so the east-facing wall takes the light.
        assert!(
            t[Face::East as usize] > t[Face::West as usize],
            "east {} west {} with the moon due east",
            t[Face::East as usize],
            t[Face::West as usize]
        );
        // ...and the two walls perpendicular to it are between the extremes.
        for f in [Face::North, Face::South] {
            assert!(t[f as usize] <= t[Face::East as usize]);
            assert!(t[f as usize] >= t[Face::West as usize]);
        }
    }

    #[test]
    fn moving_the_moon_moves_which_wall_is_lit() {
        let brightest = |deg: f64| {
            let a = Atmos { moon: true, moon_az: trig::from_degrees(deg), moon_alt: 6, ..Default::default() };
            let t = a.lambert();
            (0..4).max_by_key(|i| t[*i]).unwrap()
        };
        let east = brightest(0.0);
        let south = brightest(90.0);
        let west = brightest(180.0);
        assert_ne!(east, south, "the moon moved a quarter turn and nothing changed");
        assert_ne!(south, west);
        assert_ne!(east, west);
    }

    #[test]
    fn a_moonless_night_has_no_diffuse_term_at_all() {
        let a = Atmos { moon: false, ..Default::default() };
        assert_eq!(a.lambert(), [0; arch::NORMALS]);
    }

    #[test]
    fn the_roof_is_always_lit_when_the_moon_is_up() {
        for deg in (0..360).step_by(30) {
            let a = Atmos {
                moon: true,
                moon_az: trig::from_degrees(deg as f64),
                moon_alt: 9,
                ..Default::default()
            };
            assert!(a.lambert()[arch::ROOF] >= 0, "a roof in shadow at {deg} degrees");
        }
    }

    #[test]
    fn the_offsets_stay_inside_the_luminance_ramp() {
        // Eight levels total, so an offset that could move a building by
        // more than a few steps would clip half the palette away.
        for deg in (0..360).step_by(7) {
            for alt in [0, 6, 12, 24] {
                let a = Atmos {
                    moon: true,
                    moon_az: trig::from_degrees(deg as f64),
                    moon_alt: alt,
                    ..Default::default()
                };
                for v in a.lambert() {
                    assert!((-2..=1).contains(&v), "offset {v} at {deg} degrees, altitude {alt}");
                }
            }
        }
    }

    #[test]
    fn no_rain_means_no_overlay() {
        let mut f = Frame::new(20, 10);
        let a = Atmos { rain: 0, ..Default::default() };
        a.rain_over(&mut f, &Camera::default());
        assert!(f.cels.iter().all(|c| *c == Cel::EMPTY));
    }

    #[test]
    fn rain_covers_some_of_the_screen_but_not_all_of_it() {
        let mut f = Frame::new(80, 40);
        let a = Atmos { rain: 4, ..Default::default() };
        a.rain_over(&mut f, &Camera::default());
        let wet = f.cels.iter().filter(|c| **c != Cel::EMPTY).count();
        assert!(wet > 50, "only {wet} cells of rain");
        assert!(wet < f.cels.len() / 2, "{wet} cells - that is a wall of water");
    }

    /// Rain is weather in the distance, not spots on the lens.
    ///
    /// It used to fall over the buildings too, at a fifth of the density, on
    /// the grounds that rain in front of a facade is what rain looks like
    /// from inside it.  It is, and at any density you can see it also looks
    /// like a dirty screen - and this is a city where the near buildings are
    /// the picture.  Now the facade stays dry and the sky behind it does the
    /// weather.
    #[test]
    fn rain_falls_against_the_sky_and_not_over_the_buildings() {
        // Left half is a lit facade, right half is night sky.
        let mut f = Frame::new(80, 60);
        for y in 0..60 {
            for x in 0..40 {
                f.put(x, y, Cel { glyph: catalog::G_SOLID, color: rgb_index(palette::H_BLUE, 6) });
            }
        }
        let a = Atmos { rain: 8, ..Default::default() };
        a.rain_over(&mut f, &Camera { yaw: 0, ..Default::default() });

        let is_rain = |c: Cel| (catalog::G_RAIN..catalog::G_MOON).contains(&c.glyph);
        let built: usize = (0..60)
            .flat_map(|y| (0..40).map(move |x| (x, y)))
            .filter(|&(x, y)| is_rain(f.get(x, y)))
            .count();
        let sky: usize = (0..60)
            .flat_map(|y| (40..80).map(move |x| (x, y)))
            .filter(|&(x, y)| is_rain(f.get(x, y)))
            .count();

        assert!(sky > 100, "the sky is barely raining ({sky} cells)");
        assert_eq!(built, 0, "{built} cells of rain over the buildings");
    }

    /// The streaks fall downwards.
    ///
    /// Two links in one chain, and the bug was in the join between them.
    /// [`crate::font::rain_streak`] shifts its pattern *up* the cell as the
    /// phase rises - that is asserted in `font` - so the phase has to count
    /// down with the tick for the rain to come down.  The obvious "add the
    /// tick" makes rain that rises, and nobody looks at a still frame long
    /// enough to notice which way the streaks are going.
    #[test]
    fn rain_falls_downwards() {
        for tick in [0u32, 1, 2, 7, 100] {
            let mut f = Frame::new(24, 24);
            let a = Atmos { rain: 8, tick, ..Default::default() };
            a.rain_over(&mut f, &Camera::default());
            let mut seen = 0;
            for y in 0..24i32 {
                for x in 0..24i32 {
                    let g = f.get(x, y).glyph;
                    if !(catalog::G_RAIN..catalog::G_RAIN + 8).contains(&g) {
                        continue;
                    }
                    seen += 1;
                    let want = ((y as u32).wrapping_sub(tick * 3) & 7) as u8;
                    assert_eq!(
                        g - catalog::G_RAIN,
                        want,
                        "tick {tick}, row {y}: the streak is not walking down the screen"
                    );
                }
            }
            assert!(seen > 0, "tick {tick}: no rain to check");
        }
    }

    #[test]
    fn a_faded_out_building_counts_as_sky() {
        // Something the haze has taken to black should get the full fall,
        // because that is what it looks like.
        let mut f = Frame::new(60, 40);
        for c in f.cels.iter_mut() {
            *c = Cel { glyph: catalog::G_SOLID, color: rgb_index(palette::H_BLUE, 0) };
        }
        let a = Atmos { rain: 8, ..Default::default() };
        a.rain_over(&mut f, &Camera::default());
        let wet = f
            .cels
            .iter()
            .filter(|c| (catalog::G_RAIN..catalog::G_MOON).contains(&c.glyph))
            .count();
        assert!(wet > 100, "only {wet} cells fell on the faded-out wall");
    }

    #[test]
    fn the_default_is_a_drizzle_rather_than_a_downpour() {
        let mut f = Frame::new(100, 40);
        Atmos::default().rain_over(&mut f, &Camera::default());
        let wet = f.cels.iter().filter(|c| **c != Cel::EMPTY).count();
        assert!(
            wet * 10 < f.cels.len(),
            "the default weather wets {wet} of {} cells",
            f.cels.len()
        );
    }
}

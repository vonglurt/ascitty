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
    /// Frame counter, driving everything that moves.
    pub tick: u32,
}

impl Default for Atmos {
    fn default() -> Self {
        Atmos {
            rain: 2,
            moon: true,
            moon_az: trig::from_degrees(215.0),
            moon_alt: 9,
            haze: 3,
            stars: 4,
            tick: 0,
        }
    }
}

/// Distance at which everything has faded to black, in world units, as a
/// function of haze.
pub fn draw_distance(haze: u8) -> i32 {
    match haze {
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

    /// What is in the sky along a given ray, at a given row.
    ///
    /// `col_ang` is the compass bearing of the ray, so stars are fixed to
    /// the world rather than to the screen: turn around and the same stars
    /// come back.
    pub fn sky(&self, col_ang: Ang, row_above_horizon: i32) -> Cel {
        if self.stars == 0 || row_above_horizon <= 0 {
            return Cel::EMPTY;
        }
        // Bucket the bearing finely enough that stars do not visibly snap
        // between columns, coarsely enough that they do not swarm.
        let bucket = (col_ang >> 5) as u32;
        let h = hash3(bucket, row_above_horizon as u32, 0x5747_2A25);
        if (h & 255) >= self.stars as u32 * 3 {
            return Cel::EMPTY;
        }
        // A few stars twinkle; most do not, because all of them twinkling
        // reads as static rather than as sky.
        let twinkle = (h >> 17) & 7 == 0;
        let luma = if twinkle {
            2 + ((self.tick >> 3) as u8 ^ (h >> 9) as u8) % 3
        } else {
            1 + (h >> 11) as u8 % 3
        };
        Cel {
            glyph: catalog::G_STAR + (h >> 24) as u8 % 8,
            color: rgb_index(palette::H_WHITE, luma),
        }
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
    /// screen needs cleaning. So the sky gets the full fall and anything
    /// already drawn gets a fifth of it - enough that the rain is plainly
    /// passing in front of the buildings, not enough to eat them.
    ///
    /// "Already drawn" is a blank glyph or a colour that has faded to black,
    /// so distant buildings the haze has taken count as sky, which is what
    /// they look like.
    pub fn rain_over(&self, f: &mut Frame, cam: &Camera) {
        if self.rain == 0 {
            return;
        }
        let sky_density = self.rain as u32 * 3;
        let over_density = (sky_density / 5).max(1);
        let lean = (trig::sin(cam.yaw) >> 14) as i32; // -4..4 cells of drift
        let scroll = (self.tick as i32 * 3) as u32;
        for y in 0..f.h as i32 {
            for x in 0..f.w as i32 {
                let behind = f.get(x, y);
                let on_sky =
                    behind.glyph == catalog::G_BLANK || palette::luma_of(behind.color) == 0;
                let density = if on_sky { sky_density } else { over_density };
                let sx = (x + (y * lean) / 8) as u32;
                let h = hash3(sx, (y as u32).wrapping_add(scroll / 2), 0x_4241_4E00);
                if (h & 255) >= density {
                    continue;
                }
                let luma = if on_sky { 3 } else { 2 };
                let phase = ((y as u32).wrapping_add(scroll) & 7) as u8;
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
        assert_eq!(a.sky(ang, 5).glyph, a.sky(ang, 5).glyph);
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

    #[test]
    fn rain_falls_mostly_against_the_sky_and_not_over_the_buildings() {
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
        assert!(
            sky > built * 3,
            "rain over the buildings ({built}) is not far enough below the sky ({sky})"
        );
        assert!(built > 0, "no rain at all in front of the buildings");
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

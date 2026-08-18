//! Weather and sky: rain, the moon, stars, and the haze that eats distance.
//!
//! All four are cheap on purpose.  Rain is not particles - it is a glyph
//! family and a scrolling hash, so a downpour costs one table lookup per wet
//! cell and nothing at all per drop.  Stars are a hash of the direction you
//! are facing, so they stay put as you turn without being stored.  The moon
//! is four glyphs.  Every one of these has to survive on a 1.76 MHz machine,
//! and none of them may allocate.

use crate::camera::Camera;
use crate::catalog;
use crate::fixed::{self, Fx};
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
            rain: 3,
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
    /// Rain is in front of everything, so it goes on last, and it leans with
    /// the camera's heading so that turning into the weather looks like
    /// turning into the weather.
    pub fn rain_over(&self, f: &mut Frame, cam: &Camera) {
        if self.rain == 0 {
            return;
        }
        let density = self.rain as u32 * 5;
        let lean = (trig::sin(cam.yaw) >> 14) as i32; // -4..4 cells of drift
        let scroll = (self.tick as i32 * 3) as u32;
        for y in 0..f.h as i32 {
            for x in 0..f.w as i32 {
                let sx = (x + (y * lean) / 8) as u32;
                let h = hash3(sx, (y as u32).wrapping_add(scroll / 2), 0x_4241_4E00);
                if (h & 255) >= density {
                    continue;
                }
                // Rain in front of a lit window is brighter than rain in
                // front of the sky, which is what makes it read as falling
                // through the light rather than as screen dirt.
                let behind = f.get(x, y);
                let luma = if behind.glyph == catalog::G_BLANK { 2 } else { 4 };
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

    /// Whether the ground should be wet, which the ground pass uses to put
    /// puddles and reflections down.
    pub fn wet(&self) -> bool {
        self.rain >= 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}

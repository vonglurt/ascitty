//! Where you are standing and which way you are looking.
//!
//! The camera is a first-person one: eye height a bit under two units, feet
//! on the ground, and it cannot walk through a building.  There is no flying
//! mode, because the whole point of a city rendered at eye level is that the
//! towers are things you look *up* at.

use crate::fixed::{self, Fx};
use crate::trig::{self, Ang};
use crate::world::City;

/// Eye height above the pavement, in world units.
///
/// A cell is about six metres, so this is 1.8 m - a person.  It is worth
/// being fussy about: the eye height *is* the horizon, and the horizon is
/// where every vanishing point in the picture converges.  Half a metre out
/// and the whole city looks like a model of a city.
pub const EYE: Fx = fixed::ratio(3, 10);

/// How close you may get to a wall.
pub const RADIUS: Fx = fixed::ratio(1, 5);

/// The camera.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// Position, in cell units.
    pub x: Fx,
    /// Position, in cell units.
    pub y: Fx,
    /// Eye height above the ground.
    pub z: Fx,
    /// Heading.
    pub yaw: Ang,
    /// Vertical look, in screen rows added to the horizon.  Rows rather than
    /// an angle: a text renderer can only shear the horizon, not rotate it,
    /// and pretending otherwise would put curved building edges on a grid
    /// that has no way to draw them.
    pub pitch: i32,
    /// Half-width of the camera plane.  Larger is a wider lens.
    pub fov: Fx,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            x: fixed::from_int(8),
            y: fixed::from_int(8),
            z: EYE,
            yaw: 0,
            pitch: 0,
            fov: fixed::ratio(2, 3),
        }
    }
}

impl Camera {
    /// Drop the camera in the street nearest `(x, y)`.
    ///
    /// Two failures to avoid, in order of how bad they are: spawning inside
    /// a building, which a first-person renderer cannot recover from at all,
    /// and spawning in the middle of a block, which merely means the first
    /// thing anyone sees is a courtyard.  So the search prefers roadway,
    /// falls back to any walkable ground, and only then gives up.
    pub fn spawn(city: &City, x: i32, y: i32) -> Camera {
        let mut best = (x, y);
        let mut fallback: Option<(i32, i32)> = None;
        'search: for r in 0..48i32 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue; // only the ring, not the disc
                    }
                    let (px, py) = (x + dx, y + dy);
                    if !city.walkable(px, py) {
                        continue;
                    }
                    if city.at(px, py).kind == crate::world::Kind::Road {
                        best = (px, py);
                        break 'search;
                    }
                    fallback.get_or_insert((px, py));
                }
            }
        }
        if !city.walkable(best.0, best.1) {
            if let Some(f) = fallback {
                best = f;
            }
        }
        Camera {
            x: fixed::from_int(best.0) + fixed::HALF,
            y: fixed::from_int(best.1) + fixed::HALF,
            ..Default::default()
        }
    }

    /// Unit vector along the view direction.
    #[inline(always)]
    pub fn dir(&self) -> (Fx, Fx) {
        (trig::cos(self.yaw), trig::sin(self.yaw))
    }

    /// The camera plane: perpendicular to the direction, scaled by the
    /// field of view.  A ray is `dir + plane * t` for `t` in `-1..=1`, and
    /// because its component along `dir` is always exactly one, the distance
    /// it travels *is* the perpendicular distance - which is the whole
    /// reason to do it this way rather than with per-column angles.  No
    /// fisheye correction, no cosine per column.
    #[inline(always)]
    pub fn plane(&self) -> (Fx, Fx) {
        let (dx, dy) = self.dir();
        (fixed::mul(-dy, self.fov), fixed::mul(dx, self.fov))
    }

    /// Turn.  Positive is clockwise, seen from above.
    pub fn turn(&mut self, by: i32) {
        self.yaw = self.yaw.wrapping_add(by as Ang);
    }

    /// Look up or down, clamped so the horizon stays on screen.
    pub fn look(&mut self, rows: i32, limit: i32) {
        self.pitch = (self.pitch + rows).clamp(-limit, limit);
    }

    /// Walk, sliding along walls rather than stopping dead at them.
    ///
    /// The two axes are resolved separately, which is what makes a corner
    /// feel like a corner: running into a wall at an angle should slide you
    /// along it, not pin you to it.
    pub fn walk(&mut self, city: &City, forward: Fx, strafe: Fx) {
        let (dx, dy) = self.dir();
        let (px, py) = (-dy, dx);
        let mx = fixed::mul(dx, forward) + fixed::mul(px, strafe);
        let my = fixed::mul(dy, forward) + fixed::mul(py, strafe);

        let nx = self.x + mx;
        if clear(city, nx + RADIUS.copysign(mx), self.y) {
            self.x = nx;
        }
        let ny = self.y + my;
        if clear(city, self.x, ny + RADIUS.copysign(my)) {
            self.y = ny;
        }
    }
}

/// Whether a point is in open air at eye level.
#[inline]
fn clear(city: &City, x: Fx, y: Fx) -> bool {
    city.walkable(fixed::floor(x), fixed::floor(y))
}

/// `Fx` has no `copysign`, and the sign that matters here is the *movement's*
/// - the collision probe has to be pushed in the direction of travel.
trait CopySign {
    fn copysign(self, sign: Fx) -> Fx;
}

impl CopySign for Fx {
    #[inline(always)]
    fn copysign(self, sign: Fx) -> Fx {
        if sign < 0 {
            -self
        } else if sign > 0 {
            self
        } else {
            0
        }
    }
}

/// One unit of walking speed, per second.
pub const WALK_SPEED: Fx = fixed::ratio(9, 2);
/// Turning speed, in angle units per second.
pub const TURN_SPEED: i32 = 24_000;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::City;

    #[test]
    fn a_ray_at_the_centre_of_the_screen_is_the_view_direction() {
        let c = Camera { yaw: trig::from_degrees(37.0), ..Default::default() };
        let (dx, dy) = c.dir();
        let len = fixed::mul(dx, dx) + fixed::mul(dy, dy);
        assert!((len - fixed::ONE).abs() < 64, "the view direction is not a unit vector");
    }

    #[test]
    fn the_plane_is_perpendicular_to_the_direction() {
        for deg in [0.0, 17.0, 90.0, 213.5, 359.0] {
            let c = Camera { yaw: trig::from_degrees(deg), ..Default::default() };
            let (dx, dy) = c.dir();
            let (px, py) = c.plane();
            let dot = fixed::mul(dx, px) + fixed::mul(dy, py);
            assert!(dot.abs() < 64, "plane is not perpendicular at {deg} degrees: {dot}");
        }
    }

    #[test]
    fn spawn_never_lands_inside_a_building() {
        let city = City::generate(7);
        for (x, y) in [(0, 0), (48, 48), (95, 95), (30, 61)] {
            let c = Camera::spawn(&city, x, y);
            assert!(
                city.walkable(fixed::floor(c.x), fixed::floor(c.y)),
                "spawned inside a building near {x},{y}"
            );
        }
    }

    #[test]
    fn walking_into_a_wall_does_not_go_through_it() {
        let city = City::generate(11);
        let mut c = Camera::spawn(&city, 48, 48);
        for _ in 0..4000 {
            c.walk(&city, fixed::ratio(1, 8), 0);
            c.turn(1237);
            assert!(
                city.walkable(fixed::floor(c.x), fixed::floor(c.y)),
                "walked into a building at {},{}",
                fixed::to_f32(c.x),
                fixed::to_f32(c.y)
            );
        }
    }

    #[test]
    fn turning_wraps_instead_of_overflowing() {
        let mut c = Camera::default();
        for _ in 0..1000 {
            c.turn(30_000); // would overflow an i16 in a dozen steps
        }
    }

    #[test]
    fn pitch_is_clamped() {
        let mut c = Camera::default();
        c.look(500, 12);
        assert_eq!(c.pitch, 12);
        c.look(-5000, 12);
        assert_eq!(c.pitch, -12);
    }
}

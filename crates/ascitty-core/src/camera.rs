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
    /// Drop the camera on the pavement nearest `(x, y)`.
    ///
    /// Two failures to avoid, in order of how bad they are: spawning inside
    /// a building, which a first-person renderer cannot recover from at all,
    /// and spawning in the middle of a block, which merely means the first
    /// thing anyone sees is a courtyard.  So the search prefers the
    /// pedestrian network - pavement, plaza, park - falls back to any
    /// walkable ground, and only then gives up.
    ///
    /// It preferred the *carriageway* until traffic learned to keep its
    /// lane, at which point standing in the middle of one stopped being
    /// harmless.
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
                    if !city.open(px, py) {
                        continue;
                    }
                    // The pavement, the parks and the plazas - the places a
                    // person may stand.  It used to be the nearest road,
                    // which put you in the middle of a carriageway with the
                    // traffic coming, and put everything placed relative to
                    // you there as well.  The walking network is the same
                    // map the pedestrians use, so spawning on it means
                    // spawning somewhere they could have walked from.
                    if city.walk.at(px, py) == crate::walk::Foot::Path {
                        best = (px, py);
                        break 'search;
                    }
                    fallback.get_or_insert((px, py));
                }
            }
        }
        if !city.open(best.0, best.1) {
            if let Some(f) = fallback {
                best = f;
            }
        }
        let mut cam = Camera {
            x: fixed::from_int(best.0) + fixed::HALF,
            y: fixed::from_int(best.1) + fixed::HALF,
            ..Default::default()
        };
        // The ground is not at sea level everywhere.  A camera left at the
        // default eye height on ground that has risen is *underneath* it,
        // and everything it renders is wrong in a way that looks like the
        // renderer is broken rather than like the camera is buried.
        cam.stand(city);
        cam
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
    pub fn walk(&mut self, city: &City, forward: Fx, strafe: Fx) {
        let (dx, dy) = self.dir();
        let (px, py) = (-dy, dx);
        self.slide(
            city,
            fixed::mul(dx, forward) + fixed::mul(px, strafe),
            fixed::mul(dy, forward) + fixed::mul(py, strafe),
        );
    }

    /// Put the eye at head height above whatever the ground is doing here.
    ///
    /// Called every frame rather than only on moving, because the ground
    /// under a stationary camera can still change - the terrain generator
    /// levels a pad under every building it raises, so a camera standing
    /// next to a lot can find itself on a kerb that was not there when the
    /// city was first laid out.
    pub fn stand(&mut self, city: &City) {
        self.z = city.ground(fixed::floor(self.x), fixed::floor(self.y)) + EYE;
    }

    /// Move by a world-space delta, sliding along walls.
    ///
    /// The two axes are resolved separately, which is what makes a corner
    /// feel like a corner: running into a wall at an angle should slide you
    /// along it, not pin you to it.
    ///
    /// Separate from [`Camera::walk`] because the autopilot needs to move
    /// along its *heading* while the camera is looking somewhere else - you
    /// do not stop walking to look up at a building.
    pub fn slide(&mut self, city: &City, mx: Fx, my: Fx) {
        self.slide_where(mx, my, |x, y| city.open(x, y));
    }

    /// Move by a world-space delta, but only onto ground `allowed` accepts.
    ///
    /// The autopilot needs this. "Not built on" is the right test for a
    /// vehicle and the wrong one for a camera touring the streets: parks and
    /// plazas pass it, they are clearings in the middle of blocks, and a
    /// camera that drifts into one is surrounded on all sides with nothing
    /// to look at but the backs of buildings.
    pub fn slide_where(&mut self, mx: Fx, my: Fx, allowed: impl Fn(i32, i32) -> bool) {
        let nx = self.x + mx;
        if allowed(fixed::floor(nx + RADIUS.copysign(mx)), fixed::floor(self.y)) {
            self.x = nx;
        }
        let ny = self.y + my;
        if allowed(fixed::floor(self.x), fixed::floor(ny + RADIUS.copysign(my))) {
            self.y = ny;
        }
    }
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

/// The camera-plane half-width for a horizontal field of view in degrees.
///
/// `fov` is stored as a half-width rather than as an angle because that is
/// what the ray equation wants - see `docs/raytracing.md` §1.1 - but nobody
/// thinks in half-widths, so this is the conversion:
///
/// ```text
///     half_width = tan(degrees / 2)
/// ```
///
/// Clamped well short of 180, where the tangent goes to infinity and the
/// projection with it.  Note that this is a *planar* projection, so a very
/// wide angle stretches the edges of the frame hard - that is not a defect,
/// it is what a wide rectilinear lens does, and past about 120 degrees it
/// stops being a view and starts being a smear.
pub fn fov_for_degrees(deg: f64) -> Fx {
    let half = (deg.clamp(20.0, 160.0) / 2.0).to_radians();
    fixed::from_f64(half.tan())
}

/// The horizontal field of view, in degrees, for a plane half-width.
pub fn degrees_for_fov(fov: Fx) -> f64 {
    2.0 * fixed::to_f32(fov).atan().to_degrees() as f64
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
                city.open(fixed::floor(c.x), fixed::floor(c.y)),
                "spawned inside a building near {x},{y}"
            );
        }
    }

    /// You start on the pavement, not in the road.
    #[test]
    fn spawn_puts_you_where_a_person_may_stand() {
        for seed in [1u32, 7, 99, 4242] {
            let city = City::generate(seed);
            for (x, y) in [(0, 0), (48, 48), (95, 95), (30, 61), (128, 128)] {
                let c = Camera::spawn(&city, x, y);
                let (cx, cy) = (fixed::floor(c.x), fixed::floor(c.y));
                assert_eq!(
                    city.walk.at(cx, cy),
                    crate::walk::Foot::Path,
                    "seed {seed}: spawned at {cx},{cy} near {x},{y}, which is not pavement"
                );
            }
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
                city.open(fixed::floor(c.x), fixed::floor(c.y)),
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
    fn the_field_of_view_round_trips_through_degrees() {
        for deg in [40.0, 67.0, 90.0, 110.0, 140.0] {
            let fov = fov_for_degrees(deg);
            let back = degrees_for_fov(fov);
            assert!((back - deg).abs() < 1.0, "{deg} degrees came back as {back}");
        }
    }

    #[test]
    fn a_wider_angle_is_a_wider_plane() {
        assert!(fov_for_degrees(110.0) > fov_for_degrees(67.0));
        assert!(fov_for_degrees(67.0) > fov_for_degrees(40.0));
    }

    #[test]
    fn the_default_is_about_sixty_seven_degrees() {
        // The figure the whole renderer was tuned against, written down so
        // that changing it is a decision rather than a drift.
        let d = degrees_for_fov(Camera::default().fov);
        assert!((d - 67.0).abs() < 2.0, "the default field of view is {d} degrees");
    }

    #[test]
    fn an_absurd_angle_is_clamped_rather_than_exploding() {
        // tan(90 degrees) is infinite and the projection divides by it.
        for deg in [0.0, 1.0, 179.0, 360.0, -50.0] {
            let fov = fov_for_degrees(deg);
            assert!(fov > 0 && fov < fixed::from_int(20), "{deg} degrees gave a plane of {fov}");
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

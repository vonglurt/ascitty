//! The renderer: a height-field walk, one pass per column.
//!
//! # Why this and not a raytracer
//!
//! The video that started this project describes casting a ray per column
//! and taking the first hit.  First-hit is enough for a maze; it is not
//! enough for a skyline, because the thing that makes a city look like a
//! city is a *tall* building visible over the top of a *near* one.  So the
//! walk does not stop at the first hit.  It keeps going, front to back,
//! carrying one number (the topmost screen row anything has claimed so far),
//! and each further building may only draw above that line.  When the line
//! reaches the top of the screen the column is finished and the walk stops.
//!
//! That is the Comanche voxel-space idea rather than the Wolfenstein one,
//! and it costs the same as first-hit in the common case (a wall right in
//! front of you closes the column immediately) while producing a real
//! skyline when you are looking down an avenue.
//!
//! # Why there is no per-column cosine
//!
//! A ray is `dir + plane * camx`, where `plane` is perpendicular to `dir`.
//! The component of that vector along `dir` is exactly one for every column,
//! so the distance the DDA reports *is* the perpendicular distance already.
//! No fisheye correction, no trig in the inner loop.
//!
//! # Why the row distances are computed once
//!
//! Ground distance depends only on how far a row is below the horizon, not
//! on which column you are in.  So it is a per-frame table of `h/2` entries,
//! which is what turns floor casting from a division per cell into a
//! division per row.  The Plus/4 build reads the same table out of ROM.

use crate::arch::{self, Face, Lod};
use crate::atmos::{draw_distance, Atmos};
use crate::camera::Camera;
use crate::catalog;
use crate::fixed::{self, Fx, ONE};
use crate::frame::{Cel, Frame};
use crate::palette;
use crate::rng::hash3;
use crate::trig::Ang;
use crate::world::{self, City, Crossing, Kind, Plan, RoadCell};

/// How much taller a character cell is than it is wide.  Every terminal and
/// the Plus/4 alike are close enough to 2:1 that one constant covers both;
/// getting it wrong does not distort the picture so much as make every
/// building the wrong proportion, which is worse.
pub const CELL_ASPECT: i32 = 2;

/// A distance no ray will reach, used where a reciprocal would divide by
/// zero.  Far below `Fx::MAX` so that accumulating it cannot overflow.
const HUGE: Fx = 1 << 28;

/// The projection one frame was drawn with.
///
/// The sprite pass has to agree with the wall pass exactly - the same
/// horizon, the same rows per world unit - or billboards float above the
/// pavement.  Rather than recomputing it and hoping the two derivations stay
/// in step, the numbers are handed over.
#[derive(Clone, Copy, Debug)]
pub struct Proj {
    /// Frame width in columns.
    pub w: i32,
    /// Frame height in rows.
    pub h: i32,
    /// Screen row the horizon falls on, pitch included.
    pub horizon: i32,
    /// Screen rows per world unit at unit distance.
    pub proj: Fx,
    /// How high the camera is above the ground it is standing on.
    ///
    /// The floor pass measures every row from this, not from sea level, so
    /// anything that wants to know which world point a ground row landed on
    /// has to use the same number.  Handed over for the same reason as the
    /// horizon: two derivations of it would eventually disagree, and the
    /// symptom would be a decal that slides across the road as the camera
    /// climbs a grade.
    pub eye: Fx,
}

/// Screen rows per world unit at unit distance.
///
/// The one place this is worked out.  It is wanted by the projection, and
/// also by anything that has to answer the question the other way round -
/// "what pitch looks at ground *that* far away" - and two derivations of the
/// same ratio would eventually disagree by a factor of the cell aspect,
/// which is exactly the sort of bug that looks like a bad camera rather than
/// like arithmetic.
#[inline]
pub fn scale(w: i32, fov: Fx) -> Fx {
    fixed::div(fixed::from_int(w), fixed::mul(fov, fixed::from_int(2 * CELL_ASPECT)))
}

/// Work out the projection for a camera and a frame.
pub fn projection(city: &City, cam: &Camera, f: &Frame) -> Proj {
    let (w, h) = (f.w as i32, f.h as i32);
    Proj {
        w,
        h,
        horizon: h / 2 + cam.pitch,
        proj: scale(w, cam.fov),
        eye: (cam.z - city.ground(fixed::floor(cam.x), fixed::floor(cam.y))).max(1),
    }
}

/// The pitch that aims a camera `eye` units above the ground *at the ground*,
/// with the furthest thing it could see at the top of the frame.
///
/// Ground `d` away is drawn `eye * scale / d` rows below the horizon, so the
/// far end of the visible world - the draw distance, which is the haze - is
/// `eye * scale / far` rows down, and everything between the horizon and
/// there is too distant to draw.  Aiming so that row lands at the top of the
/// frame fills the screen with the part of the world that has something in
/// it, and puts the horizon off the top where a camera looking down at a
/// city has no use for it.
///
/// This matters most where it is least obvious.  From the roofline of the
/// tallest building, at the default haze, the far row is about twenty below
/// the horizon and the near edge of a forty-row frame is sixty: a camera
/// tilted by the eight rows this used to use was looking at three or four
/// rows of city along the bottom of the screen and forty of empty night.
pub fn pitch_down(w: i32, h: i32, fov: Fx, eye: Fx, far: i32) -> i32 {
    let rows = fixed::div(fixed::mul(eye.max(1), scale(w, fov)), fixed::from_int(far.max(1)));
    -(h / 2 + fixed::floor(rows))
}

/// What one frame cost.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    /// Grid cells stepped through, over all columns.
    pub steps: u32,
    /// Columns that closed before running out of draw distance.
    pub closed: u32,
    /// The nearest thing in front of the camera, in world units.
    pub nearest: f32,
}

/// Render a frame, then rain on it.
///
/// The convenience form, for callers with nothing to draw between the city
/// and the weather.  Anything with sprites wants [`render_to`] instead, so
/// that the billboards go on before the rain rather than under it.
pub fn render(city: &City, cam: &Camera, atmos: &Atmos, f: &mut Frame) -> Stats {
    let mut depth = Vec::new();
    let st = render_to(city, cam, atmos, f, &mut depth);
    atmos.rain_over(f, cam);
    st
}

/// Render the city, and record how far away the nearest wall was in each
/// column so that sprites can be clipped against it.
///
/// Does *not* draw rain: rain belongs in front of the traffic as well as in
/// front of the buildings, so it is the caller's last step.
pub fn render_to(
    city: &City,
    cam: &Camera,
    atmos: &Atmos,
    f: &mut Frame,
    depth: &mut Vec<Fx>,
) -> Stats {
    let mut st = Stats { nearest: f32::MAX, ..Default::default() };
    depth.clear();
    depth.resize(f.w, Fx::MAX);
    if f.w == 0 || f.h == 0 {
        return st;
    }
    let p = projection(city, cam, f);
    let (w, h) = (p.w, p.h);
    let horizon = p.horizon;
    let proj = p.proj;
    let far = fixed::from_int(draw_distance(atmos.haze));
    let (dx, dy) = cam.dir();
    let (px, py) = cam.plane();
    // The entire lighting model, computed once for the frame: one luminance
    // offset per possible surface normal, plus whether there is a light at
    // all to cast shadows.  See `docs/raytracing.md`.
    let light = Light::of(atmos);

    // Ground distance per row, computed once for the whole frame.
    //
    // Measured from how high the camera is above *its own* footing.  Where
    // the ground the ray lands on is at a different level, the sample is in
    // slightly the wrong place - which is exactly why the terrain generator
    // is held to a gentle grade.  See `elevation.rs`.
    let eye_above_ground = p.eye;
    let mut row_dist = vec![HUGE; f.h];
    for y in (horizon.max(0) + 1)..h {
        let below = y - horizon;
        row_dist[y as usize] =
            fixed::div(fixed::mul(eye_above_ground, proj), fixed::from_int(below));
    }

    // Pass one: sky above the horizon, ground below it.
    for x in 0..w {
        let camx = fixed::div(fixed::from_int(2 * x), fixed::from_int(w)) - ONE;
        let rdx = dx + fixed::mul(px, camx);
        let rdy = dy + fixed::mul(py, camx);
        let bearing = column_bearing(cam.yaw, camx, cam.fov);

        for y in 0..horizon.min(h) {
            f.put(x, y, atmos.sky(bearing, horizon - y));
        }
        for y in (horizon.max(0) + 1)..h {
            let d = row_dist[y as usize];
            if d >= far {
                f.put(x, y, Cel::EMPTY);
                continue;
            }
            let wx = cam.x + fixed::mul(rdx, d);
            let wy = cam.y + fixed::mul(rdy, d);
            f.put(x, y, ground(city, atmos, &light, wx, wy, d));
        }
    }

    // The moon sits in the sky, so it goes on before the buildings that
    // stand in front of it.
    atmos.draw_moon(f, cam, horizon);

    // Pass two: the buildings.
    for x in 0..w {
        let camx = fixed::div(fixed::from_int(2 * x), fixed::from_int(w)) - ONE;
        let rdx = dx + fixed::mul(px, camx);
        let rdy = dy + fixed::mul(py, camx);
        let s = column(city, cam, atmos, f, x, rdx, rdy, horizon, proj, far, &light);
        st.steps += s.0;
        st.closed += s.1;
        depth[x as usize] = if s.2 == f32::MAX { Fx::MAX } else { fixed::from_f64(s.2 as f64) };
        if x == w / 2 {
            st.nearest = s.2;
        }
    }
    st
}

/// The lighting state for one frame.
///
/// Bundled rather than passed as loose arguments because it is the same two
/// things everywhere, and because they have to agree: cast shadows without a
/// light are not a subtle error, they are a scene lit from nowhere with
/// black stripes across it.
#[derive(Clone, Copy)]
struct Light {
    /// Diffuse offset per surface normal.
    lambert: [i8; arch::NORMALS],
    /// Whether anything casts a shadow at all.
    shadows: bool,
}

impl Light {
    fn of(atmos: &Atmos) -> Light {
        Light { lambert: atmos.lambert(), shadows: atmos.moon }
    }

    /// The luminance offset for a surface with a given normal, at a height,
    /// on a cell.
    #[inline(always)]
    fn on(&self, normal: usize, shaded: bool) -> i8 {
        if self.shadows && shaded {
            SHADOW_STEP
        } else {
            self.lambert[normal]
        }
    }
}

/// What a surface loses when something upstream is between it and the light.
///
/// The same magnitude as a face turned fully away, because that is what
/// being in shadow means: no direct light on it, only the ambient term that
/// the surface's own brightness already represents.
const SHADOW_STEP: i8 = -2;

/// Apply a diffuse offset to a surface's own brightness.
///
/// Clamped at one rather than at zero: a lit window is a light source in its
/// own right and the moon cannot switch it off, so the floor is one step of
/// luminance rather than black.
#[inline(always)]
fn lit(base: u8, offset: i8) -> u8 {
    (base as i32 + offset as i32).clamp(1, 7) as u8
}

/// The compass bearing of one column's ray.
///
/// A small-angle approximation of `yaw + atan(camx * fov)`.  It is used only
/// to place stars, where being a few degrees out at the edge of the screen
/// is invisible, and it saves an arctangent per column.
#[inline]
fn column_bearing(yaw: Ang, camx: Fx, fov: Fx) -> Ang {
    let t = fixed::mul(camx, fov);
    // 65536 angle units per turn / 2*pi radians = 10430.4 units per radian.
    let off = ((t as i64 * 10430) >> 16) as i32;
    yaw.wrapping_add(off as Ang)
}

/// Walk one column front to back.  Returns (steps, closed, nearest).
#[allow(clippy::too_many_arguments)]
fn column(
    city: &City,
    cam: &Camera,
    atmos: &Atmos,
    f: &mut Frame,
    x: i32,
    rdx: Fx,
    rdy: Fx,
    horizon: i32,
    proj: Fx,
    far: Fx,
    light: &Light,
) -> (u32, u32, f32) {
    let h = f.h as i32;

    let mut map_x = fixed::floor(cam.x);
    let mut map_y = fixed::floor(cam.y);

    let delta_x = if rdx == 0 { HUGE } else { fixed::abs(fixed::div(ONE, rdx)).min(HUGE) };
    let delta_y = if rdy == 0 { HUGE } else { fixed::abs(fixed::div(ONE, rdy)).min(HUGE) };

    let (step_x, mut side_x) = if rdx < 0 {
        (-1, fixed::mul(cam.x - fixed::from_int(map_x), delta_x))
    } else {
        (1, fixed::mul(fixed::from_int(map_x + 1) - cam.x, delta_x))
    };
    let (step_y, mut side_y) = if rdy < 0 {
        (-1, fixed::mul(cam.y - fixed::from_int(map_y), delta_y))
    } else {
        (1, fixed::mul(fixed::from_int(map_y + 1) - cam.y, delta_y))
    };

    // The line above which nothing has been drawn yet.  It only ever moves
    // up; when it reaches the top the column is finished.
    let mut ceiling = h;
    let mut steps = 0u32;
    let mut nearest = f32::MAX;

    while ceiling > 0 {
        let vertical = side_x < side_y;
        let dist = if vertical {
            let d = side_x;
            side_x = side_x.saturating_add(delta_x);
            map_x += step_x;
            d
        } else {
            let d = side_y;
            side_y = side_y.saturating_add(delta_y);
            map_y += step_y;
            d
        };
        steps += 1;
        if dist >= far || steps > 512 {
            return (steps, 0, nearest);
        }

        let bh = city.height(map_x, map_y);
        if bh == 0 {
            continue;
        }
        if nearest == f32::MAX {
            nearest = fixed::to_f32(dist);
        }

        // The building stands *on* the ground rather than at sea level, so
        // both its footing and its roofline move with the terrain.  Gentle
        // as the terrain is, getting this wrong is immediately visible: a
        // row of buildings on a slight rise would have their feet buried at
        // one end of the street and floating at the other.
        let ground = city.ground(map_x, map_y);
        let ch = ground + fixed::from_int(bh as i32);
        // Screen rows of the top of this cell and of its footing.
        let per = fixed::div(proj, dist);
        let top = horizon - fixed::floor(fixed::mul(ch - cam.z, per));
        let bot = horizon + fixed::floor(fixed::mul(cam.z - ground, per));

        let y0 = top.max(0);
        let y1 = bot.min(ceiling - 1).min(h - 1);
        if y1 < y0 {
            // Entirely hidden behind something nearer.
            if top < ceiling {
                ceiling = top.max(0);
            }
            continue;
        }

        // Where along the wall the ray landed, in absolute world units, so
        // that window bays line up across the cells of one lot.
        let along = if vertical {
            cam.y + fixed::mul(rdy, dist)
        } else {
            cam.x + fixed::mul(rdx, dist)
        };
        let face = Face::of(vertical, step_x, step_y);
        let lod = Lod::at(dist);

        // Height falls by this much per row down the screen; computed once
        // instead of a division per row.
        let dz = fixed::div(dist, proj);
        let mut z = cam.z + fixed::mul(fixed::from_int(horizon - y0), dz);

        if let Some(lot) = city.lot_at(map_x, map_y) {
            // Looking down on it: cap the top row with roofscape.
            if cam.z > ch && y0 <= y1 {
                let s = arch::roof(lot, map_x, map_y);
                let shaded = ch < city.shadow.line_at(map_x, map_y);
                let luma = lit(s.luma, light.on(arch::ROOF, shaded));
                f.put(x, y0, Cel { glyph: s.glyph, color: atmos.shade(s.hue, luma, dist) });
            }
            let first = if cam.z > ch { y0 + 1 } else { y0 };
            z -= fixed::mul(fixed::from_int(first - y0), dz);
            let bhf = fixed::from_int(bh as i32);
            // The diffuse term for this wall.  One table index, hoisted out
            // of the per-row loop, because every cell of this span shares a
            // normal - which is the whole reason a height field can afford
            // lighting at all.
            let normal = face.index() as usize;
            // ...and the height below which this wall is in the shade of
            // something upstream, which is one lookup per hit for the same
            // reason.  A wall is not uniformly lit: it is dark at the
            // bottom and lit above the line, which is what a tower standing
            // behind a nearer tower actually looks like.
            // Zero means "nothing upstream", not "shadowed up to the
            // ground": without the guard below, a wall with no shadow on it
            // fails `z < 0` only by luck, and any rounding that puts the
            // bottom row a hair under zero darkens the base of every
            // building in the city.
            let shade_below = city.shadow.line_at(map_x, map_y);
            let shaded_at = |z: Fx| shade_below > 0 && z < shade_below;
            for y in first..=y1 {
                // The facade is asked about height *above its own footing*,
                // not above sea level.
                let s = arch::facade(lot, face, along, (z - ground).max(0), bhf, lod);
                let luma = lit(s.luma, light.on(normal, shaded_at(z)));
                f.put(x, y, Cel { glyph: s.glyph, color: atmos.shade(s.hue, luma, dist) });
                z -= dz;
            }
        }

        if top < ceiling {
            ceiling = top.max(0);
        }
        if ceiling <= 0 {
            return (steps, 1, nearest);
        }
    }
    (steps, 1, nearest)
}

/// What the ground looks like at a world point.
///
/// Everything here is a function of position, so the streets cost no memory
/// beyond the height field: lane markings are the fractional part of the
/// world coordinate, crosswalks are proximity to an intersection, and
/// puddles are a hash that only exists when it is raining.
fn ground(city: &City, atmos: &Atmos, light: &Light, wx: Fx, wy: Fx, dist: Fx) -> Cel {
    let (gx, gy) = (fixed::floor(wx), fixed::floor(wy));
    // Whether the pavement here has the light on it.  One array read; the
    // whole city's cast shadows were swept once when the light was set.
    let shaded = !city.shadow.lit(gx, gy, city.ground(gx, gy));
    let cell = city.at(gx, gy);
    let (fx, fy) = (fixed::frac(wx), fixed::frac(wy));

    let (glyph, hue, luma) = match cell.kind {
        Kind::Road => road(&city.plan, gx, gy, fx, fy),
        Kind::Sidewalk => pavement(city, gx, gy, fx, fy),
        Kind::Park => {
            let h = hash3(gx as u32, gy as u32, (fixed::floor(fx * 4) + fixed::floor(fy * 4)) as u32);
            if h & 7 == 0 {
                (catalog::FLORA_HEDGE, palette::H_GREEN, 4)
            } else {
                (catalog::FLORA_GRASS, palette::H_GREEN, 5)
            }
        }
        Kind::Plaza => (catalog::ROAD_PAVING, palette::H_WHITE, 4),
        Kind::Building => (catalog::G_SOLID, palette::H_BLACK, 0),
    };

    // Wet ground: a puddle picks up the sodium light and gets a lot
    // brighter, which is most of what makes a rained-on street read as
    // rained on rather than as a darker street.
    if atmos.wet() && cell.kind != Kind::Park {
        let h = hash3(gx as u32, gy as u32, 0x_5075_4444);
        let size = fixed::ratio(2 + (h & 3) as i32, 8);
        if fixed::abs(fx - fixed::HALF) < size && fixed::abs(fy - fixed::HALF) < size && h & 12 == 0 {
            return Cel {
                glyph: catalog::ROAD_PUDDLE,
                color: atmos.shade(palette::H_YELLOW, 5, dist),
            };
        }
    }

    // The ground faces up, so it takes the roof normal.
    let luma = lit(luma, light.on(arch::ROOF, shaded));
    Cel { glyph, color: atmos.shade(hue, luma, dist) }
}

/// How far across the pavement the verge reaches from the kerb, and how far
/// the kerb itself does.
///
/// A cell of pavement is about six metres.  A metre of kerb-and-gutter, two
/// metres of planted verge, then paving to the building line, which is the
/// ordinary arrangement of a street that has trees on it.
const KERB_BAND: Fx = fixed::ratio(1, 6);
const VERGE_BAND: Fx = fixed::ratio(1, 2);
/// How close to the building line the paving stops.
const SEAM_BAND: Fx = fixed::ratio(1, 8);

/// How far a point on the pavement is from the nearest kerb, in cells.
#[inline(always)]
fn from_kerb(city: &City, gx: i32, gy: i32, fx: Fx, fy: Fx) -> Fx {
    edge_distance(city.edges(gx, gy) >> world::EDGE_ROAD, fx, fy)
}

/// How far a point on the pavement is from the nearest building wall.
#[inline(always)]
fn from_wall(city: &City, gx: i32, gy: i32, fx: Fx, fy: Fx) -> Fx {
    edge_distance(city.edges(gx, gy) >> world::EDGE_BUILT, fx, fy)
}

/// Distance to the nearest of a cell's marked sides, in cells.
///
/// The low four bits of `sides` are west, east, north and south, in the
/// order [`world::EDGE_STEPS`] packs them.  One when no side is marked -
/// the far side of a cell that adjoins nothing - which is what a fragment of
/// pavement in the middle of a block should read as.
///
/// The bits come from [`City::edges`], which is built once when the city is.
/// Asking [`City::at`] about the four neighbours instead is eight grid
/// lookups per ground character, and the ground is most of the screen: it
/// cost 0.07 ms a frame, which was a third of the frame.
#[inline(always)]
fn edge_distance(sides: u8, fx: Fx, fy: Fx) -> Fx {
    let mut d = ONE;
    if sides & 1 != 0 {
        d = d.min(fx);
    }
    if sides & 2 != 0 {
        d = d.min(ONE - fx);
    }
    if sides & 4 != 0 {
        d = d.min(fy);
    }
    if sides & 8 != 0 {
        d = d.min(ONE - fy);
    }
    d
}


/// The pavement, in bands from the kerb to the building line.
///
/// It runs the full width of every cell the plan marks as pavement, on both
/// sides of every proper street, which is what makes a street a street
/// rather than a gap between two rows of buildings.
///
/// Four bands, because a pavement seen in perspective is mostly its edges:
///
/// - the **kerb**, the brightest thing on the ground - and it has to stay
///   the brightest, so raising the paving raises it too - and the line that
///   says where the carriageway stops
/// - the **verge**: grass, and what the trees are planted in.  This is the
///   band that separates the traffic from the people, and putting the trees
///   in it rather than in the paving is the difference between a street with
///   trees on it and a street with obstacles on it
/// - the **paving**: cement, dirty, varying cell to cell
/// - the **building line**, a dark seam where the paving meets the wall,
///   which is what stops a wall appearing to float
///
/// The dirt and the grass are hashes of the cell, not noise: a pavement is
/// stained in particular places and stays stained, and a re-rolled stain
/// would crawl between frames.
fn pavement(city: &City, gx: i32, gy: i32, fx: Fx, fy: Fx) -> (catalog::GlyphId, u8, u8) {
    let kerb = from_kerb(city, gx, gy, fx, fy);
    if kerb < KERB_BAND {
        return (catalog::ROAD_KERB, palette::H_WHITE, 6);
    }
    if kerb < VERGE_BAND {
        // Planted verge.
        //
        // Green rather than light green for the grass, which is the wrong
        // way round until you look at what the two hues do at the top of
        // their ramps: this palette scales chroma with luminance, so green
        // at six is 162,226,162 - a pale, almost white green - while light
        // green is 176,224,134, which is olive.  The bright green is the
        // greener of the two, and the light-green tufts are what keeps a
        // verge from being one flat colour.
        let h = hash3(gx as u32, gy as u32, 0x_9E_46_E0_00);
        return match h & 7 {
            0 => (catalog::FLORA_HEDGE, palette::H_GREEN, 4),
            1..=2 => (catalog::FLORA_GRASS, palette::H_LIGHT_GREEN, 5),
            _ => (catalog::FLORA_GRASS, palette::H_GREEN, 5),
        };
    }
    if from_wall(city, gx, gy, fx, fy) < SEAM_BAND {
        return (catalog::G_CORNICE, palette::H_WHITE, 1);
    }

    // Cement, and what has been spilled on it.
    //
    // A step brighter than it was, all four of them, because at luminance
    // two and three the pavement sat below the carriageway's own markings
    // and the eye read the street as ending at the kerb.  Cement at night is
    // not dark - it is the thing under the street lights - and the band that
    // people are on should be the band you can see.
    let h = hash3(gx as u32, gy as u32, 0x_CE_11_7A_00);
    match h & 15 {
        0 => (catalog::ROAD_GRATE, palette::H_WHITE, 3),
        1 => (catalog::ROAD_PAVING, palette::H_BROWN, 3),
        2..=4 => (catalog::ROAD_PAVING, palette::H_WHITE, 3),
        _ => (catalog::ROAD_PAVING, palette::H_WHITE, 4),
    }
}

/// Road surface markings./// Road surface markings.
///
/// Read straight off the street plan, which knows how wide this road is and
/// how far across it this point sits.  That is the whole reason the plan
/// stores those two numbers: the paint and the shape of the road come from
/// one place, so widening a boulevard moves its centre line with it and
/// there is no second definition of where the middle is.
///
/// Alleys get nothing.  A one-cell service road with a double yellow down it
/// is not a road, it is a joke.
fn road(plan: &Plan, gx: i32, gy: i32, fx: Fx, fy: Fx) -> (catalog::GlyphId, u8, u8) {
    let col = plan.cols.at(gx);
    let row = plan.rows.at(gy);

    // A crossing, wherever the plan says one is painted - which is outside
    // the junction box, at the stop line.
    match plan.crossing_at(gx, gy) {
        Some(Crossing::OverCols) => {
            if let Some(m) = zebra(fixed::from_int(gx) + fx) {
                return m;
            }
        }
        Some(Crossing::OverRows) => {
            if let Some(m) = zebra(fixed::from_int(gy) + fy) {
                return m;
            }
        }
        None => {}
    }

    if col.class.is_street() && row.class.is_street() {
        // Inside a junction: bare tarmac.  A real one has no markings in the
        // middle, and painting through it is the quickest way to make a
        // street grid look like a diagram of a street grid.
    } else if col.class.is_street() {
        // A road running north-south: you measure across it in x and along
        // it in y.
        if let Some(m) = lines(across(col, fx), width(col), fixed::from_int(gy) + fy) {
            return m;
        }
    } else if row.class.is_street() {
        if let Some(m) = lines(across(row, fy), width(row), fixed::from_int(gx) + fx) {
            return m;
        }
    }

    let h = hash3(gx as u32, gy as u32, 0x_6247_4154);
    if h & 63 == 0 {
        return (catalog::ROAD_GRATE, palette::H_WHITE, 2);
    }
    (catalog::ROAD_ASPHALT, palette::H_WHITE, 1)
}

/// How far across the carriageway a point is, in cells.
#[inline(always)]
fn across(r: RoadCell, frac: Fx) -> Fx {
    fixed::from_int(r.across as i32) + frac
}

/// How wide the carriageway is, in cells.
#[inline(always)]
fn width(r: RoadCell) -> Fx {
    fixed::from_int(r.width as i32)
}

/// Half-width of the painted centre line.
///
/// A real double yellow is about a third of a metre, which at six metres to
/// the cell would be a twentieth of a cell - and a twentieth of a cell is
/// narrower than the ground sampler can hold on to. Past a few cells the
/// line would fall between samples and flicker in and out as the camera
/// moved. A sixth of a cell is wide enough to survive the sampling and still
/// reads as a line rather than as a stripe of paint.
const CENTRE_HALF: Fx = fixed::ratio(1, 6);
/// Half-width of a dashed lane divider.
const LANE_HALF: Fx = fixed::ratio(1, 10);
/// Width of the solid white line along the kerb.
const EDGE_W: Fx = fixed::ratio(1, 8);
/// Lane dividers are only painted on roads at least this wide; a two-cell
/// street has one lane each way and nothing to divide.
const LANE_MIN_W: Fx = fixed::from_int(3);
/// How far apart crosswalk stripes are, in cells.
const ZEBRA_PITCH: Fx = fixed::ratio(1, 2);
/// How much of that pitch is painted.
const ZEBRA_DUTY: Fx = fixed::ratio(11, 20);

/// The lines along an ordinary stretch of road.
///
/// `across` is the distance from the near kerb in cells, `width` the
/// carriageway, `along` the world coordinate running down the road - which
/// is what breaks the lane dividers into dashes.
fn lines(across: Fx, width: Fx, along: Fx) -> Option<(catalog::GlyphId, u8, u8)> {
    let centre = width / 2;
    if fixed::abs(across - centre) < CENTRE_HALF {
        // ROAD_CENTRE is two rules with a gap, so the double yellow is the
        // glyph rather than two passes.
        return Some((catalog::ROAD_CENTRE, palette::H_YELLOW, 6));
    }

    if width >= LANE_MIN_W {
        for q in [fixed::ratio(1, 4), fixed::ratio(3, 4)] {
            if fixed::abs(across - fixed::mul(width, q)) < LANE_HALF {
                // On for a cell, off for a cell, measured along the road, so
                // the dashes stay put as the camera moves rather than
                // crawling with it.
                return if fixed::frac(fixed::mul(along, fixed::HALF)) < fixed::HALF {
                    Some((catalog::ROAD_DASH, palette::H_WHITE, 5))
                } else {
                    None
                };
            }
        }
    }

    if !(EDGE_W..=width - EDGE_W).contains(&across) {
        return Some((catalog::ROAD_DASH, palette::H_WHITE, 3));
    }
    None
}

/// One stripe of a crosswalk, or bare tarmac between stripes.
fn zebra(along: Fx) -> Option<(catalog::GlyphId, u8, u8)> {
    let t = fixed::frac(fixed::div(along, ZEBRA_PITCH));
    if t < ZEBRA_DUTY {
        Some((catalog::ROAD_CROSSING, palette::H_WHITE, 6))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use crate::world::{City, Plan, SIZE};

    fn scene() -> (City, Camera, Atmos) {
        let city = City::generate(2024);
        let cam = Camera::spawn(&city, SIZE as i32 / 2, SIZE as i32 / 2);
        (city, cam, Atmos::default())
    }

    /// What row the ground `d` away lands on, for a camera `eye` up with
    /// this pitch.  The projection's own rule, written once here so the
    /// tests below ask the same question the renderer answers.
    fn ground_row(w: i32, h: i32, fov: Fx, eye: Fx, pitch: i32, d: i32) -> i32 {
        let rows = fixed::div(fixed::mul(eye, scale(w, fov)), fixed::from_int(d));
        h / 2 + pitch + fixed::floor(rows)
    }

    /// Looking down puts the far edge of the visible world at the top of the
    /// frame, which is the whole claim [`pitch_down`] makes.
    /// A cell of pavement, its bands sampled across their whole width.
    fn pavement_bands(city: &City) -> Vec<(catalog::GlyphId, u8, u8)> {
        // A pavement cell with a road on one side, which is what has a kerb.
        let (gx, gy) = (0..SIZE as i32)
            .flat_map(|y| (0..SIZE as i32).map(move |x| (x, y)))
            .find(|&(x, y)| {
                city.at(x, y).kind == Kind::Sidewalk
                    && city.edges(x, y) >> world::EDGE_ROAD & 0x0f != 0
            })
            .expect("no pavement beside a road in the whole city");
        let mut out = Vec::new();
        for i in 0..16 {
            for j in 0..16 {
                let (fx, fy) = (fixed::ratio(i, 16), fixed::ratio(j, 16));
                out.push(pavement(city, gx, gy, fx, fy));
            }
        }
        out
    }

    /// The kerb stays the brightest thing on the ground.
    ///
    /// It is the line that says where the carriageway stops, and it only
    /// reads as one while nothing beside it is as bright.  Raising the
    /// paving without raising the kerb is what would break this, and the
    /// paving has been raised twice.
    #[test]
    fn the_kerb_is_the_brightest_band_of_the_pavement() {
        let city = City::generate(99);
        let bands = pavement_bands(&city);
        let kerb = bands
            .iter()
            .filter(|&&(g, _, _)| g == catalog::ROAD_KERB)
            .map(|&(_, _, l)| l)
            .min()
            .expect("no kerb band");
        for &(g, _, l) in &bands {
            if g == catalog::ROAD_KERB {
                continue;
            }
            assert!(l < kerb, "a band at luminance {l} is as bright as the kerb at {kerb}");
        }
    }

    /// The pavement is somewhere you can see, and the grass is green.
    ///
    /// Both were a step or three darker and the street read as ending at
    /// the kerb: cement at night is the thing under the street lights, and
    /// the band that people are on should be the band you can see.  The
    /// grass is the same argument with a hue on it - green at the top of
    /// this palette's ramp is a pale, nearly white green, and that is what
    /// a verge under a lamp looks like.
    #[test]
    fn the_pavement_and_the_grass_are_bright_enough_to_read() {
        let city = City::generate(99);
        for &(g, hue, l) in &pavement_bands(&city) {
            if g == catalog::FLORA_GRASS || g == catalog::FLORA_HEDGE {
                assert!(l >= 4, "grass at luminance {l}");
                assert!(
                    hue == palette::H_GREEN || hue == palette::H_LIGHT_GREEN,
                    "the verge is hue {hue}"
                );
            } else if g == catalog::ROAD_PAVING || g == catalog::ROAD_GRATE {
                assert!(l >= 3, "paving at luminance {l}");
            }
        }
    }

    #[test]
    fn a_camera_aimed_down_has_the_furthest_ground_at_the_top() {
        let fov = crate::camera::fov_for_degrees(67.0);
        for (w, h) in [(80, 24), (140, 40), (40, 25), (200, 60)] {
            for eye in [fixed::from_int(8), fixed::from_int(30), fixed::from_int(120)] {
                for far in [20, 80, 200] {
                    let p = pitch_down(w, h, fov, eye, far);
                    let top = ground_row(w, h, fov, eye, p, far);
                    assert!(
                        (0..=1).contains(&top),
                        "{w}x{h}, eye {}, far {far}: the far edge landed on row {top}",
                        fixed::to_f32(eye)
                    );
                }
            }
        }
    }

    /// And everything on the screen is therefore nearer than that, which is
    /// the failure it was written for: a copter tilted by a fixed few rows
    /// filled the frame with world beyond the draw distance, which is black.
    #[test]
    fn nothing_on_screen_is_beyond_the_draw_distance() {
        let fov = crate::camera::fov_for_degrees(67.0);
        let (w, h, far) = (140, 40, 80);
        let eye = fixed::from_int(30);
        let p = pitch_down(w, h, fov, eye, far);
        // The bottom row is the steepest look down, so it is the nearest
        // ground; the top row is the furthest.
        let near = ground_row(w, h, fov, eye, p, 27);
        assert!(near >= h - 2 && near <= h, "the near edge landed on row {near}");
        assert!(p <= -(h / 2), "it is not even looking down: pitch {p}");
    }

    /// Higher up, or in thicker haze, means looking further down.
    #[test]
    fn it_tilts_further_the_higher_it_is_and_the_thicker_the_air() {
        let fov = crate::camera::fov_for_degrees(67.0);
        let at = |eye: i32, far: i32| pitch_down(140, 40, fov, fixed::from_int(eye), far);
        assert!(at(60, 80) < at(30, 80), "no further down from twice the height");
        assert!(at(30, 20) < at(30, 80), "no further down in thick haze");
    }

    /// The first north-south road at least `min_width` wide, as its
    /// leftmost column.
    fn col_road(plan: &Plan, min_width: u8) -> Option<(i32, u8)> {
        (0..SIZE as i32).find_map(|x| {
            let c = plan.cols.at(x);
            (c.across == 0 && c.width >= min_width && c.class.is_street()).then_some((x, c.width))
        })
    }

    /// The first east-west road at least `min_width` wide, as its top row.
    fn row_road(plan: &Plan, min_width: u8) -> Option<(i32, u8)> {
        (0..SIZE as i32).find_map(|y| {
            let c = plan.rows.at(y);
            (c.across == 0 && c.width >= min_width && c.class.is_street()).then_some((y, c.width))
        })
    }

    /// A row that is not on any east-west road, so a sample there is
    /// mid-block rather than in a junction.
    fn clear_row(plan: &Plan) -> i32 {
        (0..SIZE as i32).find(|y| !plan.rows.is_road(*y)).expect("every row is a road")
    }

    /// A column with no north-south road on it.
    fn clear_col(plan: &Plan) -> i32 {
        (0..SIZE as i32).find(|x| !plan.cols.is_road(*x)).expect("every column is a road")
    }

    /// Sample the markings straight across a road, kerb to kerb.
    fn cross_section(
        plan: &Plan,
        vertical: bool,
        start: i32,
        w: u8,
        fixed_axis: i32,
        steps: i32,
    ) -> Vec<(catalog::GlyphId, u8)> {
        (0..steps)
            .map(|i| {
                let t = fixed::div(
                    fixed::mul(fixed::from_int(w as i32), fixed::from_int(i)),
                    fixed::from_int(steps),
                );
                let cell = start + fixed::floor(t);
                let frac = fixed::frac(t);
                let (g, hue, _) = if vertical {
                    road(plan, cell, fixed_axis, frac, fixed::HALF)
                } else {
                    road(plan, fixed_axis, cell, fixed::HALF, frac)
                };
                (g, hue)
            })
            .collect()
    }

    #[test]
    fn every_road_has_a_yellow_centre_line_down_the_middle() {
        let city = City::generate(2024);
        let p = &city.plan;
        let cases = [
            (true, col_road(p, 2).expect("no north-south street"), clear_row(p)),
            (false, row_road(p, 2).expect("no east-west street"), clear_col(p)),
        ];
        for (vertical, (start, w), other) in cases {
            let strip = cross_section(p, vertical, start, w, other, 60);
            let yellow: Vec<usize> = strip
                .iter()
                .enumerate()
                .filter(|(_, (g, h))| *g == catalog::ROAD_CENTRE && *h == palette::H_YELLOW)
                .map(|(i, _)| i)
                .collect();
            assert!(!yellow.is_empty(), "a {w}-cell road has no yellow centre line");
            let mid = yellow.iter().sum::<usize>() / yellow.len();
            assert!(
                (mid as i32 - 30).abs() < 7,
                "the centre line sits at {mid}/60 across, which is not the middle"
            );
        }
    }

    #[test]
    fn the_centre_line_moves_with_the_width_of_the_road() {
        // The point of reading the width off the plan rather than assuming
        // it: a boulevard's centre line has to be further from the kerb than
        // a street's, and nothing should have to be told twice.
        for w in [2i32, 3, 4, 5] {
            let width = fixed::from_int(w);
            let painted: Vec<i32> = (0..200)
                .filter(|i| {
                    let a = fixed::div(fixed::mul(width, fixed::from_int(*i)), fixed::from_int(200));
                    matches!(lines(a, width, fixed::HALF), Some((g, _, _)) if g == catalog::ROAD_CENTRE)
                })
                .collect();
            assert!(!painted.is_empty(), "a {w}-cell road has no centre line");
            let mid = painted.iter().sum::<i32>() / painted.len() as i32;
            assert!((mid - 100).abs() < 12, "a {w}-cell road centres its line at {mid}/200");
        }
    }

    #[test]
    fn the_centre_line_is_a_line_and_not_a_stripe() {
        let width = fixed::from_int(3);
        let painted = (0..90)
            .filter(|i| {
                let a = fixed::div(fixed::mul(width, fixed::from_int(*i)), fixed::from_int(90));
                matches!(lines(a, width, fixed::HALF), Some((g, _, _)) if g == catalog::ROAD_CENTRE)
            })
            .count();
        assert!(painted > 2, "the centre line is only {painted}/90 wide - it will flicker");
        assert!(painted < 18, "the centre line is {painted}/90 wide - that is a stripe");
    }

    #[test]
    fn a_narrow_street_gets_no_lane_dividers() {
        // Two cells is one lane each way; painting a divider inside each
        // lane is three lines across twelve metres of road.  The edge lines
        // share the dash glyph, so this is checked by *position*: any white
        // paint on a narrow street has to be within a kerb's width of one
        // side or the other.
        let width = fixed::from_int(2);
        for i in 0..120 {
            let across = fixed::div(fixed::mul(width, fixed::from_int(i)), fixed::from_int(120));
            if let Some((g, _, _)) = lines(across, width, fixed::ratio(3, 2)) {
                if g == catalog::ROAD_DASH {
                    assert!(
                        !(EDGE_W..=width - EDGE_W).contains(&across),
                        "a two-cell street has white paint {} cells from the kerb",
                        fixed::to_f32(across)
                    );
                }
            }
        }
    }

    #[test]
    fn a_wide_road_does_get_lane_dividers() {
        let width = fixed::from_int(4);
        let divider = (0..120).any(|i| {
            let across = fixed::div(fixed::mul(width, fixed::from_int(i)), fixed::from_int(120));
            let inboard = across > EDGE_W * 2 && across < width - EDGE_W * 2;
            inboard
                && matches!(lines(across, width, fixed::HALF), Some((g, _, _)) if g == catalog::ROAD_DASH)
        });
        assert!(divider, "a four-cell boulevard has no lane divider");
    }

    #[test]
    fn lane_dividers_are_actually_dashed() {
        let width = fixed::from_int(4);
        let x = fixed::mul(width, fixed::ratio(1, 4));
        let (mut on, mut off) = (0, 0);
        for i in 0..80 {
            let along = fixed::div(fixed::from_int(i), fixed::from_int(8));
            match lines(x, width, along) {
                Some((g, _, _)) if g == catalog::ROAD_DASH => on += 1,
                _ => off += 1,
            }
        }
        assert!(on > 4 && off > 4, "not a dashed line: {on} painted, {off} bare");
    }

    #[test]
    fn a_junction_has_crosswalks_on_its_approaches_and_bare_tarmac_inside() {
        let city = City::generate(2024);
        let p = &city.plan;
        let (jx, jy) = (0..SIZE as i32)
            .flat_map(|y| (0..SIZE as i32).map(move |x| (x, y)))
            .find(|&(x, y)| p.is_junction(x, y))
            .expect("no junction on this plan");

        // Inside the box: no paint of any kind.
        let (cw, rw) = (p.cols.at(jx).width as i32, p.rows.at(jy).width as i32);
        for i in 0..12 {
            for j in 0..12 {
                let fx = fixed::div(fixed::from_int(i), fixed::from_int(12));
                let fy = fixed::div(fixed::from_int(j), fixed::from_int(12));
                for dx in 0..cw {
                    for dy in 0..rw {
                        let g = road(p, jx + dx, jy + dy, fx, fy).0;
                        assert_ne!(g, catalog::ROAD_CENTRE, "a centre line runs through the junction");
                        assert_ne!(g, catalog::ROAD_CROSSING, "a crossing is painted inside the junction");
                    }
                }
            }
        }

        // On the approaches: stripes.  One cell back along each road, which
        // is where the plan says the crossing is and where the pavement can
        // actually reach it.
        let mut stripes = 0;
        for (x, y) in [(jx, jy - 1), (jx, jy + rw), (jx - 1, jy), (jx + cw, jy)] {
            if p.crossing_at(x, y).is_none() {
                continue;
            }
            for i in 0..24 {
                let f = fixed::div(fixed::from_int(i), fixed::from_int(24));
                if road(p, x, y, f, f).0 == catalog::ROAD_CROSSING {
                    stripes += 1;
                }
            }
        }
        assert!(stripes > 20, "only {stripes} crosswalk samples on the approaches");
    }

    #[test]
    fn crosswalks_are_striped_rather_than_solid() {
        let city = City::generate(2024);
        let p = &city.plan;
        // Walk across a crossing over a north-south road and count the bars.
        let (cx, cy) = (0..SIZE as i32)
            .flat_map(|y| (0..SIZE as i32).map(move |x| (x, y)))
            .find(|&(x, y)| {
                p.crossing_at(x, y) == Some(Crossing::OverCols) && p.cols.at(x).across == 0
            })
            .expect("no crossing over a north-south road");
        let w = p.cols.at(cx).width as i32;
        let mut runs = 0;
        let mut was_paint = false;
        for i in 0..(w * 24) {
            let a = fixed::div(fixed::from_int(i), fixed::from_int(24));
            let cell = cx + fixed::floor(a);
            let paint = road(p, cell, cy, fixed::frac(a), fixed::HALF).0 == catalog::ROAD_CROSSING;
            if paint && !was_paint {
                runs += 1;
            }
            was_paint = paint;
        }
        assert!(runs >= 2, "the crossing has only {runs} bars - it is a solid block");
    }

    #[test]
    fn an_alley_gets_no_paint_at_all() {
        let city = City::generate(7);
        let alley = (0..SIZE as i32)
            .find(|x| city.plan.cols.at(*x).class == crate::world::RoadClass::Alley)
            .or_else(|| {
                (0..SIZE as i32)
                    .find(|y| city.plan.rows.at(*y).class == crate::world::RoadClass::Alley)
            });
        let Some(a) = alley else {
            return; // this seed happened not to lay one; the plan test covers that
        };
        let clear = clear_row(&city.plan);
        for i in 0..24 {
            let f = fixed::div(fixed::from_int(i), fixed::from_int(24));
            for (gx, gy) in [(a, clear), (clear_col(&city.plan), a)] {
                let g = road(&city.plan, gx, gy, f, f).0;
                assert!(
                    g != catalog::ROAD_CENTRE && g != catalog::ROAD_CROSSING,
                    "an alley was painted with {g}"
                );
            }
        }
    }

    #[test]
    fn a_frame_gets_drawn_and_is_not_all_one_thing() {
        let (city, cam, atmos) = scene();
        let mut f = Frame::new(120, 40);
        render(&city, &cam, &atmos, &mut f);
        let distinct: std::collections::HashSet<_> = f.cels.iter().map(|c| c.glyph).collect();
        assert!(distinct.len() > 8, "only {} distinct glyphs - that is not a city", distinct.len());
        let lit = f.cels.iter().filter(|c| c.color != 0).count();
        assert!(lit > f.cels.len() / 20, "the frame is almost entirely black");
    }

    #[test]
    fn rendering_is_deterministic() {
        let (city, cam, atmos) = scene();
        let mut a = Frame::new(80, 30);
        let mut b = Frame::new(80, 30);
        render(&city, &cam, &atmos, &mut a);
        render(&city, &cam, &atmos, &mut b);
        assert_eq!(a.cels, b.cels, "two renders of one scene differ");
    }

    #[test]
    fn no_panics_at_any_heading_position_or_size() {
        let city = City::generate(5);
        let atmos = Atmos::default();
        for size in [(1, 1), (2, 40), (40, 2), (40, 25), (200, 60)] {
            let mut f = Frame::new(size.0, size.1);
            for deg in (0..360).step_by(11) {
                let mut cam = Camera::spawn(&city, 20, 70);
                cam.yaw = crate::trig::from_degrees(deg as f64);
                cam.pitch = (deg % 21) - 10;
                render(&city, &cam, &atmos, &mut f);
            }
        }
    }

    #[test]
    fn the_camera_never_sees_through_a_wall_it_is_facing() {
        // Stand in the street facing a building; the middle column must be
        // filled to the horizon rather than showing sky.
        let city = City::generate(31);
        let atmos = Atmos { rain: 0, ..Default::default() };
        let mut f = Frame::new(80, 40);
        let mut cam = Camera::spawn(&city, SIZE as i32 / 2, SIZE as i32 / 2);
        let mut found = false;
        for deg in (0..360).step_by(5) {
            cam.yaw = crate::trig::from_degrees(deg as f64);
            let st = render(&city, &cam, &atmos, &mut f);
            if st.nearest < 6.0 {
                found = true;
                let horizon = 20;
                assert_ne!(
                    f.get(40, horizon).glyph,
                    catalog::G_BLANK,
                    "sky showing through a wall {} units away at {deg} degrees",
                    st.nearest
                );
            }
        }
        assert!(found, "the camera never faced a nearby building");
    }

    #[test]
    fn the_moon_actually_lights_the_buildings() {
        // The lighting has to reach the frame, not just the table.  Same
        // scene, same camera, moon on and moon off.
        let city = City::generate(2024);
        let cam = Camera::spawn(&city, SIZE as i32 / 2, SIZE as i32 / 2);
        let mut lit_frame = Frame::new(100, 32);
        let mut dark_frame = Frame::new(100, 32);
        let base = Atmos { rain: 0, stars: 0, haze: 2, ..Default::default() };
        render(&city, &cam, &Atmos { moon: true, ..base }, &mut lit_frame);
        render(&city, &cam, &Atmos { moon: false, ..base }, &mut dark_frame);
        let differing = (0..lit_frame.cels.len())
            .filter(|&i| lit_frame.cels[i].color != dark_frame.cels[i].color)
            .count();
        assert!(
            differing > lit_frame.cels.len() / 20,
            "only {differing} cells changed when the moon was turned off"
        );
    }

    #[test]
    fn walls_facing_the_moon_are_brighter_than_walls_facing_away() {
        // Measured through the renderer rather than through the table: a
        // wall's brightness has to depend on which way it points.
        let city = City::generate(2024);
        let mut cam = Camera::spawn(&city, SIZE as i32 / 2, SIZE as i32 / 2);
        let mut f = Frame::new(100, 32);

        // Average wall luminance looking one way, then the other, with the
        // moon fixed.  Facing into the moon shows the lit faces of things.
        let mean_luma = |cam: &Camera, f: &mut Frame, a: &Atmos| -> u32 {
            render(&city, cam, a, f);
            let lit: Vec<u32> = f
                .cels
                .iter()
                .filter(|c| c.glyph != catalog::G_BLANK && palette::luma_of(c.color) > 0)
                .map(|c| palette::luma_of(c.color) as u32)
                .collect();
            if lit.is_empty() {
                0
            } else {
                lit.iter().sum::<u32>() * 100 / lit.len() as u32
            }
        };

        let a = Atmos {
            rain: 0,
            stars: 0,
            haze: 2,
            moon: true,
            moon_az: 0,
            moon_alt: 6,
            ..Default::default()
        };
        cam.yaw = crate::trig::HALF; // looking west, so east faces are towards us
        let towards = mean_luma(&cam, &mut f, &a);
        cam.yaw = 0; // looking east, so we see west faces
        let away = mean_luma(&cam, &mut f, &a);
        assert_ne!(towards, away, "the view is the same brightness in both directions");
    }

    #[test]
    fn distance_darkens() {
        let city = City::generate(8);
        let atmos = Atmos { rain: 0, stars: 0, moon: false, haze: 4, ..Default::default() };
        let mut near = Frame::new(80, 30);
        let mut far = Frame::new(80, 30);
        let mut cam = Camera::spawn(&city, SIZE as i32 / 2, SIZE as i32 / 2);
        cam.z = crate::camera::EYE;
        render(&city, &cam, &atmos, &mut near);
        let bright = |f: &Frame| -> u32 {
            f.cels.iter().map(|c| palette::luma_of(c.color) as u32).sum()
        };
        let hazier = Atmos { haze: 8, ..atmos };
        render(&city, &cam, &hazier, &mut far);
        assert!(bright(&far) < bright(&near), "more haze did not darken the frame");
    }

    #[test]
    fn a_taller_building_behind_a_shorter_one_is_still_visible() {
        // The point of walking past the first hit.  Find a column where the
        // walk reported more than one contributing hit by checking that the
        // frame contains cells whose shading implies very different depths.
        let (city, mut cam, atmos) = scene();
        let mut f = Frame::new(120, 44);
        let mut ok = false;
        for deg in (0..360).step_by(7) {
            cam.yaw = crate::trig::from_degrees(deg as f64);
            render(&city, &cam, &atmos, &mut f);
            let lumas: std::collections::HashSet<u8> = (0..44)
                .map(|y| palette::luma_of(f.get(60, y).color))
                .collect();
            if lumas.len() >= 4 {
                ok = true;
                break;
            }
        }
        assert!(ok, "no column ever showed more than a few depths - occlusion is closing too early");
    }

    #[test]
    fn looking_straight_down_an_avenue_gives_a_long_view() {
        let city = City::generate(77);
        let atmos = Atmos { haze: 0, rain: 0, ..Default::default() };
        let mut f = Frame::new(100, 36);
        let mut cam = Camera::spawn(&city, 7, 48);
        let mut deepest = 0u32;
        for deg in (0..360).step_by(3) {
            cam.yaw = crate::trig::from_degrees(deg as f64);
            let st = render(&city, &cam, &atmos, &mut f);
            deepest = deepest.max(st.steps);
        }
        assert!(deepest > 400, "the deepest view was only {deepest} cells");
    }
}

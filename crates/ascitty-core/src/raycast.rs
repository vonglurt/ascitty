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
use crate::world::{City, Kind, AVE_PERIOD, AVE_WIDTH, ST_PERIOD, ST_WIDTH};

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
}

/// Work out the projection for a camera and a frame.
pub fn projection(cam: &Camera, f: &Frame) -> Proj {
    let (w, h) = (f.w as i32, f.h as i32);
    Proj {
        w,
        h,
        horizon: h / 2 + cam.pitch,
        proj: fixed::div(
            fixed::from_int(w),
            fixed::mul(cam.fov, fixed::from_int(2 * CELL_ASPECT)),
        ),
    }
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
    let p = projection(cam, f);
    let (w, h) = (p.w, p.h);
    let horizon = p.horizon;
    let proj = p.proj;
    let far = fixed::from_int(draw_distance(atmos.haze));
    let (dx, dy) = cam.dir();
    let (px, py) = cam.plane();

    // Ground distance per row, computed once for the whole frame.
    let mut row_dist = vec![HUGE; f.h];
    for y in (horizon.max(0) + 1)..h {
        let below = y - horizon;
        row_dist[y as usize] = fixed::div(fixed::mul(cam.z, proj), fixed::from_int(below));
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
            f.put(x, y, ground(city, atmos, wx, wy, d));
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
        let s = column(city, cam, atmos, f, x, rdx, rdy, horizon, proj, far);
        st.steps += s.0;
        st.closed += s.1;
        depth[x as usize] = if s.2 == f32::MAX { Fx::MAX } else { fixed::from_f64(s.2 as f64) };
        if x == w / 2 {
            st.nearest = s.2;
        }
    }
    st
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

        let cell = city.at(map_x, map_y);
        if cell.height == 0 {
            continue;
        }
        if nearest == f32::MAX {
            nearest = fixed::to_f32(dist);
        }

        let ch = fixed::from_int(cell.height as i32);
        // Screen rows of the top of this cell and of its footing.
        let per = fixed::div(proj, dist);
        let top = horizon - fixed::floor(fixed::mul(ch - cam.z, per));
        let bot = horizon + fixed::floor(fixed::mul(cam.z, per));

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
                f.put(x, y0, Cel { glyph: s.glyph, color: atmos.shade(s.hue, s.luma, dist) });
            }
            let first = if cam.z > ch { y0 + 1 } else { y0 };
            z -= fixed::mul(fixed::from_int(first - y0), dz);
            for y in first..=y1 {
                let s = arch::facade(lot, face, along, z.max(0), ch, lod);
                f.put(x, y, Cel { glyph: s.glyph, color: atmos.shade(s.hue, s.luma, dist) });
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
fn ground(city: &City, atmos: &Atmos, wx: Fx, wy: Fx, dist: Fx) -> Cel {
    let (gx, gy) = (fixed::floor(wx), fixed::floor(wy));
    let cell = city.at(gx, gy);
    let (fx, fy) = (fixed::frac(wx), fixed::frac(wy));

    let (glyph, hue, luma) = match cell.kind {
        Kind::Road => road(gx, gy, fx, fy),
        Kind::Sidewalk => {
            // The kerb is the edge of the sidewalk that faces the road.
            let kerb = !city.at(gx, gy).kind.eq(&Kind::Road)
                && (city.at(gx - 1, gy).kind == Kind::Road && fx < fixed::ratio(1, 5)
                    || city.at(gx + 1, gy).kind == Kind::Road && fx > fixed::ratio(4, 5)
                    || city.at(gx, gy - 1).kind == Kind::Road && fy < fixed::ratio(1, 5)
                    || city.at(gx, gy + 1).kind == Kind::Road && fy > fixed::ratio(4, 5));
            if kerb {
                (catalog::ROAD_KERB, palette::H_WHITE, 4)
            } else {
                (catalog::ROAD_PAVING, palette::H_WHITE, 2)
            }
        }
        Kind::Park => {
            let h = hash3(gx as u32, gy as u32, (fixed::floor(fx * 4) + fixed::floor(fy * 4)) as u32);
            if h & 7 == 0 {
                (catalog::FLORA_HEDGE, palette::H_GREEN, 3)
            } else {
                (catalog::FLORA_GRASS, palette::H_GREEN, 2)
            }
        }
        Kind::Plaza => (catalog::ROAD_PAVING, palette::H_WHITE, 2),
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

    Cel { glyph, color: atmos.shade(hue, luma, dist) }
}

/// Road surface markings.
///
/// Worked out in *continuous* coordinates across the carriageway rather than
/// per cell. A cell is six metres; a painted line is not, and the previous
/// version - which asked "is this the middle cell of the avenue" - could only
/// ever put the centre line down the middle of a whole cell. On a two-cell
/// cross street the true centre is the *boundary* between its two cells, so
/// there was no cell to put it in and the line sat off to one side.
///
/// Measuring across the road instead makes it correct for any width, and it
/// is the same arithmetic for avenues and cross streets.
fn road(gx: i32, gy: i32, fx: Fx, fy: Fx) -> (catalog::GlyphId, u8, u8) {
    let ax_cell = gx.rem_euclid(AVE_PERIOD as i32);
    let sy_cell = gy.rem_euclid(ST_PERIOD as i32);
    let on_ave = ax_cell < AVE_WIDTH as i32;
    let on_st = sy_cell < ST_WIDTH as i32;

    // Where this point is across each family of road, in cells, and how far
    // along it. `across` drives the lines; `along` drives the dashes.
    let across_ave = fixed::from_int(ax_cell) + fx;
    let across_st = fixed::from_int(sy_cell) + fy;
    let world_x = fixed::from_int(gx) + fx;
    let world_y = fixed::from_int(gy) + fy;

    if on_ave && on_st {
        if let Some(m) = crossing(across_ave, across_st, world_x, world_y) {
            return m;
        }
    } else if on_ave {
        if let Some(m) = lines(across_ave, AVE_W, world_y) {
            return m;
        }
    } else if on_st {
        if let Some(m) = lines(across_st, ST_W, world_x) {
            return m;
        }
    }

    let h = hash3(gx as u32, gy as u32, 0x_6247_4154);
    if h & 63 == 0 {
        return (catalog::ROAD_GRATE, palette::H_WHITE, 2);
    }
    (catalog::ROAD_ASPHALT, palette::H_WHITE, 1)
}

/// Width of an avenue's carriageway, in cells.
const AVE_W: Fx = fixed::from_int(AVE_WIDTH as i32);
/// Width of a cross street's carriageway.
const ST_W: Fx = fixed::from_int(ST_WIDTH as i32);

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
/// How far into the junction the crosswalk band reaches from each edge.
const ZEBRA_BAND: Fx = fixed::ratio(2, 5);

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

/// The markings inside a junction.
///
/// No centre line and no lane dividers - a real junction has bare tarmac in
/// the middle, and painting through it is the single quickest way to make a
/// street grid look like a diagram of a street grid. What it does have is a
/// crosswalk across each of the four approaches, laid just inside the edge
/// of the box.
///
/// The stripes run *with* the traffic and repeat *across* it, which is the
/// way round they are painted: a pedestrian walking north over an avenue
/// crosses a ladder of north-south bars.
fn crossing(across_ave: Fx, across_st: Fx, world_x: Fx, world_y: Fx) -> Option<(catalog::GlyphId, u8, u8)> {
    // The two crosswalks over the avenue, at the north and south edges.
    if !(ZEBRA_BAND..=ST_W - ZEBRA_BAND).contains(&across_st) {
        return zebra(world_x);
    }
    // ...and the two over the cross street, at the east and west edges.
    if !(ZEBRA_BAND..=AVE_W - ZEBRA_BAND).contains(&across_ave) {
        return zebra(world_y);
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
    use crate::world::{City, SIZE};

    fn scene() -> (City, Camera, Atmos) {
        let city = City::generate(2024);
        let cam = Camera::spawn(&city, 48, 48);
        (city, cam, Atmos::default())
    }

    /// Sample the markings straight down the cross-section of a road, from
    /// one kerb to the other, at a fixed point along it.
    fn cross_section(ave: bool, along_cell: i32, steps: i32) -> Vec<(catalog::GlyphId, u8)> {
        (0..steps)
            .map(|i| {
                let t = fixed::div(fixed::from_int(i), fixed::from_int(steps));
                let width = if ave { AVE_WIDTH } else { ST_WIDTH } as i32;
                let across = fixed::mul(t, fixed::from_int(width));
                let cell = fixed::floor(across);
                let frac = fixed::frac(across);
                let (g, hue, _) = if ave {
                    road(cell, along_cell, frac, fixed::HALF)
                } else {
                    road(along_cell, cell, fixed::HALF, frac)
                };
                (g, hue)
            })
            .collect()
    }

    /// A cell that is on an avenue but not at a junction.
    fn mid_avenue_row() -> i32 {
        (0..SIZE as i32)
            .find(|y| !crate::world::on_street(*y as usize))
            .expect("no cross-street-free row")
    }

    /// A cell that is on a cross street but not at a junction.
    fn mid_street_col() -> i32 {
        (0..SIZE as i32)
            .find(|x| !crate::world::on_avenue(*x as usize))
            .expect("no avenue-free column")
    }

    #[test]
    fn every_road_has_a_yellow_centre_line_down_the_middle() {
        for (ave, along) in [(true, mid_avenue_row()), (false, mid_street_col())] {
            let strip = cross_section(ave, along, 60);
            let yellow: Vec<usize> = strip
                .iter()
                .enumerate()
                .filter(|(_, (g, h))| *g == catalog::ROAD_CENTRE && *h == palette::H_YELLOW)
                .map(|(i, _)| i)
                .collect();
            assert!(
                !yellow.is_empty(),
                "{} has no yellow centre line",
                if ave { "the avenue" } else { "the cross street" }
            );
            // ...and it is in the middle, not off to one side.
            let mid = yellow.iter().sum::<usize>() / yellow.len();
            assert!(
                (mid as i32 - 30).abs() < 6,
                "the centre line sits at {mid}/60 across, which is not the middle"
            );
        }
    }

    #[test]
    fn the_centre_line_is_a_line_and_not_a_stripe() {
        // It has to be narrow enough to read as paint and wide enough to
        // survive point sampling; a third of the carriageway is neither.
        let strip = cross_section(true, mid_avenue_row(), 90);
        let painted = strip.iter().filter(|(g, _)| *g == catalog::ROAD_CENTRE).count();
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
        let along = fixed::ratio(3, 2);
        for i in 0..120 {
            let across = fixed::div(fixed::mul(ST_W, fixed::from_int(i)), fixed::from_int(120));
            if let Some((g, _, _)) = lines(across, ST_W, along) {
                if g == catalog::ROAD_DASH {
                    assert!(
                        !(EDGE_W..=ST_W - EDGE_W).contains(&across),
                        "a two-cell street has white paint {} cells from the kerb",
                        fixed::to_f32(across)
                    );
                }
            }
        }
    }

    #[test]
    fn a_wide_road_does_get_lane_dividers() {
        // The other half of the rule, so that widening a street silently
        // losing its lanes would fail rather than pass.
        let along = fixed::HALF;
        let divider = (0..120).any(|i| {
            let across = fixed::div(fixed::mul(AVE_W, fixed::from_int(i)), fixed::from_int(120));
            let inboard = across > EDGE_W * 2 && across < AVE_W - EDGE_W * 2;
            inboard
                && matches!(lines(across, AVE_W, along), Some((g, _, _)) if g == catalog::ROAD_DASH)
        });
        assert!(divider, "a three-cell avenue has no lane divider");
    }

    #[test]
    fn lane_dividers_are_actually_dashed() {
        // Walk down the avenue at the quarter-width divider and check the
        // paint comes and goes.
        let x = fixed::mul(AVE_W, fixed::ratio(1, 4));
        let y0 = mid_avenue_row();
        let (mut on, mut off) = (0, 0);
        for i in 0..80 {
            let along = fixed::div(fixed::from_int(i), fixed::from_int(8));
            let cell = y0 + fixed::floor(along);
            if crate::world::on_street(cell.max(0) as usize) {
                continue;
            }
            match lines(x, AVE_W, fixed::from_int(cell) + fixed::frac(along)) {
                Some((g, _, _)) if g == catalog::ROAD_DASH => on += 1,
                _ => off += 1,
            }
        }
        assert!(on > 4 && off > 4, "not a dashed line: {on} painted, {off} bare");
    }

    #[test]
    fn junctions_have_crosswalks_and_no_centre_line() {
        let (jx, jy) = (0..SIZE as i32)
            .flat_map(|y| (0..SIZE as i32).map(move |x| (x, y)))
            .find(|&(x, y)| {
                crate::world::on_avenue(x as usize) && crate::world::on_street(y as usize)
            })
            .expect("no junction on this grid");

        let mut stripes = 0;
        let mut centre = 0;
        for i in 0..24 {
            for j in 0..24 {
                let fx = fixed::div(fixed::from_int(i), fixed::from_int(24));
                let fy = fixed::div(fixed::from_int(j), fixed::from_int(24));
                for dx in 0..AVE_WIDTH as i32 {
                    for dy in 0..ST_WIDTH as i32 {
                        match road(jx + dx, jy + dy, fx, fy).0 {
                            g if g == catalog::ROAD_CROSSING => stripes += 1,
                            g if g == catalog::ROAD_CENTRE => centre += 1,
                            _ => {}
                        }
                    }
                }
            }
        }
        assert!(stripes > 100, "only {stripes} crosswalk samples inside the junction");
        assert_eq!(centre, 0, "the centre line was painted straight through the junction");
    }

    #[test]
    fn crosswalks_are_striped_rather_than_solid() {
        let (jx, jy) = (0..SIZE as i32)
            .flat_map(|y| (0..SIZE as i32).map(move |x| (x, y)))
            .find(|&(x, y)| {
                crate::world::on_avenue(x as usize) && crate::world::on_street(y as usize)
            })
            .unwrap();
        // Walk across the north crosswalk of the junction and count the bars.
        let mut runs = 0;
        let mut was_paint = false;
        for i in 0..(AVE_WIDTH as i32 * 24) {
            let across = fixed::div(fixed::from_int(i), fixed::from_int(24));
            let cell = jx + fixed::floor(across);
            let paint = road(cell, jy, fixed::frac(across), fixed::ratio(1, 8)).0
                == catalog::ROAD_CROSSING;
            if paint && !was_paint {
                runs += 1;
            }
            was_paint = paint;
        }
        assert!(runs >= 3, "the crosswalk has only {runs} bars - it is a solid block");
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
        let mut cam = Camera::spawn(&city, 48, 48);
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
    fn distance_darkens() {
        let city = City::generate(8);
        let atmos = Atmos { rain: 0, stars: 0, moon: false, haze: 4, ..Default::default() };
        let mut near = Frame::new(80, 30);
        let mut far = Frame::new(80, 30);
        let mut cam = Camera::spawn(&city, 48, 48);
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

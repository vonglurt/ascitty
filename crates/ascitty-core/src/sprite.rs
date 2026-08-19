//! Billboards: everything in the city that is not a building.
//!
//! Lamp posts, hydrants, trees, traffic, pedestrians, the fare markers and
//! the coins are all drawn the same way - as a flat card standing at a world
//! position, always facing the camera, clipped against the depth of the wall
//! behind it.
//!
//! # The shapes are ASCII art in the source
//!
//! A sprite is written as eight rows of eight characters, right here in the
//! file, and each character names a glyph from the catalogue.  That is not
//! laziness: the whole project is about what a shape looks like when it is
//! made of characters, and a sprite editor that is not itself made of
//! characters would be lying about the medium.  It also means adding a
//! mailbox is four lines of art rather than a data file and a loader.
//!
//! # Clipping
//!
//! The renderer leaves behind one distance per screen column - how far away
//! the nearest wall in that column was.  A billboard nearer than that is
//! drawn; further, it is not.  Per column rather than per cell, which is the
//! usual approximation and wrong only when something should be visible over
//! the top of a *near* building, which for a hydrant it never is.

use crate::atmos::Atmos;
use crate::camera::Camera;
use crate::catalog::{self, GlyphId};
use crate::fixed::{self, Fx, ONE};
use crate::frame::{Cel, Frame};
use crate::palette;
use crate::raycast::{Proj, CELL_ASPECT};
use crate::trig::{self, Ang};

/// What a billboard is a picture of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stamp {
    /// A street light: post, arm, lamp.
    LampPost,
    /// A traffic signal on a pole.
    Signal,
    /// A fire hydrant.
    Hydrant,
    /// A mailbox - a thing that exists to be knocked over.
    Mailbox,
    /// A street tree.
    Tree,
    /// A bollard.
    Bollard,
    /// A parking meter.
    Meter,
    /// A car, seen from behind or in front.
    Car,
    /// A boxy four-wheel-drive, seen from the side.
    JeepSide,
    /// A long, low car of about 1972, seen from the side.
    MuscleSide,
    /// A bus, seen from the side.
    BusSide,
    /// The cab.  Longer than a saloon, with a checker band and a sign on
    /// the roof, because a taxi you are chasing has to be identifiable at a
    /// glance among a street full of traffic.
    Taxi,
    /// A car that has been hit hard enough to show it.
    Wreck,
    /// A bus.
    Bus,
    /// A person.
    Ped,
    /// A fare coin, spinning.
    Coin,
    /// The marker over a waiting passenger.
    Pickup,
    /// The marker over their destination.
    Dropoff,
    /// What is left of something you drove through.
    Debris,
}

/// One billboard in the world.
#[derive(Clone, Copy, Debug)]
pub struct Billboard {
    /// Position.
    pub x: Fx,
    /// Position.
    pub y: Fx,
    /// Height of the bottom of the card above the pavement.
    pub base: Fx,
    /// Width of the card, in world units.
    pub w: Fx,
    /// Height of the card, in world units.
    pub h: Fx,
    /// What it is a picture of.
    pub stamp: Stamp,
    /// Hue, where the stamp does not insist on its own.
    pub hue: u8,
    /// Animation phase - the spin of a coin, the stride of a walker.
    pub phase: u8,
    /// How far over it has been knocked, 0 upright to 8 flat.  Knocked-over
    /// things lean, which is cheaper and reads better than an animation.
    pub lean: u8,
}

impl Billboard {
    /// An upright thing on the pavement.
    pub fn upright(stamp: Stamp, x: Fx, y: Fx, w: Fx, h: Fx, hue: u8) -> Billboard {
        Billboard { x, y, base: 0, w, h, stamp, hue, phase: 0, lean: 0 }
    }
}

// --- the art ---------------------------------------------------------------
//
// Eight rows, top first, eight columns.  A space is transparent.  Every
// other character names a catalogue glyph in `glyph_for` below, and the
// second character of the pair - in `LIT` - says how bright it is.

/// A street light: a mast, a lamp, and the glow around it.
///
/// The glow is drawn rather than simulated.  A point source with real
/// falloff would light the pavement under it, which needs a second pass over
/// the ground; a halo of two densities around the bulb costs two glyphs and
/// reads, at this size, as the same thing.  The bulb sits at the very top so
/// that the mast can be lengthened without the head growing with it.
#[rustfmt::skip]
const LAMP_POST_ART: [&str; 8] = [
    "  .:*:. ",
    "  :*o*: ",
    "  .:*:. ",
    "   ||   ",
    "   ||   ",
    "   ||   ",
    "   ||   ",
    "  ====  ",
];

#[rustfmt::skip]
const SIGNAL_ART: [&str; 8] = [
    "  ####  ",
    "  #r #  ",
    "  #y #  ",
    "  #g #  ",
    "  ####  ",
    "   ||   ",
    "   ||   ",
    "  ====  ",
];

#[rustfmt::skip]
const HYDRANT_ART: [&str; 8] = [
    "        ",
    "        ",
    "        ",
    "        ",
    "   #    ",
    "  ###   ",
    "  ###   ",
    " #####  ",
];

#[rustfmt::skip]
const MAILBOX_ART: [&str; 8] = [
    "        ",
    "        ",
    "  ####  ",
    " ###### ",
    " ###### ",
    "   ||   ",
    "   ||   ",
    "  ====  ",
];

#[rustfmt::skip]
const TREE_ART: [&str; 8] = [
    "  TTT   ",
    " TTTTT  ",
    "TTTTTTT ",
    "TTTTTTT ",
    " TTTTT  ",
    "  t t   ",
    "   t    ",
    "  ===   ",
];

#[rustfmt::skip]
const BOLLARD_ART: [&str; 8] = [
    "        ",
    "        ",
    "        ",
    "        ",
    "   ##   ",
    "   ##   ",
    "   ##   ",
    "  ====  ",
];

#[rustfmt::skip]
const METER_ART: [&str; 8] = [
    "        ",
    "        ",
    "   ##   ",
    "   o    ",
    "   |    ",
    "   |    ",
    "   |    ",
    "  ==    ",
];

/// A boxy four-wheel-drive, seen from the side: upright glass, no overhang
/// worth speaking of, wheels at the corners.
#[rustfmt::skip]
const JEEP_SIDE_ART: [&str; 8] = [
    "        ",
    "        ",
    "  ####  ",
    " ###### ",
    "########",
    "########",
    " o    o ",
    "        ",
];

/// A long, low car of about 1972, seen from the side: most of it is bonnet,
/// the cabin is set well back, and the roofline runs into the boot.
#[rustfmt::skip]
const MUSCLE_SIDE_ART: [&str; 8] = [
    "        ",
    "        ",
    "        ",
    "   ###  ",
    " ###### ",
    "########",
    " o    o ",
    "        ",
];

/// A bus from the side: a box on wheels, and nothing else to say about it.
#[rustfmt::skip]
const BUS_SIDE_ART: [&str; 8] = [
    "        ",
    "########",
    "#.#.#.##",
    "#.#.#.##",
    "########",
    "########",
    " o    o ",
    "        ",
];

#[rustfmt::skip]
const CAR_ART: [&str; 8] = [
    "        ",
    "        ",
    "  ####  ",
    " ###### ",
    "L######R",
    "LL####RR",
    " oo  oo ",
    "  ====  ",
];

#[rustfmt::skip]
const TAXI_ART: [&str; 8] = [
    "   SS   ",
    "  ####  ",
    " ###### ",
    " ###### ",
    "LkkkkkkR",
    "L######R",
    "LL#  #RR",
    " oo  oo ",
];

#[rustfmt::skip]
const WRECK_ART: [&str; 8] = [
    "        ",
    "    #   ",
    "  ##.#  ",
    " #.###. ",
    "L#.##.#R",
    "LL#..#RR",
    " o    o ",
    "  ====  ",
];

#[rustfmt::skip]
const BUS_ART: [&str; 8] = [
    "        ",
    "########",
    "#......#",
    "#......#",
    "########",
    "L######R",
    " oo  oo ",
    " ====== ",
];

#[rustfmt::skip]
const PED_ART: [&str; 8] = [
    "        ",
    "        ",
    "   o    ",
    "   #    ",
    "  ###   ",
    "   #    ",
    "   |    ",
    "  = =   ",
];

#[rustfmt::skip]
const COIN_ART: [&str; 8] = [
    "        ",
    "  ####  ",
    " ###### ",
    " ###### ",
    " ###### ",
    "  ####  ",
    "        ",
    "        ",
];

#[rustfmt::skip]
const PICKUP_ART: [&str; 8] = [
    "  ####  ",
    " ###### ",
    "  ####  ",
    "   ##   ",
    "        ",
    "        ",
    "        ",
    "        ",
];

#[rustfmt::skip]
const DEBRIS_ART: [&str; 8] = [
    "        ",
    "        ",
    "        ",
    "        ",
    "        ",
    "        ",
    " .  . . ",
    " ...#.. ",
];

impl Stamp {
    /// The art for this stamp.
    fn art(self) -> &'static [&'static str; 8] {
        match self {
            Stamp::LampPost => &LAMP_POST_ART,
            Stamp::Signal => &SIGNAL_ART,
            Stamp::Hydrant => &HYDRANT_ART,
            Stamp::Mailbox => &MAILBOX_ART,
            Stamp::Tree => &TREE_ART,
            Stamp::Bollard => &BOLLARD_ART,
            Stamp::Meter => &METER_ART,
            Stamp::Car => &CAR_ART,
            Stamp::JeepSide => &JEEP_SIDE_ART,
            Stamp::MuscleSide => &MUSCLE_SIDE_ART,
            Stamp::BusSide => &BUS_SIDE_ART,
            Stamp::Taxi => &TAXI_ART,
            Stamp::Wreck => &WRECK_ART,
            Stamp::Bus => &BUS_ART,
            Stamp::Ped => &PED_ART,
            Stamp::Coin => &COIN_ART,
            Stamp::Pickup | Stamp::Dropoff => &PICKUP_ART,
            Stamp::Debris => &DEBRIS_ART,
        }
    }

    /// Whether this belongs at the kerb rather than set back on the
    /// pavement.
    ///
    /// Street lighting and signals stand at the edge of the carriageway,
    /// which is where they are of use; a lamp in the middle of the pavement
    /// lights the wall.  Vegetation goes in the verge behind them.
    pub fn kerbside(self) -> bool {
        matches!(self, Stamp::LampPost | Stamp::Signal | Stamp::Hydrant | Stamp::Bollard)
    }

    /// Whether this is planted, and therefore belongs in the verge.
    pub fn planted(self) -> bool {
        matches!(self, Stamp::Tree)
    }

    /// Whether this is something a car can flatten.
    pub fn frangible(self) -> bool {
        matches!(
            self,
            Stamp::LampPost | Stamp::Mailbox | Stamp::Hydrant | Stamp::Bollard | Stamp::Meter | Stamp::Signal
        )
    }
}

/// One art character, as a glyph and a brightness, or `None` for transparent.
///
/// The characters chosen for the art are the ones that *look* like what they
/// mean in a fixed-width editor, so the art above is legible as art.  What
/// they map to is a different question and is answered here.
fn glyph_for(c: char, hue: u8, phase: u8) -> Option<(GlyphId, u8, u8)> {
    Some(match c {
        ' ' => return None,
        // A body panel.  A step darker than it was: a car is a thing with a
        // shape, and at the top of the ramp it was a lamp with a shape.
        '#' => (catalog::G_SOLID, hue, 5),
        '.' => (catalog::shade(3), hue, 3),
        '|' => (catalog::ST_POST, palette::H_WHITE, 3),
        '=' => (catalog::G_CORNICE + 3, palette::H_WHITE, 2),
        // Lights, and which end of the car you are looking at.
        //
        // The low bit of the phase says the camera is in front of it.  White
        // means it is coming towards you and red means it is going away,
        // which is the only cue in the frame that says which way a car is
        // pointing - a box of body panels does not, and "am I about to hit
        // that" is a question about direction rather than about position.
        'o' => {
            if phase & 1 == 1 {
                (catalog::ST_LAMP, palette::H_WHITE, 7)
            } else {
                (catalog::ST_LAMP, palette::H_RED, 5)
            }
        }
        'T' => (catalog::FLORA_CANOPY, palette::H_GREEN, 4),
        't' => (catalog::FLORA_TRUNK, palette::H_BROWN, 3),
        'L' => (catalog::G_QUAD + 12 - 1, hue, 4), // lower half - the flank
        'R' => (catalog::G_QUAD + 12 - 1, hue, 4),
        // The checker band, and the sign on the roof.
        'k' => (catalog::G_QUAD + 6 - 1, palette::H_WHITE, 7),
        'S' => (catalog::G_SOLID, palette::H_YELLOW, 7),
        // Two densities of halo around a lamp.  Sparse dithers rather than
        // solid glyphs: a glow has no edge, and anything with an edge reads
        // as a lampshade.
        '*' => (catalog::G_HAZE + 3, palette::H_YELLOW, 6),
        ':' => (catalog::G_HAZE + 1, palette::H_YELLOW, 4),
        // The three signal aspects, only one of which is ever lit; which one
        // comes from the phase, so a junction full of them stays in step.
        'r' => (catalog::G_SOLID, palette::H_RED, if phase.is_multiple_of(3) { 7 } else { 1 }),
        'y' => (catalog::G_SOLID, palette::H_YELLOW, if phase % 3 == 1 { 7 } else { 1 }),
        'g' => (catalog::G_SOLID, palette::H_GREEN, if phase % 3 == 2 { 7 } else { 1 }),
        _ => (catalog::G_SOLID, hue, 5),
    })
}

/// How wide a box of `len` by `wid` looks, and whether you are seeing it
/// side-on.
///
/// `yaw` is the way the box is pointing; `(vx, vy)` is the direction from
/// the camera to it.  The silhouette of a rectangle is
///
/// ```text
///     width = len * |sin t| + wid * |cos t|
/// ```
///
/// where `t` is the angle between the two - so a car looked at down its own
/// length is as wide as a car is wide, broadside it is as wide as a car is
/// long, and everything between is the two corners.  That single expression
/// is the whole of what makes traffic read as boxes driving along a street
/// rather than as cards turning to face you: a car that passes across the
/// view stretches out and then shortens again as it goes.
///
/// Exact, and it costs two multiplies.  There is no approximation here to
/// justify - the projection of a rectangle onto a line really is this.
pub fn silhouette(len: Fx, wid: Fx, yaw: Ang, vx: Fx, vy: Fx) -> (Fx, bool) {
    // The car's own axis, and the axis across it.
    let (ax, ay) = (trig::cos(yaw), trig::sin(yaw));
    // Normalise the view direction, cheaply: the octagonal norm is within
    // six per cent, and six per cent of a car's length is a tenth of a
    // character at any distance you can see one.
    let (a, b) = (fixed::abs(vx), fixed::abs(vy));
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    let n = hi + fixed::mul(lo, fixed::ratio(3, 8));
    if n <= 0 {
        return (wid, false);
    }
    let (ux, uy) = (fixed::div(vx, n), fixed::div(vy, n));
    // |cos t| is the view along the car's axis; |sin t| is across it.
    let along = fixed::abs(fixed::mul(ux, ax) + fixed::mul(uy, ay));
    let across = fixed::abs(fixed::mul(ux, -ay) + fixed::mul(uy, ax));
    let width = fixed::mul(len, across) + fixed::mul(wid, along);
    (width, across > along)
}

/// Draw one billboard.
///
/// Returns false if it was entirely off screen or entirely hidden, which the
/// caller can use to skip the work of animating things nobody can see.
pub fn draw(f: &mut Frame, depth: &[Fx], cam: &Camera, atmos: &Atmos, p: &Proj, b: &Billboard) -> bool {
    let (dx, dy) = cam.dir();
    let (px, py) = cam.plane();

    // Into camera space.  The 2x2 matrix [plane | dir] takes camera space to
    // world space, so its inverse takes the sprite the other way; `ty` comes
    // out as the perpendicular distance, in the same units the wall depths
    // are in, which is what makes the comparison below meaningful.
    let (rx, ry) = (b.x - cam.x, b.y - cam.y);
    let det = fixed::mul(px, dy) - fixed::mul(dx, py);
    if det == 0 {
        return false;
    }
    let tx = fixed::div(fixed::mul(dy, rx) - fixed::mul(dx, ry), det);
    let ty = fixed::div(fixed::mul(px, ry) - fixed::mul(py, rx), det);
    if ty < fixed::ratio(1, 8) {
        return false; // behind the camera, or on top of it
    }
    if ty > fixed::from_int(crate::atmos::draw_distance(atmos.haze)) {
        return false;
    }

    // Screen position and size.
    let centre = fixed::mul(
        fixed::from_int(p.w / 2),
        ONE + fixed::div(tx, ty),
    );
    let cx = fixed::floor(centre);
    // Columns per world unit is rows per world unit times the cell aspect.
    let half_w = fixed::floor(fixed::div(
        fixed::mul(fixed::mul(b.w, p.proj), fixed::from_int(CELL_ASPECT)),
        fixed::mul(ty, fixed::from_int(2)),
    ))
    .max(0);
    let rows = fixed::floor(fixed::div(fixed::mul(b.h, p.proj), ty)).max(0);
    if rows == 0 || half_w == 0 {
        return false;
    }
    // The foot of the card, and the top, in screen rows.
    //
    // `p.eye` is the camera's height *above the ground*, which is the number
    // the ground plane is drawn with - see `raycast::projection`.  This used
    // `cam.z`, the camera's absolute height, and the two are not the same
    // thing anywhere the terrain has risen: measured across four places in
    // one city, an eye height of 0.71 to 0.80 against a `cam.z` of 1.17 to
    // 1.83.  Sprites were therefore drawn with an eye height twice to two
    // and a half times too big, which pushes their feet that much further
    // below the horizon - and because the error scales with `1/ty` like
    // everything else in the projection, a car ten cells away sat eleven
    // rows too low while a distant one was almost right.  That is what
    // "the cars are in the wrong place and drift as they recede" was.
    let foot = p.horizon + fixed::floor(fixed::div(fixed::mul(p.eye - b.base, p.proj), ty));
    let top = foot - rows;

    // Leaning: a knocked-over lamp post pivots about its foot.  Shearing the
    // card is not a rotation and does not pretend to be - but at eight
    // characters tall the difference between a shear and a rotation is
    // roughly one character, and a shear is two additions.
    let shear = b.lean as i32;

    let mut drawn = false;
    let art = b.stamp.art();
    for sy in top.max(0)..=(foot - 1).min(p.h - 1) {
        if sy < 0 {
            continue;
        }
        let v = ((sy - top) * 8 / rows.max(1)).clamp(0, 7) as usize;
        // Rows nearer the top lean further over.
        let lean_px = shear * (7 - v as i32) * half_w / 8;
        for sx in (cx - half_w + lean_px).max(0)..=(cx + half_w + lean_px).min(p.w - 1) {
            let col = sx as usize;
            if col >= depth.len() || depth[col] < ty {
                continue; // a building is in the way
            }
            let u = ((sx - (cx - half_w + lean_px)) * 8 / (2 * half_w).max(1)).clamp(0, 7) as usize;
            let ch = art[v].as_bytes().get(u).copied().unwrap_or(b' ') as char;
            let Some((g, hue, luma)) = glyph_for(ch, b.hue, b.phase) else {
                continue;
            };
            f.put(sx, sy, Cel { glyph: g, color: atmos.shade(hue, luma, ty) });
            drawn = true;
        }
    }
    drawn
}

/// Draw a list of billboards, furthest first so nearer ones win.
pub fn draw_all(
    f: &mut Frame,
    depth: &[Fx],
    cam: &Camera,
    atmos: &Atmos,
    p: &Proj,
    boards: &mut [Billboard],
    order: &mut Vec<(Fx, usize)>,
) {
    order.clear();
    for (i, b) in boards.iter().enumerate() {
        let (rx, ry) = (b.x - cam.x, b.y - cam.y);
        // Squared distance would overflow Q16.16 at city scale; the
        // octagonal norm does not, and sorting only needs an ordering.
        let (a, c) = (fixed::abs(rx), fixed::abs(ry));
        let (hi, lo) = if a > c { (a, c) } else { (c, a) };
        order.push((hi + lo * 3 / 8, i));
    }
    order.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    for &(_, i) in order.iter() {
        draw(f, depth, cam, atmos, p, &boards[i]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raycast;
    use crate::trig;
    use crate::world::City;

    fn setup() -> (City, Camera, Atmos, Frame, Vec<Fx>, Proj) {
        let city = City::generate(12);
        let mut cam = Camera::spawn(&city, 48, 48);
        cam.yaw = 0;
        let atmos = Atmos { rain: 0, ..Default::default() };
        let mut f = Frame::new(100, 36);
        let mut depth = Vec::new();
        raycast::render_to(&city, &cam, &atmos, &mut f, &mut depth);
        let p = raycast::projection(&city, &cam, &f);
        (city, cam, atmos, f, depth, p)
    }

    #[test]
    fn every_stamp_has_eight_rows_of_eight() {
        for s in [
            Stamp::LampPost, Stamp::Signal, Stamp::Hydrant, Stamp::Mailbox, Stamp::Tree,
            Stamp::Bollard, Stamp::Meter, Stamp::Car, Stamp::Wreck, Stamp::Bus, Stamp::Ped,
            Stamp::Coin, Stamp::Pickup, Stamp::Debris,
        ] {
            let art = s.art();
            assert_eq!(art.len(), 8, "{s:?} has the wrong number of rows");
            for (i, row) in art.iter().enumerate() {
                assert_eq!(row.len(), 8, "{s:?} row {i} is {:?}", row);
                assert!(row.is_ascii(), "{s:?} row {i} is not ASCII");
            }
        }
    }

    #[test]
    fn something_right_in_front_of_the_camera_gets_drawn() {
        let (_c, cam, atmos, mut f, depth, p) = setup();
        let (dx, dy) = cam.dir();
        let b = Billboard::upright(
            Stamp::LampPost,
            cam.x + fixed::mul(dx, fixed::from_int(2)),
            cam.y + fixed::mul(dy, fixed::from_int(2)),
            fixed::ratio(1, 2),
            fixed::from_int(2),
            palette::H_WHITE,
        );
        assert!(draw(&mut f, &depth, &cam, &atmos, &p, &b), "nothing was drawn");
    }

    #[test]
    fn something_behind_the_camera_is_not_drawn() {
        let (_c, cam, atmos, mut f, depth, p) = setup();
        let (dx, dy) = cam.dir();
        let b = Billboard::upright(
            Stamp::Car,
            cam.x - fixed::mul(dx, fixed::from_int(4)),
            cam.y - fixed::mul(dy, fixed::from_int(4)),
            ONE,
            ONE,
            palette::H_YELLOW,
        );
        assert!(!draw(&mut f, &depth, &cam, &atmos, &p, &b));
    }

    #[test]
    fn something_beyond_the_draw_distance_is_not_drawn() {
        let (_c, cam, atmos, mut f, depth, p) = setup();
        let (dx, dy) = cam.dir();
        let far = fixed::from_int(crate::atmos::draw_distance(atmos.haze) + 20);
        let b = Billboard::upright(
            Stamp::Car,
            cam.x + fixed::mul(dx, far),
            cam.y + fixed::mul(dy, far),
            ONE,
            ONE,
            palette::H_YELLOW,
        );
        assert!(!draw(&mut f, &depth, &cam, &atmos, &p, &b));
    }

    #[test]
    fn a_sprite_behind_a_wall_is_hidden() {
        let (_c, cam, atmos, mut f, depth, p) = setup();
        // Put it past the nearest wall in the middle column.
        let mid = depth[p.w as usize / 2];
        if mid >= fixed::from_int(crate::atmos::draw_distance(atmos.haze)) {
            return; // nothing in the way to test against
        }
        let (dx, dy) = cam.dir();
        let d = mid + fixed::from_int(3);
        let b = Billboard::upright(
            Stamp::Car,
            cam.x + fixed::mul(dx, d),
            cam.y + fixed::mul(dy, d),
            ONE,
            ONE,
            palette::H_YELLOW,
        );
        assert!(!draw(&mut f, &depth, &cam, &atmos, &p, &b), "drew a car through a building");
    }

    #[test]
    fn a_sprite_gets_smaller_as_it_gets_further_away() {
        let (_c, cam, atmos, mut f, depth, p) = setup();
        let (dx, dy) = cam.dir();
        let filled = |d: i32, f: &mut Frame| {
            f.clear();
            let b = Billboard::upright(
                Stamp::Bus,
                cam.x + fixed::mul(dx, fixed::from_int(d)),
                cam.y + fixed::mul(dy, fixed::from_int(d)),
                ONE,
                ONE,
                palette::H_YELLOW,
            );
            draw(f, &depth, &cam, &atmos, &p, &b);
            f.cels.iter().filter(|c| **c != Cel::EMPTY).count()
        };
        let near = filled(2, &mut f);
        let far = filled(6, &mut f);
        assert!(near > far, "a bus at 2 units ({near}) is no bigger than one at 6 ({far})");
        assert!(far > 0, "the far bus vanished entirely");
    }

    #[test]
    fn a_sprite_off_to_the_side_lands_off_to_the_side() {
        let (_c, mut cam, atmos, mut f, depth, p) = setup();
        cam.yaw = 0;
        let put = |ang: f64, f: &mut Frame| -> Option<i32> {
            f.clear();
            let a = trig::from_degrees(ang);
            let b = Billboard::upright(
                Stamp::Coin,
                cam.x + fixed::mul(trig::cos(a), fixed::from_int(4)),
                cam.y + fixed::mul(trig::sin(a), fixed::from_int(4)),
                ONE,
                ONE,
                palette::H_YELLOW,
            );
            draw(f, &depth, &cam, &atmos, &p, &b);
            (0..p.w).find(|&x| (0..p.h).any(|y| f.get(x, y) != Cel::EMPTY))
        };
        let left = put(-15.0, &mut f);
        let right = put(15.0, &mut f);
        if let (Some(l), Some(r)) = (left, right) {
            assert!(l < r, "a sprite to the left ({l}) drew right of one to the right ({r})");
        }
    }

    #[test]
    fn drawing_a_crowd_never_panics() {
        let (_c, cam, atmos, mut f, depth, p) = setup();
        let mut boards: Vec<Billboard> = (0..200)
            .map(|i| {
                let mut b = Billboard::upright(
                    Stamp::Ped,
                    cam.x + fixed::ratio(i % 37 - 18, 2),
                    cam.y + fixed::ratio(i / 37 - 2, 2),
                    fixed::ratio(1, 3),
                    ONE,
                    palette::H_PINK,
                );
                b.lean = (i % 9) as u8;
                b.phase = (i % 3) as u8;
                b
            })
            .collect();
        let mut order = Vec::new();
        draw_all(&mut f, &depth, &cam, &atmos, &p, &mut boards, &mut order);
        assert_eq!(order.len(), 200);
    }
}

#[cfg(test)]
mod silhouette_tests {
    use super::*;
    use crate::fixed;

    /// A box looked at down its own length is as wide as the box is wide,
    /// and broadside it is as wide as the box is long.
    #[test]
    fn a_car_is_as_long_from_the_side_as_it_is_wide_from_behind() {
        let (len, wid) = (fixed::from_int(2), fixed::ratio(6, 5));
        // Pointing east, seen from the west: end-on.
        let (w, side) = silhouette(len, wid, 0, ONE, 0);
        assert_eq!(w, wid, "a car seen down its own length should be its width");
        assert!(!side);
        // Pointing east, seen from the north: broadside.
        let (w, side) = silhouette(len, wid, 0, 0, ONE);
        assert_eq!(w, len, "a car seen broadside should be its length");
        assert!(side);
    }

    /// ...and between those two at every angle from every heading, give or
    /// take the octagonal norm.
    ///
    /// The tolerance is not slack.  The view direction is normalised with
    /// `max + 3/8 min` rather than a square root, which is short by up to
    /// about three per cent at forty-five degrees, and the error lands on
    /// both terms of the silhouette.  Three per cent of a car is a fiftieth
    /// of a character at any distance you can see one, and a square root per
    /// vehicle per frame is not worth it - but the bound is real and is
    /// stated here rather than hidden in a fudge factor.
    #[test]
    fn the_silhouette_is_between_the_width_and_the_length() {
        let (len, wid) = (fixed::from_int(2), fixed::ratio(6, 5));
        let slack = fixed::mul(len, fixed::ratio(1, 20));
        for yaw in (0..65_536).step_by(1_021) {
            for view in (0..65_536).step_by(997) {
                let (vx, vy) = (trig::cos(view as Ang), trig::sin(view as Ang));
                let (w, side) = silhouette(len, wid, yaw as Ang, vx, vy);
                assert!(
                    w >= wid - slack && w <= len + wid,
                    "yaw {yaw} view {view} gave {}",
                    fixed::to_f32(w)
                );
                // Side-on means the long dimension dominates, so the
                // silhouette has to be at least as wide as the car is.
                if side {
                    assert!(w > wid - slack, "yaw {yaw} view {view} claims side-on at {}", fixed::to_f32(w));
                }
            }
        }
    }
}


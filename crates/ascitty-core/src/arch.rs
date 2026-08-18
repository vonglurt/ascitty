//! Building architecture: what a wall looks like at a given point.
//!
//! A tower in this renderer is not a coloured box.  When a ray lands on a
//! facade, this module is asked what is at that exact spot, and it answers
//! from the archetype, the floor, the bay and a hash - never from stored
//! geometry.  A sixty-storey tower costs the same to look at as a shed.
//!
//! What that buys, in the order the eye notices it:
//!
//! - **The zipper.**  Windows repeat on a fixed floor and bay pitch, so a
//!   tall face is a regular grid of light running the full height.  On a
//!   [`Arch::Slab`] the piers between bays run uninterrupted top to bottom
//!   and the grid reads as vertical stripes; on a [`Arch::CurtainWall`]
//!   there are no piers and it reads as a mesh.  Same geometry, different
//!   building.
//! - **The fire escape.**  A brick walk-up carries an iron staircase on its
//!   street face: landings on the floor lines, a zigzag between them, a drop
//!   ladder at the bottom.  It is the most recognisable thing on a prewar
//!   building and it is two glyph families and a hash.
//! - **The ground floor.**  Shopfronts are brighter, taller and more
//!   irregular than the floors above, which is what stops a street from
//!   looking like a filing cabinet standing on end.
//! - **The top.**  A cornice, a parapet, and on the Deco towers a crown of
//!   piers that carry past the last floor.

use crate::catalog::{self, GlyphId};
use crate::fixed::{self, Fx};
use crate::rng::hash3;
use crate::world::{Arch, Lot};

/// Floors per world unit of height.
///
/// One cell is about 6 m - an avenue is three cells, which is a real
/// avenue, and a lot is two to five, which is a real frontage.  Two floors
/// to a unit therefore puts a floor at 3 m, and a sixty-unit tower at
/// 360 m, which is an Empire State Building.
pub const FLOORS_PER_UNIT: i32 = 2;

/// Window bays per world unit across a facade.  Three to a six-metre cell is
/// a two-metre bay: a window and its pier.
pub const BAYS_PER_UNIT: i32 = 3;

/// Height of the ground-floor shopfront, in world units.
pub const GROUND: Fx = fixed::ratio(1, 2);

/// Which wall of a lot a ray hit.  A fire escape is on the *street* face,
/// so the renderer has to know which one that is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// Facing -y.
    North,
    /// Facing +x.
    East,
    /// Facing +y.
    South,
    /// Facing -x.
    West,
}

impl Face {
    /// Index 0..3, for hashing.
    pub fn index(self) -> u32 {
        self as u32
    }

    /// The face a ray hit, from which grid plane it crossed and which way
    /// it was going.
    #[inline(always)]
    pub fn of(vertical: bool, step_x: i32, step_y: i32) -> Face {
        if vertical {
            if step_x > 0 {
                Face::West
            } else {
                Face::East
            }
        } else if step_y > 0 {
            Face::North
        } else {
            Face::South
        }
    }
}

/// How much of a facade one character cell has to hold.
///
/// Near the camera a cell shows part of one window; far away it shows four
/// floors at once.  Drawing the near tile at distance produces a shimmering
/// mess as the sampling grid beats against the window pitch, so the tile
/// coarsens with distance instead - the ordinary mipmap argument, arrived at
/// for the ordinary reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lod {
    /// One cell is part of one window.
    Near,
    /// One cell is about one window.
    Mid,
    /// One cell is a few windows.
    Far,
    /// One cell is a whole patch of facade.
    Distant,
}

impl Lod {
    /// Pick a level of detail from the perpendicular distance in world units.
    #[inline(always)]
    pub fn at(dist: Fx) -> Lod {
        let d = fixed::floor(dist);
        if d < 3 {
            Lod::Near
        } else if d < 9 {
            Lod::Mid
        } else if d < 26 {
            Lod::Far
        } else {
            Lod::Distant
        }
    }

    /// Which facade tile configuration this level uses.
    fn cfg(self) -> u8 {
        match self {
            Lod::Near => 0,
            Lod::Mid => 1,
            Lod::Far => 2,
            Lod::Distant => 3,
        }
    }

    /// How many floors and bays one screen cell covers, near enough.
    ///
    /// This is the number that stops a distant tower turning into static.
    /// A cell 40 units away spans four floors; asking "is *this* window
    /// lit" four times and getting four different answers produces noise at
    /// exactly the frequency the eye is worst at.  Quantising the floor and
    /// bay indices to the cell's own footprint instead means one cell asks
    /// the question once, so a far facade reads as coherent patches - which
    /// is what a real one looks like from four hundred metres.
    fn quant(self) -> i32 {
        match self {
            Lod::Near => 1,
            Lod::Mid => 1,
            Lod::Far => 2,
            Lod::Distant => 4,
        }
    }
}

/// What is at one point on a wall.
#[derive(Clone, Copy, Debug)]
pub struct Surface {
    /// Which catalogue shape to draw.
    pub glyph: GlyphId,
    /// Hue to draw it in - usually the lot's, but ironwork and signage have
    /// their own.
    pub hue: u8,
    /// How bright before distance is applied, 0..=7.
    pub luma: u8,
}

/// Sample a facade.
///
/// `along` is the world coordinate running across the face and `z` is the
/// height above the pavement, both in cell units.  `local_h` is the height
/// of the *cell* that was hit rather than of the whole lot, so a setback
/// tier ends where that tier ends.
pub fn facade(lot: &Lot, face: Face, along: Fx, z: Fx, local_h: Fx, lod: Lod) -> Surface {
    let hue = lot.hue;
    let floor = fixed::floor(fixed::mul(z, fixed::from_int(FLOORS_PER_UNIT)));
    let bay = fixed::floor(fixed::mul(along, fixed::from_int(BAYS_PER_UNIT)));
    let f = face.index();

    // The top two courses: cornice, then parapet above it.
    let top_floor = fixed::floor(fixed::mul(local_h, fixed::from_int(FLOORS_PER_UNIT)));
    if floor >= top_floor - 1 {
        let g = match lot.arch {
            Arch::Prewar | Arch::LowRise => catalog::G_CORNICE + 1,
            Arch::Deco => catalog::G_CORNICE + 2,
            _ => catalog::G_CORNICE + 3,
        };
        return Surface { glyph: g, hue, luma: lot.luma };
    }

    // Ground floor: lit shopfront, with a sign band over it.
    if z < GROUND {
        let h = hash3(lot.seed, f, bay as u32);
        let sign = h & 7 == 0;
        return if sign {
            Surface { glyph: catalog::ST_SIGN, hue: sign_hue(h), luma: 7 }
        } else if z < fixed::mul(GROUND, fixed::ratio(1, 4)) {
            Surface { glyph: catalog::G_CORNICE, hue, luma: 3 }
        } else {
            Surface { glyph: catalog::facade_tile(lod.cfg(), 0), hue: shop_hue(h), luma: 7 }
        };
    }

    // The fire escape, if this building has one and this is its face.
    if lot.arch.has_fire_escape() {
        if let Some(s) = fire_escape(lot, face, bay, floor, z, top_floor) {
            return s;
        }
    }

    // Piers.  On a slab they are the point of the building; on a Deco tower
    // they are every third bay and they carry the crown.
    let pier_pitch = match lot.arch {
        Arch::Slab => 2,
        Arch::Deco => 3,
        _ => 0,
    };
    if pier_pitch != 0 && bay.rem_euclid(pier_pitch) == 0 {
        return Surface { glyph: catalog::G_MULLION + 2, hue, luma: lot.luma.saturating_sub(1).max(1) };
    }

    // Every building expresses its corners, whatever it is made of - the
    // vertical line where two faces meet is the strongest cue that a tower
    // is a box standing in space rather than a flat patch of light.
    let face_bays = span_bays(lot);
    let local_bay = bay.rem_euclid(face_bays.max(1));
    if local_bay == 0 || local_bay == face_bays - 1 {
        return Surface { glyph: catalog::G_MULLION + 2, hue, luma: lot.luma.saturating_sub(1).max(1) };
    }

    // A spandrel course every few floors on the older buildings, which is
    // what makes a brick face read as brick rather than as a gradient.
    if matches!(lot.arch, Arch::Prewar) && floor % 4 == 3 {
        return Surface { glyph: catalog::G_CORNICE, hue, luma: 3 };
    }

    // An ordinary window.
    //
    // Two decisions, taken at deliberately different rates:
    //
    // - *Which tile* is a property of the **building**, not of the window.
    //   One tower is drawn in `X`, its neighbour in `0`, a third in `8`.
    //   That is what makes a skyline read as a row of distinct buildings
    //   rather than as one textured mass, and it is why the tile index
    //   comes from the lot's seed and nothing else.
    // - *Whether the light is on* is a property of the window, sampled at
    //   the resolution the screen cell can actually resolve.
    let q = lod.quant();
    let qf = floor / q;
    let qb = bay / q;
    let h = hash3(lot.seed, f * 8192 + qf as u32, qb as u32);
    let occupied = (h >> 3) & 15;

    // How much of this building is lit is a property of *the building* -
    // one tower works late and its neighbour is empty - with the archetype
    // only nudging it.  A curtain-wall office block is lit later than a
    // brick walk-up whatever else is true of either.
    let bias: i32 = match lot.arch {
        Arch::CurtainWall => 1,
        Arch::Slab | Arch::Setback | Arch::Deco => 0,
        Arch::Prewar => -2,
        Arch::LowRise => -3,
    };
    let threshold = (lot.lit as i32 + bias).clamp(1, 15) as u32;
    if occupied >= threshold {
        // A dark window is not black - it still catches the sky.
        return Surface { glyph: catalog::shade(1), hue, luma: (lot.luma / 3).max(1) };
    }
    Surface {
        glyph: catalog::facade_tile(lod.cfg(), house_style(lot)),
        hue,
        luma: (lot.luma as u32 + (h >> 20) % 2).min(7) as u8,
    }
}

/// Which of the four lit patterns this *building* is drawn in.  Constant for
/// the life of the lot, which is the whole point.
#[inline]
fn house_style(lot: &Lot) -> u8 {
    (lot.seed >> 17) as u8 & 3
}

/// How many bays fit across the widest face of a lot.
#[inline]
fn span_bays(lot: &Lot) -> i32 {
    (lot.w().max(lot.d()) as i32) * BAYS_PER_UNIT
}

/// The ironwork, or `None` if this spot on the wall is not on it.
fn fire_escape(lot: &Lot, face: Face, bay: i32, floor: i32, z: Fx, top: i32) -> Option<Surface> {
    // Which face carries it, and where along that face, is fixed per lot -
    // it must not move between frames or between viewing angles.
    let pick = hash3(lot.seed, 0xF14E_5C00, 0);
    if face.index() != pick & 3 {
        return None;
    }
    let width = 2;
    let span = span_bays(lot);
    let start = (pick >> 8) as i32 % (span - width).max(1);
    let local = bay.rem_euclid(span.max(1)) - start;
    if !(0..width).contains(&local) {
        return None;
    }
    if floor >= top - 1 {
        return None; // the escape stops below the cornice
    }

    let hue = crate::palette::H_WHITE;
    // Within a floor: a landing at the bottom of it, stairs above.
    let sub = fixed::frac(fixed::mul(z, fixed::from_int(FLOORS_PER_UNIT)));
    let on_landing = sub < fixed::ratio(1, 3);
    Some(if floor <= 1 {
        Surface { glyph: catalog::FIRE_LADDER, hue, luma: 3 }
    } else if on_landing {
        let g = if local == width - 1 { catalog::FIRE_RAIL } else { catalog::FIRE_LANDING };
        Surface { glyph: g, hue, luma: 4 }
    } else if floor % 2 == 0 {
        Surface { glyph: catalog::FIRE_ZIG_R, hue, luma: 3 }
    } else {
        Surface { glyph: catalog::FIRE_ZIG_L, hue, luma: 3 }
    })
}

/// What is on top of a building, seen from above its roofline.
pub fn roof(lot: &Lot, x: i32, y: i32) -> Surface {
    let h = hash3(lot.seed, x as u32, y as u32);
    let g = match h & 7 {
        0 => catalog::G_ROOF + 1, // water tank
        1 => catalog::G_ROOF + 2, // plant housing
        2 if lot.height > 24 => catalog::G_ROOF + 3, // mast
        _ => catalog::G_ROOF,
    };
    Surface { glyph: g, hue: lot.hue, luma: 3 }
}

/// Shopfront hues: warm, because a lit shop at night is warm and a blue one
/// looks like an aquarium.
fn shop_hue(h: u32) -> u8 {
    const WARM: [u8; 4] = [
        crate::palette::H_YELLOW,
        crate::palette::H_ORANGE,
        crate::palette::H_WHITE,
        crate::palette::H_YELLOW_GREEN,
    ];
    WARM[(h >> 5) as usize & 3]
}

/// Sign hues: saturated, because neon is.
fn sign_hue(h: u32) -> u8 {
    const NEON: [u8; 4] = [
        crate::palette::H_RED,
        crate::palette::H_PINK,
        crate::palette::H_CYAN,
        crate::palette::H_GREEN,
    ];
    NEON[(h >> 11) as usize & 3]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{Arch, City, Lot};

    fn lot(arch: Arch, height: u8) -> Lot {
        Lot { x0: 10, y0: 10, x1: 13, y1: 13, height, arch, hue: 6, luma: 6, lit: 10, seed: 0x1234_5678 }
    }

    #[test]
    fn a_wall_answers_the_same_way_every_time() {
        let l = lot(Arch::CurtainWall, 30);
        let z = fixed::ratio(37, 4);
        let u = fixed::ratio(11, 3);
        let a = facade(&l, Face::North, u, z, fixed::from_int(30), Lod::Mid);
        let b = facade(&l, Face::North, u, z, fixed::from_int(30), Lod::Mid);
        assert_eq!(a.glyph, b.glyph);
        assert_eq!(a.luma, b.luma);
    }

    #[test]
    fn the_top_of_a_building_is_a_cornice() {
        let l = lot(Arch::Prewar, 8);
        let h = fixed::from_int(8);
        let s = facade(&l, Face::North, fixed::ONE, h - fixed::ratio(1, 8), h, Lod::Near);
        assert!((catalog::G_CORNICE..catalog::G_FIRE).contains(&s.glyph), "no cornice at the top");
    }

    #[test]
    fn the_ground_floor_is_brighter_than_the_offices() {
        let l = lot(Arch::CurtainWall, 20);
        let h = fixed::from_int(20);
        let shop = facade(&l, Face::North, fixed::ONE, fixed::ratio(1, 3), h, Lod::Near);
        let office = facade(&l, Face::North, fixed::ONE, fixed::from_int(8), h, Lod::Near);
        assert!(shop.luma >= office.luma, "the shopfront is darker than the tenth floor");
    }

    #[test]
    fn a_slab_is_striped_with_piers_and_a_curtain_wall_is_not() {
        // Both express their corners - that is what makes either of them
        // look like a box.  What tells them apart is the *interior*: a slab
        // repeats a pier every second bay all the way across, a curtain wall
        // has nothing between its corners but glass.
        let h = fixed::from_int(20);
        let z = fixed::from_int(7);
        let slab = lot(Arch::Slab, 20);
        let glass = lot(Arch::CurtainWall, 20);
        let pier = catalog::G_MULLION + 2;
        let count = |l: &Lot| {
            (0..96)
                .filter(|i| {
                    facade(l, Face::North, fixed::ratio(*i, 8), z, h, Lod::Near).glyph == pier
                })
                .count()
        };
        let (s, g) = (count(&slab), count(&glass));
        assert!(s > g * 2, "a slab ({s}) is barely more striped than a curtain wall ({g})");
        assert!(g > 0, "the curtain wall lost its corners");
    }

    #[test]
    fn only_brick_buildings_carry_a_fire_escape() {
        assert!(Arch::Prewar.has_fire_escape());
        assert!(Arch::LowRise.has_fire_escape());
        assert!(!Arch::CurtainWall.has_fire_escape());
        assert!(!Arch::Slab.has_fire_escape());
        assert!(!Arch::Setback.has_fire_escape());
    }

    #[test]
    fn a_prewar_building_has_ironwork_on_exactly_one_face() {
        let l = lot(Arch::Prewar, 9);
        let h = fixed::from_int(9);
        let faces: Vec<Face> = [Face::North, Face::East, Face::South, Face::West]
            .into_iter()
            .filter(|&f| {
                (0..200).any(|i| {
                    let u = fixed::ratio(i, 16);
                    let z = fixed::ratio(i % 50 + 8, 8);
                    let g = facade(&l, f, u, z, h, Lod::Near).glyph;
                    (catalog::G_FIRE..catalog::G_ROOF).contains(&g)
                })
            })
            .collect();
        assert_eq!(faces.len(), 1, "fire escapes on {faces:?}");
    }

    #[test]
    fn the_fire_escape_stays_in_the_same_place_as_you_walk_past() {
        let l = lot(Arch::Prewar, 9);
        let h = fixed::from_int(9);
        let face = [Face::North, Face::East, Face::South, Face::West]
            .into_iter()
            .find(|&f| {
                (0..200).any(|i| {
                    let g = facade(&l, f, fixed::ratio(i, 16), fixed::from_int(3), h, Lod::Near).glyph;
                    (catalog::G_FIRE..catalog::G_ROOF).contains(&g)
                })
            })
            .expect("no fire escape at all");
        // The set of bays carrying ironwork must not depend on level of detail.
        let bays = |lod| -> Vec<i32> {
            (0..200)
                .filter(|i| {
                    let g = facade(&l, face, fixed::ratio(*i, 16), fixed::from_int(3), h, lod).glyph;
                    (catalog::G_FIRE..catalog::G_ROOF).contains(&g)
                })
                .collect()
        };
        assert_eq!(bays(Lod::Near), bays(Lod::Distant), "the escape moved when the camera did");
        assert!(!bays(Lod::Near).is_empty());
    }

    #[test]
    fn level_of_detail_coarsens_with_distance() {
        assert_eq!(Lod::at(fixed::from_int(1)), Lod::Near);
        assert_eq!(Lod::at(fixed::from_int(5)), Lod::Mid);
        assert_eq!(Lod::at(fixed::from_int(15)), Lod::Far);
        assert_eq!(Lod::at(fixed::from_int(60)), Lod::Distant);
    }

    #[test]
    fn every_sampled_glyph_is_in_the_catalogue() {
        let city = City::generate(3);
        for l in city.lots.iter().take(200) {
            let h = fixed::from_int(l.height as i32);
            for i in 0..120 {
                let s = facade(
                    l,
                    Face::of(i % 2 == 0, 1, -1),
                    fixed::ratio(i, 7),
                    fixed::ratio(i * 3, 11),
                    h,
                    Lod::at(fixed::from_int(i % 40)),
                );
                assert!((s.glyph as usize) < catalog::N_GLYPHS);
                assert!(s.luma < 8);
                assert!(s.hue < 16);
            }
        }
    }
}

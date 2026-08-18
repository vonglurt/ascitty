//! The city: a grid, the lots on it, and the generator that lays them out.
//!
//! The world is a **height field**, not a set of boxes.  Every cell carries
//! one height, and a building is a run of cells that happen to share a lot.
//! That choice is what makes the renderer cheap - a height field can be
//! walked front to back in a single pass per column, and the skyline falls
//! out of it - and it is also what makes setbacks free: a tower that steps
//! in as it rises is a lot whose edge cells are shorter than its middle,
//! which costs nothing at render time and reads correctly from every angle.
//!
//! Nothing here is stored per floor.  A sixty-storey tower is one lot record
//! and a handful of cell heights; its windows are a hash of
//! (lot, face, floor, bay), evaluated when a ray happens to land on one.

use crate::rng::{hash3, Rng};

/// The city is this many cells on a side.
pub const SIZE: usize = 96;

/// What occupies a cell at ground level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    /// Carriageway.  Cars go here.
    Road = 0,
    /// Sidewalk, kerbed, with lamps and trees on it.
    Sidewalk = 1,
    /// Built on.  `height` is above zero and `lot` names the building.
    Building = 2,
    /// Open ground inside a block - a park or a plaza.
    Park = 3,
    /// Paved open ground.
    Plaza = 4,
}

/// One cell of the height field.
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    /// What is here.
    pub kind: Kind,
    /// Height in world units, where one unit is one cell width.  Zero for
    /// anything you can walk on.
    pub height: u8,
    /// Index into [`City::lots`], or [`NO_LOT`].
    pub lot: u16,
    /// Per-cell noise, so identical cells still differ in their detail.
    pub seed: u8,
}

/// The lot index meaning "none".
pub const NO_LOT: u16 = u16::MAX;

/// How a building is put together.  This is the part the eye actually reads:
/// two towers of the same height and colour are still obviously different
/// buildings if one is a glass slab and the other is a brick walk-up with an
/// iron staircase bolted to the front of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Arch {
    /// Continuous glass grid, no expressed floors, corner to corner.
    CurtainWall = 0,
    /// Vertical piers running the full height with window slots between
    /// them - the stripe that reads as a zipper down the face.
    Slab = 1,
    /// Brick, punched windows, a cornice, and a fire escape on the street
    /// face.  The short ones between the towers.
    Prewar = 2,
    /// Steps inward as it rises.  The wedding cake.
    Setback = 3,
    /// Setback plus a crown: piers that carry past the top floor.
    Deco = 4,
    /// Two or three storeys of shopfront.
    LowRise = 5,
}

impl Arch {
    /// Decode from the byte the generator and the baked city both store.
    pub fn from_u8(v: u8) -> Arch {
        match v {
            0 => Arch::CurtainWall,
            1 => Arch::Slab,
            2 => Arch::Prewar,
            3 => Arch::Setback,
            4 => Arch::Deco,
            _ => Arch::LowRise,
        }
    }

    /// Whether this kind of building carries a fire escape on its street
    /// face.  Only the brick ones do; a curtain wall has nowhere to bolt it.
    pub fn has_fire_escape(self) -> bool {
        matches!(self, Arch::Prewar | Arch::LowRise)
    }
}

/// One building.
#[derive(Clone, Copy, Debug)]
pub struct Lot {
    /// West edge of the footprint, inclusive, in cells.
    pub x0: u8,
    /// North edge of the footprint, inclusive.
    pub y0: u8,
    /// East edge of the footprint, inclusive.
    pub x1: u8,
    /// South edge of the footprint, inclusive.
    pub y1: u8,
    /// Height of the tallest part, in world units.
    pub height: u8,
    /// How it is built.
    pub arch: Arch,
    /// Hue of the facade, as a [`crate::palette`] hue index.
    pub hue: u8,
    /// Everything else about it - which windows are lit, where the fire
    /// escape sits, what is on the roof.
    pub seed: u32,
}

impl Lot {
    /// Width in cells.
    pub fn w(&self) -> u8 {
        self.x1 - self.x0 + 1
    }
    /// Depth in cells.
    pub fn d(&self) -> u8 {
        self.y1 - self.y0 + 1
    }
}

/// A generated city.
pub struct City {
    /// The height field, row-major.
    pub cells: Vec<Cell>,
    /// The buildings.
    pub lots: Vec<Lot>,
    /// The seed it was generated from.
    pub seed: u32,
}

// --- street layout ---------------------------------------------------------
//
// Avenues run north-south and are wide; streets run east-west and are
// narrow.  That asymmetry is the single most Manhattan thing in the file:
// it gives long sightlines one way and short ones the other, so turning
// ninety degrees changes what the city looks like instead of just rotating
// it.

/// Spacing of avenues, in cells.
pub const AVE_PERIOD: usize = 14;
/// Width of an avenue carriageway.
pub const AVE_WIDTH: usize = 3;
/// Spacing of cross streets.
pub const ST_PERIOD: usize = 9;
/// Width of a cross street carriageway.
pub const ST_WIDTH: usize = 2;

/// Whether a column is inside an avenue.
#[inline]
pub fn on_avenue(x: usize) -> bool {
    x % AVE_PERIOD < AVE_WIDTH
}

/// Whether a row is inside a cross street.
#[inline]
pub fn on_street(y: usize) -> bool {
    y % ST_PERIOD < ST_WIDTH
}

/// Whether a column is deep enough inside a block to be built on - not in
/// the avenue, and not on the sidewalk that flanks it.
#[inline]
pub fn interior_x(x: usize) -> bool {
    !on_avenue(x) && !on_avenue(x.wrapping_sub(1)) && !on_avenue(x + 1)
}

/// The same test for a row and its cross street.
#[inline]
pub fn interior_y(y: usize) -> bool {
    !on_street(y) && !on_street(y.wrapping_sub(1)) && !on_street(y + 1)
}

impl City {
    /// Read a cell, clamped - rays that leave the map see empty ground
    /// rather than an index panic.
    #[inline(always)]
    pub fn at(&self, x: i32, y: i32) -> Cell {
        if x < 0 || y < 0 || x >= SIZE as i32 || y >= SIZE as i32 {
            return Cell { kind: Kind::Road, height: 0, lot: NO_LOT, seed: 0 };
        }
        self.cells[y as usize * SIZE + x as usize]
    }

    /// Height at a cell, zero outside the map.
    #[inline(always)]
    pub fn height(&self, x: i32, y: i32) -> u8 {
        self.at(x, y).height
    }

    /// The lot a cell belongs to, if any.
    #[inline(always)]
    pub fn lot_at(&self, x: i32, y: i32) -> Option<&Lot> {
        let l = self.at(x, y).lot;
        if l == NO_LOT {
            None
        } else {
            self.lots.get(l as usize)
        }
    }

    /// Whether a cell can be stood in.
    #[inline(always)]
    pub fn walkable(&self, x: i32, y: i32) -> bool {
        self.at(x, y).height == 0
    }

    /// Generate a city from a seed.
    pub fn generate(seed: u32) -> City {
        let mut rng = Rng::new(seed);
        let mut cells = vec![
            Cell { kind: Kind::Road, height: 0, lot: NO_LOT, seed: 0 };
            SIZE * SIZE
        ];
        let mut lots: Vec<Lot> = Vec::new();

        // Pass 1: the street grid, and the sidewalk that rings every block.
        for y in 0..SIZE {
            for x in 0..SIZE {
                let road = on_avenue(x) || on_street(y);
                let near = on_avenue(x.wrapping_sub(1)) || on_avenue(x + 1)
                    || on_street(y.wrapping_sub(1)) || on_street(y + 1);
                cells[y * SIZE + x].kind = if road {
                    Kind::Road
                } else if near {
                    Kind::Sidewalk
                } else {
                    Kind::Plaza // provisional: block interior, built on below
                };
                cells[y * SIZE + x].seed = rng.next_u32() as u8;
            }
        }

        // Pass 2: find each block interior and fill it.
        //
        // A block interior is a maximal run of cells that is neither road nor
        // sidewalk.  Scanning for maximal runs - rather than stepping by the
        // street period - is what keeps this correct when the two periods are
        // changed independently, and it guarantees forward progress on every
        // iteration, which the arithmetic version did not.
        let mut y = 0;
        while y < SIZE {
            if !interior_y(y) {
                y += 1;
                continue;
            }
            let y0 = y;
            while y < SIZE && interior_y(y) {
                y += 1;
            }
            let y1 = y - 1;

            let mut x = 0;
            while x < SIZE {
                if !interior_x(x) {
                    x += 1;
                    continue;
                }
                let x0 = x;
                while x < SIZE && interior_x(x) {
                    x += 1;
                }
                let x1 = x - 1;
                fill_block(&mut cells, &mut lots, &mut rng, x0, y0, x1, y1);
            }
        }

        City { cells, lots, seed }
    }
}

/// Distance from the middle of the map, 0 at the centre and 255 at the
/// corners.  Downtown is in the middle, which is what gives the skyline a
/// shape instead of a uniform carpet of towers.
fn downtown(x: usize, y: usize) -> u32 {
    let c = SIZE as i32 / 2;
    let (dx, dy) = ((x as i32 - c).abs(), (y as i32 - c).abs());
    let d = dx.max(dy) as u32;
    (d * 255 / (SIZE as u32 / 2)).min(255)
}

/// Fill one block interior: either open it as a park, or subdivide it into
/// lots and raise a building on each.
fn fill_block(
    cells: &mut [Cell],
    lots: &mut Vec<Lot>,
    rng: &mut Rng,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
) {
    let far = downtown((x0 + x1) / 2, (y0 + y1) / 2);

    // One block in nine is left open, and it is likelier out in the
    // neighbourhoods than it is in the middle of downtown.
    if rng.chance(1 + far / 40, 12) {
        let park = rng.chance(2, 3);
        for y in y0..=y1 {
            for x in x0..=x1 {
                cells[y * SIZE + x].kind = if park { Kind::Park } else { Kind::Plaza };
            }
        }
        return;
    }

    // Subdivide.  Split the longer axis until every piece is small enough to
    // be one address.
    let mut queue = vec![(x0, y0, x1, y1)];
    let mut out = Vec::new();
    while let Some((ax, ay, bx, by)) = queue.pop() {
        let (w, h) = (bx - ax + 1, by - ay + 1);
        let big = w.max(h);
        if big <= 3 || (big <= 5 && rng.chance(1, 3)) {
            out.push((ax, ay, bx, by));
            continue;
        }
        if w >= h {
            let cut = ax + 1 + rng.below((w - 2) as u32) as usize;
            queue.push((ax, ay, cut, by));
            queue.push((cut + 1, ay, bx, by));
        } else {
            let cut = ay + 1 + rng.below((h - 2) as u32) as usize;
            queue.push((ax, ay, bx, cut));
            queue.push((ax, cut + 1, bx, by));
        }
    }

    for (ax, ay, bx, by) in out {
        raise(cells, lots, rng, far, ax, ay, bx, by);
    }
}

/// Palette of facade hues.  Deliberately narrow: a night city is mostly two
/// or three colours of glass with the odd lit brick face, and a wider
/// palette reads as confetti rather than as a place.
const FACADE_HUES: [u8; 8] = [
    crate::palette::H_BLUE,
    crate::palette::H_LIGHT_BLUE,
    crate::palette::H_DARK_BLUE,
    crate::palette::H_CYAN,
    crate::palette::H_YELLOW,
    crate::palette::H_ORANGE,
    crate::palette::H_RED,
    crate::palette::H_BLUE_GREEN,
];

/// Put a building on one lot.
fn raise(
    cells: &mut [Cell],
    lots: &mut Vec<Lot>,
    rng: &mut Rng,
    far: u32,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
) {
    let footprint = (x1 - x0 + 1) * (y1 - y0 + 1);

    // Height falls off from downtown, and a big footprint can carry a taller
    // building than a narrow one - which is why the tall things cluster and
    // the gaps between them are filled with walk-ups.
    let ceiling = (56u32.saturating_sub(far * 46 / 255)).max(4);
    let base = 2 + rng.below(ceiling.max(1));
    let bonus = if footprint >= 9 && rng.chance(1, 3) { rng.below(ceiling) } else { 0 };
    let height = (base + bonus).clamp(2, 60) as u8;

    let arch = if height <= 3 {
        Arch::LowRise
    } else if height <= 9 {
        if rng.chance(3, 4) { Arch::Prewar } else { Arch::LowRise }
    } else if height >= 28 {
        match rng.below(3) {
            0 => Arch::Setback,
            1 => Arch::Deco,
            _ => Arch::CurtainWall,
        }
    } else {
        match rng.below(4) {
            0 => Arch::Slab,
            1 => Arch::CurtainWall,
            2 => Arch::Setback,
            _ => Arch::Prewar,
        }
    };

    let hue = FACADE_HUES[rng.below(FACADE_HUES.len() as u32) as usize];
    let idx = lots.len() as u16;
    lots.push(Lot {
        x0: x0 as u8,
        y0: y0 as u8,
        x1: x1 as u8,
        y1: y1 as u8,
        height,
        arch,
        hue,
        seed: rng.next_u32(),
    });

    for y in y0..=y1 {
        for x in x0..=x1 {
            let c = &mut cells[y * SIZE + x];
            c.kind = Kind::Building;
            c.lot = idx;
            c.height = cell_height(height, arch, x0, y0, x1, y1, x, y);
        }
    }
}

/// The height of one cell of a lot.
///
/// This is where the silhouette comes from.  A slab is flat-topped; a
/// setback loses a tier for every ring you move out from the middle; a Deco
/// tower does the same but keeps its corner piers, so the crown is a notch
/// taller than the shoulders.
fn cell_height(
    height: u8,
    arch: Arch,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    x: usize,
    y: usize,
) -> u8 {
    // How many rings in from the lot edge this cell sits.
    let ring = (x - x0).min(x1 - x).min(y - y0).min(y1 - y) as u32;
    match arch {
        Arch::CurtainWall | Arch::Slab | Arch::Prewar | Arch::LowRise => height,
        Arch::Setback => {
            let tiers = (height as u32 / 4).clamp(1, 5);
            let step = height as u32 / (tiers + 2);
            let lost = step * (tiers.saturating_sub(ring.min(tiers)));
            (height as u32).saturating_sub(lost).max(2) as u8
        }
        Arch::Deco => {
            let tiers = (height as u32 / 5).clamp(1, 4);
            let step = height as u32 / (tiers + 3);
            let lost = step * (tiers.saturating_sub(ring.min(tiers)));
            let corner = (x == x0 || x == x1) && (y == y0 || y == y1);
            let h = (height as u32).saturating_sub(lost).max(2);
            (if corner { h + step / 2 } else { h }).min(63) as u8
        }
    }
}

/// A stable per-cell detail value - which way a lamp faces, whether this
/// paving slab has a grate in it.  Costs nothing to store because it is not
/// stored.
#[inline(always)]
pub fn detail(x: i32, y: i32, salt: u32) -> u32 {
    hash3(x as u32, y as u32, salt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn city() -> City {
        City::generate(0x0C17_7A00)
    }

    #[test]
    fn generation_is_deterministic() {
        let a = City::generate(99);
        let b = City::generate(99);
        assert_eq!(a.lots.len(), b.lots.len());
        for i in 0..a.cells.len() {
            assert_eq!(a.cells[i].height, b.cells[i].height, "cell {i} differs");
            assert_eq!(a.cells[i].kind, b.cells[i].kind);
        }
    }

    #[test]
    fn different_seeds_build_different_cities() {
        let a = City::generate(1);
        let b = City::generate(2);
        let same = (0..a.cells.len()).filter(|&i| a.cells[i].height == b.cells[i].height).count();
        assert!(same < a.cells.len() * 9 / 10, "two seeds produced nearly the same city");
    }

    #[test]
    fn roads_are_always_walkable() {
        let c = city();
        for y in 0..SIZE {
            for x in 0..SIZE {
                if on_avenue(x) || on_street(y) {
                    assert_eq!(c.at(x as i32, y as i32).height, 0, "a building stands in the road at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn the_grid_is_connected_enough_to_walk() {
        // Every avenue must be reachable along every cross street: the two
        // families of road have to actually intersect.
        let c = city();
        let mut crossings = 0;
        for y in 0..SIZE {
            for x in 0..SIZE {
                if on_avenue(x) && on_street(y) && c.walkable(x as i32, y as i32) {
                    crossings += 1;
                }
            }
        }
        assert!(crossings > 100, "only {crossings} intersections - the grid is not a grid");
    }

    #[test]
    fn buildings_always_carry_a_lot_and_a_height() {
        let c = city();
        for cell in &c.cells {
            if cell.kind == Kind::Building {
                assert_ne!(cell.lot, NO_LOT);
                assert!(cell.height > 0);
                assert!((cell.lot as usize) < c.lots.len());
            } else {
                assert_eq!(cell.height, 0);
            }
        }
    }

    #[test]
    fn setbacks_step_inwards_rather_than_outwards() {
        // The middle of a setback lot must never be shorter than its edge.
        let h_edge = cell_height(40, Arch::Setback, 0, 0, 6, 6, 0, 3);
        let h_mid = cell_height(40, Arch::Setback, 0, 0, 6, 6, 3, 3);
        assert!(h_mid > h_edge, "setback got taller towards the street");
    }

    #[test]
    fn a_slab_is_flat_topped() {
        for x in 0..=5 {
            assert_eq!(cell_height(30, Arch::Slab, 0, 0, 5, 5, x, 2), 30);
        }
    }

    #[test]
    fn downtown_is_taller_than_the_edge() {
        let c = city();
        let mid = SIZE / 2;
        let tall_mid: u32 = (mid - 8..mid + 8)
            .flat_map(|y| (mid - 8..mid + 8).map(move |x| (x, y)))
            .map(|(x, y)| c.height(x as i32, y as i32) as u32)
            .max()
            .unwrap();
        let tall_edge: u32 = (0..16)
            .flat_map(|y| (0..16).map(move |x| (x, y)))
            .map(|(x, y)| c.height(x as i32, y as i32) as u32)
            .max()
            .unwrap();
        assert!(tall_mid > tall_edge, "downtown ({tall_mid}) is no taller than the edge ({tall_edge})");
    }

    #[test]
    fn lots_do_not_overlap() {
        let c = city();
        let mut owner = vec![NO_LOT; SIZE * SIZE];
        for (i, l) in c.lots.iter().enumerate() {
            for y in l.y0..=l.y1 {
                for x in l.x0..=l.x1 {
                    let p = y as usize * SIZE + x as usize;
                    assert_eq!(owner[p], NO_LOT, "cell {x},{y} claimed twice");
                    owner[p] = i as u16;
                }
            }
        }
    }

    #[test]
    fn there_are_enough_buildings_to_be_a_city() {
        let c = city();
        assert!(c.lots.len() > 200, "only {} lots", c.lots.len());
        let tall = c.lots.iter().filter(|l| l.height >= 20).count();
        assert!(tall > 10, "only {tall} towers - this is a village");
    }

    #[test]
    fn every_archetype_gets_built() {
        let c = City::generate(4242);
        for a in [Arch::CurtainWall, Arch::Slab, Arch::Prewar, Arch::Setback, Arch::Deco, Arch::LowRise] {
            assert!(c.lots.iter().any(|l| l.arch == a), "{a:?} never appears");
        }
    }
}

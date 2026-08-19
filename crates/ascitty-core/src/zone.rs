//! The zoning map: what a piece of ground is *for*.
//!
//! Separate from what is built on it and separate from how it is built.
//! Three questions that keep getting confused:
//!
//! - **Zone** - what this ground is for. Downtown, commercial, residential,
//!   civic, park. A property of the *place*.
//! - **Use** - what a particular building is. An office or a home. Follows
//!   from the zone, but not always: there are apartments downtown.
//! - **Archetype** - how it is built. Curtain wall, brick walk-up, setback
//!   tower. Follows from the use and the height.
//!
//! Keeping them apart is what stops the generator collapsing into "tall
//! things are blue glass and short things are brown brick". A twelve-storey
//! residential slab and a twelve-storey office slab are different buildings
//! and should look it.
//!
//! # The rings
//!
//! The world is laid out as rings of blocks measured from the middle:
//!
//! ```text
//!        ring 0-3          the downtown core - towers, wide arterials
//!        ring 4-7          the rest of the city - mixed, falling away
//!        ring 8-10         suburb - single-storey houses on wide plots
//!        ring 11           farmland - fields, with the odd farmhouse
//!        ring 12           the last houses, and no road leads out of them
//!        ring 13+          past the end of the world
//! ```
//!
//! measured in *blocks* rather than cells, because that is the unit a city
//! is actually laid out in and it survives the block pitch being changed.
//!
//! The five rings outside the city are there to answer a question the map
//! edge asks and nothing used to answer: *why can I not drive that way?*  A
//! city that stops at a kerb with nothing beyond it reads as a diorama on a
//! table.  A city that thins into suburbs, then into fields, then into a
//! last road with houses on one side of it reads as a place that continues
//! over the horizon - and by the time you have driven far enough to find
//! the edge, the answer to "why not further" is "there is nothing out
//! there", which is the true answer.  It also gives the skyline somewhere
//! to be seen *from*: with the draw distance a block longer, the towers are
//! visible from the fields, so the way back to the middle is something you
//! can see rather than something you have to remember.

use crate::rng::hash3;

/// The downtown core is this many blocks across.
pub const CORE_BLOCKS: i32 = 8;

/// The built city is this many blocks across.
pub const CITY_BLOCKS: i32 = 16;

/// Rings of suburb outside the built city.
///
/// Three, which is the width the streets of houses were asked for.
pub const SUBURB_RINGS: i32 = 3;

/// Rings of farmland outside the suburb.
pub const FARM_RINGS: i32 = 1;

/// Rings of houses outside the farmland, which no road leads out of.
pub const OUTSKIRT_RINGS: i32 = 1;

/// Everything outside the city, in rings.
pub const OUTER_RINGS: i32 = SUBURB_RINGS + FARM_RINGS + OUTSKIRT_RINGS;

/// The whole inhabited world, in blocks across.
pub const WORLD_BLOCKS: i32 = CITY_BLOCKS + 2 * OUTER_RINGS;

/// The ring at which the city stops and the suburb starts.
pub const CITY_EDGE: i32 = CITY_BLOCKS / 2;
/// The ring at which the suburb stops and the fields start.
pub const SUBURB_EDGE: i32 = CITY_EDGE + SUBURB_RINGS;
/// The ring at which the fields stop and the last houses start.
pub const FARM_EDGE: i32 = SUBURB_EDGE + FARM_RINGS;
/// The ring past which there is nothing at all.
pub const WORLD_EDGE: i32 = FARM_EDGE + OUTSKIRT_RINGS;

/// Nominal distance from one road to the next, in cells.
///
/// The plan does not actually use a fixed pitch - roads are laid at varying
/// spacing - but the map has to be *sized* for a whole number of blocks, and
/// the rings have to be measured in something. This is that nominal figure:
/// an eight-cell block, a cell of pavement each side, and a road.
pub const BLOCK_PITCH: usize = 13;

/// The smallest a block may be, in buildable cells on a side.
pub const MIN_BLOCK: usize = 8;

/// What a piece of ground is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Zone {
    /// The core. Office towers, the widest streets, the tallest things.
    Downtown = 0,
    /// Offices and shops, mid-rise.
    Commercial = 1,
    /// Homes. Lower, denser, and lit differently.
    Residential = 2,
    /// A civic block: broad, low, and set back behind open ground.
    Civic = 3,
    /// Open green.
    Park = 4,
    /// Past the built edge.
    Fringe = 5,
    /// Houses, one storey, on plots wide enough to see between.
    Suburb = 6,
    /// Fields, with the odd farmhouse standing in one.
    Farm = 7,
    /// Sand and water: the south edge of the world.
    Shore = 8,
}

impl Zone {
    /// Decode from the byte the map stores.
    pub fn from_u8(v: u8) -> Zone {
        match v {
            0 => Zone::Downtown,
            1 => Zone::Commercial,
            2 => Zone::Residential,
            3 => Zone::Civic,
            4 => Zone::Park,
            6 => Zone::Suburb,
            7 => Zone::Farm,
            8 => Zone::Shore,
            _ => Zone::Fringe,
        }
    }

    /// Whether anything gets built here at all.
    ///
    /// Farmland is not built by the block filler even though it has houses
    /// on it: a farm is a field with a house standing in the corner of it,
    /// which is a different operation from subdividing a block into lots.
    /// See `world::fill_block`.
    pub fn is_built(self) -> bool {
        !matches!(self, Zone::Park | Zone::Fringe | Zone::Farm | Zone::Shore)
    }

    /// Whether this ground is open green rather than paved.
    pub fn is_green(self) -> bool {
        matches!(self, Zone::Park | Zone::Farm)
    }

    /// The tallest building this zone will carry, in cells, before the
    /// ring falloff and the lot's own footprint are taken into account.
    pub fn ceiling(self) -> u32 {
        match self {
            Zone::Downtown => 84,
            Zone::Commercial => 40,
            Zone::Residential => 22,
            Zone::Civic => 10,
            // A cell is six metres, so a house is one of them and the
            // clamp in `world::raise` is what actually holds it there.
            Zone::Suburb => 3,
            Zone::Farm => 3,
            Zone::Park | Zone::Fringe | Zone::Shore => 0,
        }
    }
}

/// What a building is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Use {
    /// Offices, shops, a lobby at the bottom and a mechanical floor at the
    /// top. Big glass, cold light, regular floors, lit late and unevenly.
    Commercial = 0,
    /// Homes. Smaller punched windows, balconies, warm light, and a lit
    /// pattern that is scattered rather than banded - people are in or they
    /// are not, floor by floor and flat by flat.
    Residential = 1,
    /// A library, a courthouse, a station. Broad, low, colonnaded.
    Civic = 2,
}

impl Use {
    /// Decode from a stored byte.
    pub fn from_u8(v: u8) -> Use {
        match v {
            1 => Use::Residential,
            2 => Use::Civic,
            _ => Use::Commercial,
        }
    }
}

/// The zoning map.
#[derive(Clone)]
pub struct ZoneMap {
    size: usize,
    zones: Vec<u8>,
}

impl ZoneMap {
    /// Zone a whole map from a seed.
    pub fn generate(size: usize, seed: u32) -> ZoneMap {
        let mut zones = vec![Zone::Fringe as u8; size * size];
        for y in 0..size {
            for x in 0..size {
                zones[y * size + x] = classify(x, y, size, seed) as u8;
            }
        }
        ZoneMap { size, zones }
    }

    /// The zone at a cell.  Off the map is fringe.
    #[inline(always)]
    pub fn at(&self, x: i32, y: i32) -> Zone {
        if x < 0 || y < 0 || x as usize >= self.size || y as usize >= self.size {
            return Zone::Fringe;
        }
        Zone::from_u8(self.zones[y as usize * self.size + x as usize])
    }

    /// How far out a cell is, in blocks from the middle.
    #[inline(always)]
    pub fn ring(&self, x: i32, y: i32) -> i32 {
        ring_of(x, y, self.size)
    }

    /// How tall this ground can carry, 0 to 255, after the ring falloff.
    ///
    /// Full strength inside the core, then down to a fifth at the built
    /// edge. This is the "decreasing height" the layout is built around:
    /// the towers are in the middle, and what continues outwards is small.
    pub fn intensity(&self, x: i32, y: i32) -> u32 {
        let r = self.ring(x, y);
        let core = CORE_BLOCKS / 2;
        if r <= core {
            255
        } else if r >= CITY_EDGE {
            // Everything past the built edge is houses, and a house is a
            // house whether it is the first one out of town or the last one
            // before the fields.  The falloff has already done its work by
            // here; carrying it on would only make the suburb shrink into
            // the ground.
            50
        } else {
            let t = (r - core) as u32;
            let span = (CITY_EDGE - core) as u32;
            255 - (205 * t / span.max(1))
        }
    }
}

/// How far out a cell is, in blocks from the middle of the map.
///
/// Measured between *block* coordinates, not cell ones.  Dividing a cell
/// distance by the block pitch looks equivalent and is not: the middle of
/// the map is rarely on a block boundary, so the ring steps somewhere in
/// the middle of a block and the zone changes half way along a street.
fn ring_of(x: i32, y: i32, size: usize) -> i32 {
    let pitch = BLOCK_PITCH as i32;
    let centre_block = (size / BLOCK_PITCH) as i32 / 2;
    let (bx, by) = (x.div_euclid(pitch), y.div_euclid(pitch));
    (bx - centre_block).abs().max((by - centre_block).abs())
}

/// What one cell is zoned as.
fn classify(x: usize, y: usize, size: usize, seed: u32) -> Zone {
    let r = ring_of(x as i32, y as i32, size);
    let core = CORE_BLOCKS / 2;
    let edge = CITY_EDGE;

    // The sea, along the south edge and only there.
    //
    // One coast rather than four: an island reads as a model of a city and
    // a coastline reads as a city that has a sea on one side of it, which
    // is what most of them have.  It is always the south so that "drive
    // towards the water" is a direction you can learn.
    if is_shore(y as i32, size) {
        return Zone::Shore;
    }

    if r >= WORLD_EDGE {
        return Zone::Fringe;
    }
    if r >= edge {
        // Outside the city.  Blocks are zoned by ring rather than by dice:
        // the point of the outer rings is that they are legible from a long
        // way off, and a scatter of farmland through the suburbs would read
        // as gaps rather than as countryside.
        let (bx, by) = (x / BLOCK_PITCH, y / BLOCK_PITCH);
        let h = hash3(bx as u32, by as u32, seed ^ 0x_0FAA_3300);
        return if r >= FARM_EDGE {
            Zone::Suburb
        } else if r >= SUBURB_EDGE {
            Zone::Farm
        } else if h % 100 < 12 {
            // A green somewhere in the suburb, which is what a suburb has.
            Zone::Park
        } else {
            Zone::Suburb
        };
    }

    // Zoning is decided per *block*, not per cell - a zone that changes
    // half way along a street is not a zone.
    let (bx, by) = (x / BLOCK_PITCH, y / BLOCK_PITCH);
    let h = hash3(bx as u32, by as u32, seed ^ 0x_2013_0000);

    // A park somewhere in most rings, and more of them further out.
    let park_odds = 4 + (r as u32) * 3;
    if h % 100 < park_odds {
        return Zone::Park;
    }
    if (h >> 7) % 100 < 6 {
        return Zone::Civic;
    }

    if r <= core {
        // Downtown, with pockets of housing in it - which is what makes a
        // core look inhabited rather than like an office park after six.
        if (h >> 13) % 100 < 18 {
            Zone::Residential
        } else {
            Zone::Downtown
        }
    } else {
        // Outside the core the mix tilts steadily towards housing.
        let commercial = 70u32.saturating_sub((r - core) as u32 * 14);
        if (h >> 13) % 100 < commercial {
            Zone::Commercial
        } else {
            Zone::Residential
        }
    }
}

/// Whether this row is sea or the sand in front of it.
///
/// The shore is the last block-row of the map plus the margin beyond it, so
/// it occupies the ground the fringe used to waste.  Measured in cells from
/// the south edge rather than in rings, because a coastline is a line and
/// not a square.
pub fn is_shore(y: i32, size: usize) -> bool {
    y >= size as i32 - SHORE_CELLS
}

/// Whether this row is deep enough to be water rather than sand.
pub fn is_water(y: i32, size: usize) -> bool {
    y >= size as i32 - SHORE_CELLS + BEACH_CELLS
}

/// How much of the south edge is coast, in cells.
pub const SHORE_CELLS: i32 = BLOCK_PITCH as i32 * 2;
/// How much of that coast is dry sand.
pub const BEACH_CELLS: i32 = 5;

/// What a building in this zone is used for.
pub fn use_for(zone: Zone, rng: &mut crate::rng::Rng) -> Use {
    match zone {
        Zone::Downtown => {
            if rng.chance(1, 8) {
                Use::Residential
            } else {
                Use::Commercial
            }
        }
        Zone::Commercial => {
            if rng.chance(1, 4) {
                Use::Residential
            } else {
                Use::Commercial
            }
        }
        Zone::Residential => {
            if rng.chance(1, 6) {
                Use::Commercial
            } else {
                Use::Residential
            }
        }
        Zone::Civic => Use::Civic,
        Zone::Suburb | Zone::Farm => Use::Residential,
        Zone::Park | Zone::Fringe | Zone::Shore => Use::Residential,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: usize = BLOCK_PITCH * (WORLD_BLOCKS as usize + 2);

    fn map() -> ZoneMap {
        ZoneMap::generate(N, 2024)
    }

    #[test]
    fn the_middle_of_the_map_is_downtown() {
        let z = map();
        let c = N as i32 / 2;
        let core: Vec<Zone> = (-6..=6)
            .flat_map(|dy| (-6..=6).map(move |dx| (dx, dy)))
            .map(|(dx, dy)| z.at(c + dx, c + dy))
            .collect();
        assert!(
            core.iter().filter(|k| **k == Zone::Downtown).count() > core.len() / 2,
            "the middle of the map is {core:?}"
        );
    }

    #[test]
    fn the_edge_of_the_map_is_fringe() {
        let z = map();
        assert_eq!(z.at(1, 1), Zone::Fringe);
        // ...except in the south, where it is the sea.
        assert_eq!(z.at(N as i32 - 2, N as i32 - 2), Zone::Shore);
        assert_eq!(z.at(N as i32 - 2, 1), Zone::Fringe);
    }

    /// The rings outside the city are in the order they were asked for:
    /// suburb, then fields, then one last row of houses.
    #[test]
    fn the_world_thins_out_in_bands_beyond_the_city() {
        let z = map();
        let c = N as i32 / 2;
        // Walk north from the middle, which never meets the shore, and
        // record the first zone seen in each ring.
        let mut seen: Vec<(i32, Zone)> = Vec::new();
        for y in (0..c).rev() {
            seen.push((z.ring(c, y), z.at(c, y)));
        }
        for (r, zone) in seen {
            let want_built_out = match r {
                r if r < CITY_EDGE => continue, // the city itself is a mix
                r if r < SUBURB_EDGE => zone == Zone::Suburb || zone == Zone::Park,
                r if r < FARM_EDGE => zone == Zone::Farm,
                r if r < WORLD_EDGE => zone == Zone::Suburb,
                _ => zone == Zone::Fringe,
            };
            assert!(want_built_out, "ring {r} is {zone:?}");
        }
    }

    #[test]
    fn intensity_falls_off_from_the_core_and_never_rises() {
        let z = map();
        let c = N as i32 / 2;
        let mut last = z.intensity(c, c);
        assert_eq!(last, 255);
        for d in 0..(N as i32 / 2) {
            let v = z.intensity(c + d, c);
            assert!(v <= last, "intensity rose from {last} to {v} at {d} cells out");
            last = v;
        }
        assert!(last <= 50, "the edge is still at intensity {last}");
    }

    #[test]
    fn the_core_is_the_size_it_says_it_is() {
        let z = map();
        let c = N as i32 / 2;
        // Full intensity through the core blocks and no further.
        let core_cells = (CORE_BLOCKS / 2) * BLOCK_PITCH as i32;
        assert_eq!(z.intensity(c, c), 255);
        assert_eq!(z.intensity(c + core_cells - BLOCK_PITCH as i32, c), 255);
        assert!(z.intensity(c + core_cells + BLOCK_PITCH as i32, c) < 255);
    }

    #[test]
    fn the_ring_is_constant_across_a_block() {
        let z = map();
        for b in 0..(N / BLOCK_PITCH) {
            let x0 = b * BLOCK_PITCH;
            let first = z.ring(x0 as i32, (N / 2) as i32);
            for d in 0..BLOCK_PITCH {
                assert_eq!(
                    z.ring((x0 + d) as i32, (N / 2) as i32),
                    first,
                    "the ring steps inside block {b}"
                );
            }
        }
    }

    #[test]
    fn housing_appears_and_so_do_offices() {
        let z = map();
        let mut counts = std::collections::HashMap::new();
        for y in 0..N as i32 {
            for x in 0..N as i32 {
                *counts.entry(z.at(x, y)).or_insert(0u32) += 1;
            }
        }
        for k in [Zone::Downtown, Zone::Commercial, Zone::Residential, Zone::Park] {
            assert!(counts.get(&k).copied().unwrap_or(0) > 200, "only {:?} of {k:?}", counts.get(&k));
        }
    }

    #[test]
    fn a_zone_does_not_change_half_way_along_a_block() {
        let z = map();
        // Every cell of a block interior has to agree.
        for by in 1..(CITY_BLOCKS - 1) {
            for bx in 1..(CITY_BLOCKS - 1) {
                let (x0, y0) = (bx as usize * BLOCK_PITCH, by as usize * BLOCK_PITCH);
                let first = z.at(x0 as i32, y0 as i32);
                for dy in 0..BLOCK_PITCH {
                    for dx in 0..BLOCK_PITCH {
                        assert_eq!(
                            z.at((x0 + dx) as i32, (y0 + dy) as i32),
                            first,
                            "block {bx},{by} changes zone at {dx},{dy}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn zones_carry_sensible_ceilings() {
        assert!(Zone::Downtown.ceiling() > Zone::Commercial.ceiling());
        assert!(Zone::Commercial.ceiling() > Zone::Residential.ceiling());
        assert_eq!(Zone::Park.ceiling(), 0);
        assert!(!Zone::Park.is_built());
        assert!(!Zone::Fringe.is_built());
        assert!(Zone::Downtown.is_built());
    }

    #[test]
    fn use_follows_zone_but_not_slavishly() {
        let mut rng = crate::rng::Rng::new(5);
        let downtown: Vec<Use> = (0..200).map(|_| use_for(Zone::Downtown, &mut rng)).collect();
        let resi: Vec<Use> = (0..200).map(|_| use_for(Zone::Residential, &mut rng)).collect();
        assert!(downtown.iter().filter(|u| **u == Use::Commercial).count() > 140);
        assert!(downtown.contains(&Use::Residential), "no flats downtown at all");
        assert!(resi.iter().filter(|u| **u == Use::Residential).count() > 140);
        assert!(resi.contains(&Use::Commercial), "no shops in the suburbs");
    }
}

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

use crate::elevation::Elevation;
use crate::rng::{hash3, Rng};
use crate::shadow::{self, ShadowMap};
use crate::walk::WalkMap;
use crate::zone::{self, Use, Zone, ZoneMap, BLOCK_PITCH, CITY_BLOCKS, MIN_BLOCK};

/// The city is this many cells on a side.
///
/// Derived rather than chosen: sixteen blocks of built city, plus a block of
/// outskirts on each side so that the built area has an edge you can walk to
/// rather than one that coincides with the end of the world.  The lot record
/// stores its footprint corners as `u8`, so this must stay under 256, which
/// eighteen blocks of thirteen cells comfortably does.
pub const SIZE: usize = BLOCK_PITCH * (CITY_BLOCKS as usize + 2);

/// A cell is about this many metres across.  Not used in any calculation -
/// the renderer works in cells throughout - but every dimension in this file
/// was chosen against it, so it is written down.
pub const METRES_PER_CELL: u32 = 6;

/// What occupies a cell at ground level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    /// Carriageway.  Cars go here.
    Road = 0,
    /// Pavement, kerbed, with lamps and trees on it.
    Sidewalk = 1,
    /// Built on.  `height` is above zero and `lot` names the building.
    Building = 2,
    /// Open ground inside a block - a park.
    Park = 3,
    /// Paved open ground - a plaza.
    Plaza = 4,
}

/// One cell of the height field.
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    /// What is here.
    pub kind: Kind,
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
    /// What it is for.  Orthogonal to `arch`: a twelve-storey residential
    /// slab and a twelve-storey office slab are built the same way and are
    /// different buildings.
    pub use_: Use,
    /// Hue of the facade, as a [`crate::palette`] hue index.
    pub hue: u8,
    /// Base luminance of the facade, 3 to 7.
    ///
    /// Hue alone is not enough variety.  A street of buildings that differ
    /// only in colour reads as a colour chart; what tells two real buildings
    /// apart at a glance is as often how *bright* one is - dark stone next
    /// to pale glass next to a blazing tower.
    pub luma: u8,
    /// How many of this building's windows are lit, 0 to 15.
    ///
    /// Per building rather than per archetype, so one tower can be working
    /// late and its neighbour empty.
    pub lit: u8,
    /// Everything else about it - which windows fall where, where the fire
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
    /// The street system.  The renderer reads this directly for markings, so
    /// the paint on the road and the shape of the road come from one place.
    pub plan: Plan,
    /// What each piece of ground is for.
    pub zones: ZoneMap,
    /// How high the ground is, and how high what stands on it is.
    pub elev: Elevation,
    /// Where a person on foot may be.
    pub walk: WalkMap,
    /// The height of the shadow line at each cell, for the current light.
    ///
    /// Cast once, when the light moves - see [`City::relight`].  It is one
    /// sweep of the whole grid, which is nothing once and far too much per
    /// frame.
    pub shadow: ShadowMap,
    /// Which sides of each cell face a road, and which face a building.
    ///
    /// Eight bits a cell: the low four are road to the west, east, north and
    /// south, the high four are building in the same order.  See
    /// [`City::edges`].
    edges: Vec<u8>,
    /// The seed it was generated from.
    pub seed: u32,
}

/// Bit positions in [`City::edges`].
pub const EDGE_ROAD: u8 = 0;
/// Bit positions in [`City::edges`], for a building rather than a road.
pub const EDGE_BUILT: u8 = 4;
/// The four neighbours, in the order the edge bits are packed.
pub const EDGE_STEPS: [(i32, i32); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];

// --- the street plan -------------------------------------------------------
//
// The streets used to be arithmetic: an avenue wherever `x % 14 < 3`, a
// cross street wherever `y % 9 < 2`.  That is one line of code and it gives
// a city where every block is the same size, every road is the same width,
// and every junction is the same junction.  It reads as a diagram.
//
// So the plan is *generated* instead, once per city, as two independent
// axes: a list of roads with varying class, width and spacing.  The grid is
// still a grid - this is Manhattan, not Boston - but no two blocks are the
// same and the roads have a hierarchy, which is what a street system
// actually is.

/// What kind of road a cell belongs to.
///
/// Ordered by size, so `class >= RoadClass::Street` is a meaningful test -
/// it is the line below which a road gets no markings, no sidewalk and no
/// crosswalks, because it is not a street, it is a gap between buildings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum RoadClass {
    /// Not a road at all.
    None = 0,
    /// One cell.  Service access between buildings; no pavement, no paint.
    Alley = 1,
    /// Two cells - one lane each way.
    Street = 2,
    /// Three cells.
    Avenue = 3,
    /// Four or five cells.
    Boulevard = 4,
    /// Twelve to sixteen cells - the better part of a hundred metres of
    /// carriageway, kerb to kerb.  There are one or two of these in a city
    /// and they run its whole length; everything about the core is arranged
    /// around them.
    Arterial = 5,
}

impl RoadClass {
    fn from_u8(v: u8) -> RoadClass {
        match v {
            1 => RoadClass::Alley,
            2 => RoadClass::Street,
            3 => RoadClass::Avenue,
            4 => RoadClass::Boulevard,
            5 => RoadClass::Arterial,
            _ => RoadClass::None,
        }
    }

    /// Whether this road is paved for traffic rather than being a gap.
    pub fn is_street(self) -> bool {
        self >= RoadClass::Street
    }
}

/// One coordinate's worth of the plan, along one axis.
#[derive(Clone, Copy, Debug)]
pub struct RoadCell {
    /// What kind of road.
    pub class: RoadClass,
    /// How many cells in from the near kerb this one is.
    pub across: u8,
    /// How wide the whole carriageway is.
    pub width: u8,
}

impl RoadCell {
    /// Not a road.
    pub const NONE: RoadCell = RoadCell { class: RoadClass::None, across: 0, width: 0 };

    /// Whether this is a road at all.
    pub fn is_road(&self) -> bool {
        self.class != RoadClass::None
    }
}

/// The roads along one axis of the map.
///
/// Three parallel byte arrays rather than an array of structs, because every
/// lookup on the render path wants exactly one of the three.
#[derive(Clone)]
pub struct Axis {
    class: Vec<u8>,
    across: Vec<u8>,
    width: Vec<u8>,
}

impl Axis {
    /// What is at this coordinate.  Off the map is not a road.
    #[inline(always)]
    pub fn at(&self, i: i32) -> RoadCell {
        if i < 0 || i as usize >= self.class.len() {
            return RoadCell::NONE;
        }
        let i = i as usize;
        RoadCell {
            class: RoadClass::from_u8(self.class[i]),
            across: self.across[i],
            width: self.width[i],
        }
    }

    /// Whether this coordinate is on a road of any kind.
    #[inline(always)]
    pub fn is_road(&self, i: i32) -> bool {
        i >= 0 && (i as usize) < self.class.len() && self.class[i as usize] != 0
    }

    /// Whether this coordinate is on a road that has pavement beside it.
    #[inline(always)]
    pub fn is_street(&self, i: i32) -> bool {
        self.at(i).class.is_street()
    }

    /// Lay out one axis.
    ///
    /// `long` is the axis the big roads run down.  The two axes are given
    /// different characters on purpose: long sightlines one way and short
    /// ones the other is the single most Manhattan thing about this city,
    /// and it means turning ninety degrees changes what you are looking at
    /// rather than just rotating it.
    fn generate(rng: &mut Rng, long: bool) -> Axis {
        let mut class = vec![0u8; SIZE];
        let mut across = vec![0u8; SIZE];
        let mut width = vec![0u8; SIZE];

        // The one arterial on this axis, placed before anything else so it
        // lands where it belongs - through the middle of the city, not
        // wherever the sequence of gaps happened to arrive.
        // Signed arithmetic, then cast.  Casting the jitter to `usize`
        // first turns every leftward offset into a number near 2^64 and the
        // addition overflows, which release builds wrap back to the right
        // answer and debug builds - every test run - panic on.
        let jitter = rng.range(-(BLOCK_PITCH as i32), BLOCK_PITCH as i32);
        let arterial = (SIZE as i32 / 2 + jitter).max(0) as usize;

        // Start part way in, so the edge of the map is never a kerb.
        let mut i = rng.range(3, 11) as usize;
        let mut placed_arterial = false;
        while i < SIZE {
            // An arterial takes precedence over whatever would have gone
            // here, once, as soon as the walk reaches the middle.
            let c = if !placed_arterial && i + 16 >= arterial {
                placed_arterial = true;
                RoadClass::Arterial
            } else {
                pick_class(rng, long)
            };
            let w = road_width(rng, c);
            if i + w >= SIZE {
                break;
            }
            for k in 0..w {
                class[i + k] = c as u8;
                across[i + k] = k as u8;
                width[i + k] = w as u8;
            }
            i += w + gap_after(rng, c, long);
        }
        Axis { class, across, width }
    }
}

/// Which kind of road comes next.
///
/// The weights are the whole character of the city.  On the long axis it is
/// mostly avenues with the occasional boulevard; on the short axis mostly
/// streets.  Alleys turn up on both, and they are what stop a block from
/// always being a single undivided slab.
fn pick_class(rng: &mut Rng, long: bool) -> RoadClass {
    let r = rng.below(100);
    if long {
        match r {
            0..=14 => RoadClass::Boulevard,
            15..=64 => RoadClass::Avenue,
            65..=86 => RoadClass::Street,
            _ => RoadClass::Alley,
        }
    } else {
        match r {
            0..=5 => RoadClass::Boulevard,
            6..=25 => RoadClass::Avenue,
            26..=87 => RoadClass::Street,
            _ => RoadClass::Alley,
        }
    }
}

/// How wide a road of this class is, in cells.
fn road_width(rng: &mut Rng, c: RoadClass) -> usize {
    match c {
        RoadClass::None => 0,
        RoadClass::Alley => 1,
        RoadClass::Street => 2,
        RoadClass::Avenue => 3,
        RoadClass::Boulevard => 4 + rng.below(2) as usize,
        RoadClass::Arterial => 12 + rng.below(5) as usize,
    }
}

/// How much buildable ground follows a road of this class.
///
/// Bigger roads get bigger blocks after them, which is what makes the
/// hierarchy visible from the ground: you can tell you have come out onto a
/// boulevard because you can see a long way in both directions and the next
/// crossing is a long way off.  An alley gets a short gap, so it reads as a
/// service road splitting one block rather than as a thin street.
fn gap_after(rng: &mut Rng, c: RoadClass, long: bool) -> usize {
    let (lo, hi) = match (c, long) {
        (RoadClass::Alley, _) => (4, 8),
        (RoadClass::Street, true) => (10, 16),
        (RoadClass::Street, false) => (7, 12),
        (RoadClass::Avenue, true) => (12, 20),
        (RoadClass::Avenue, false) => (8, 14),
        (RoadClass::Boulevard, true) => (16, 26),
        (RoadClass::Boulevard, false) => (10, 17),
        (RoadClass::Arterial, true) => (18, 28),
        (RoadClass::Arterial, false) => (14, 22),
        (RoadClass::None, _) => (10, 14),
    };
    // Clamped, not merely chosen, so the guarantee holds however the figures
    // above are retuned: a block is at least `MIN_BLOCK` cells of buildable
    // ground plus a cell of pavement on each side.  Below that there is no
    // room for a building with a front and a back, and a subdivision that
    // tried would produce lots one cell deep.
    (rng.range(lo, hi) as usize).max(MIN_BLOCK + 2)
}

/// Which road a crossing crosses.
///
/// The stripes run *with* the traffic on the road being crossed and repeat
/// *across* it, which is the way round they are painted: someone walking
/// east over a north-south avenue crosses a ladder of north-south bars.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Crossing {
    /// Crossing a north-south road.  Stripes repeat along x.
    OverCols,
    /// Crossing an east-west road.  Stripes repeat along y.
    OverRows,
}

/// The street system: the roads on both axes.
#[derive(Clone)]
pub struct Plan {
    /// Roads running north-south, indexed by column.
    pub cols: Axis,
    /// Roads running east-west, indexed by row.
    pub rows: Axis,
}

impl Plan {
    /// Generate a street system.
    pub fn generate(rng: &mut Rng) -> Plan {
        Plan { cols: Axis::generate(rng, true), rows: Axis::generate(rng, false) }
    }

    /// Whether a cell is carriageway.
    #[inline(always)]
    pub fn is_road(&self, x: i32, y: i32) -> bool {
        self.cols.is_road(x) || self.rows.is_road(y)
    }

    /// Whether a cell is a junction of two proper streets.
    #[inline(always)]
    pub fn is_junction(&self, x: i32, y: i32) -> bool {
        self.cols.is_street(x) && self.rows.is_street(y)
    }

    /// If this cell is a pedestrian crossing, which road it crosses.
    ///
    /// Crossings sit **outside** the junction box, one cell back along the
    /// road, which is where a stop line goes and where they are painted in
    /// life.  Putting them inside the box instead - which is the obvious
    /// thing, and was wrong here for a while - produces a crossing that no
    /// pavement touches: every orthogonal neighbour of a junction cell is
    /// either more junction or plain carriageway, so the pavements end up as
    /// isolated rings around each block with no way between them.
    #[inline(always)]
    pub fn crossing_at(&self, x: i32, y: i32) -> Option<Crossing> {
        if self.cols.is_street(x)
            && !self.rows.is_road(y)
            && (self.rows.is_street(y - 1) || self.rows.is_street(y + 1))
        {
            return Some(Crossing::OverCols);
        }
        if self.rows.is_street(y)
            && !self.cols.is_road(x)
            && (self.cols.is_street(x - 1) || self.cols.is_street(x + 1))
        {
            return Some(Crossing::OverRows);
        }
        None
    }

    /// Whether a cell has pavement on it: not carriageway, but beside one.
    ///
    /// Only proper streets get a pavement.  An alley has buildings coming
    /// straight down to it, which is what an alley is.
    #[inline(always)]
    pub fn is_sidewalk(&self, x: i32, y: i32) -> bool {
        if self.is_road(x, y) {
            return false;
        }
        self.cols.is_street(x - 1)
            || self.cols.is_street(x + 1)
            || self.rows.is_street(y - 1)
            || self.rows.is_street(y + 1)
    }

    /// Whether a cell can be built on.
    #[inline(always)]
    pub fn is_buildable(&self, x: i32, y: i32) -> bool {
        x >= 0
            && y >= 0
            && (x as usize) < SIZE
            && (y as usize) < SIZE
            && !self.is_road(x, y)
            && !self.is_sidewalk(x, y)
    }
}

impl City {
    /// Read a cell, clamped - rays that leave the map see empty ground
    /// rather than an index panic.
    #[inline(always)]
    pub fn at(&self, x: i32, y: i32) -> Cell {
        if x < 0 || y < 0 || x >= SIZE as i32 || y >= SIZE as i32 {
            return Cell { kind: Kind::Road, lot: NO_LOT, seed: 0 };
        }
        self.cells[y as usize * SIZE + x as usize]
    }

    /// Height of the building at a cell, zero outside the map.
    #[inline(always)]
    pub fn height(&self, x: i32, y: i32) -> u8 {
        self.elev.building(x, y)
    }

    /// Ground level at a cell, in cells.
    #[inline(always)]
    pub fn ground(&self, x: i32, y: i32) -> crate::fixed::Fx {
        self.elev.ground(x, y)
    }

    /// Recast the shadows for a new light direction.
    ///
    /// One pass over the whole grid.  Call it when the light moves; do not
    /// call it per frame.
    pub fn relight(&mut self, az: crate::trig::Ang, slope: crate::fixed::Fx) {
        self.shadow = ShadowMap::cast(&self.elev, az, slope);
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

    /// Whether a cell is unbuilt: nothing stands here.
    ///
    /// Named for what it measures rather than for who might use it.  This
    /// was `walkable` for most of the project's life, which was wrong in a
    /// way that cost real time: it is the test a *vehicle* wants, and a
    /// pedestrian wants [`crate::walk::WalkMap`] instead.  Reading the name
    /// as a statement about people is what put pedestrians in the middle of
    /// the avenue and sent the camera wandering through parks.
    ///
    /// The predicate is unchanged; only the name now says what it is.
    #[inline(always)]
    pub fn open(&self, x: i32, y: i32) -> bool {
        self.elev.open(x, y)
    }

    /// Which of a cell's four sides face a road, and which face a building.
    ///
    /// The pavement is drawn in bands - kerb, planted verge, paving - and
    /// which band a point falls in depends on how far it is from the nearest
    /// road edge and from the nearest building edge.  Working that out from
    /// [`City::at`] costs up to eight grid lookups *per ground character*,
    /// and the ground is most of the screen: it took the host frame from
    /// 0.16 ms to 0.23.
    ///
    /// It is also a property of the map, which does not change.  So it is
    /// computed once, when the city is generated, and the renderer reads one
    /// byte and tests bits.
    #[inline(always)]
    pub fn edges(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x >= SIZE as i32 || y >= SIZE as i32 {
            0
        } else {
            self.edges[y as usize * SIZE + x as usize]
        }
    }

    /// Whether a vehicle may be on this cell.
    ///
    /// The carriageway and nothing else.  A car is not blocked by a park
    /// bench, it is blocked by the park: `open` is the wrong test for
    /// anything that is supposed to stay on the road, and using it is how
    /// you get a taxi driving across a plaza to save four cells.
    #[inline(always)]
    pub fn drivable(&self, x: i32, y: i32) -> bool {
        x >= 0
            && y >= 0
            && x < SIZE as i32
            && y < SIZE as i32
            && self.at(x, y).kind == Kind::Road
            && self.open(x, y)
    }

    /// The nearest carriageway cell to a point, or `None` within `limit`.
    ///
    /// Used to put both ends of a route on the road before searching:
    /// a fare marker is placed on the road, but a taxi that has been driven
    /// into a plaza is not, and a search that starts off the network finds
    /// nothing and reports the city disconnected.
    pub fn nearest_road(&self, x: i32, y: i32, limit: i32) -> Option<(i32, i32)> {
        if self.drivable(x, y) {
            return Some((x, y));
        }
        for r in 1..=limit {
            for dy in -r..=r {
                for dx in -r..=r {
                    // Only the ring, not the filled square: the interior was
                    // covered by a smaller `r`.
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    if self.drivable(x + dx, y + dy) {
                        return Some((x + dx, y + dy));
                    }
                }
            }
        }
        None
    }

    /// A driving route between two cells, as a list of cells.
    ///
    /// Breadth-first over the carriageway, so the route is the shortest one
    /// in cells.  It is not the shortest in *time* - it does not know that
    /// an arterial is faster than an alley - and it is not run per frame:
    /// a cabbie plans once per fare and then follows what it planned.
    ///
    /// The walking network has [`crate::walk::WalkMap::route`], which is the
    /// same search over a different set of cells.  They are deliberately not
    /// shared code: the two networks disagree about almost every cell in the
    /// city, and a single `route` taking a predicate would invite exactly
    /// the confusion that [`City::open`] already cost once.
    pub fn drive_route(
        &self,
        from: (i32, i32),
        to: (i32, i32),
        budget: usize,
    ) -> Option<Vec<(i32, i32)>> {
        let from = self.nearest_road(from.0, from.1, 8)?;
        let to = self.nearest_road(to.0, to.1, 8)?;
        if from == to {
            return Some(vec![from]);
        }
        let idx = |x: i32, y: i32| y as usize * SIZE + x as usize;
        // `came` doubles as the visited set: usize::MAX means unseen.
        let mut came = vec![usize::MAX; SIZE * SIZE];
        let mut queue = std::collections::VecDeque::new();
        came[idx(from.0, from.1)] = idx(from.0, from.1);
        queue.push_back(from);
        let mut seen = 0usize;

        while let Some((x, y)) = queue.pop_front() {
            seen += 1;
            if seen > budget {
                return None;
            }
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (x + dx, y + dy);
                if !self.drivable(nx, ny) {
                    continue;
                }
                let ni = idx(nx, ny);
                if came[ni] != usize::MAX {
                    continue;
                }
                came[ni] = idx(x, y);
                if (nx, ny) == to {
                    let mut path = vec![to];
                    let mut cur = ni;
                    while came[cur] != cur {
                        cur = came[cur];
                        path.push(((cur % SIZE) as i32, (cur / SIZE) as i32));
                    }
                    path.reverse();
                    return Some(path);
                }
                queue.push_back((nx, ny));
            }
        }
        None
    }

    /// Generate a city from a seed.
    ///
    /// Four layers, in the order they depend on each other: the roads decide
    /// where the blocks are, the zoning decides what those blocks are for,
    /// the elevation carries what gets built, and the walking network is
    /// derived from all three.
    pub fn generate(seed: u32) -> City {
        let mut rng = Rng::new(seed);
        let plan = Plan::generate(&mut rng);
        let zones = ZoneMap::generate(SIZE, seed);
        let mut elev = Elevation::generate(SIZE, seed);
        let mut cells = vec![Cell { kind: Kind::Road, lot: NO_LOT, seed: 0 }; SIZE * SIZE];
        let mut lots: Vec<Lot> = Vec::new();

        // Pass 1: lay the plan onto the grid.
        for y in 0..SIZE as i32 {
            for x in 0..SIZE as i32 {
                let i = y as usize * SIZE + x as usize;
                cells[i].kind = if plan.is_road(x, y) {
                    Kind::Road
                } else if plan.is_sidewalk(x, y) {
                    Kind::Sidewalk
                } else {
                    Kind::Plaza // provisional: block interior, built on below
                };
                cells[i].seed = rng.next_u32() as u8;
            }
        }

        // Pass 2: find each block and fill it.
        //
        // A block is a maximal run of buildable cells in both directions.
        // Scanning for runs - rather than stepping at a fixed period - is
        // what lets the roads be irregular in the first place.
        //
        // Each block is reached exactly once, at its **top row**: a run is
        // only filled when the cell above its left end is not buildable.
        // Without that test a block is refilled once per row it spans, and
        // the symptom is subtle - buildings quietly reroll their height and
        // colour, and plazas turn into towers on the second pass.
        for y in 0..SIZE as i32 {
            let mut x = 0i32;
            while x < SIZE as i32 {
                if !plan.is_buildable(x, y) {
                    x += 1;
                    continue;
                }
                let x0 = x;
                while x + 1 < SIZE as i32 && plan.is_buildable(x + 1, y) {
                    x += 1;
                }
                let x1 = x;
                if !plan.is_buildable(x0, y - 1) {
                    let mut y1 = y;
                    while y1 + 1 < SIZE as i32 && plan.is_buildable(x0, y1 + 1) {
                        y1 += 1;
                    }
                    let mut site = Site {
                        cells: &mut cells,
                        lots: &mut lots,
                        elev: &mut elev,
                        zones: &zones,
                        rng: &mut rng,
                        seed,
                    };
                    fill_block(
                        &mut site,
                        Rect {
                            x0: x0 as usize,
                            y0: y as usize,
                            x1: x1 as usize,
                            y1: y1 as usize,
                        },
                    );
                }
                x = x1 + 1;
            }
        }

        // Pass 2a: put a kerb under the pavement.
        //
        // Done after the buildings, because raising a lot's pad would undo
        // it, and before the shadow sweep, because the sweep reads the
        // finished surface.  One step - 18 cm - which is the smallest the
        // elevation map can express and exactly what a kerb is.
        for y in 0..SIZE as i32 {
            for x in 0..SIZE as i32 {
                if cells[y as usize * SIZE + x as usize].kind == Kind::Sidewalk {
                    elev.raise(x, y, crate::elevation::KERB);
                }
            }
        }

        // Pass 3: derive where a person may walk.
        let walk = WalkMap::build(SIZE, &plan, |x, y| {
            cells[y as usize * SIZE + x as usize].kind
        });

        // Pass 4: sweep the shadows for the default light.
        let shadow = ShadowMap::cast(&elev, shadow::DEFAULT_AZ, shadow::DEFAULT_SLOPE);

        // Which sides of each cell face a road and which face a building.
        // Once, here, rather than eight grid lookups per ground character in
        // the renderer - see [`City::edges`].
        let mut edges = vec![0u8; SIZE * SIZE];
        for y in 0..SIZE as i32 {
            for x in 0..SIZE as i32 {
                let mut bits = 0u8;
                for (i, (dx, dy)) in EDGE_STEPS.iter().enumerate() {
                    let (nx, ny) = (x + dx, y + dy);
                    let kind = if nx < 0 || ny < 0 || nx >= SIZE as i32 || ny >= SIZE as i32 {
                        Kind::Road
                    } else {
                        cells[ny as usize * SIZE + nx as usize].kind
                    };
                    match kind {
                        Kind::Road => bits |= 1 << (EDGE_ROAD + i as u8),
                        Kind::Building => bits |= 1 << (EDGE_BUILT + i as u8),
                        _ => {}
                    }
                }
                edges[y as usize * SIZE + x as usize] = bits;
            }
        }

        City { cells, lots, plan, zones, elev, walk, shadow, edges, seed }
    }
}

/// A rectangle of cells, inclusive at both ends.
///
/// A lot is a rectangle; saying so once is cheaper than passing `x0, y0,
/// x1, y1` at every boundary and never being sure which pair is which.
#[derive(Clone, Copy, Debug)]
struct Rect {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

impl Rect {
    fn w(&self) -> usize {
        self.x1 - self.x0 + 1
    }
    fn h(&self) -> usize {
        self.y1 - self.y0 + 1
    }
    /// How many rings in from the edge a cell sits.  This is the whole of
    /// the setback model.
    fn ring(&self, x: usize, y: usize) -> u32 {
        (x - self.x0).min(self.x1 - x).min(y - self.y0).min(self.y1 - y) as u32
    }
}

/// Side of the noise lattice, in cells.  Sixteen puts a cluster about every
/// hundred metres, which is the scale a district changes on.
const FIELD_CELL: usize = 16;

/// A smooth value noise field, 0 to 255.
///
/// Bilinear interpolation over a lattice of hashed corners. Integer
/// throughout - the whole point of the fixed-point discipline is that the
/// city generator can be transcribed, and a generator that needs floating
/// point cannot be.
fn field(x: usize, y: usize, seed: u32) -> u32 {
    let (gx, gy) = (x / FIELD_CELL, y / FIELD_CELL);
    let (fx, fy) = ((x % FIELD_CELL) as u32, (y % FIELD_CELL) as u32);
    let corner = |ix: usize, iy: usize| hash3(ix as u32, iy as u32, seed ^ 0x_D157_0000) & 255;
    let n = FIELD_CELL as u32;
    let top = corner(gx, gy) * (n - fx) + corner(gx + 1, gy) * fx;
    let bot = corner(gx, gy + 1) * (n - fx) + corner(gx + 1, gy + 1) * fx;
    (top * (n - fy) + bot * fy) / (n * n)
}

/// Fill one block interior: either open it as a park, or subdivide it into
/// Everything the block filler is working with.
///
/// Introduced because the two functions below were taking eight loose
/// arguments between them and it was never clear at a call site which `&mut`
/// was which.  They are the same six things every time, they all live for
/// exactly as long as `City::generate`, and naming them once says so.
struct Site<'a> {
    cells: &'a mut [Cell],
    lots: &'a mut Vec<Lot>,
    elev: &'a mut Elevation,
    zones: &'a ZoneMap,
    rng: &'a mut Rng,
    seed: u32,
}

/// Fill one block interior: either open it as a park, or subdivide it into
/// lots and raise a building on each.
fn fill_block(site: &mut Site, block: Rect) {
    let Rect { x0, y0, x1, y1 } = block;
    let mid = ((x0 + x1) / 2, (y0 + y1) / 2);
    let zone = site.zones.at(mid.0 as i32, mid.1 as i32);

    // Some ground is not built on at all, and which ground that is comes
    // from the zoning rather than from a dice roll per block - a park is a
    // place, not an accident.
    if !zone.is_built() {
        let green = zone == Zone::Park;
        for y in y0..=y1 {
            for x in x0..=x1 {
                site.cells[y * SIZE + x].kind = if green { Kind::Park } else { Kind::Plaza };
            }
        }
        return;
    }

    // A civic block keeps most of its ground open and puts one broad low
    // building in the middle of it, which is what a courthouse looks like
    // from the street.
    let civic = zone == Zone::Civic;

    // Subdivide.  Split the longer axis until every piece is small enough to
    // be one address.  Downtown lots are bigger, because downtown buildings
    // are - a tower needs a footprint and a walk-up does not.
    let min_lot = match zone {
        Zone::Downtown => 5,
        Zone::Commercial => 4,
        _ => 3,
    };
    let mut queue = vec![(x0, y0, x1, y1)];
    let mut out = Vec::new();
    while let Some((ax, ay, bx, by)) = queue.pop() {
        let (w, h) = (bx - ax + 1, by - ay + 1);
        let big = w.max(h);
        if big <= min_lot || (big <= min_lot + 2 && site.rng.chance(1, 3)) {
            out.push((ax, ay, bx, by));
            continue;
        }
        if w >= h {
            let cut = ax + 1 + site.rng.below((w - 2) as u32) as usize;
            queue.push((ax, ay, cut, by));
            queue.push((cut + 1, ay, bx, by));
        } else {
            let cut = ay + 1 + site.rng.below((h - 2) as u32) as usize;
            queue.push((ax, ay, bx, cut));
            queue.push((ax, cut + 1, bx, by));
        }
    }

    // One palette for the whole block, so a street reads as a street rather
    // than as a shuffled deck.  Neighbouring blocks draw from neighbouring
    // lattice cells, so the palette drifts across the map instead of
    // changing at every kerb.
    let palette_id = (field(mid.0, mid.1, site.seed ^ 0x_9A11_E77E) / 32) as usize;

    for (i, (ax, ay, bx, by)) in out.iter().copied().enumerate() {
        // On a civic block only the middle lot is built.
        if civic && i != out.len() / 2 {
            for y in ay..=by {
                for x in ax..=bx {
                    site.cells[y * SIZE + x].kind = Kind::Plaza;
                }
            }
            continue;
        }
        raise(site, zone, palette_id, Rect { x0: ax, y0: ay, x1: bx, y1: by });
    }
}

/// Facade palettes, one per district.
///
/// Each is a small set of hues that belong together, because a real
/// neighbourhood does have a colour: the glass towers are blue and cyan, the
/// prewar blocks are brick and ochre, the strip near the water is neon.  A
/// single flat list of hues picked at random per building reads as confetti,
/// whatever the individual colours are.
const PALETTES: [&[u8]; 8] = [
    // Glass downtown
    &[crate::palette::H_BLUE, crate::palette::H_LIGHT_BLUE, crate::palette::H_CYAN],
    // Brick and stone
    &[crate::palette::H_BROWN, crate::palette::H_ORANGE, crate::palette::H_RED],
    // Pale offices
    &[crate::palette::H_WHITE, crate::palette::H_LIGHT_BLUE, crate::palette::H_YELLOW],
    // Deep glass
    &[crate::palette::H_DARK_BLUE, crate::palette::H_BLUE, crate::palette::H_BLUE_GREEN],
    // A sodium-lit older quarter
    &[crate::palette::H_YELLOW, crate::palette::H_ORANGE, crate::palette::H_BROWN],
    // Green glass and copper
    &[crate::palette::H_BLUE_GREEN, crate::palette::H_LIGHT_GREEN, crate::palette::H_CYAN],
    // Mixed midtown
    &[
        crate::palette::H_BLUE,
        crate::palette::H_ORANGE,
        crate::palette::H_WHITE,
        crate::palette::H_RED,
    ],
    // A neon strip
    &[crate::palette::H_PINK, crate::palette::H_PURPLE, crate::palette::H_CYAN],
];

/// Put a building on one lot.
fn raise(site: &mut Site, zone: Zone, palette_id: usize, lot: Rect) {
    let footprint = (lot.w() * lot.h()) as u32;
    let use_ = zone::use_for(zone, site.rng);

    // The tallest thing this ground could carry.  Three things multiply
    // together, and they are the layout the whole city is arranged around:
    //
    //   the zone       an office tower may be tall; a house may not
    //   the ring       full height in the core, a fifth of it at the edge
    //   the footprint  a tower needs ground under it
    //
    // The ring term is what the "decreasing height outwards" is: intensity
    // is 255 through the downtown blocks and falls to 50 by the built edge.
    let intensity = site.zones.intensity(lot.x0 as i32, lot.y0 as i32);
    let zoned = zone.ceiling() * intensity / 255;
    let ceiling = (zoned * (8 + footprint.min(24)) / 20).clamp(3, 96);

    // Height is drawn from a skewed distribution, not a uniform one.
    //
    // Uniform gives a city where every height is equally common, and the eye
    // reads that as noise - no general roofline for anything to stand above.
    // Real building heights are closer to a power law: a great many short
    // ones, progressively fewer tall ones, and the occasional landmark far
    // above everything near it.  Four bands approximate that closely enough
    // and are legible.
    let roll = site.rng.below(1000);
    let low = (ceiling / 5).max(3) as i32;
    let mid = (ceiling / 2).max(low as u32 + 2) as i32;
    let height = if roll < 520 {
        site.rng.range(2, low) // the fabric: walk-ups and low commercial
    } else if roll < 840 {
        site.rng.range(low, mid) // mid-rise, the bulk of a real skyline
    } else if roll < 985 {
        site.rng.range(mid, (ceiling as i32).max(mid + 1)) // towers
    } else {
        // A landmark, allowed to break the local roofline.  Rare enough that
        // finding one is an event, and gated on a footprint that could
        // actually carry it.
        let over = if footprint >= 12 { ceiling * 3 / 2 } else { ceiling + 4 };
        site.rng.range(ceiling as i32, over as i32)
    }
    // A fifth off everything.
    //
    // Not a change to the distribution - the bands, the landmark rule and
    // the roofline they produce are all the same shape - but to the scale of
    // it.  A cell is six metres and a fifty-cell tower is three hundred of
    // them, which is taller than anything in the city it is dressed as; and
    // from a car, a street whose walls run off the top of the frame at every
    // junction is a corridor rather than a place.  Four fifths puts the
    // tallest at about two hundred and forty metres and gives the sky back.
    * 4
        / 5;
    let height = height.clamp(2, 96) as u8;

    let arch = pick_arch(site.rng, use_, height);

    let palette = PALETTES[palette_id.min(PALETTES.len() - 1)];
    let hue = palette[site.rng.below(palette.len() as u32) as usize];

    // Brightness, biased by how built-up the area is: downtown glass is lit
    // late and the outskirts are not.
    let luma = (3 + site.rng.below(3) + u32::from(intensity > 200)).min(7) as u8;

    // Occupancy.  Offices empty out and light up unevenly; homes are mostly
    // occupied.  The spread is wide on purpose - the towers that are nearly
    // dark are what make the ones that are nearly full look full.
    let lit = match use_ {
        Use::Residential => site.rng.range(8, 15),
        Use::Civic => site.rng.range(3, 8),
        Use::Commercial => match site.rng.below(10) {
            0 => site.rng.range(1, 4),
            1..=2 => site.rng.range(4, 8),
            3..=7 => site.rng.range(8, 12),
            _ => site.rng.range(11, 15),
        },
    } as u8;

    // Cut the whole footprint to one pad before anything stands on it.
    site.elev.level(lot.x0, lot.y0, lot.x1, lot.y1);

    let idx = site.lots.len() as u16;
    site.lots.push(Lot {
        x0: lot.x0 as u8,
        y0: lot.y0 as u8,
        x1: lot.x1 as u8,
        y1: lot.y1 as u8,
        height,
        arch,
        use_,
        hue,
        luma,
        lit,
        seed: site.rng.next_u32(),
    });

    for y in lot.y0..=lot.y1 {
        for x in lot.x0..=lot.x1 {
            let c = &mut site.cells[y * SIZE + x];
            c.kind = Kind::Building;
            c.lot = idx;
            site.elev.build(x as i32, y as i32, cell_height(height, arch, lot, x, y));
        }
    }
}

/// What a building of a given height is likely to be built as.
fn pick_arch(rng: &mut Rng, use_: Use, height: u8) -> Arch {
    // What a building is for narrows how it is likely to be built.  Nobody
    // puts a curtain wall on a walk-up and nobody bolts a fire escape to an
    // office tower, so the two questions are asked in the right order:
    // use first, then height.
    match use_ {
        Use::Civic => {
            if height <= 4 {
                Arch::LowRise
            } else {
                Arch::Prewar
            }
        }
        Use::Residential => {
            if height <= 4 {
                Arch::LowRise
            } else if height <= 14 {
                // Brick, punched windows, a fire escape on the street face.
                if rng.chance(4, 5) {
                    Arch::Prewar
                } else {
                    Arch::LowRise
                }
            } else if rng.chance(3, 5) {
                // The post-war residential slab: long, flat, repetitive.
                Arch::Slab
            } else {
                Arch::Setback
            }
        }
        Use::Commercial => {
            if height <= 3 {
                Arch::LowRise
            } else if height <= 10 {
                if rng.chance(3, 4) {
                    Arch::Prewar
                } else {
                    Arch::LowRise
                }
            } else if height >= 34 {
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
            }
        }
    }
}

/// The height of one cell of a lot.
///
/// This is where the silhouette comes from.  A slab is flat-topped; a
/// setback loses a tier for every ring you move out from the middle; a Deco
/// tower does the same but keeps its corner piers, so the crown is a notch
/// taller than the shoulders.
fn cell_height(height: u8, arch: Arch, lot: Rect, x: usize, y: usize) -> u8 {
    let ring = lot.ring(x, y);
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
            let corner = (x == lot.x0 || x == lot.x1) && (y == lot.y0 || y == lot.y1);
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
            let (x, y) = ((i % SIZE) as i32, (i / SIZE) as i32);
            assert_eq!(a.height(x, y), b.height(x, y), "cell {i} differs");
            assert_eq!(a.elev.ground_steps(x, y), b.elev.ground_steps(x, y));
            assert_eq!(a.cells[i].kind, b.cells[i].kind);
        }
    }

    #[test]
    fn different_seeds_build_different_cities() {
        let a = City::generate(1);
        let b = City::generate(2);
        let same = (0..a.cells.len())
            .filter(|&i| {
                let (x, y) = ((i % SIZE) as i32, (i / SIZE) as i32);
                a.height(x, y) == b.height(x, y)
            })
            .count();
        assert!(same < a.cells.len() * 9 / 10, "two seeds produced nearly the same city");
    }

    #[test]
    fn roads_are_always_walkable() {
        let c = city();
        for y in 0..SIZE as i32 {
            for x in 0..SIZE as i32 {
                if c.plan.is_road(x, y) {
                    assert_eq!(c.height(x, y), 0, "a building stands in the road at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn the_plan_lays_roads_of_more_than_one_width() {
        // The whole point of generating a street system rather than
        // computing one: if every road comes out the same width, nothing has
        // been gained over `x % 14 < 3`.
        let c = city();
        let widths: std::collections::HashSet<u8> = (0..SIZE as i32)
            .filter_map(|i| {
                let r = c.plan.cols.at(i);
                r.is_road().then_some(r.width)
            })
            .chain((0..SIZE as i32).filter_map(|i| {
                let r = c.plan.rows.at(i);
                r.is_road().then_some(r.width)
            }))
            .collect();
        assert!(widths.len() >= 3, "the plan only has widths {widths:?}");
    }

    #[test]
    fn the_plan_lays_roads_of_more_than_one_class() {
        let c = city();
        let classes: std::collections::HashSet<RoadClass> = (0..SIZE as i32)
            .map(|i| c.plan.cols.at(i).class)
            .chain((0..SIZE as i32).map(|i| c.plan.rows.at(i).class))
            .filter(|k| *k != RoadClass::None)
            .collect();
        assert!(classes.len() >= 3, "the plan only has classes {classes:?}");
    }

    #[test]
    fn roads_never_run_straight_into_each_other() {
        // Two roads with no block between them are one wide road with a
        // seam down it, and the marking code would paint two centre lines
        // three metres apart.  Checked by *shape* rather than by a length
        // limit, because an arterial is legitimately sixteen cells wide:
        // every unbroken run of road must be exactly one road, so all its
        // cells agree on a width and the run is that long.
        for seed in [1u32, 2, 3, 4, 5] {
            let c = City::generate(seed);
            for axis in [&c.plan.cols, &c.plan.rows] {
                let mut i = 0i32;
                while i < SIZE as i32 {
                    if !axis.is_road(i) {
                        i += 1;
                        continue;
                    }
                    let w = axis.at(i).width as i32;
                    for k in 0..w {
                        let r = axis.at(i + k);
                        assert!(r.is_road(), "seed {seed}: a road at {i} is {w} wide but ends at {}", i + k);
                        assert_eq!(r.width as i32, w, "seed {seed}: two roads meet at {}", i + k);
                        assert_eq!(r.across as i32, k, "seed {seed}: offsets do not run 0..{w} at {i}");
                    }
                    assert!(!axis.is_road(i + w), "seed {seed}: a road runs on past its width at {i}");
                    i += w;
                }
            }
        }
    }

    #[test]
    fn an_alley_alone_never_produces_a_pavement() {
        // An alley is a gap between buildings, not a small street.  A cell
        // beside one may still be pavement if a *proper* street is also next
        // to it - that is a corner, and it is correct - so the claim has to
        // be about cells whose only neighbouring road is the alley.
        let c = City::generate(7);
        let mut checked = 0;
        for y in 0..SIZE as i32 {
            for x in 0..SIZE as i32 {
                if c.plan.is_road(x, y) {
                    continue;
                }
                let neighbours = [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)];
                let roads: Vec<RoadClass> = neighbours
                    .iter()
                    .filter_map(|(dx, dy)| {
                        let (nx, ny) = (x + dx, y + dy);
                        c.plan.is_road(nx, ny).then(|| {
                            let col = c.plan.cols.at(nx).class;
                            if col != RoadClass::None {
                                col
                            } else {
                                c.plan.rows.at(ny).class
                            }
                        })
                    })
                    .collect();
                if roads.is_empty() || roads.iter().any(|k| k.is_street()) {
                    continue;
                }
                checked += 1;
                assert_ne!(
                    c.at(x, y).kind,
                    Kind::Sidewalk,
                    "a cell at {x},{y} whose only neighbour is an alley grew a pavement"
                );
            }
        }
        assert!(checked > 20, "only {checked} alley-only cells - the test proved nothing");
    }

    #[test]
    fn no_block_is_smaller_than_the_minimum() {
        for seed in [1u32, 2, 3, 4, 5, 99] {
            let c = City::generate(seed);
            // Every run of buildable ground, on both axes, through the
            // middle of the map where the blocks are.
            let mid = SIZE as i32 / 2;
            for (horizontal, fixed_axis) in [(true, mid), (false, mid)] {
                // Only blocks that get built on.  The strips left over at
                // the very edge of the map are whatever is between the last
                // road and the end of the world; they are zoned as outskirts
                // and nothing is raised on them, so their size is not a
                // claim this makes.
                let mut run = 0usize;
                let mut start = 0i32;
                for i in 0..SIZE as i32 {
                    let (x, y) = if horizontal { (i, fixed_axis) } else { (fixed_axis, i) };
                    if c.plan.is_buildable(x, y) {
                        if run == 0 {
                            start = i;
                        }
                        run += 1;
                        continue;
                    }
                    if run > 0 {
                        let (sx, sy) =
                            if horizontal { (start, fixed_axis) } else { (fixed_axis, start) };
                        if c.zones.at(sx, sy).is_built() {
                            assert!(
                                run >= MIN_BLOCK,
                                "seed {seed}: a built block only {run} cells across at {sx},{sy}"
                            );
                        }
                    }
                    run = 0;
                }
            }
        }
    }

    #[test]
    fn blocks_come_in_more_than_one_size() {
        let c = city();
        let mut widths = std::collections::HashSet::new();
        let mut run = 0u32;
        let y = (0..SIZE as i32)
            .find(|y| (0..SIZE as i32).all(|x| !c.plan.rows.is_road(*y) || !c.plan.cols.is_road(x)))
            .unwrap_or(SIZE as i32 / 2);
        for x in 0..SIZE as i32 {
            if c.plan.is_buildable(x, y) {
                run += 1;
            } else if run > 0 {
                widths.insert(run);
                run = 0;
            }
        }
        assert!(widths.len() >= 3, "every block along the row is one of {widths:?} wide");
    }

    #[test]
    fn the_grid_is_connected_enough_to_walk() {
        // The two families of road have to actually intersect.
        let c = city();
        let mut crossings = 0;
        for y in 0..SIZE as i32 {
            for x in 0..SIZE as i32 {
                if c.plan.is_junction(x, y) && c.open(x, y) {
                    crossings += 1;
                }
            }
        }
        assert!(crossings > 100, "only {crossings} intersections - the grid is not a grid");
    }

    #[test]
    fn buildings_always_carry_a_lot_and_a_height() {
        let c = city();
        for (i, cell) in c.cells.iter().enumerate() {
            let (x, y) = ((i % SIZE) as i32, (i / SIZE) as i32);
            if cell.kind == Kind::Building {
                assert_ne!(cell.lot, NO_LOT);
                assert!(c.height(x, y) > 0, "a building at {x},{y} has no height");
                assert!((cell.lot as usize) < c.lots.len());
            } else {
                assert_eq!(c.height(x, y), 0, "something stands on open ground at {x},{y}");
            }
        }
    }

    #[test]
    fn setbacks_step_inwards_rather_than_outwards() {
        // The middle of a setback lot must never be shorter than its edge.
        let block = Rect { x0: 0, y0: 0, x1: 6, y1: 6 };
        let h_edge = cell_height(40, Arch::Setback, block, 0, 3);
        let h_mid = cell_height(40, Arch::Setback, block, 3, 3);
        assert!(h_mid > h_edge, "setback got taller towards the street");
    }

    #[test]
    fn a_slab_is_flat_topped() {
        for x in 0..=5 {
            assert_eq!(cell_height(30, Arch::Slab, Rect { x0: 0, y0: 0, x1: 5, y1: 5 }, x, 2), 30);
        }
    }

    #[test]
    fn downtown_is_taller_than_the_outskirts() {
        // Sampled over a couple of blocks rather than a few cells: the very
        // middle of the map is as likely as not to be the junction of the
        // two arterials, which is a large amount of tarmac and no buildings
        // at all.
        let c = city();
        let mid = SIZE as i32 / 2;
        let tallest = |x0: i32, y0: i32, n: i32| -> u32 {
            (y0..y0 + n)
                .flat_map(|y| (x0..x0 + n).map(move |x| (x, y)))
                .map(|(x, y)| c.height(x, y) as u32)
                .max()
                .unwrap()
        };
        let core = tallest(mid - 26, mid - 26, 52);
        let out = tallest(BLOCK_PITCH as i32, BLOCK_PITCH as i32, 40);
        assert!(core > out * 2, "downtown tops out at {core} and the outskirts at {out}");
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
    fn heights_are_skewed_rather_than_uniform() {
        // The distribution is the point.  Uniform heights read as noise -
        // no general roofline for anything to stand above - so the shape is
        // checked, not just the range.
        let c = city();
        let mut h: Vec<u32> = c.lots.iter().map(|l| l.height as u32).collect();
        h.sort_unstable();
        let median = h[h.len() / 2];
        let p90 = h[h.len() * 9 / 10];
        let max = *h.last().unwrap();
        assert!(median * 2 < p90, "median {median} and p90 {p90} - that is a uniform spread");
        assert!(p90 * 3 / 2 < max, "p90 {p90} and tallest {max} - nothing stands above the rest");
        assert!(max > 30, "the tallest building is only {max}");
        let short = h.iter().filter(|v| **v <= 6).count();
        assert!(short * 4 > h.len(), "only {short} of {} buildings are low-rise", h.len());
    }

    #[test]
    fn buildings_vary_in_colour_and_in_brightness() {
        let c = city();
        let hues: std::collections::HashSet<u8> = c.lots.iter().map(|l| l.hue).collect();
        let lumas: std::collections::HashSet<u8> = c.lots.iter().map(|l| l.luma).collect();
        let lits: std::collections::HashSet<u8> = c.lots.iter().map(|l| l.lit).collect();
        assert!(hues.len() >= 8, "only {} hues in the whole city", hues.len());
        assert!(lumas.len() >= 3, "only {} brightnesses", lumas.len());
        assert!(lits.len() >= 6, "only {} occupancy levels", lits.len());
    }

    #[test]
    fn colour_is_coherent_across_a_neighbourhood() {
        // The property that matters is not "this lot got its block's
        // palette" - a lot is a subdivision of a block and its midpoint can
        // fall in a different lattice cell than the block's did, so that
        // test would be checking an implementation detail and getting it
        // wrong.  What matters visually is that two buildings near each
        // other are much likelier to share a colour than two far apart.
        // That is what makes a city read as neighbourhoods rather than as
        // confetti, and it is measurable.
        let c = city();
        let (mut near_same, mut near, mut far_same, mut far) = (0u32, 0u32, 0u32, 0u32);
        for (i, a) in c.lots.iter().enumerate() {
            for b in c.lots.iter().skip(i + 1) {
                let d = (a.x0 as i32 - b.x0 as i32).abs() + (a.y0 as i32 - b.y0 as i32).abs();
                if d <= 8 {
                    near += 1;
                    near_same += u32::from(a.hue == b.hue);
                } else if d >= 48 {
                    far += 1;
                    far_same += u32::from(a.hue == b.hue);
                }
            }
        }
        assert!(near > 200 && far > 200, "not enough pairs to measure: {near} near, {far} far");
        // Percentages, to keep the comparison readable in a failure message.
        let p_near = near_same * 100 / near;
        let p_far = far_same * 100 / far;
        assert!(
            p_near > p_far * 3 / 2,
            "neighbours share a hue {p_near}% of the time and strangers {p_far}% - \
             colour is not tied to place"
        );
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

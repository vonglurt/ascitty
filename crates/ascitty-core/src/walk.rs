//! The walking system: where a person on foot may be, and how they get
//! about.
//!
//! The driving network and the walking network are not the same network and
//! never were. A car belongs on the carriageway and nowhere else; a person
//! belongs on the pavement, in the parks and plazas, and on the
//! carriageway only where it is painted for them to cross. Treating "not a
//! building" as one walkable space is what produced pedestrians strolling
//! down the middle of an avenue.
//!
//! So this is a second map over the same grid, one byte a cell, built once
//! when the city is generated.
//!
//! # Crossings
//!
//! A junction is the only place the two networks legitimately meet, and the
//! crossing cells are the ones just *outside* the junction box - at the stop
//! line, spanning the road, with pavement at each end. They come from
//! [`crate::world::Plan::crossing_at`], which is also what the renderer
//! paints the zebra bars from, so what a pedestrian walks over and what you
//! see them walk over cannot drift apart.
//!
//! # Routing
//!
//! Not a graph search. A pedestrian in this city has somewhere to be but no
//! opinion about the shortest way there, so [`WalkMap::step_toward`] is a
//! greedy step that prefers pavement, accepts a crossing, and turns along
//! the kerb when the direct way is blocked. That is enough to keep a crowd
//! flowing along the pavements and through the junctions, and it costs four
//! lookups.

use crate::trig::{self, Ang};
use crate::world::{Kind, Plan, RoadClass};

/// What a cell offers somebody on foot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Foot {
    /// You may not be here at all.
    Blocked = 0,
    /// Pavement, park or plaza.  Where a pedestrian belongs.
    Path = 1,
    /// Carriageway painted for crossing.  Passable, and not somewhere to
    /// linger.
    Crossing = 2,
}

impl Foot {
    /// Whether a person may stand here.
    #[inline(always)]
    pub fn passable(self) -> bool {
        self != Foot::Blocked
    }
}

/// The pedestrian network.
#[derive(Clone)]
pub struct WalkMap {
    size: usize,
    cells: Vec<u8>,
}

impl WalkMap {
    /// Build the walking network from the street plan and what got built.
    pub fn build(size: usize, plan: &Plan, kind_at: impl Fn(i32, i32) -> Kind) -> WalkMap {
        let mut cells = vec![Foot::Blocked as u8; size * size];
        for y in 0..size as i32 {
            for x in 0..size as i32 {
                let f = classify(plan, &kind_at, x, y, size as i32);
                cells[y as usize * size + x as usize] = f as u8;
            }
        }
        WalkMap { size, cells }
    }

    /// What this cell offers on foot.
    #[inline(always)]
    pub fn at(&self, x: i32, y: i32) -> Foot {
        if x < 0 || y < 0 || x as usize >= self.size || y as usize >= self.size {
            return Foot::Blocked;
        }
        match self.cells[y as usize * self.size + x as usize] {
            1 => Foot::Path,
            2 => Foot::Crossing,
            _ => Foot::Blocked,
        }
    }

    /// Whether a person may be here.
    #[inline(always)]
    pub fn passable(&self, x: i32, y: i32) -> bool {
        self.at(x, y).passable()
    }

    /// The nearest cell a person may stand on, spiralling out.  Used to put
    /// somebody down without dropping them inside a wall.
    pub fn nearest(&self, x: i32, y: i32, limit: i32) -> Option<(i32, i32)> {
        if self.passable(x, y) {
            return Some((x, y));
        }
        for r in 1..limit {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue; // the ring, not the disc
                    }
                    if self.passable(x + dx, y + dy) {
                        return Some((x + dx, y + dy));
                    }
                }
            }
        }
        None
    }

    /// A route between two cells, as a list of cells, or `None` if there
    /// isn't one within `budget` cells of searching.
    ///
    /// Breadth-first, so the route it finds is the shortest one. This is the
    /// query for when the answer has to be right - "is this network actually
    /// connected", "can a passenger reach that door" - and it is far too
    /// expensive to run per pedestrian per frame, which is what
    /// [`WalkMap::step_toward`] is for.
    ///
    /// The distinction matters. A greedy stepper cannot get out of a
    /// U-shaped dead end, and no amount of tuning will make it; a search
    /// can, and costs a visited set the size of the map.
    pub fn route(&self, from: (i32, i32), to: (i32, i32), budget: usize) -> Option<Vec<(i32, i32)>> {
        if !self.passable(from.0, from.1) || !self.passable(to.0, to.1) {
            return None;
        }
        if from == to {
            return Some(vec![from]);
        }
        let idx = |x: i32, y: i32| y as usize * self.size + x as usize;
        // `came_from` doubles as the visited set: usize::MAX means unseen.
        let mut came = vec![usize::MAX; self.size * self.size];
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
                if !self.passable(nx, ny) {
                    continue;
                }
                let ni = idx(nx, ny);
                if came[ni] != usize::MAX {
                    continue;
                }
                came[ni] = idx(x, y);
                if (nx, ny) == to {
                    // Walk the chain back, then turn it the right way round.
                    let mut path = vec![to];
                    let mut cur = ni;
                    while came[cur] != cur {
                        cur = came[cur];
                        path.push(((cur % self.size) as i32, (cur / self.size) as i32));
                    }
                    path.reverse();
                    return Some(path);
                }
                queue.push_back((nx, ny));
            }
        }
        None
    }

    /// One step from `from` towards `to`, as a compass heading.
    ///
    /// Greedy, cheap, and explicitly **not guaranteed to arrive**: it has no
    /// memory, so a U-shaped obstacle will hold it forever. That is an
    /// acceptable trade for a crowd of pedestrians who have somewhere to be
    /// and no opinion about the shortest way there - each of them costs four
    /// lookups a frame. When arrival matters, use [`WalkMap::route`].
    ///
    /// Greedy and cheap: try the axis with the most ground to make up, fall
    /// back to the other, then to either of the two remaining directions.
    /// Returns `None` only when a pedestrian is completely boxed in, which
    /// on this map means they were put somewhere they should not have been.
    pub fn step_toward(&self, from: (i32, i32), to: (i32, i32)) -> Option<Ang> {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        if dx == 0 && dy == 0 {
            return None;
        }
        let east = if dx >= 0 { 1 } else { -1 };
        let south = if dy >= 0 { 1 } else { -1 };

        // Preference order: the long axis first, then the short one, then
        // the two sideways options, so a blocked walker turns along the kerb
        // instead of stopping dead.
        let mut order = [(east, 0), (0, south), (0, -south), (-east, 0)];
        if dy.abs() > dx.abs() {
            order.swap(0, 1);
        }
        for (sx, sy) in order {
            if sx == 0 && sy == 0 {
                continue;
            }
            if self.passable(from.0 + sx, from.1 + sy) {
                return Some(heading(sx, sy));
            }
        }
        None
    }
}

/// The compass heading of a unit step.
fn heading(sx: i32, sy: i32) -> Ang {
    match (sx, sy) {
        (1, 0) => 0,
        (0, 1) => trig::QUARTER,
        (-1, 0) => trig::HALF,
        _ => trig::HALF.wrapping_add(trig::QUARTER),
    }
}

/// What one cell offers on foot.
fn classify(
    plan: &Plan,
    kind_at: &impl Fn(i32, i32) -> Kind,
    x: i32,
    y: i32,
    size: i32,
) -> Foot {
    if x < 0 || y < 0 || x >= size || y >= size {
        return Foot::Blocked;
    }
    match kind_at(x, y) {
        Kind::Building => Foot::Blocked,
        // The beach is walkable and the sea is not, which is the whole of
        // what the shore means to anything that moves.
        Kind::Sidewalk | Kind::Park | Kind::Plaza | Kind::Sand => Foot::Path,
        Kind::Water => Foot::Blocked,
        Kind::Road => {
            // An alley is a gap between buildings, and people walk down it.
            // Making it blocked looks tidy and cuts the network in half:
            // a block ringed only by alleys gets no pavement at all, so a
            // park inside one becomes an enclosure nobody can reach.
            if plan.cols.at(x).class == RoadClass::Alley
                || plan.rows.at(y).class == RoadClass::Alley
            {
                return Foot::Path;
            }
            // The rest of the carriageway is crossable only where a
            // crossing is painted, which is the same test the renderer uses.
            if plan.crossing_at(x, y).is_some() {
                Foot::Crossing
            } else {
                Foot::Blocked
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{City, SIZE};

    fn city() -> City {
        City::generate(2024)
    }

    #[test]
    fn buildings_are_never_walkable() {
        let c = city();
        for y in 0..SIZE as i32 {
            for x in 0..SIZE as i32 {
                if c.at(x, y).kind == Kind::Building {
                    assert!(!c.walk.passable(x, y), "you can walk into the building at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn pavement_is_always_walkable() {
        let c = city();
        let mut n = 0;
        for y in 0..SIZE as i32 {
            for x in 0..SIZE as i32 {
                if c.at(x, y).kind == Kind::Sidewalk {
                    assert_eq!(c.walk.at(x, y), Foot::Path, "pavement at {x},{y} is not walkable");
                    n += 1;
                }
            }
        }
        assert!(n > 500, "only {n} cells of pavement in the whole city");
    }

    #[test]
    fn the_open_carriageway_is_not_walkable() {
        let c = city();
        let mut blocked = 0;
        for y in 0..SIZE as i32 {
            for x in 0..SIZE as i32 {
                let alley = c.plan.cols.at(x).class == crate::world::RoadClass::Alley
                    || c.plan.rows.at(y).class == crate::world::RoadClass::Alley;
                if c.at(x, y).kind == Kind::Road
                    && c.plan.crossing_at(x, y).is_none()
                    && !alley
                {
                    assert!(!c.walk.passable(x, y), "you can stroll down the road at {x},{y}");
                    blocked += 1;
                }
            }
        }
        assert!(blocked > 1000, "only {blocked} cells of open road");
    }

    #[test]
    fn junctions_offer_a_crossing() {
        let c = city();
        let crossings = (0..SIZE as i32)
            .flat_map(|y| (0..SIZE as i32).map(move |x| (x, y)))
            .filter(|&(x, y)| c.walk.at(x, y) == Foot::Crossing)
            .count();
        assert!(crossings > 200, "only {crossings} crossing cells in the whole city");
    }

    #[test]
    fn a_crossing_always_has_somewhere_to_arrive() {
        // The property that was missing: a crossing has to touch ground a
        // pedestrian can already be on, at both ends, or it is a stripe of
        // paint in the middle of a road that nobody can reach.
        let c = city();
        let mut n = 0;
        for y in 0..SIZE as i32 {
            for x in 0..SIZE as i32 {
                if c.walk.at(x, y) != Foot::Crossing {
                    continue;
                }
                n += 1;
                let touches = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .filter(|(dx, dy)| c.walk.passable(x + dx, y + dy))
                    .count();
                assert!(touches >= 2, "a crossing at {x},{y} touches only {touches} passable cells");
            }
        }
        assert!(n > 200, "only {n} crossing cells");
    }

    #[test]
    fn the_pavements_are_actually_connected() {
        // The property that matters: you can get from one side of the
        // downtown to the other on foot, crossing the roads at the
        // crossings.  Checked with the search rather than with the greedy
        // stepper, because the stepper is explicitly allowed to get stuck
        // and a test of it would be testing the wrong thing.
        let c = city();
        let mid = SIZE as i32 / 2;
        let from = c.walk.nearest(mid - 30, mid - 30, 40).expect("nowhere to start");
        let to = c.walk.nearest(mid + 30, mid + 30, 40).expect("nowhere to finish");
        let path = c
            .walk
            .route(from, to, SIZE * SIZE)
            .unwrap_or_else(|| panic!("no way on foot from {from:?} to {to:?}"));
        assert!(path.len() > 40, "a suspiciously short route: {} cells", path.len());
        for w in path.windows(2) {
            let d = (w[0].0 - w[1].0).abs() + (w[0].1 - w[1].1).abs();
            assert_eq!(d, 1, "the route jumps from {:?} to {:?}", w[0], w[1]);
            assert!(c.walk.passable(w[1].0, w[1].1));
        }
    }

    #[test]
    fn a_route_to_somewhere_unreachable_gives_up_rather_than_hanging() {
        let c = city();
        let mid = SIZE as i32 / 2;
        let from = c.walk.nearest(mid, mid, 40).unwrap();
        // The middle of a building is not on the network at all.
        let inside = (0..SIZE as i32)
            .flat_map(|y| (0..SIZE as i32).map(move |x| (x, y)))
            .find(|&(x, y)| c.at(x, y).kind == Kind::Building)
            .unwrap();
        assert!(c.walk.route(from, inside, SIZE * SIZE).is_none());
    }

    #[test]
    fn a_route_to_where_you_already_are_is_one_cell() {
        let c = city();
        let mid = SIZE as i32 / 2;
        let at = c.walk.nearest(mid, mid, 40).unwrap();
        assert_eq!(c.walk.route(at, at, 16), Some(vec![at]));
    }

    #[test]
    fn stepping_never_leaves_the_network() {
        let c = city();
        let mid = SIZE as i32 / 2;
        let start = c.walk.nearest(mid, mid, 40).unwrap();
        let mut at = start;
        for i in 0..3000 {
            let goal = (mid + (i % 61) - 30, mid + (i % 47) - 23);
            let Some(a) = c.walk.step_toward(at, goal) else { continue };
            let (dx, dy) = match a {
                0 => (1, 0),
                x if x == trig::QUARTER => (0, 1),
                x if x == trig::HALF => (-1, 0),
                _ => (0, -1),
            };
            at = (at.0 + dx, at.1 + dy);
            assert!(c.walk.passable(at.0, at.1), "walked off the network at {at:?}");
        }
    }

    #[test]
    fn nearest_finds_somewhere_to_stand_or_admits_it_cannot() {
        let c = city();
        // The middle of a building.
        let inside = (0..SIZE as i32)
            .flat_map(|y| (0..SIZE as i32).map(move |x| (x, y)))
            .find(|&(x, y)| c.at(x, y).kind == Kind::Building)
            .unwrap();
        let found = c.walk.nearest(inside.0, inside.1, 30).expect("nowhere near a building");
        assert!(c.walk.passable(found.0, found.1));
    }

    #[test]
    fn arriving_returns_no_step() {
        let c = city();
        assert!(c.walk.step_toward((10, 10), (10, 10)).is_none());
    }
}


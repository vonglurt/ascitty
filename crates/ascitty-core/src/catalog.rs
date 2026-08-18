//! The glyph catalogue: 128 shapes, generated, indexed, and shared.
//!
//! This is the single vocabulary the renderer speaks.  It does not emit
//! characters - it emits *catalogue indices*, and each target renders an
//! index its own way:
//!
//! | Target | How an index becomes a shape |
//! |---|---|
//! | Plus/4 | screen code `index + 64`, drawn from the baked charset in RAM |
//! | Terminal, Unicode | the block/box character in [`UNICODE`] |
//! | Terminal, ASCII | the typeable character in [`ASCII`] - the tty mode the project is named for |
//! | Terminal, PETSCII | the ROM screen code in [`PETSCII`], for an unmodified character set |
//!
//! Because the Plus/4 mapping is the identity, the machine has no glyph
//! selection cost at all: the renderer's output byte *is* the screen byte.
//! The terminal pays one array index.
//!
//! 128 is not arbitrary.  A Plus/4 character set is 256 definitions of 8
//! bytes; the ROM's lower 64 screen codes carry the alphabet, the digits and
//! the punctuation the status line needs, so the custom half starts at 64
//! and runs to 191.  That leaves the HUD legible without a second charset
//! bank.

use crate::font::{self, Bitmap, Xform};

/// A catalogue index.  Also the Plus/4 screen code, less [`PLUS4_BASE`].
pub type GlyphId = u8;

/// Number of glyphs in the catalogue.
pub const N_GLYPHS: usize = 128;

/// Screen code of catalogue index 0 on the Plus/4.
pub const PLUS4_BASE: u8 = 64;

// --- the layout ------------------------------------------------------------

/// Nothing.  Sky, and the inside of a dark window.
pub const G_BLANK: GlyphId = 0;
/// Seven ordered-dither levels, `G_DITHER + 0..7`, lightest first.
pub const G_DITHER: GlyphId = 1;
/// A fully covered cell.
pub const G_SOLID: GlyphId = 8;
/// The fifteen non-empty 2x2 quadrant masks, `G_QUAD + (mask - 1)`.
pub const G_QUAD: GlyphId = 9;
/// Eight partial fills from the bottom up, `G_FILL + 0..7`.
///
/// These are what a roofline lands on.  A tower top that falls between two
/// character rows picks the eighth-step nearest its true height, so the
/// skyline steps in eighths of a cell rather than in whole cells.
pub const G_FILL: GlyphId = 24;
/// Eight edge slopes, `G_SLOPE + 0..7`, at 45 and 22.5 degrees.
pub const G_SLOPE: GlyphId = 32;
/// Sixteen facade tiles, `G_FACADE + 0..15`.  See [`facade_tile`].
pub const G_FACADE: GlyphId = 40;
/// Four vertical mullions, `G_MULLION + 0..3`.
pub const G_MULLION: GlyphId = 56;
/// Four horizontal bands - spandrel, cornice, setback ledge, parapet.
pub const G_CORNICE: GlyphId = 60;
/// Six fire-escape parts, `G_FIRE + 0..5`.  See [`FIRE_ZIG_L`] and friends.
pub const G_FIRE: GlyphId = 64;
/// Four roofscape parts - parapet, water tower, plant housing, antenna.
pub const G_ROOF: GlyphId = 70;
/// Eight ground surfaces, `G_ROAD + 0..7`.
pub const G_ROAD: GlyphId = 74;
/// Six plants, `G_FLORA + 0..5`.
pub const G_FLORA: GlyphId = 82;
/// Six pieces of street furniture, `G_STREET + 0..5`.
pub const G_STREET: GlyphId = 88;
/// Four vehicle parts, `G_VEHICLE + 0..3`.
pub const G_VEHICLE: GlyphId = 94;
/// Two pedestrian glyphs.
pub const G_PED: GlyphId = 98;
/// Eight rain phases, `G_RAIN + 0..7`.
pub const G_RAIN: GlyphId = 100;
/// The moon, as four quadrants in reading order.
pub const G_MOON: GlyphId = 108;
/// The moon's halo, as four quadrants in reading order.
pub const G_HALO: GlyphId = 112;
/// Eight star positions within a cell, `G_STAR + 0..7`.
pub const G_STAR: GlyphId = 116;
/// Four haze levels, sparser than the lightest dither.
pub const G_HAZE: GlyphId = 124;

// Named offsets within the families, so call sites read as English.

/// A zigzag stair leaning right - the outward-facing half of a fire escape.
pub const FIRE_ZIG_R: GlyphId = G_FIRE;
/// The same stair leaning left; the two alternate up the building.
pub const FIRE_ZIG_L: GlyphId = G_FIRE + 1;
/// A landing: the horizontal platform the stairs meet.
pub const FIRE_LANDING: GlyphId = G_FIRE + 2;
/// Vertical railing, drawn at the outer edge of a landing.
pub const FIRE_RAIL: GlyphId = G_FIRE + 3;
/// The counterweighted drop ladder at the bottom of the run.
pub const FIRE_LADDER: GlyphId = G_FIRE + 4;
/// The bracket tying the ironwork back to the wall.
pub const FIRE_BRACKET: GlyphId = G_FIRE + 5;

/// Plain asphalt.
pub const ROAD_ASPHALT: GlyphId = G_ROAD;
/// A lane dash.
pub const ROAD_DASH: GlyphId = G_ROAD + 1;
/// The double centre line.
pub const ROAD_CENTRE: GlyphId = G_ROAD + 2;
/// A crosswalk stripe.
pub const ROAD_CROSSING: GlyphId = G_ROAD + 3;
/// The kerb, where road meets sidewalk.
pub const ROAD_KERB: GlyphId = G_ROAD + 4;
/// Paving.
pub const ROAD_PAVING: GlyphId = G_ROAD + 5;
/// A storm grate.
pub const ROAD_GRATE: GlyphId = G_ROAD + 6;
/// Standing water, which is where the rain ends up.
pub const ROAD_PUDDLE: GlyphId = G_ROAD + 7;

/// Dense tree canopy.
pub const FLORA_CANOPY: GlyphId = G_FLORA;
/// Thin canopy, for the edge of a crown.
pub const FLORA_LEAF: GlyphId = G_FLORA + 1;
/// A trunk.
pub const FLORA_TRUNK: GlyphId = G_FLORA + 2;
/// A hedge.
pub const FLORA_HEDGE: GlyphId = G_FLORA + 3;
/// Grass.
pub const FLORA_GRASS: GlyphId = G_FLORA + 4;
/// A street planter.
pub const FLORA_PLANTER: GlyphId = G_FLORA + 5;

/// A lamp post.
pub const ST_POST: GlyphId = G_STREET;
/// The lamp head, and the only thing in the city that lights the ground.
pub const ST_LAMP: GlyphId = G_STREET + 1;
/// A traffic signal.
pub const ST_SIGNAL: GlyphId = G_STREET + 2;
/// A sign board.
pub const ST_SIGN: GlyphId = G_STREET + 3;
/// A hydrant.
pub const ST_HYDRANT: GlyphId = G_STREET + 4;
/// A bollard.
pub const ST_BOLLARD: GlyphId = G_STREET + 5;

/// Index of the facade tile with `bays` windows across, `floors` down and
/// lit pattern `lit`.  Four configurations, four patterns each.
///
/// The configuration is chosen by how far away the wall is, which is what
/// produces the "zipper" the eye reads on a tall slab: close up a floor is
/// four cells and the windows are individually resolved; at a distance a
/// single cell holds four floors, and the tile that fits there is the one
/// with the tight vertical repeat.
#[inline]
pub const fn facade_tile(cfg: u8, lit: u8) -> GlyphId {
    G_FACADE + ((cfg & 3) << 2) + (lit & 3)
}

/// The eighth-step fill whose top edge is nearest `eighths` from the bottom.
#[inline]
pub const fn fill_step(eighths: u8) -> GlyphId {
    if eighths == 0 {
        G_BLANK
    } else if eighths >= 8 {
        G_SOLID
    } else {
        G_FILL + eighths - 1
    }
}

/// The dither nearest a 0..=8 intensity.
#[inline]
pub const fn shade(level: u8) -> GlyphId {
    if level == 0 {
        G_BLANK
    } else if level >= 8 {
        G_SOLID
    } else {
        G_DITHER + level - 1
    }
}

// --- construction ----------------------------------------------------------

/// The catalogue: 128 bitmaps and their names.
pub struct Catalog {
    /// The bitmaps, indexed by [`GlyphId`].
    pub bitmaps: [Bitmap; N_GLYPHS],
    /// A short name per glyph, for the sheet the bake tool prints.
    pub names: [&'static str; N_GLYPHS],
}

/// Build the catalogue.  Pure, deterministic, and the only definition of
/// what any of these shapes are.
pub fn build() -> Catalog {
    let mut bm = [font::BLANK; N_GLYPHS];
    let mut names = [""; N_GLYPHS];
    let mut put = |i: GlyphId, b: Bitmap, n: &'static str| {
        bm[i as usize] = b;
        names[i as usize] = n;
    };

    put(G_BLANK, font::BLANK, "blank");
    for l in 0..7u8 {
        put(G_DITHER + l, font::dither((l as u32 + 1) * 8), "dither");
    }
    put(G_SOLID, font::SOLID, "solid");

    for m in 1..16u8 {
        put(G_QUAD + m - 1, font::quadrant(m), "quad");
    }

    // Bottom-up eighths: rows 7 down to 8-n.
    for n in 1..=8u8 {
        let mut b = font::BLANK;
        for r in (8 - n)..8 {
            b[r as usize] = 0xff;
        }
        put(G_FILL + n - 1, b, "fill");
    }

    // Eight edges: four 45-degree corners and four half-cell steps.
    put(G_SLOPE, font::half_plane(1, 1, 8), "slope-nw");
    put(G_SLOPE + 1, font::half_plane(-1, 1, 0), "slope-ne");
    put(G_SLOPE + 2, font::half_plane(1, -1, 0), "slope-sw");
    put(G_SLOPE + 3, font::half_plane(-1, -1, -8), "slope-se");
    put(G_SLOPE + 4, font::half_plane(1, 2, 12), "slope-shallow-l");
    put(G_SLOPE + 5, font::half_plane(-1, 2, 4), "slope-shallow-r");
    put(G_SLOPE + 6, font::half_plane(2, 1, 12), "slope-steep-l");
    put(G_SLOPE + 7, font::half_plane(-2, 1, 4), "slope-steep-r");

    // Facades.  Four configurations, coarse to fine, four lit patterns each.
    // The lit patterns are fixed rather than random so that a wall does not
    // change its mind between frames; variety comes from which tile a given
    // (lot, floor, bay) hashes to, not from the tile itself.
    const CFG: [(u32, u32); 4] = [(2, 2), (2, 4), (4, 4), (4, 2)];
    const LIT: [u8; 4] = [0xff, 0b1011_0110, 0b0110_1101, 0b1101_0011];
    for (c, &(bays, floors)) in CFG.iter().enumerate() {
        for (l, &lit) in LIT.iter().enumerate() {
            put(
                facade_tile(c as u8, l as u8),
                font::facade(bays, floors, lit),
                "facade",
            );
        }
    }

    put(G_MULLION, font::vrule(0, 1), "mullion-l");
    put(G_MULLION + 1, font::vrule(7, 1), "mullion-r");
    put(G_MULLION + 2, font::vrule(3, 2), "mullion-c");
    put(
        G_MULLION + 3,
        font::over(&font::vrule(0, 1), &font::vrule(7, 1)),
        "mullion-lr",
    );

    put(G_CORNICE, font::hrule(0, 1), "spandrel");
    put(G_CORNICE + 1, font::hrule(0, 3), "cornice");
    put(
        G_CORNICE + 2,
        font::over(&font::hrule(0, 2), &font::hrule(6, 2)),
        "ledge",
    );
    put(G_CORNICE + 3, font::hrule(6, 2), "parapet");

    // Fire escapes.  The zigzag is the readable signature of a prewar
    // building at any distance, so it gets its own glyphs rather than being
    // approximated by a dither: a diagonal run with a rail above it.
    let zig_r = font::over(&font::half_plane(-3, 4, 2), &font::half_plane(3, -4, -14));
    put(FIRE_ZIG_R, zig_r, "fire-zig-r");
    put(FIRE_ZIG_L, font::xform(&zig_r, Xform::FlipH), "fire-zig-l");
    put(
        FIRE_LANDING,
        font::over(&font::hrule(3, 1), &font::hrule(7, 1)),
        "fire-landing",
    );
    put(
        FIRE_RAIL,
        font::over(&font::hrule(3, 1), &font::vrule(6, 1)),
        "fire-rail",
    );
    let mut ladder = font::over(&font::vrule(2, 1), &font::vrule(5, 1));
    for r in [1u32, 3, 5, 7] {
        ladder = font::over(&ladder, &font::hrule(r, 1));
    }
    put(FIRE_LADDER, ladder, "fire-ladder");
    put(
        FIRE_BRACKET,
        font::over(&font::vrule(0, 1), &font::half_plane(3, 4, 20)),
        "fire-bracket",
    );

    put(G_ROOF, font::hrule(5, 3), "roof-parapet");
    put(
        G_ROOF + 1,
        font::over(&font::hrule(2, 3), &font::vrule(2, 4)),
        "roof-tank",
    );
    put(G_ROOF + 2, font::over(&font::hrule(4, 4), &font::vrule(1, 2)), "roof-plant");
    put(
        G_ROOF + 3,
        font::over(&font::vrule(3, 1), &font::hrule(0, 1)),
        "roof-mast",
    );

    put(ROAD_ASPHALT, font::dither(4), "asphalt");
    put(ROAD_DASH, font::hrule(3, 2), "lane-dash");
    put(
        ROAD_CENTRE,
        font::over(&font::hrule(2, 1), &font::hrule(5, 1)),
        "centre-line",
    );
    put(ROAD_CROSSING, font::vrule(1, 3), "crossing");
    put(ROAD_KERB, font::hrule(6, 2), "kerb");
    put(
        ROAD_PAVING,
        font::over(&font::hrule(0, 1), &font::vrule(0, 1)),
        "paving",
    );
    put(ROAD_GRATE, font::dither(24), "grate");
    put(ROAD_PUDDLE, font::hrule(4, 3), "puddle");

    put(FLORA_CANOPY, font::dither(44), "canopy");
    put(FLORA_LEAF, font::dither(20), "leaf");
    put(FLORA_TRUNK, font::vrule(3, 2), "trunk");
    put(FLORA_HEDGE, font::over(&font::dither(36), &font::hrule(7, 1)), "hedge");
    put(FLORA_GRASS, font::dither(12), "grass");
    put(
        FLORA_PLANTER,
        font::over(&font::hrule(5, 3), &font::dither(16)),
        "planter",
    );

    put(ST_POST, font::vrule(3, 1), "post");
    put(
        ST_LAMP,
        font::over(&font::hrule(1, 2), &font::vrule(3, 1)),
        "lamp",
    );
    put(
        ST_SIGNAL,
        font::over(&font::speck(5), &font::over(&font::speck(9), &font::vrule(3, 1))),
        "signal",
    );
    put(ST_SIGN, font::hrule(1, 4), "sign");
    put(ST_HYDRANT, font::over(&font::hrule(4, 4), &font::hrule(3, 1)), "hydrant");
    put(ST_BOLLARD, font::vrule(3, 2), "bollard");

    put(G_VEHICLE, font::hrule(4, 3), "car-body");
    put(G_VEHICLE + 1, font::speck(6), "car-light");
    put(G_VEHICLE + 2, font::hrule(2, 5), "bus");
    put(G_VEHICLE + 3, font::over(&font::hrule(4, 3), &font::hrule(2, 1)), "taxi");

    put(G_PED, font::over(&font::vrule(3, 1), &font::speck(5)), "ped");
    put(G_PED + 1, font::over(&font::vrule(4, 1), &font::speck(6)), "ped-alt");

    for p in 0..8u8 {
        put(G_RAIN + p, font::rain_streak(-3, p as u32), "rain");
    }

    for q in 0..4u8 {
        put(
            G_MOON + q,
            font::disc_quadrant(7, (q & 1) as u32, (q >> 1) as u32),
            "moon",
        );
        let halo = font::less(
            &font::disc_quadrant(8, (q & 1) as u32, (q >> 1) as u32),
            &font::disc_quadrant(7, (q & 1) as u32, (q >> 1) as u32),
        );
        put(G_HALO + q, halo, "halo");
    }

    for s in 0..8u8 {
        put(G_STAR + s, font::speck(s as u32 * 2 + 1), "star");
    }

    put(G_HAZE, font::speck(3), "haze-0");
    put(G_HAZE + 1, font::dither(2), "haze-1");
    put(G_HAZE + 2, font::dither(4), "haze-2");
    put(G_HAZE + 3, font::dither(6), "haze-3");

    Catalog { bitmaps: bm, names }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::coverage;

    #[test]
    fn every_slot_is_named() {
        let c = build();
        for i in 0..N_GLYPHS {
            assert!(!c.names[i].is_empty(), "glyph {i} has no name");
        }
    }

    #[test]
    fn only_blank_is_empty() {
        let c = build();
        for i in 0..N_GLYPHS {
            if i as GlyphId == G_BLANK {
                continue;
            }
            assert!(coverage(&c.bitmaps[i]) > 0, "glyph {i} ({}) is blank", c.names[i]);
        }
    }

    #[test]
    fn the_fill_ramp_climbs_by_eight_pixels() {
        let c = build();
        for n in 1..=8u8 {
            assert_eq!(coverage(&c.bitmaps[fill_step(n) as usize]), n as u32 * 8);
        }
        assert_eq!(fill_step(0), G_BLANK);
        assert_eq!(fill_step(20), G_SOLID);
    }

    #[test]
    fn the_shade_ramp_climbs_by_eight_pixels() {
        let c = build();
        for l in 1..=8u8 {
            assert_eq!(coverage(&c.bitmaps[shade(l) as usize]), l as u32 * 8);
        }
    }

    #[test]
    fn facade_indices_stay_inside_their_family() {
        for cfg in 0..4u8 {
            for lit in 0..4u8 {
                let i = facade_tile(cfg, lit);
                assert!((G_FACADE..G_MULLION).contains(&i));
            }
        }
    }

    #[test]
    fn the_catalogue_fits_a_plus4_charset_half() {
        assert!(PLUS4_BASE as usize + N_GLYPHS <= 256);
    }

    #[test]
    fn fire_escape_zigzags_mirror_each_other() {
        let c = build();
        assert_eq!(
            coverage(&c.bitmaps[FIRE_ZIG_L as usize]),
            coverage(&c.bitmaps[FIRE_ZIG_R as usize])
        );
        assert_ne!(c.bitmaps[FIRE_ZIG_L as usize], c.bitmaps[FIRE_ZIG_R as usize]);
    }
}

//! Rendering a catalogue index on a terminal.
//!
//! The Plus/4 draws catalogue index `n` by poking `n + 64` and letting the
//! TED read the baked charset.  A terminal cannot do that - it can only
//! print characters somebody else designed - so each index needs a stand-in
//! chosen from what the terminal has.
//!
//! Two modes, and the difference between them is a policy about what
//! "ASCII art" is allowed to mean:
//!
//! - [`Mode::Ascii`] uses only the 95 printable characters of 7-bit ASCII.
//!   Every one of them is on the keyboard in front of you.  This is the mode
//!   the project is named for, it is the one that survives `ssh` to anything,
//!   and it is the reference the other modes are judged against.
//! - [`Mode::Unicode`] uses the block-element and box-drawing ranges, whose
//!   quadrants and eighths are the same vocabulary the Commodore character
//!   ROM had - which is why the catalogue is built out of quadrants and
//!   eighths in the first place.
//!
//! The stand-ins are chosen by hand, not by matching bitmaps.  A terminal's
//! font is unknown at build time and `X` is not `X`-shaped in every one of
//! them, so an automatic match would be fitting to a guess.  What *is*
//! checked, in the tests below, is that each table is complete, that the
//! ASCII table stays inside 7 bits, and that the ramps within a family stay
//! monotonic - because a ramp that reverses is visible as a seam and a
//! wrong-but-consistent character is not.

use crate::catalog::{GlyphId, N_GLYPHS, PLUS4_BASE};

/// How a catalogue index becomes something the output device can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    /// 7-bit printable ASCII only.
    #[default]
    Ascii,
    /// Block elements and box drawing.
    Unicode,
}

impl Mode {
    /// Parse a mode name, for the command line.
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "ascii" | "tty" | "7bit" => Some(Mode::Ascii),
            "unicode" | "blocks" | "utf8" => Some(Mode::Unicode),
            _ => None,
        }
    }

    /// The character standing in for a catalogue index.
    #[inline(always)]
    pub fn glyph(self, g: GlyphId) -> char {
        let i = (g as usize).min(N_GLYPHS - 1);
        match self {
            Mode::Ascii => ASCII[i],
            Mode::Unicode => UNICODE[i],
        }
    }
}

/// The Plus/4 screen code for a catalogue index.  The mapping is an
/// addition, which is the point of laying the catalogue out this way.
#[inline(always)]
pub const fn screen_code(g: GlyphId) -> u8 {
    g.wrapping_add(PLUS4_BASE)
}

/// 7-bit ASCII stand-ins, indexed by catalogue index.
///
/// Ordered so that families read as ramps: the dithers climb
/// `. : - = + * #`, the eighth-fills climb `_ . - = + # @`, and the four
/// facade configurations are four visually distinct window textures rather
/// than four densities of the same one - a slab and a prewar block should
/// not be told apart only by brightness.
#[rustfmt::skip]
pub const ASCII: [char; N_GLYPHS] = [
    // 0 blank
    ' ',
    // 1..7 dither, lightest to heaviest
    '.', ':', '-', '=', '+', '*', '#',
    // 8 solid
    '@',
    // 9..23 quadrants, masks 1..15 (bit0 TL, bit1 TR, bit2 BL, bit3 BR)
    '`', '\'', '~', ',', '[', '/', 'F', '.',
    '\\', ']', '7', '_', 'L', 'J', '@',
    // 24..31 eighth-fills from the bottom up
    '_', '_', '.', '-', '=', '+', '#', '@',
    // 32..39 slopes: nw ne sw se, then shallow l/r, steep l/r
    '\\', '/', '/', '\\', '\\', '/', '\\', '/',
    // 40..55 facades: four configurations x four lit patterns
    '8', '0', 'o', '.',
    'H', '#', '=', '-',
    'X', 'x', '+', ':',
    'M', 'W', 'm', 'n',
    // 56..59 mullions
    '|', '|', '!', 'H',
    // 60..63 spandrel, cornice, ledge, parapet
    '~', '=', '"', '_',
    // 64..69 fire escape: zig r, zig l, landing, rail, ladder, bracket
    '/', '\\', '=', '|', '#', '[',
    // 70..73 roofscape: parapet, tank, plant, mast
    '^', 'T', 'n', '!',
    // 74..81 road: asphalt dash centre crossing kerb paving grate puddle
    '.', '-', '=', '|', '_', '+', ':', '~',
    // 82..87 flora: canopy leaf trunk hedge grass planter
    '%', '&', '|', 'w', '"', 'u',
    // 88..93 street: post lamp signal sign hydrant bollard
    '|', 'Y', '!', '#', 'h', 'i',
    // 94..97 vehicles: body light bus taxi
    '=', 'o', 'B', 'T',
    // 98..99 pedestrians
    'i', 'j',
    // 100..107 rain phases
    '/', '\'', '/', '`', '/', '\'', '/', '`',
    // 108..111 moon quadrants, reading order
    '/', '\\', '\\', '/',
    // 112..115 halo quadrants
    '.', '.', '.', '.',
    // 116..123 stars
    '.', '\'', '.', '`', '.', '\'', '.', '`',
    // 124..127 haze
    ' ', '.', '.', ':',
];

/// Block-element stand-ins, indexed by catalogue index.
#[rustfmt::skip]
pub const UNICODE: [char; N_GLYPHS] = [
    ' ',
    '░', '░', '▒', '▒', '▒', '▓', '▓',
    '█',
    // quadrants: these are exact - the catalogue's masks were numbered to
    // match this range's ordering
    '▘', '▝', '▀', '▖', '▌', '▞', '▛', '▗',
    '▚', '▐', '▜', '▄', '▙', '▟', '█',
    // eighth-fills: also exact
    '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█',
    // slopes
    '◤', '◥', '◣', '◢', '◤', '◥', '◣', '◢',
    // facades
    '▩', '▦', '▤', '░',
    '▥', '▨', '▧', '▒',
    '▦', '▩', '▒', '░',
    '▧', '▨', '▤', '▥',
    // mullions
    '▏', '▕', '│', '║',
    // spandrel, cornice, ledge, parapet
    '▔', '━', '═', '▂',
    // fire escape
    '╱', '╲', '┿', '╎', '╫', '┤',
    // roofscape
    '▀', '╤', '╥', '┃',
    // road
    '·', '╌', '═', '┃', '▁', '┼', '▒', '~',
    // flora
    '▓', '▒', '│', '▄', '▁', '▃',
    // street furniture
    '│', '╤', '╪', '▬', '╻', '╽',
    // vehicles
    '▬', '•', '▭', '▬',
    // pedestrians
    '╽', '╿',
    // rain
    '╱', '·', '╱', '·', '╱', '·', '╱', '·',
    // moon
    '▛', '▜', '▙', '▟',
    // halo
    '·', '·', '·', '·',
    // stars
    '·', '∙', '·', '∙', '·', '∙', '·', '∙',
    // haze
    ' ', '·', '░', '░',
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{fill_step, shade, G_DITHER, G_FILL, G_QUAD};

    #[test]
    fn ascii_table_is_seven_bit_and_printable() {
        for (i, &c) in ASCII.iter().enumerate() {
            assert!(
                (' '..='~').contains(&c),
                "glyph {i} maps to {c:?}, which is not a typeable ASCII character"
            );
        }
    }

    #[test]
    fn unicode_table_has_no_control_characters() {
        for (i, &c) in UNICODE.iter().enumerate() {
            assert!(!c.is_control(), "glyph {i} maps to a control character");
        }
    }

    #[test]
    fn both_tables_are_complete() {
        assert_eq!(ASCII.len(), N_GLYPHS);
        assert_eq!(UNICODE.len(), N_GLYPHS);
    }

    #[test]
    fn the_unicode_eighth_fills_are_the_real_ones() {
        let want = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        for (n, &w) in want.iter().enumerate() {
            assert_eq!(Mode::Unicode.glyph(G_FILL + n as u8), w);
        }
    }

    #[test]
    fn the_unicode_quadrants_are_the_real_ones() {
        // Mask 3 is top-left plus top-right, which is the upper half block.
        assert_eq!(Mode::Unicode.glyph(G_QUAD + 3 - 1), '▀');
        assert_eq!(Mode::Unicode.glyph(G_QUAD + 12 - 1), '▄');
        assert_eq!(Mode::Unicode.glyph(G_QUAD + 5 - 1), '▌');
        assert_eq!(Mode::Unicode.glyph(G_QUAD + 10 - 1), '▐');
    }

    #[test]
    fn the_ascii_dither_ramp_never_reverses() {
        // Hand-ranked visual weight of the characters the ramp uses.
        let weight = |c: char| " .:-=+*#@".find(c).expect("ramp uses an unranked character");
        let mut last = 0;
        for l in 0..=8u8 {
            let w = weight(Mode::Ascii.glyph(shade(l)));
            assert!(w >= last, "the ASCII dither ramp goes backwards at level {l}");
            last = w;
        }
        assert_eq!(Mode::Ascii.glyph(shade(0)), ' ');
        assert_eq!(Mode::Ascii.glyph(G_DITHER), '.');
    }

    #[test]
    fn the_fill_ramp_ends_solid_in_both_modes() {
        assert_eq!(Mode::Ascii.glyph(fill_step(8)), '@');
        assert_eq!(Mode::Unicode.glyph(fill_step(8)), '█');
    }

    #[test]
    fn screen_codes_stay_inside_the_charset() {
        for g in 0..N_GLYPHS as u8 {
            assert!(screen_code(g) >= PLUS4_BASE);
            assert!(screen_code(g) as usize <= 255);
        }
    }

    #[test]
    fn mode_names_parse() {
        assert_eq!(Mode::parse("ascii"), Some(Mode::Ascii));
        assert_eq!(Mode::parse("unicode"), Some(Mode::Unicode));
        assert_eq!(Mode::parse("nonsense"), None);
    }
}

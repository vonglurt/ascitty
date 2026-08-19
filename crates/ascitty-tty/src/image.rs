//! A frame as pixels.
//!
//! This is the one definition of what a picture of a frame looks like, and
//! both file formats - [`crate::png`] for a still, [`crate::gif`] for a
//! moving one - are containers around it.  Written once because two copies
//! of "what colour is this pixel" would start identical and drift.
//!
//! It is not a screenshot of a terminal.  A cell is a glyph and a colour
//! byte, a glyph is an 8x8 bitmap this program generated itself, and the
//! colour byte is the TED's - so blowing a cell up and painting the set
//! bits is not an approximation of the output, it *is* the output, drawn
//! the way the Plus/4 draws it and with none of a terminal's font in the
//! way.
//!
//! A pixel is a palette index, and the index *is* the renderer's colour
//! byte: both formats here take a 128-entry palette, so the picture data is
//! very nearly a dump of the frame buffer.

use ascitty_core::catalog::{self, Catalog};
use ascitty_core::frame::Frame;
use ascitty_core::palette;

/// A glyph is eight pixels wide.
pub const CELL_W: usize = 8;

/// And is drawn sixteen pixels tall: eight glyph rows, each drawn twice.
///
/// A character cell is about twice as tall as it is wide on a terminal and
/// on the Plus/4 alike - [`ascitty_core::raycast::CELL_ASPECT`] is the
/// renderer's name for it, and the projection is built around it - so a
/// picture with square cells is a picture of a city squashed flat.
pub const CELL_H: usize = 16;

/// Entries in the palette both formats carry.
pub const COLORS: usize = 128;

/// A frame turned into pixels.
pub struct Raster {
    /// Width in pixels.
    pub w: usize,
    /// Height in pixels.
    pub h: usize,
    /// One palette index per pixel, row-major.
    pub px: Vec<u8>,
}

/// Draw every cell of a frame into a buffer of palette indices.
pub fn raster(f: &Frame) -> Raster {
    let cat = catalog::build();
    let (w, h) = (f.w * CELL_W, f.h * CELL_H);
    let mut px = vec![palette::BLACK; w * h];
    for row in 0..f.h {
        for col in 0..f.w {
            let cel = f.get(col as i32, row as i32);
            if cel.color == palette::BLACK {
                continue; // already black, whatever the glyph is
            }
            draw_cell(&mut px, w, col, row, &cat, cel.glyph, cel.color);
        }
    }
    Raster { w, h, px }
}

/// One cell, eight bits wide and sixteen pixels tall.
///
/// Set bits take the cell's colour and clear ones are left black - black
/// rather than a background colour because that is what the renderer means.
/// A cell carries one colour, exactly as the TED's colour RAM does, and
/// everything this program draws is lit shapes against the night.
fn draw_cell(px: &mut [u8], w: usize, col: usize, row: usize, cat: &Catalog, g: u8, c: u8) {
    let bitmap = &cat.bitmaps[g as usize];
    for (line, bits) in bitmap.iter().enumerate() {
        if *bits == 0 {
            continue;
        }
        for x in 0..CELL_W {
            // Bit 7 is leftmost - the order the TED reads a character
            // definition, so no reversal here or in the baked font.
            if bits & (0x80 >> x) == 0 {
                continue;
            }
            let px_x = col * CELL_W + x;
            for dup in 0..CELL_H / 8 {
                let px_y = row * CELL_H + line * (CELL_H / 8) + dup;
                px[px_y * w + px_x] = c;
            }
        }
    }
}

/// The 128-entry TED palette as packed RGB triples, which is the layout
/// both a PNG `PLTE` chunk and a GIF colour table want.
pub fn palette() -> Vec<u8> {
    let mut t = Vec::with_capacity(COLORS * 3);
    for (r, g, b) in palette::rgb_table().iter() {
        t.extend_from_slice(&[*r, *g, *b]);
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use ascitty_core::catalog::{G_BLANK, G_SOLID};
    use ascitty_core::frame::Cel;

    #[test]
    fn a_blank_frame_is_entirely_black() {
        let r = raster(&Frame::new(3, 2));
        assert_eq!(r.w, 3 * CELL_W);
        assert_eq!(r.h, 2 * CELL_H);
        assert!(r.px.iter().all(|&p| p == palette::BLACK));
    }

    #[test]
    fn a_solid_cell_fills_its_whole_box_and_nothing_else() {
        let mut f = Frame::new(2, 1);
        f.put(0, 0, Cel { glyph: G_SOLID, color: 0x71 });
        f.put(1, 0, Cel { glyph: G_BLANK, color: 0x71 });
        let r = raster(&f);
        for y in 0..r.h {
            for x in 0..r.w {
                let want = if x < CELL_W { 0x71 } else { palette::BLACK };
                assert_eq!(r.px[y * r.w + x], want, "at {x},{y}");
            }
        }
    }

    #[test]
    fn the_glyph_is_drawn_the_right_way_up_and_round() {
        // The top-left quadrant, which is asymmetric in both directions and
        // so catches a flip either way.
        let mut f = Frame::new(1, 1);
        f.put(0, 0, Cel { glyph: catalog::G_QUAD, color: 0x71 });
        let r = raster(&f);
        let lit = |x: usize, y: usize| r.px[y * r.w + x] != palette::BLACK;
        let corners = [lit(0, 0), lit(r.w - 1, 0), lit(0, r.h - 1), lit(r.w - 1, r.h - 1)];
        assert_eq!(corners.iter().filter(|&&l| l).count(), 1, "not one lit corner: {corners:?}");
    }

    #[test]
    fn the_palette_is_the_whole_ted_table() {
        let p = palette();
        assert_eq!(p.len(), COLORS * 3);
        assert_eq!(&p[..3], &[0, 0, 0]); // black is index zero
    }
}

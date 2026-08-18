//! The frame buffer: one glyph and one colour per character cell.
//!
//! Two bytes a cell, which is exactly what the Plus/4 has - a screen byte
//! and a colour byte - so this buffer is not a host convenience that has to
//! be translated later.  It is the same thing the TED reads, laid out the
//! same way.

use crate::catalog::{GlyphId, G_BLANK};
use crate::palette::{Color, BLACK};

/// One character cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cel {
    /// Catalogue index.
    pub glyph: GlyphId,
    /// Packed `hue << 3 | luminance`.
    pub color: Color,
}

impl Cel {
    /// An empty cell.
    pub const EMPTY: Cel = Cel { glyph: G_BLANK, color: BLACK };
}

impl Default for Cel {
    fn default() -> Self {
        Cel::EMPTY
    }
}

/// A screenful.
#[derive(Clone)]
pub struct Frame {
    /// Columns.
    pub w: usize,
    /// Rows.
    pub h: usize,
    /// Cells, row-major.
    pub cels: Vec<Cel>,
}

impl Frame {
    /// A cleared frame.
    pub fn new(w: usize, h: usize) -> Frame {
        Frame { w, h, cels: vec![Cel::EMPTY; w * h] }
    }

    /// Resize, discarding contents.
    pub fn resize(&mut self, w: usize, h: usize) {
        if w != self.w || h != self.h {
            self.w = w;
            self.h = h;
            self.cels = vec![Cel::EMPTY; w * h];
        }
    }

    /// Clear to black.
    pub fn clear(&mut self) {
        self.cels.fill(Cel::EMPTY);
    }

    /// Write a cell, ignoring anything off-screen.
    #[inline(always)]
    pub fn put(&mut self, x: i32, y: i32, c: Cel) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.cels[y as usize * self.w + x as usize] = c;
        }
    }

    /// Read a cell, or [`Cel::EMPTY`] off-screen.
    #[inline(always)]
    pub fn get(&self, x: i32, y: i32) -> Cel {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.cels[y as usize * self.w + x as usize]
        } else {
            Cel::EMPTY
        }
    }

    /// Write a run of ASCII text, for the status line.  Characters outside
    /// the catalogue's reach are dropped rather than approximated.
    pub fn text(&mut self, x: i32, y: i32, s: &str, color: Color, map: impl Fn(char) -> Option<GlyphId>) {
        for (i, ch) in s.chars().enumerate() {
            if let Some(g) = map(ch) {
                self.put(x + i as i32, y, Cel { glyph: g, color });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_bounds_writes_are_dropped_not_wrapped() {
        let mut f = Frame::new(4, 3);
        let c = Cel { glyph: 9, color: 40 };
        f.put(-1, 0, c);
        f.put(4, 0, c);
        f.put(0, 3, c);
        assert!(f.cels.iter().all(|&x| x == Cel::EMPTY), "a write escaped the frame");
    }

    #[test]
    fn writes_land_where_they_are_put() {
        let mut f = Frame::new(4, 3);
        let c = Cel { glyph: 9, color: 40 };
        f.put(2, 1, c);
        assert_eq!(f.get(2, 1), c);
        assert_eq!(f.cels[4 + 2], c);
    }

    #[test]
    fn resize_only_reallocates_on_a_real_change() {
        let mut f = Frame::new(80, 24);
        f.put(0, 0, Cel { glyph: 5, color: 3 });
        f.resize(80, 24);
        assert_eq!(f.get(0, 0).glyph, 5, "a no-op resize threw the frame away");
        f.resize(40, 25);
        assert_eq!(f.cels.len(), 1000);
        assert_eq!(f.get(0, 0), Cel::EMPTY);
    }
}

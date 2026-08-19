//! Writing the demonstration out as an animated GIF.
//!
//! `make cast` records the demonstration as an asciinema `.cast`, which is
//! the right format for the thing - it is text with timestamps, it stays
//! sharp at any size, and it is a few hundred kilobytes.  It is also not
//! something a README on a web page can play.  This is the same
//! demonstration in the one moving format a web page will play anywhere.
//!
//! # Why the format fits
//!
//! GIF is a palette format with at most 256 colours, which is usually the
//! thing wrong with it.  Here the renderer has 128 and they go in the
//! colour table unchanged, so nothing is quantised, dithered or lost - the
//! picture in the file is exactly the picture on the screen, the same way
//! [`crate::png`] is for a still.
//!
//! # Only what moved
//!
//! Each frame is written as the smallest rectangle that differs from the
//! frame before it, and pixels inside that rectangle which did not change
//! are written as the transparent index so the previous frame shows
//! through.  A city at night is mostly black sky and the camera is often
//! turning slowly, so the difference is routinely a fraction of the screen.
//! Measured over a thirty-second demonstration: about a quarter of the size
//! of the same frames written whole.
//!
//! The one thing this costs is that the file must be played from the
//! beginning, which is what a README does anyway.
//!
//! # There is no GIF library here either
//!
//! The workspace has no dependencies.  GIF's compression is LZW over a
//! dictionary that starts as the palette and grows a code per string seen,
//! which is about eighty lines including the bit packing.

use crate::image::{self, Raster, COLORS};
use ascitty_core::frame::Frame;
use std::collections::HashMap;

/// The transparent index.
///
/// One past the palette, so it cannot collide with a colour the renderer
/// might choose.  The colour table is rounded up to 256 entries for it -
/// GIF only allows powers of two - and the entries past 128 are never
/// drawn.
const TRANSPARENT: u8 = COLORS as u8;

/// An animation being built up a frame at a time.
pub struct Gif {
    out: Vec<u8>,
    prev: Vec<u8>,
    w: usize,
    h: usize,
    /// Delay between frames, in hundredths of a second.
    delay: u16,
    frames: u32,
}

impl Gif {
    /// Start an animation that will play at `fps` and loop forever.
    ///
    /// The delay is in hundredths of a second, which is all GIF can say, so
    /// the frame rate is rounded to whatever that allows: 20 fps is exact,
    /// 30 is not and becomes 33.
    pub fn new(fps: u32) -> Gif {
        Gif {
            out: Vec::new(),
            prev: Vec::new(),
            w: 0,
            h: 0,
            delay: (100 / fps.clamp(1, 50)) as u16,
            frames: 0,
        }
    }

    /// How many frames have been added.
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// Add a frame.
    ///
    /// The first one sizes the animation and writes the header; any later
    /// frame of a different size is ignored, because a GIF cannot change
    /// size part way through and a terminal being resized mid-recording is
    /// not worth failing a build over.
    pub fn push(&mut self, f: &Frame) {
        let Raster { w, h, px } = image::raster(f);
        if self.frames == 0 {
            self.w = w;
            self.h = h;
            self.header();
        } else if w != self.w || h != self.h {
            return;
        }
        self.frames += 1;

        let (x0, y0, x1, y1) = match self.changed(&px) {
            Some(r) => r,
            // Nothing moved.  Rather than write an empty frame, hold the
            // previous one on screen for another interval.
            None => {
                self.hold();
                return;
            }
        };
        let (fw, fh) = (x1 - x0, y1 - y0);

        // Inside the rectangle, unchanged pixels are transparent.
        let mut sub = Vec::with_capacity(fw * fh);
        for y in y0..y1 {
            for x in x0..x1 {
                let i = y * w + x;
                sub.push(if self.prev.is_empty() || self.prev[i] != px[i] {
                    px[i]
                } else {
                    TRANSPARENT
                });
            }
        }

        self.control(self.delay);
        self.descriptor(x0, y0, fw, fh);
        self.out.extend_from_slice(&lzw(&sub, 8));
        self.prev = px;
    }

    /// Finish the file.
    pub fn finish(mut self) -> Vec<u8> {
        if self.frames == 0 {
            self.header();
        }
        self.out.push(0x3b); // trailer
        self.out
    }

    /// Header, colour table, and the loop-forever extension.
    fn header(&mut self) {
        self.out.extend_from_slice(b"GIF89a");
        self.out.extend_from_slice(&(self.w as u16).to_le_bytes());
        self.out.extend_from_slice(&(self.h as u16).to_le_bytes());
        // Global table present, 8 bits per colour, 256 entries.
        self.out.extend_from_slice(&[0xf7, 0, 0]);
        let mut table = image::palette();
        table.resize(256 * 3, 0);
        self.out.extend_from_slice(&table);
        // NETSCAPE2.0: the de facto way to say "loop forever".
        self.out.extend_from_slice(b"\x21\xff\x0bNETSCAPE2.0\x03\x01\x00\x00\x00");
    }

    /// Graphic control extension: how long this frame stays up, that the
    /// next one is drawn over it, and which index means "leave what is
    /// already there".
    fn control(&mut self, delay: u16) {
        self.out.extend_from_slice(&[0x21, 0xf9, 0x04, 0x05]);
        self.out.extend_from_slice(&delay.to_le_bytes());
        self.out.extend_from_slice(&[TRANSPARENT, 0x00]);
    }

    /// Image descriptor: where this frame's rectangle goes.
    fn descriptor(&mut self, x: usize, y: usize, w: usize, h: usize) {
        self.out.push(0x2c);
        self.out.extend_from_slice(&(x as u16).to_le_bytes());
        self.out.extend_from_slice(&(y as u16).to_le_bytes());
        self.out.extend_from_slice(&(w as u16).to_le_bytes());
        self.out.extend_from_slice(&(h as u16).to_le_bytes());
        self.out.push(0x00); // no local table, not interlaced
    }

    /// Hold the previous frame for another interval, as a one-pixel
    /// transparent frame - which is the cheapest thing GIF can say.
    fn hold(&mut self) {
        self.control(self.delay);
        self.descriptor(0, 0, 1, 1);
        self.out.extend_from_slice(&lzw(&[TRANSPARENT], 8));
    }

    /// The bounding box of everything that differs from the previous frame,
    /// or `None` if nothing does.
    fn changed(&self, px: &[u8]) -> Option<(usize, usize, usize, usize)> {
        if self.prev.is_empty() {
            return Some((0, 0, self.w, self.h));
        }
        let (mut x0, mut y0, mut x1, mut y1) = (self.w, self.h, 0usize, 0usize);
        for y in 0..self.h {
            let row = y * self.w;
            for x in 0..self.w {
                if px[row + x] != self.prev[row + x] {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
        if x1 > x0 {
            Some((x0, y0, x1, y1))
        } else {
            None
        }
    }
}

/// GIF's LZW: the data block, ready to append.
///
/// The dictionary starts as one code per palette entry plus a clear code
/// and an end code, and grows by one code for every string seen twice.  The
/// code width grows with it, from `min + 1` bits up to twelve, at which
/// point the dictionary is cleared and it starts again - which is not an
/// optimisation, it is the only thing the format allows.
fn lzw(data: &[u8], min_code: u8) -> Vec<u8> {
    let clear = 1u16 << min_code;
    let end = clear + 1;

    let mut out = vec![min_code];
    let mut bits = BitWriter::default();
    let mut dict: HashMap<(u16, u8), u16> = HashMap::new();
    let mut next = end + 1;
    let mut width = min_code as u32 + 1;

    bits.push(clear as u32, width);
    let mut run: Option<u16> = None;
    for &b in data {
        let prefix = match run {
            None => {
                run = Some(b as u16);
                continue;
            }
            Some(p) => p,
        };
        match dict.get(&(prefix, b)) {
            Some(&code) => run = Some(code),
            None => {
                bits.push(prefix as u32, width);
                dict.insert((prefix, b), next);
                next += 1;
                if next as u32 > (1 << width) && width < 12 {
                    width += 1;
                } else if next == 4096 {
                    bits.push(clear as u32, width);
                    dict.clear();
                    next = end + 1;
                    width = min_code as u32 + 1;
                }
                run = Some(b as u16);
            }
        }
    }
    if let Some(p) = run {
        bits.push(p as u32, width);
    }
    bits.push(end as u32, width);

    // The data goes out in sub-blocks of at most 255 bytes, each with its
    // length in front, and a zero byte ends the lot.
    let packed = bits.finish();
    for part in packed.chunks(255) {
        out.push(part.len() as u8);
        out.extend_from_slice(part);
    }
    out.push(0);
    out
}

/// A stream of codes, least significant bit first.
#[derive(Default)]
struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl BitWriter {
    fn push(&mut self, v: u32, n: u32) {
        self.acc |= v << self.n;
        self.n += n;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ascitty_core::catalog::G_SOLID;
    use ascitty_core::frame::Cel;

    fn one_cell(color: u8) -> Frame {
        let mut f = Frame::new(2, 2);
        f.put(0, 0, Cel { glyph: G_SOLID, color });
        f
    }

    #[test]
    fn it_writes_something_that_starts_and_ends_like_a_gif() {
        let mut g = Gif::new(20);
        g.push(&one_cell(0x71));
        let out = g.finish();
        assert_eq!(&out[..6], b"GIF89a");
        assert_eq!(u16::from_le_bytes(out[6..8].try_into().unwrap()), 2 * 8);
        assert_eq!(u16::from_le_bytes(out[8..10].try_into().unwrap()), 2 * 16);
        assert_eq!(*out.last().unwrap(), 0x3b);
    }

    #[test]
    fn an_empty_animation_is_still_a_valid_file() {
        let out = Gif::new(20).finish();
        assert_eq!(&out[..6], b"GIF89a");
        assert_eq!(*out.last().unwrap(), 0x3b);
    }

    #[test]
    fn a_still_animation_costs_almost_nothing_per_frame() {
        let mut g = Gif::new(20);
        let f = one_cell(0x71);
        g.push(&f);
        let first = {
            let mut probe = Gif::new(20);
            probe.push(&f);
            probe.finish().len()
        };
        for _ in 0..50 {
            g.push(&f);
        }
        let out = g.finish();
        assert_eq!(g_frames(&out), 51, "not every frame was written");
        // Twenty-four bytes measured: the control extension, a one-pixel
        // image descriptor, and four bytes of compressed nothing.
        assert!(out.len() < first + 51 * 32, "a held frame cost {} bytes", (out.len() - first) / 51);
    }

    #[test]
    fn every_frame_is_recorded() {
        let mut g = Gif::new(20);
        for i in 0..8u8 {
            g.push(&one_cell(0x10 | (i & 0x0f)));
        }
        assert_eq!(g.frames(), 8);
        assert_eq!(g_frames(&g.finish()), 8);
    }

    /// How many image descriptors a file contains.
    ///
    /// Counted by walking the blocks rather than by searching for the
    /// descriptor byte, because 0x2c is also a perfectly ordinary byte in
    /// the middle of compressed data.
    fn g_frames(out: &[u8]) -> u32 {
        let mut i = 13 + 256 * 3; // header, screen descriptor, colour table
        let mut n = 0;
        while i < out.len() {
            match out[i] {
                0x3b => break,
                0x21 => {
                    i += 2; // extension introducer and label
                    while out[i] != 0 {
                        i += 1 + out[i] as usize; // a sub-block
                    }
                    i += 1;
                }
                0x2c => {
                    n += 1;
                    i += 10; // introducer, position, size, flags
                    i += 1; // minimum code size
                    while out[i] != 0 {
                        i += 1 + out[i] as usize;
                    }
                    i += 1;
                }
                b => panic!("unknown block {b:#x} at {i}"),
            }
        }
        n
    }

    #[test]
    fn the_compressed_codes_decode_to_what_went_in() {
        let cases: Vec<Vec<u8>> = vec![
            vec![0],
            vec![5; 10_000],
            (0..=255u8).cycle().take(9_000).collect(),
            (0..40_000u32).map(|i| (i % 7) as u8 * 11).collect(),
        ];
        for case in cases {
            let packed = lzw(&case, 8);
            assert_eq!(unlzw(&packed), case, "a {} byte case did not survive", case.len());
        }
    }

    /// An LZW decoder, for the round-trip test only.
    fn unlzw(block: &[u8]) -> Vec<u8> {
        let min = block[0] as u32;
        // Undo the sub-block framing.
        let mut data = Vec::new();
        let mut i = 1;
        while block[i] != 0 {
            let n = block[i] as usize;
            data.extend_from_slice(&block[i + 1..i + 1 + n]);
            i += 1 + n;
        }

        let clear = 1u16 << min;
        let end = clear + 1;
        let mut table: Vec<Vec<u8>> = (0..=end).map(|c| vec![c as u8]).collect();
        let mut width = min + 1;
        let mut out = Vec::new();
        let mut prev: Option<u16> = None;
        let (mut acc, mut n, mut pos) = (0u32, 0u32, 0usize);
        loop {
            while n < width && pos < data.len() {
                acc |= (data[pos] as u32) << n;
                n += 8;
                pos += 1;
            }
            if n < width {
                break;
            }
            let code = (acc & ((1 << width) - 1)) as u16;
            acc >>= width;
            n -= width;

            if code == clear {
                table.truncate(end as usize + 1);
                width = min + 1;
                prev = None;
                continue;
            }
            if code == end {
                break;
            }
            let entry = if (code as usize) < table.len() {
                table[code as usize].clone()
            } else {
                let p = table[prev.expect("a first code cannot be deferred") as usize].clone();
                let mut e = p.clone();
                e.push(p[0]);
                e
            };
            out.extend_from_slice(&entry);
            if let Some(p) = prev {
                let mut grown = table[p as usize].clone();
                grown.push(entry[0]);
                table.push(grown);
                if table.len() as u32 + 1 > (1 << width) && width < 12 {
                    width += 1;
                }
            }
            prev = Some(code);
        }
        out
    }
}

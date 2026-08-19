//! Writing a frame out as a PNG.
//!
//! A README on a web page cannot show a terminal, and the plain-text shots
//! the documentation uses throw away the colour, which is half of what the
//! renderer does.  This writes the frame as a picture instead.  What a cell
//! looks like as pixels is [`crate::image`]; this file is the container.
//!
//! # There is no PNG library here
//!
//! The workspace has no dependencies and this file does not add one.  A PNG
//! is four chunks and a zlib stream, and the zlib stream is the only part
//! with any work in it: the encoder below is LZ77 against a 32 KB window
//! with a three-byte hash chain, emitted with deflate's *fixed* Huffman
//! codes.  Fixed codes cost a few per cent against building an optimal tree
//! and save the entire tree-building half of a deflate encoder.
//!
//! On these pictures - large flat fields of black with glyph edges through
//! them - that lands around thirty to one, which is the difference between
//! a repository with pictures in it and one nobody wants to clone.

use crate::image::{self, Raster};
use ascitty_core::frame::Frame;

/// Render a frame as an indexed-colour PNG.
///
/// The palette is the 128-entry TED table, so the index of a pixel *is* the
/// colour byte the renderer chose for that cell - which makes the file a
/// fairly direct dump of the frame buffer rather than a picture of one.
pub fn encode(f: &Frame) -> Vec<u8> {
    let Raster { w, h, px } = image::raster(f);

    // One byte per pixel, plus the filter byte every scanline has to carry.
    // Filter zero - none - throughout: the bytes are palette indices, and
    // the difference between two palette indices does not mean anything.
    let mut raw = Vec::with_capacity((w + 1) * h);
    for y in 0..h {
        raw.push(0);
        raw.extend_from_slice(&px[y * w..(y + 1) * w]);
    }

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 3, 0, 0, 0]); // 8 bits, indexed, no interlace

    let mut png = Vec::from(*b"\x89PNG\r\n\x1a\n");
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"PLTE", &image::palette());
    chunk(&mut png, b"IDAT", &zlib(&raw));
    chunk(&mut png, b"IEND", &[]);
    png
}

/// Append a PNG chunk: length, type, payload, CRC of type and payload.
fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(body);
    let mut crc = Crc::new();
    crc.eat(tag);
    crc.eat(body);
    out.extend_from_slice(&crc.done().to_be_bytes());
}

/// Wrap deflated data in a zlib stream: two header bytes and an Adler-32.
fn zlib(data: &[u8]) -> Vec<u8> {
    // 0x78 0x01: deflate, 32 KB window, no preset dictionary, and a check
    // value that makes the pair a multiple of 31.
    let mut out = vec![0x78, 0x01];
    out.extend_from_slice(&deflate(data));
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

// --- deflate ---------------------------------------------------------------

/// Window size, and the mask that wraps an index into it.
const WINDOW: usize = 32 * 1024;
/// Longest match deflate can encode.
const MAX_MATCH: usize = 258;
/// Shortest match worth encoding: two bytes cost more as a match than as
/// themselves.
const MIN_MATCH: usize = 3;
/// How far back along a hash chain to look before taking what we have.
///
/// The chain is in most-recent-first order, and a nearer match encodes in
/// fewer bits as well as being found sooner, so the first few candidates are
/// where nearly all of the compression is.  Sixty-four is well past the knee
/// on this kind of picture and keeps a full-size shot under a second.
const CHAIN: usize = 64;
/// Entries in the hash table.
const HASH: usize = 1 << 15;

/// Deflate `data` into a single fixed-Huffman block.
fn deflate(data: &[u8]) -> Vec<u8> {
    let mut bits = BitWriter::default();
    bits.push(1, 1); // final block
    bits.push(1, 2); // fixed Huffman

    // head[h] is the most recent position whose three bytes hash to h, and
    // prev[p & mask] is the position before that one.  Two arrays and no
    // allocation per byte, which is the whole of an LZ77 match finder.
    let mut head = vec![usize::MAX; HASH];
    let mut prev = vec![usize::MAX; WINDOW];
    let mask = WINDOW - 1;

    let mut i = 0;
    while i < data.len() {
        let (mut best_len, mut best_dist) = (0, 0);
        if i + MIN_MATCH <= data.len() {
            let h = hash(&data[i..]);
            let mut cand = head[h];
            let floor = i.saturating_sub(WINDOW);
            for _ in 0..CHAIN {
                if cand == usize::MAX || cand < floor {
                    break;
                }
                let len = match_len(data, cand, i);
                if len > best_len {
                    best_len = len;
                    best_dist = i - cand;
                    if len == MAX_MATCH {
                        break;
                    }
                }
                cand = prev[cand & mask];
            }
        }

        if best_len >= MIN_MATCH {
            emit_match(&mut bits, best_len, best_dist);
            // Every position inside the match still has to be registered, or
            // the next match cannot start in the middle of this one.
            for k in 0..best_len {
                insert(data, i + k, &mut head, &mut prev, mask);
            }
            i += best_len;
        } else {
            emit_literal(&mut bits, data[i]);
            insert(data, i, &mut head, &mut prev, mask);
            i += 1;
        }
    }

    emit_code(&mut bits, 256); // end of block
    bits.finish()
}

/// Record position `p` in the chain for whatever its three bytes hash to.
#[inline]
fn insert(data: &[u8], p: usize, head: &mut [usize], prev: &mut [usize], mask: usize) {
    if p + MIN_MATCH <= data.len() {
        let h = hash(&data[p..]);
        prev[p & mask] = head[h];
        head[h] = p;
    }
}

/// Hash the three bytes at the front of `s` into the table.
#[inline]
fn hash(s: &[u8]) -> usize {
    let v = (s[0] as u32) << 16 | (s[1] as u32) << 8 | s[2] as u32;
    // Knuth's multiplicative hash, taken from the top bits.
    ((v.wrapping_mul(2_654_435_761)) >> 17) as usize & (HASH - 1)
}

/// How many bytes match between `a` and `b`, capped and never running past
/// the end of the data.
#[inline]
fn match_len(data: &[u8], a: usize, b: usize) -> usize {
    let cap = MAX_MATCH.min(data.len() - b);
    let mut n = 0;
    while n < cap && data[a + n] == data[b + n] {
        n += 1;
    }
    n
}

/// Lengths 3..258 in deflate's 29 codes: the base length of each, and how
/// many extra bits follow it.
const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// Distances 1..32768 in deflate's 30 codes, the same way.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Write one literal byte.
fn emit_literal(bits: &mut BitWriter, b: u8) {
    emit_code(bits, b as u16);
}

/// Write a back-reference: a length code, then a distance code.
fn emit_match(bits: &mut BitWriter, len: usize, dist: usize) {
    let l = LEN_BASE.iter().rposition(|&b| b as usize <= len).unwrap();
    emit_code(bits, 257 + l as u16);
    bits.push((len - LEN_BASE[l] as usize) as u32, LEN_EXTRA[l] as u32);

    let d = DIST_BASE.iter().rposition(|&b| b as usize <= dist).unwrap();
    // Distance codes are five plain bits under the fixed scheme, most
    // significant first like every other Huffman code here.
    bits.push_rev(d as u32, 5);
    bits.push((dist - DIST_BASE[d] as usize) as u32, DIST_EXTRA[d] as u32);
}

/// Write a literal or length symbol in deflate's fixed code.
///
/// The four ranges are the fixed table, and they are not contiguous: 0-143
/// are eight bits, 144-255 are nine, 256-279 are seven and 280-287 are eight
/// again.  Codes go out most significant bit first; the extra bits that
/// follow a length or a distance go out least significant bit first, which
/// is the one genuinely confusing thing about deflate.
fn emit_code(bits: &mut BitWriter, sym: u16) {
    let (code, n) = match sym {
        0..=143 => (0x30 + sym as u32, 8),
        144..=255 => (0x190 + sym as u32 - 144, 9),
        256..=279 => (sym as u32 - 256, 7),
        _ => (0xc0 + sym as u32 - 280, 8),
    };
    bits.push_rev(code, n);
}

/// A stream of bits, least significant first within each byte - which is how
/// deflate packs, and the opposite of how it writes Huffman codes.
#[derive(Default)]
struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl BitWriter {
    /// Push `n` bits of `v`, least significant bit first.
    fn push(&mut self, v: u32, n: u32) {
        self.acc |= v << self.n;
        self.n += n;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// Push `n` bits of `v`, most significant bit first - for Huffman codes.
    fn push_rev(&mut self, v: u32, n: u32) {
        for i in (0..n).rev() {
            self.push((v >> i) & 1, 1);
        }
    }

    /// Flush the last partial byte.
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

// --- checksums -------------------------------------------------------------

/// Adler-32, which is what a zlib stream ends with.
fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    b << 16 | a
}

/// CRC-32, which is what every PNG chunk ends with.
struct Crc(u32);

impl Crc {
    fn new() -> Crc {
        Crc(0xffff_ffff)
    }

    fn eat(&mut self, data: &[u8]) {
        for &x in data {
            let mut c = (self.0 ^ x as u32) & 0xff;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            self.0 = c ^ (self.0 >> 8);
        }
    }

    fn done(self) -> u32 {
        self.0 ^ 0xffff_ffff
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_writes_something_that_starts_like_a_png() {
        let png = encode(&Frame::new(4, 2));
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        // Eight pixels wide per cell and sixteen tall.
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 32);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 32);
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }

    #[test]
    fn the_deflate_stream_round_trips() {
        // Every kind of input the encoder can meet: incompressible, one long
        // run, and text with matches at every distance.
        let noise: Vec<u8> = (0..5000u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
        let flat = vec![7u8; 70_000];
        let mixed: Vec<u8> = "the cab takes fares on its own, "
            .repeat(400)
            .into_bytes();
        for case in [Vec::new(), vec![0u8], noise, flat, mixed] {
            let packed = deflate(&case);
            assert_eq!(inflate(&packed), case, "a {} byte case did not survive", case.len());
        }
    }

    #[test]
    fn a_black_frame_is_far_smaller_than_the_pixels_it_stands_for() {
        let png = encode(&Frame::new(150, 44));
        let pixels = image::raster(&Frame::new(150, 44)).px.len();
        assert!(png.len() * 20 < pixels, "{} bytes for {pixels} pixels", png.len());
    }

    #[test]
    fn the_checksums_are_the_standard_ones() {
        // Both figures are the ones every implementation of these agrees on
        // for "abc", which is the point of quoting a known answer.
        assert_eq!(adler32(b"abc"), 0x024d0127);
        let mut c = Crc::new();
        c.eat(b"abc");
        assert_eq!(c.done(), 0x352441c2);
    }

    /// A deflate *decoder*, for the round-trip test only.
    ///
    /// Only the fixed-Huffman case, because that is the only case the
    /// encoder emits, and a test that decoded more than the encoder writes
    /// would be testing something this program does not do.
    fn inflate(data: &[u8]) -> Vec<u8> {
        let mut r = Reader { data, pos: 0, bit: 0 };
        let mut out = Vec::new();
        assert_eq!(r.bits(1), 1, "not a final block");
        assert_eq!(r.bits(2), 1, "not a fixed-Huffman block");
        loop {
            let sym = r.symbol();
            match sym {
                256 => break,
                0..=255 => out.push(sym as u8),
                _ => {
                    let l = sym as usize - 257;
                    let len = LEN_BASE[l] as usize + r.bits(LEN_EXTRA[l] as u32) as usize;
                    let d = r.bits_rev(5) as usize;
                    let dist = DIST_BASE[d] as usize + r.bits(DIST_EXTRA[d] as u32) as usize;
                    for _ in 0..len {
                        let b = out[out.len() - dist];
                        out.push(b);
                    }
                }
            }
        }
        out
    }

    struct Reader<'a> {
        data: &'a [u8],
        pos: usize,
        bit: u32,
    }

    impl Reader<'_> {
        fn bit(&mut self) -> u32 {
            let b = (self.data[self.pos] >> self.bit) & 1;
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.pos += 1;
            }
            b as u32
        }

        /// `n` bits, least significant first: the extra bits after a code.
        fn bits(&mut self, n: u32) -> u32 {
            let mut v = 0;
            for i in 0..n {
                v |= self.bit() << i;
            }
            v
        }

        /// `n` bits, most significant first: a Huffman code.
        fn bits_rev(&mut self, n: u32) -> u32 {
            let mut v = 0;
            for _ in 0..n {
                v = v << 1 | self.bit();
            }
            v
        }

        /// One literal or length symbol, by the fixed code's ranges.
        fn symbol(&mut self) -> u16 {
            let c = self.bits_rev(7);
            if c <= 0b0010111 {
                return 256 + c as u16;
            }
            let c = c << 1 | self.bit();
            if (0x30..=0xbf).contains(&c) {
                return (c - 0x30) as u16;
            }
            if (0xc0..=0xc7).contains(&c) {
                return 280 + (c - 0xc0) as u16;
            }
            let c = c << 1 | self.bit();
            144 + (c - 0x190) as u16
        }
    }
}

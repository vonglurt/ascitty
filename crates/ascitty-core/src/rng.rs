//! A xorshift32 the 6502 can also run.
//!
//! The city is generated, not stored, so the generator has to produce the
//! same city on both targets from the same seed.  xorshift32 is three
//! shifts and three XORs on 32 bits, which cc65 compiles into something
//! survivable and which cannot drift the way a libc `rand()` would.

/// A deterministic 32-bit generator.
#[derive(Clone, Copy, Debug)]
pub struct Rng(u32);

impl Rng {
    /// Seed the generator.  Zero is remapped, since xorshift's zero state
    /// is a fixed point that only ever produces zero.
    pub const fn new(seed: u32) -> Self {
        Rng(if seed == 0 { 0x9e37_79b9 } else { seed })
    }

    /// Next raw word.
    #[inline(always)]
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Next value in `0..n`.  Uses the high bits, which mix best.
    #[inline(always)]
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        ((self.next_u32() as u64 * n as u64) >> 32) as u32
    }

    /// Next value in `lo..=hi`.
    #[inline(always)]
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo + 1).max(1) as u32) as i32
    }

    /// True with probability `num/den`.
    #[inline(always)]
    pub fn chance(&mut self, num: u32, den: u32) -> bool {
        self.below(den) < num
    }
}

/// A stateless hash, for "what does *this* cell look like" questions that
/// must answer the same way every frame without storing anything.
///
/// This is the whole reason facades are not stored: a 24-storey tower is
/// 24 rows of windows per face, and the city has hundreds of towers.  The
/// window at (lot, face, floor, bay) is a hash lookup, so the facade costs
/// no memory at all.
#[inline(always)]
pub const fn hash3(a: u32, b: u32, c: u32) -> u32 {
    let mut h = a.wrapping_mul(0x9e37_79b1);
    h ^= b.wrapping_mul(0x85eb_ca6b);
    h = h.rotate_left(13);
    h ^= c.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_f491);
    h ^ (h >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_gets_stuck_at_zero() {
        let mut r = Rng::new(0);
        for _ in 0..64 {
            assert_ne!(r.next_u32(), 0);
        }
    }

    #[test]
    fn same_seed_same_city() {
        let a: Vec<u32> = (0..64).scan(Rng::new(7), |r, _| Some(r.next_u32())).collect();
        let b: Vec<u32> = (0..64).scan(Rng::new(7), |r, _| Some(r.next_u32())).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn below_stays_in_range() {
        let mut r = Rng::new(1);
        for _ in 0..4096 {
            assert!(r.below(7) < 7);
        }
    }

    #[test]
    fn hash_decorrelates_neighbours() {
        // Adjacent cells must not produce adjacent-looking windows.
        let a = hash3(10, 10, 0);
        let b = hash3(10, 11, 0);
        assert_ne!(a >> 24, b >> 24);
    }
}

//! Angles, and the sine table both targets share.
//!
//! An angle is a `u16` covering one full turn, so it wraps for free on
//! overflow and there is never a range reduction to get wrong.  The table
//! has 1024 entries; the Plus/4 build bakes the same curve at 256 entries
//! (see `ascitty-bake`), which is 1.4 degrees of angular resolution - about
//! a third of a character cell at the edge of a 40-column screen, so the
//! coarser table is not visible.

use crate::fixed::{self, Fx};

/// One full turn.  `Ang` is deliberately `u16`: it wraps.
pub type Ang = u16;

/// Quarter turn.
pub const QUARTER: Ang = 0x4000;
/// Half turn.
pub const HALF: Ang = 0x8000;

/// Entries in the host sine table.
pub const TRIG_LEN: usize = 1024;
const SHIFT: u32 = 16 - 10; // u16 angle -> 1024-entry index

static SIN_TABLE: std::sync::OnceLock<[Fx; TRIG_LEN]> = std::sync::OnceLock::new();

/// The shared sine table, built once on first use.
pub fn sin_table() -> &'static [Fx; TRIG_LEN] {
    SIN_TABLE.get_or_init(|| {
        let mut t = [0; TRIG_LEN];
        for (i, slot) in t.iter_mut().enumerate() {
            let theta = (i as f64) * std::f64::consts::TAU / TRIG_LEN as f64;
            *slot = fixed::from_f64(theta.sin());
        }
        t
    })
}

/// Sine of an angle.
#[inline(always)]
pub fn sin(a: Ang) -> Fx {
    sin_table()[(a >> SHIFT) as usize]
}

/// Cosine of an angle.
#[inline(always)]
pub fn cos(a: Ang) -> Fx {
    sin(a.wrapping_add(QUARTER))
}

/// Convert degrees to an angle.  Setup and tests only.
pub fn from_degrees(deg: f64) -> Ang {
    ((deg / 360.0 * 65536.0).round() as i64 & 0xffff) as Ang
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixed::ONE;

    #[test]
    fn cardinal_points_are_exact_enough() {
        assert_eq!(sin(0), 0);
        assert!((sin(QUARTER) - ONE).abs() <= 1);
        assert!(sin(HALF).abs() <= 64);
        assert!((sin(HALF + QUARTER) + ONE).abs() <= 1);
    }

    #[test]
    fn identity_holds_across_the_table() {
        for i in 0..TRIG_LEN {
            let a = (i << SHIFT) as Ang;
            let s = sin(a);
            let c = cos(a);
            let sum = fixed::mul(s, s) + fixed::mul(c, c);
            // Two Q16.16 squares and a table quantised at 1024 steps.
            assert!((sum - ONE).abs() < 64, "sin^2+cos^2 off at {i}: {sum}");
        }
    }

    #[test]
    fn angles_wrap_rather_than_overflow() {
        assert_eq!(sin(0), sin(0u16.wrapping_sub(0)));
        let a: Ang = 0xffff;
        let _ = sin(a.wrapping_add(1)); // must not panic
    }
}

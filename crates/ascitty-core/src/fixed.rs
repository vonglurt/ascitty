//! Q16.16 fixed point.
//!
//! Every number the renderer carries through a frame is one of these.  Not
//! because the host cannot afford floats - it can - but because the 6502
//! target cannot, and a renderer written twice in two number systems is a
//! renderer whose two halves cannot be diffed against each other.  The
//! Plus/4 build narrows the same quantities to Q8.8; the *shape* of the
//! arithmetic is identical, so a disagreement between the two is a bug in
//! one of them rather than a property of the number type.
//!
//! Q16.16 means bit 16 is the ones place: `ONE` is `0x0001_0000`.  The
//! integer range is +-32768 cells, which is 512x the widest city the
//! generator will build, and the fractional resolution is 1/65536 of a
//! cell - about 0.2 mm if a cell is a 12 m city lot.

/// A Q16.16 fixed-point number.
pub type Fx = i32;

/// Number of fractional bits.
pub const FRAC: u32 = 16;
/// 1.0
pub const ONE: Fx = 1 << FRAC;
/// 0.5
pub const HALF: Fx = ONE / 2;
/// The largest representable value, used as "no hit".
pub const FX_MAX: Fx = Fx::MAX;

/// Whole number to fixed point.
#[inline(always)]
pub const fn from_int(i: i32) -> Fx {
    i << FRAC
}

/// Fixed point to whole number, rounding towards negative infinity.
#[inline(always)]
pub const fn floor(a: Fx) -> i32 {
    a >> FRAC
}

/// The fractional part, always in `0..ONE` even for negatives.
#[inline(always)]
pub const fn frac(a: Fx) -> Fx {
    a & (ONE - 1)
}

/// `a * b`, via a 64-bit intermediate so the product cannot overflow.
#[inline(always)]
pub const fn mul(a: Fx, b: Fx) -> Fx {
    (((a as i64) * (b as i64)) >> FRAC) as Fx
}

/// `a / b`.  Division by zero yields `FX_MAX` rather than trapping, because
/// the one place it can happen is a ray exactly parallel to a grid axis and
/// the caller wants "infinitely far" there, not a panic.
#[inline(always)]
pub const fn div(a: Fx, b: Fx) -> Fx {
    if b == 0 {
        FX_MAX
    } else {
        (((a as i64) << FRAC) / (b as i64)) as Fx
    }
}

/// `1 / a`, same zero rule as [`div`].
#[inline(always)]
pub const fn recip(a: Fx) -> Fx {
    div(ONE, a)
}

/// Build a fixed-point value from a ratio of whole numbers, at compile time.
#[inline(always)]
pub const fn ratio(num: i32, den: i32) -> Fx {
    div(from_int(num), from_int(den))
}

/// Absolute value.
#[inline(always)]
pub const fn abs(a: Fx) -> Fx {
    if a < 0 {
        -a
    } else {
        a
    }
}

/// Linear interpolation: `a` at `t == 0`, `b` at `t == ONE`.
#[inline(always)]
pub const fn lerp(a: Fx, b: Fx, t: Fx) -> Fx {
    a + mul(b - a, t)
}

/// Clamp into `lo..=hi`.
#[inline(always)]
pub const fn clamp(a: Fx, lo: Fx, hi: Fx) -> Fx {
    if a < lo {
        lo
    } else if a > hi {
        hi
    } else {
        a
    }
}

/// Convert to `f32`.  Diagnostics and tests only - nothing on the render
/// path may call this, or the two halves stop agreeing.
#[inline]
pub fn to_f32(a: Fx) -> f32 {
    a as f32 / ONE as f32
}

/// Convert from `f64`.  Table generation only, never per frame.
#[inline]
pub fn from_f64(v: f64) -> Fx {
    (v * ONE as f64).round() as Fx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_whole_numbers() {
        for i in -1000..1000 {
            assert_eq!(floor(from_int(i)), i);
        }
    }

    #[test]
    fn mul_and_div_invert() {
        let a = from_f64(3.75);
        let b = from_f64(-0.125);
        assert_eq!(mul(a, b), from_f64(-0.46875));
        // Fixed point division is lossy in the last bit; one ulp is fine.
        assert!((div(mul(a, b), b) - a).abs() <= 2);
    }

    #[test]
    fn frac_is_never_negative() {
        assert_eq!(frac(from_f64(-1.25)), from_f64(0.75));
        assert_eq!(frac(from_f64(2.25)), from_f64(0.25));
    }

    #[test]
    fn div_by_zero_is_infinity_not_a_trap() {
        assert_eq!(div(ONE, 0), FX_MAX);
    }
}

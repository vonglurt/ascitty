//! Colour, modelled on the Plus/4's TED rather than on the terminal's.
//!
//! The TED gives 16 hues at 8 luminances - 121 distinct colours, because
//! hue 0 is black at every level.  That layout is a gift to a renderer that
//! wants depth: **hold the hue, drop the luminance**.  A blue tower stays
//! blue as it recedes and simply gets darker, which is exactly what a night
//! city does, and it costs one subtraction rather than a colour-space
//! interpolation.  So the shading model here is the Plus/4's, and the
//! terminal emulates *it* - not the other way round.
//!
//! A colour is one byte: `hue << 3 | luminance`.  That is also the byte the
//! Plus/4 pokes into colour RAM, so the renderer's output needs no
//! translation on the target at all.

/// A packed `hue << 3 | luminance` colour byte.
pub type Color = u8;

/// Number of hues.
pub const HUES: u8 = 16;
/// Number of luminance steps per hue.
pub const LUMA: u8 = 8;

/// Pack a hue and luminance.
#[inline(always)]
pub const fn rgb_index(hue: u8, luma: u8) -> Color {
    ((hue & 0x0f) << 3) | (luma & 0x07)
}

/// The hue half of a colour byte.
#[inline(always)]
pub const fn hue_of(c: Color) -> u8 {
    c >> 3
}

/// The luminance half of a colour byte.
#[inline(always)]
pub const fn luma_of(c: Color) -> u8 {
    c & 0x07
}

/// Darken by `steps`, saturating at black.  This is the depth cue.
#[inline(always)]
pub const fn darken(c: Color, steps: u8) -> Color {
    let l = luma_of(c);
    let l = if l > steps { l - steps } else { 0 };
    rgb_index(hue_of(c), l)
}

/// Brighten by `steps`, saturating at the top luminance.
#[inline(always)]
pub const fn brighten(c: Color, steps: u8) -> Color {
    let l = luma_of(c) + steps;
    rgb_index(hue_of(c), if l > 7 { 7 } else { l })
}

/// Black - hue 0, and the background of every mode this program has.
pub const BLACK: Color = rgb_index(0, 0);

// Named hues, in TED order.
/// Hue 0: no chroma at all - black at every luminance.
pub const H_BLACK: u8 = 0;
/// Hue 1: no chroma - the grey column, from black to white.
pub const H_WHITE: u8 = 1;
/// Hue 2.
pub const H_RED: u8 = 2;
/// Hue 3.
pub const H_CYAN: u8 = 3;
/// Hue 4.
pub const H_PURPLE: u8 = 4;
/// Hue 5.
pub const H_GREEN: u8 = 5;
/// Hue 6.
pub const H_BLUE: u8 = 6;
/// Hue 7.
pub const H_YELLOW: u8 = 7;
/// Hue 8.
pub const H_ORANGE: u8 = 8;
/// Hue 9.
pub const H_BROWN: u8 = 9;
/// Hue 10.
pub const H_YELLOW_GREEN: u8 = 10;
/// Hue 11.
pub const H_PINK: u8 = 11;
/// Hue 12.
pub const H_BLUE_GREEN: u8 = 12;
/// Hue 13.
pub const H_LIGHT_BLUE: u8 = 13;
/// Hue 14.
pub const H_DARK_BLUE: u8 = 14;
/// Hue 15.
pub const H_LIGHT_GREEN: u8 = 15;

/// TED chroma angles in degrees, indexed by hue.  Hues 0 and 1 carry no
/// chroma at all - they are the black and white column - so their angle is
/// unused.  These are the angles the TED's colour clock actually generates;
/// they are not evenly spaced, which is why the Plus/4 palette has its
/// particular lopsided look.
const HUE_ANGLE: [f64; 16] = [
    0.0, 0.0, 103.0, 283.0, 53.0, 241.0, 347.0, 167.0, 129.0, 148.0, 195.0, 83.0, 265.0, 323.0,
    5.0, 213.0,
];

/// Relative luminance of each of the eight levels, normalised to 1.0.
const LUMA_LEVEL: [f64; 8] = [0.0, 0.1875, 0.25, 0.3125, 0.4375, 0.5625, 0.75, 1.0];

/// Chroma amplitude.  Low enough that the darkest luminances stay dark
/// rather than turning into saturated mud.
const SATURATION: f64 = 0.22;

/// Expand a packed colour byte to 8-bit sRGB, for terminals that can show
/// 24-bit colour.  This is the only place the Plus/4 model is left behind,
/// and it happens once into a 128-entry table rather than per cell.
pub fn to_rgb(c: Color) -> (u8, u8, u8) {
    let hue = hue_of(c);
    let y = LUMA_LEVEL[luma_of(c) as usize];
    if hue == H_BLACK {
        return (0, 0, 0);
    }
    // Chroma is scaled by luminance.  On real hardware a hue at luminance
    // zero is a very dark colour rather than true black, but this palette is
    // also the depth ramp: `darken` walks a colour down to luminance zero to
    // mean "too far away to see", and that has to arrive at black.  Scaling
    // chroma with luminance makes the bottom of every ramp the same black.
    let (u, v) = if hue == H_WHITE {
        (0.0, 0.0)
    } else {
        let th = HUE_ANGLE[hue as usize].to_radians();
        (SATURATION * y * th.sin(), SATURATION * y * th.cos())
    };
    // Rec.601 YUV -> RGB, then a mild gamma so the dark end of an eight-step
    // ramp does not collapse into one indistinguishable black.
    let f = |v: f64| -> u8 {
        let v = v.clamp(0.0, 1.0).powf(0.85);
        (v * 255.0).round() as u8
    };
    (
        f(y + 1.140 * v),
        f(y - 0.396 * u - 0.581 * v),
        f(y + 2.029 * u),
    )
}

/// The whole 128-entry palette as sRGB, built once.
pub fn rgb_table() -> &'static [(u8, u8, u8); 128] {
    static T: std::sync::OnceLock<[(u8, u8, u8); 128]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [(0, 0, 0); 128];
        for (i, slot) in t.iter_mut().enumerate() {
            *slot = to_rgb(i as Color);
        }
        t
    })
}

/// The nearest of the 8 ANSI colours plus their bright variants, for
/// terminals without 24-bit colour.  Returns an SGR foreground code.
pub fn to_ansi16(c: Color) -> u8 {
    let l = luma_of(c);
    if hue_of(c) == H_BLACK || l == 0 {
        return 30; // black
    }
    let (r, g, b) = to_rgb(c);
    let bright = l >= 5;
    let t = if bright { 110 } else { 90 };
    let bit = |v: u8, n: u8| if v > t { n } else { 0 };
    let base = bit(r, 1) | bit(g, 2) | bit(b, 4);
    let base = if base == 0 { 7 } else { base }; // never invisible
    if bright {
        90 + base
    } else {
        30 + base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_round_trips() {
        for h in 0..16u8 {
            for l in 0..8u8 {
                let c = rgb_index(h, l);
                assert_eq!(hue_of(c), h);
                assert_eq!(luma_of(c), l);
            }
        }
    }

    #[test]
    fn darkening_saturates_at_black_and_keeps_hue() {
        let c = rgb_index(H_BLUE, 6);
        assert_eq!(luma_of(darken(c, 2)), 4);
        assert_eq!(hue_of(darken(c, 99)), H_BLUE);
        assert_eq!(luma_of(darken(c, 99)), 0);
    }

    #[test]
    fn luminance_zero_is_black_for_every_hue() {
        for h in 0..16u8 {
            assert_eq!(to_rgb(rgb_index(h, 0)), (0, 0, 0));
        }
    }

    #[test]
    fn a_hue_ramp_is_monotonically_brighter() {
        let lum = |c: Color| {
            let (r, g, b) = to_rgb(c);
            r as u32 + g as u32 + b as u32
        };
        for h in 1..16u8 {
            for l in 1..8u8 {
                assert!(
                    lum(rgb_index(h, l)) >= lum(rgb_index(h, l - 1)),
                    "hue {h} went darker from luma {} to {l}",
                    l - 1
                );
            }
        }
    }
}

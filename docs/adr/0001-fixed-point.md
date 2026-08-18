# 0001 — Fixed point everywhere, not floating point

**Status:** accepted, before the first commit

## Context

The host can afford floating point. It has an FPU, it has SIMD, and a
renderer at 160×48 does not come close to saturating either. The Plus/4
cannot afford it at all.

The obvious split is floats on the host and fixed point on the target.

## Decision

Q16.16 fixed point in `ascitty-core`, Q8.8 in the Plus/4 build. No floating
point on any render path, on either target.

`fixed::to_f32` and `fixed::from_f64` exist for diagnostics, tests and table
generation, and are documented as such.

## Consequences

**A renderer written twice in two number systems is a renderer whose two
halves cannot be diffed against each other.** That is the whole argument. If
the host renders in floats and the target in fixed point, then when the two
pictures differ there is no way to tell whether the target has a bug or
whether the difference is what happens when you narrow a float to eight
fractional bits. With the same arithmetic on both sides, a disagreement is a
bug, and it has one cause.

It also means the *shape* of the arithmetic on the host is already the shape
the target needs: reciprocal instead of divide, table instead of transcendental,
octagonal norm instead of square root. Those choices are visible in the Rust
and are the reason the transcription to C was mechanical rather than a
redesign.

The cost is that the Rust is slightly less pleasant to read — `fixed::mul(a,
b)` instead of `a * b` — and that overflow is the programmer's problem. Both
have been worth it. The overflow discipline in particular found a real bug
before it shipped: the projection scale had to be narrowed from Q8.8 to Q4.4
on the target because a sixty-unit tower one cell away projects to nine
hundred rows, and nine hundred rows at Q8.8 wraps a 16-bit integer and draws
the tower upside down.

# 0002 — Rust for the host, C via cc65 for the 6502

**Status:** accepted

## Context

The brief asked for the most performant path available: Rust where possible,
an LLVM C compiler where not, and a compiled program at the end of it.

Rust *can* target the MOS 6502, through the llvm-mos fork of LLVM and a
matching `rustc`. It is genuinely impressive work.

## Decision

- **Host renderer, tooling and the bake step: Rust.** Release profile with
  LTO, one codegen unit, `opt-level = 3`.
- **Commodore Plus/4: C, compiled by cc65's `cl65` with `-Osir -Cl`.**

## Consequences

### Why not Rust on the 6502

- llvm-mos is not in Homebrew core. The brief said start with brew on macOS,
  and a toolchain that has to be built from source is a different project.
- It needs a nightly toolchain and a custom target specification.
- `core` on a machine with a 256-byte hardware stack is a fight, and the
  fight is with the language rather than with the problem.
- cc65 is in `brew`, has shipped working 6502 code for twenty years, and is
  what the sibling project in the next directory already uses — so the
  hardware knowledge, the linker configuration and the emulator harness are
  all already understood.

This is a "revisit when llvm-mos lands in Homebrew" decision, not a "never".
It is recorded in the backlog under **Dropped** with the same reasoning.

### What Rust is actually doing for the target

More than it looks. `ascitty-bake` runs the *real* generator — the same
character set, the same city, the same trigonometry — and writes the result
out as C. So the 6502 build has no second copy of anything and cannot
disagree with the host about what a dither glyph looks like or how tall a
building is.

That is the useful sense in which this project is "Rust compiled down to a
retro machine": Rust as the offline compiler, not as the runtime.

### On `-Osir -Cl`

`-Osir` is cc65's full optimiser — optimise, inline known functions, use
registers, inline more aggressively. `-Cl` makes local variables static
rather than stack-based, which on a 6502 is the difference between an
indexed access and a stack frame. Both matter: this is a renderer, and every
local in the column loop is touched thousands of times a frame.

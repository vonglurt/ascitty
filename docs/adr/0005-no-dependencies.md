# 0005 — No third-party crates

**Status:** accepted

## Context

The obvious dependencies for a program like this are a terminal crate for
raw mode and key decoding, and possibly a small maths crate.

## Decision

Zero third-party dependencies. `Cargo.toml` has an empty `[dependencies]`
for every crate in the workspace, and the only internal edges are
`tty → core` and `bake → core`.

## Consequences

The things that would have been dependencies are:

| Would have been | Is instead | Size |
|---|---|---|
| a terminal crate | `stty raw -echo` via `Command`, and a thread reading stdin | ~150 lines |
| key decoding | a `match` on the byte slice | ~25 lines |
| terminal size | `stty size` | ~15 lines |
| a maths crate | `fixed.rs` and a sine table | ~120 lines |

### Why

**The whole program has to be transcribed to a 6502 later.** A dependency
tree is a thing that has to be understood before it can be transcribed, and
"what does this crate actually do" is a question with no upper bound on the
answer. Everything in `ascitty-core` is code somebody here wrote and can
therefore port.

**`ascitty-core` compiles in about a second**, so the test suite is a tenth
of a second and the edit loop is real-time. That is not a small thing for a
renderer, where the loop is "change a constant, look at it".

**Nothing can break it from outside.** A retro-computing project has a
natural lifetime measured in years of occasional attention, and the failure
mode of a dependency tree over that horizon is well known.

### What it costs

The terminal handling is less capable than a real crate's. It does not
handle every terminal's escape sequences, it shells out to `stty` twice at
startup, and window resize is detected by polling rather than by `SIGWINCH`.
All three are acceptable and none is on a hot path.

If a real need arrives — a Windows build, say — this is a decision to
revisit rather than a principle.

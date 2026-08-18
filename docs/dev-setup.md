# Development setup

Everything here is macOS with Homebrew first, because that is the machine
this was written on. Linux notes follow each step; nothing in the project
is macOS-specific.

## The short version

```sh
brew install cc65 vice
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
git clone git@github.com:vonglurt/ascitty.git
cd ascitty
make            # host binary, .prg and .d64
make run        # play it here
```

## What each tool is for

| Tool | Used for | Without it |
|---|---|---|
| `cargo` / `rustc` | the host renderer and the bake tool | nothing builds |
| `cc65` | the Plus/4 build (`cl65`) | `make prg` fails; `make host` still works |
| VICE | `c1541` builds the `.d64`, `xplus4` runs it | no disk image, no emulator |
| `python3` | nothing required; the screenshot harness is `sh` | — |

There is deliberately **no** Node, no CMake, no Docker and no third-party
Rust crate. See [`adr/0005-no-dependencies.md`](adr/0005-no-dependencies.md).

### Homebrew

```sh
brew install cc65   # 2.19 or later
brew install vice   # 3.10 or later
```

`brew install vice` is the one people skip, and it is the one that matters
most: `c1541` makes the disk image, and `xplus4` is the only way to find out
whether the target build actually draws anything. A `.prg` that compiles is
not evidence.

VICE on Homebrew needs its data directory on the search path. Every script
here sets it, but if you run `xplus4` by hand:

```sh
export XDG_DATA_DIRS="$(brew --prefix)/share:$XDG_DATA_DIRS"
```

### Rust

Any stable toolchain from 1.70. The workspace pins nothing, because it
depends on nothing.

```sh
rustup toolchain install stable
rustup component add clippy      # `make check` uses it if present
```

### On Linux

`apt install cc65 vice` on Debian and Ubuntu; the package names are the
same. `stty` is used for terminal setup and is in coreutils. Everything
else is identical.

## The loop

The renderer is the whole program, so the loop is short on purpose.

```sh
make test       # 131 tests, about a tenth of a second
make run        # play it
make bench      # frames per second on this machine
```

For a change to the host renderer, `make test && make run` is the whole
cycle and takes about ten seconds.

For a change that crosses into the 6502 build:

```sh
make bake       # regenerate targets/plus4/gen from the Rust core
make prg        # compile it
make run4       # boot it in xplus4
```

`make prg` depends on `make bake`, so you rarely have to think about it.
What you *do* have to think about is that **nothing in `targets/plus4/gen`
is committed**: it is generated, the generator is the source of truth, and
a committed copy is a second definition waiting to disagree with the first.

### Seeing a frame without a terminal

```sh
./target/release/ascitty --shot --size 150x44 --mode ascii
```

renders one frame and prints it as plain text. No raw mode, no escape codes,
no terminal required — which is what makes it usable from a Makefile, from
CI, and from a pipe. `--shot 200` runs two hundred frames first, so the city
has had time to move.

### Seeing a frame from the Plus/4

```sh
tools/viceshot.sh build/ascitty.prg /tmp/shot.png
```

Boots the program in `xplus4` under warp, runs it for a fixed number of
*machine* cycles rather than seconds, and writes a PNG. Cycles rather than
seconds because warp makes wall-clock meaningless.

## Before pushing

```sh
make check
```

which runs the tests, builds both targets, regenerates the tables, renders a
host frame, and boots the target in the emulator to confirm it still draws
something. It is the gate; if it passes, the repository is in a state
somebody else can clone.

## Editor

Nothing is required. If you use rust-analyzer, the workspace is a plain
`Cargo.toml` at the root and needs no configuration. For the C, any editor
that can follow `#include` will want `targets/plus4/src` and
`targets/plus4/gen` on its include path — the same two `-I` paths the
Makefile passes to `cl65`.

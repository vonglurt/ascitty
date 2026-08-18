# Running ASCITTY

Two programs, and they do not need the same things.

| | Needs | Get it with |
|---|---|---|
| The terminal build | a Rust toolchain | [rustup.rs](https://rustup.rs) |
| The Plus/4 build | cc65, and VICE for the disk image | `brew install cc65 vice` |

## In a terminal

```sh
git clone git@github.com:vonglurt/ascitty.git
cd ascitty
make run
```

That is the whole thing. It renders at whatever size the terminal is, detects
the colour depth from `$COLORTERM` and `$TERM`, and restores the terminal on
exit — including if it panics, which is the case that actually matters.

### If the colours are wrong

```sh
./target/release/ascitty --color true    # 24-bit
./target/release/ascitty --color 16      # the eight ANSI colours, and bright
./target/release/ascitty --color none    # glyphs only
```

Detection reads `COLORTERM` for `truecolor` or `24bit`, then `TERM` for
`256color` or `direct`. Most modern terminals set one of them; `screen` and
some `tmux` configurations set neither.

### If the characters are wrong

```sh
./target/release/ascitty --mode ascii
```

The default is Unicode block elements, which need a font with the block and
box-drawing ranges — almost every monospace font has them, but `--mode ascii`
uses nothing outside the 95 printable characters of 7-bit ASCII, which
survives anything. Press `t` to switch at runtime.

### Without a terminal at all

```sh
./target/release/ascitty --shot --size 150x44 --mode ascii
```

renders one frame and prints it as plain text. `--shot 200` runs two hundred
frames first, so the city has had time to move.

## On a Commodore Plus/4

```sh
brew install cc65 vice
make disk
```

produces two files:

| | |
|---|---|
| `build/ascitty.prg` | loads faster; the form to `DLOAD` |
| `build/ascitty.d64` | a disk image; what an SD2IEC or an emulator wants |

### In VICE

```sh
make run4
```

or by hand, remembering that Homebrew's VICE needs its data directory on the
search path:

```sh
export XDG_DATA_DIRS="$(brew --prefix)/share:$XDG_DATA_DIRS"
xplus4 -autostart build/ascitty.d64
```

### In YAPE or plus4emu

Attach `ascitty.d64` as drive 8, then:

```
DLOAD"ASCITTY"
RUN
```

### On a real Plus/4

Copy `ascitty.d64` to an SD2IEC card, press NEXT on the device until it
mounts, then `DLOAD"*"` and `RUN`. Or copy `ascitty.prg` and `DLOAD"ASCITTY"`.

### Controls on the Plus/4

`W` and `S` walk, `A` and `D` turn, `Q` quits.

`Q` puts the ROM character set back before returning to BASIC. Without it the
`READY.` prompt is drawn in dither patterns and fire escapes, which is funny
once. If you break out of the program some other way, `SYS 65526` or a reset
will restore it.

### What to expect

About **1.3 frames a second**, and visible tearing: a frame takes longer than
the raster, so the display catches the screen part-way through and you see a
vertical wipe. Both are honest limitations of a C-compiled voxel renderer on
a 1.76 MHz 7501, both are measured rather than guessed
([docs/plus4.md](docs/plus4.md)), and both are at the top of
[the backlog](docs/backlog.md).

## Building from source

```sh
make          # everything: host, .prg, .d64
make host     # just the terminal build
make prg      # just the Plus/4 build
make test     # 131 tests
make check    # the gate before pushing
make help     # the rest
```

`make prg` depends on `make bake`, which runs the Rust core to regenerate the
character set, the trigonometry, the reciprocals and the baked city into
`targets/plus4/gen`. Those files are generated and are not committed — the
generator is the definition. If you clone and go straight to `cl65`, that is
why the includes are missing.

## Troubleshooting

**`cl65: command not found`** — `brew install cc65`. The Makefile also looks
in `$(brew --prefix)/bin` in case Homebrew is not on `PATH`.

**`c1541: command not found`** — `brew install vice`. `make prg` still works
without it; only `make disk` needs it.

**The emulator shows `LOADING` forever in a screenshot** — that is
`tools/viceshot.sh` territory. It passes `-autostartprgmode 1` to inject the
program straight into memory, because loading 19 KB through an emulated 1541
takes about ninety seconds of machine time.

**The terminal is broken after a crash** — `stty sane`, or `reset`. The
program installs a panic hook that restores it, but a `SIGKILL` outruns any
hook.

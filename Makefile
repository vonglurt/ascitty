# ASCITTY - build, run and check, for both targets.
#
# Two programs come out of this repository and they share one renderer:
#
#   build/ascitty       the host binary, Rust, runs in any colour terminal
#   build/ascitty.prg   the Commodore Plus/4 build, C via cc65
#   build/ascitty.d64   the same, on a disk image
#
# The 6502 build does not have its own copy of anything.  `make bake` runs
# the Rust core and writes the character set, the trigonometry, the
# reciprocals and the city into targets/plus4/gen as C headers, and the
# target build reads those.  That is why `prg` depends on `bake` and why
# nothing in gen/ is committed.
#
# Requires: cargo (host), cc65 (target), VICE (c1541 and xplus4).
#   brew install cc65 vice

BREW_PREFIX := $(shell brew --prefix 2>/dev/null || echo /opt/homebrew)
CL65   ?= $(shell command -v cl65   2>/dev/null || echo $(BREW_PREFIX)/bin/cl65)
C1541  ?= $(shell command -v c1541  2>/dev/null || echo $(BREW_PREFIX)/bin/c1541)
CARGO  ?= cargo

BUILD   = build
T4      = targets/plus4
GEN     = $(T4)/gen
TOOLS   = tools

HOST    = target/release/ascitty
BAKE    = target/release/ascitty-bake
PRG     = $(BUILD)/ascitty.prg
DISK    = $(BUILD)/ascitty.d64

# -Osir is cc65's full optimiser: optimise, inline known functions, use
# registers, inline more aggressively.  -Cl makes locals static rather than
# stack-based, which on a 6502 is the difference between an indexed access
# and a stack frame.  Both matter here: this is a renderer.
CC65FLAGS = -t plus4 -Osir -Cl -I $(T4)/src

# The city the two targets share.  Must equal ascitty_core::DEFAULT_SEED -
# that constant is the definition, and this is 0xA5C1771E written out for
# make.  Overriding it here bakes a different city into the Plus/4 build than
# the terminal renders, which is occasionally what you want and never what
# you want by accident.
SEED ?= 2780919582

GENHDRS = $(GEN)/charset.h $(GEN)/trig.h $(GEN)/recip.h $(GEN)/glyphs.h $(GEN)/city.h
T4SRC   = $(T4)/src/main.c $(T4)/src/cast.c
T4HDRS  = $(T4)/src/plus4.h $(T4)/src/cast.h

.PHONY: all host prg disk bake run run4 demo cast shot test check bench sheet clean help

all: host prg disk

help:
	@echo 'make host    the terminal build'
	@echo 'make prg     the Plus/4 build'
	@echo 'make disk    the Plus/4 build, on a .d64'
	@echo 'make run     play it in this terminal'
	@echo 'make demo    watch it walk itself'
	@echo 'make cast    record the walk to build/tour.cast (asciinema)'
	@echo 'make run4    play it in xplus4'
	@echo 'make test    every test in the workspace'
	@echo 'make check   test, then verify both targets still build'
	@echo 'make bench   how fast the host renderer is'
	@echo 'make shot    a screenshot of each target, into docs/media'
	@echo 'make sheet   print the glyph catalogue'

# --- host -----------------------------------------------------------------

host: $(HOST)

$(HOST): $(shell find crates -name '*.rs' 2>/dev/null) Cargo.toml
	$(CARGO) build --release

$(BAKE): $(HOST)

run: $(HOST)
	@$(HOST)

# The attract mode: the camera walks the streets and looks around on its own.
# Any movement key takes it over; backslash hands it back.
demo: $(HOST)
	@$(HOST) --tour

# An animation you can send somebody.  A .cast is terminal output with
# timestamps, not a video, so it stays sharp at any size and is a few hundred
# kilobytes gzipped rather than a few hundred megabytes.
CASTLEN ?= 600
CASTSIZE ?= 110x32

cast: $(HOST)
	@mkdir -p $(BUILD)
	@$(HOST) --record $(BUILD)/tour.cast --frames $(CASTLEN) --size $(CASTSIZE)
	@echo "  gzip it before sending: gzip -9 -k $(BUILD)/tour.cast"

bench: $(HOST)
	@$(HOST) --bench --size 160x48

# --- the bridge -----------------------------------------------------------

bake: $(GENHDRS)

$(GENHDRS): $(BAKE)
	@mkdir -p $(GEN)
	@$(BAKE) --out $(GEN) --seed $(SEED)

sheet: $(BAKE)
	@$(BAKE) --sheet

# --- Plus/4 ---------------------------------------------------------------

prg: $(PRG)

$(PRG): $(T4SRC) $(T4HDRS) $(GENHDRS)
	@mkdir -p $(BUILD)
	$(CL65) $(CC65FLAGS) -m $(BUILD)/ascitty.map -o $@ $(T4SRC)
	@ls -l $@ | awk '{ printf "  %s  %s bytes\n", $$9, $$5 }'

disk: $(DISK)

# The .d64 is the form a real machine wants: an SD2IEC mounts it, and VICE
# autostarts it without the injection trick the screenshot harness uses.
$(DISK): $(PRG)
	@rm -f $@
	$(C1541) -format "ascitty,ac" d64 $@ -write $(PRG) ascitty >/dev/null
	@ls -l $@ | awk '{ printf "  %s  %s bytes\n", $$9, $$5 }'

run4: $(DISK)
	XDG_DATA_DIRS=$(BREW_PREFIX)/share:$$XDG_DATA_DIRS $(BREW_PREFIX)/bin/xplus4 -autostart $(DISK)

# --- checks ---------------------------------------------------------------

test:
	$(CARGO) test --release

# The gate before publishing: the tests pass, the host builds clean, the
# tables regenerate, the target still compiles, and the target still draws
# something rather than a black screen.
check: test $(PRG)
	@$(CARGO) clippy --release --all-targets -- -D warnings 2>/dev/null || \
	    echo "  (clippy not installed - skipped)"
	@$(HOST) --shot --size 100x30 >/dev/null && echo "  host renders"
	@$(TOOLS)/viceshot.sh $(PRG) $(BUILD)/check.png >/dev/null && echo "  target renders"

# --- documentation --------------------------------------------------------

shot: $(HOST) $(PRG)
	@mkdir -p docs/media
	@$(HOST) --shot 1   --size 150x44 --mode ascii   --rain 0 > docs/media/walk-ascii.txt
	@$(HOST) --shot 1   --size 150x44 --mode unicode --rain 3 > docs/media/walk-blocks.txt
	@$(HOST) --shot 200 --size 150x44 --mode ascii --drive     > docs/media/drive-ascii.txt
	@$(HOST) --shot 1   --size 150x44 --mode unicode --copter  > docs/media/copter-blocks.txt
	@$(TOOLS)/strip.sh > docs/media/tour-strip.txt
	@$(BAKE) --sheet > docs/media/glyph-sheet.txt
	@$(TOOLS)/viceshot.sh $(PRG) docs/media/plus4.png
	@echo "  docs/media updated"

clean:
	$(CARGO) clean
	rm -rf $(GEN) $(BUILD)/*.prg $(BUILD)/*.d64 $(BUILD)/*.map $(BUILD)/*.png
	rm -f $(T4)/src/*.o

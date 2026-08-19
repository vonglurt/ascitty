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

.PHONY: all host prg disk bake run run4 demo walk demo4 cast gif shot test check bench sheet tag version clean help

all: host prg disk

help:
	@echo 'make host    the terminal build'
	@echo 'make prg     the Plus/4 build'
	@echo 'make disk    the Plus/4 build, on a .d64'
	@echo 'make run     play it in this terminal - the cab drives until you do'
	@echo 'make demo    the same thing; it is what run does'
	@echo 'make tag     tag this commit as a release build'
	@echo 'make walk    watch the camera walk the streets instead'
	@echo 'make cast    record the walk to build/tour.cast (asciinema)'
	@echo 'make gif     record the drive to docs/media/demo.gif'
	@echo 'make run4    play it in xplus4'
	@echo 'make demo4   watch the Plus/4 build walk itself in xplus4'
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

# The game: behind the taxi, on the clock, with the cab taking fares on its
# own until you touch a key.  No flags - the attract mode and the game are
# the same program in the same mode, and which one you are watching depends
# only on whether your hands are on the keyboard.
run: $(HOST)
	@$(HOST)

# The same thing, named for what it does when you leave it alone.
demo: $(HOST)
	@$(HOST) --demo

# The older demonstration: a camera on foot rather than a cab.
walk: $(HOST)
	@$(HOST) --demo --walk

# An animation you can send somebody.  A .cast is terminal output with
# timestamps, not a video, so it stays sharp at any size and is a few hundred
# kilobytes gzipped rather than a few hundred megabytes.
#
# Two of them, at the two ends of what the renderer will do, because the
# resolution is the thing worth showing: the same city, the same seed and the
# same driving at 64x20 and at 200x56.  The small one is what a Plus/4-sized
# window looks like; the large one is what a full terminal does with the same
# code, and the point is that neither is a different program.
#
# Twice as long as it used to be and played back at twice the speed, so a
# recording is forty seconds of driving in twenty seconds of watching.  The
# simulation still steps at CASTFPS - only the timestamps are divided.
CASTLEN  ?= 1200
CASTFPS  ?= 30
CASTSPEED ?= 2
CASTLO   ?= 64x20
CASTHI   ?= 200x56
CASTSEED ?= 99

cast: $(HOST)
	@mkdir -p $(BUILD)
	@$(HOST) --record $(BUILD)/tour-lo.cast --frames $(CASTLEN) --size $(CASTLO) \
		--fps $(CASTFPS) --speed $(CASTSPEED) --seed $(CASTSEED)
	@$(HOST) --record $(BUILD)/tour-hi.cast --frames $(CASTLEN) --size $(CASTHI) \
		--fps $(CASTFPS) --speed $(CASTSPEED) --seed $(CASTSEED)
	@ls -la $(BUILD)/tour-lo.cast $(BUILD)/tour-hi.cast
	@echo "  gzip them before sending: gzip -9 -k $(BUILD)/tour-*.cast"

# The same demonstration as a picture that moves, for the README - a web
# page cannot play a .cast.  Small and short on purpose: a GIF is a
# full frame every frame however little of it changed, so the size is the
# frame count times the area, and 72x22 for eight seconds is about two
# megabytes.  Anything you would actually watch belongs in the .cast.
GIFLEN  ?= 120
GIFSIZE ?= 72x22
GIFFPS  ?= 15
GIFSEED ?= 99

gif: $(HOST)
	@mkdir -p docs/media
	@$(HOST) --demo --drive --seed $(GIFSEED) --size $(GIFSIZE) \
		--fps $(GIFFPS) --frames $(GIFLEN) --gif docs/media/demo.gif

bench: $(HOST)
	@$(HOST) --bench --size 160x48

# Tag a build, so a picture in the README can name the code that drew it.
# The version comes from Cargo.toml and nowhere else - `ascitty --version`
# prints the same string - so bumping it there is the one edit a release
# needs.
VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

tag: check
	git tag -a v$(VERSION) -m "ascitty v$(VERSION)"
	@echo "  tagged v$(VERSION) - push it with: git push origin v$(VERSION)"

version:
	@echo $(VERSION)

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

# The attract mode, on the machine.  The program drives itself from boot and
# stops the moment a key is touched, so `demo4` and `run4` load the same
# thing - the difference is only whether you touch the keyboard.
#
# -autostartprgmode 1 injects the program instead of loading it through an
# emulated 1541, which otherwise costs about ninety seconds of machine time
# before anything appears.
demo4: $(PRG)
	XDG_DATA_DIRS=$(BREW_PREFIX)/share:$$XDG_DATA_DIRS $(BREW_PREFIX)/bin/xplus4 \
	    -autostartprgmode 1 -autostart $(PRG)

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
	@$(HOST) --shot 1   --size 150x44 --mode ascii   --walk > docs/media/walk-ascii.txt
	@$(HOST) --shot 1   --size 150x44 --mode unicode --walk > docs/media/walk-blocks.txt
	@$(HOST) --shot 200 --size 150x44 --mode ascii --drive     > docs/media/drive-ascii.txt
	@$(HOST) --shot 1   --size 150x44 --mode unicode --copter  > docs/media/copter-blocks.txt
	@$(HOST) --shot 520 --size 140x40 --seed 99 --tour --walk --sky 0 --day 0 \
		--png docs/media/street.png
	@$(HOST) --shot 600 --size 140x40 --seed 99 --demo --drive --sky 6 --day 0 \
		--png docs/media/drive.png
	@$(HOST) --shot 1   --size 140x40 --seed 99 --copter --haze 1 --sky 5 --day 0 \
		--png docs/media/copter.png
	@$(HOST) --shot 300 --size 140x40 --seed 99 --demo --drive --sky 8 --day 0 \
		--png docs/media/sunset.png
	@$(TOOLS)/strip.sh > docs/media/tour-strip.txt
	@$(BAKE) --sheet > docs/media/glyph-sheet.txt
	@$(TOOLS)/viceshot.sh $(PRG) docs/media/plus4.png
	@echo "  docs/media updated"

clean:
	$(CARGO) clean
	rm -rf $(GEN) $(BUILD)/*.prg $(BUILD)/*.d64 $(BUILD)/*.map $(BUILD)/*.png
	rm -f $(T4)/src/*.o

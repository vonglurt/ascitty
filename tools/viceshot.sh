#!/bin/sh
# Boot a .prg in xplus4, run it for a while, and save what the screen looks
# like.  Used by `make shot` and to check that the target build actually
# draws something before it is published.
#
#   tools/viceshot.sh build/ascitty.prg build/shot.png [cycles]
#
# Cycles rather than seconds because warp mode makes wall-clock meaningless:
# the Plus/4's PAL clock is 1 773 447 Hz, so a million cycles is about half
# a second of machine time however fast the host runs it.
#
# -autostartprgmode 1 injects the program straight into memory.  Loading it
# through an emulated 1541 instead takes about ninety seconds of machine
# time, and the screenshot catches the LOADING message rather than the city.
set -e
PRG="${1:?usage: viceshot.sh PRG PNG [cycles]}"
PNG="${2:?usage: viceshot.sh PRG PNG [cycles]}"
CYCLES="${3:-40000000}"
BREW_PREFIX="$(brew --prefix 2>/dev/null || echo /opt/homebrew)"

rm -f "$PNG"
XDG_DATA_DIRS="$BREW_PREFIX/share:$XDG_DATA_DIRS" \
    "$BREW_PREFIX/bin/xplus4" \
        -warp \
        -autostartprgmode 1 \
        -limitcycles "$CYCLES" \
        -exitscreenshot "$PNG" \
        -autostart "$PRG" \
        >/dev/null 2>&1 || true

if [ ! -s "$PNG" ]; then
    echo "viceshot: no screenshot was written - the emulator did not run" >&2
    exit 1
fi
echo "$PNG"

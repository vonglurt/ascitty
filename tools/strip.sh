#!/bin/sh
# A contact sheet of the autopilot's walk: the same tour sampled at a few
# points, so the documentation can show the camera moving rather than
# claiming that it does.
#
# Frames rather than seconds, because the tour is stepped at a fixed rate and
# a frame index is reproducible where a wall-clock time is not.
set -e
HOST="${HOST:-./target/release/ascitty}"
SEED="${SEED:-99}"
SIZE="${SIZE:-100x20}"
FPS=30

for FRAME in 1 90 240 420 600; do
    printf '=== t = %s s ===\n' "$(awk -v f="$FRAME" -v r="$FPS" 'BEGIN{printf "%.1f", f/r}')"
    "$HOST" --shot "$FRAME" --tour --seed "$SEED" --size "$SIZE" \
            --mode ascii --rain 0 --haze 2
    printf '\n'
done

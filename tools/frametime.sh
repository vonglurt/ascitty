#!/bin/sh
# How long the Plus/4 build takes per frame, measured on the emulator.
#
#   tools/frametime.sh [frames]
#
# Builds a variant that renders exactly N frames and then turns the border
# white, and bisects the cycle budget at which that happens.  Two runs at
# different N cancel the boot cost, which is otherwise most of the answer.
#
# Cycles rather than seconds because warp mode makes wall-clock meaningless.
set -e
SRC=targets/plus4/src
PAL=1773447          # PAL clock, Hz

white_at() {
    python3 - "$1" <<'PY'
import sys, zlib, struct
f = open(sys.argv[1], 'rb').read()
pos, idat, pal, ctype, w = 8, b'', None, 0, 0
while pos < len(f):
    ln = struct.unpack('>I', f[pos:pos+4])[0]
    typ, data = f[pos+4:pos+8], f[pos+8:pos+8+ln]
    if typ == b'IHDR': w, h, depth, ctype = struct.unpack('>IIBB', data[:10])
    elif typ == b'PLTE': pal = data
    elif typ == b'IDAT': idat += data
    pos += 12 + ln
raw = zlib.decompress(idat)
v = pal[raw[1]*3] if ctype == 3 else raw[1]
sys.exit(0 if v > 200 else 1)
PY
}

build() {
    sed 's|    for (;;) {\n        cast_frame();|X|' "$SRC/main.c" > /tmp/ft.c
    python3 - "$1" <<'PY'
import sys
n = sys.argv[1]
s = open('targets/plus4/src/main.c').read()
s = s.replace("    for (;;) {\n        cast_frame();",
              "    { unsigned int fr; for (fr = 0; fr < %s; ++fr) cast_frame(); }\n"
              "    TED_BORDER = CBYTE(7, HUE_WHITE);\n"
              "    for (;;) {\n        cast_frame();" % n)
open('/tmp/ft.c', 'w').write(s)
PY
    cl65 -t plus4 -Osir -Cl -I "$SRC" -o /tmp/ft.prg /tmp/ft.c "$SRC/cast.c" 2>/dev/null
}

# The cycle at which N frames are done, by bisection.
finish() {
    build "$1"
    LO=5000000; HI=20000000
    while :; do
        tools/viceshot.sh /tmp/ft.prg /tmp/ft.png "$HI" >/dev/null 2>&1 || true
        white_at /tmp/ft.png && break
        LO=$HI; HI=$((HI * 2))
        [ "$HI" -gt 4000000000 ] && { echo "FAILED" >&2; exit 1; }
    done
    while [ $((HI - LO)) -gt 300000 ]; do
        MID=$(((LO + HI) / 2))
        tools/viceshot.sh /tmp/ft.prg /tmp/ft.png "$MID" >/dev/null 2>&1 || true
        if white_at /tmp/ft.png; then HI=$MID; else LO=$MID; fi
    done
    echo "$HI"
}

N1=${1:-10}
N2=$((N1 * 5))
A=$(finish "$N1")
B=$(finish "$N2")
python3 -c "
per = ($B - $A) / float($N2 - $N1)
print('%d frames: %d cycles' % ($N1, $A))
print('%d frames: %d cycles' % ($N2, $B))
print('per frame: %.0f cycles = %.2f fps' % (per, $PAL / per))"

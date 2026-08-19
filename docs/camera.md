# The camera

One struct, four ways of moving it.

```rust
pub struct Camera {
    pub x: Fx, pub y: Fx,   // where you are, in cell units
    pub z: Fx,              // eye height above the pavement
    pub yaw: Ang,           // heading, a u16 that wraps
    pub pitch: i32,         // vertical look, in SCREEN ROWS
    pub fov: Fx,            // half-width of the camera plane
}
```

Two of those fields are worth a sentence each.

**`yaw` is a `u16` covering one full turn**, so it wraps for free on overflow
and there is never a range reduction to get wrong. Turning right by 30 000
units a thousand times is fine; there is no accumulated angle to normalise
and no `fmod`.

**`pitch` is in screen rows, not radians.** A text renderer can only *shear*
the horizon, not rotate it. Pretending otherwise would want curved building
edges on a grid with no way to draw them, so the camera does not pretend:
looking up moves the horizon down the screen and nothing else changes. The
one consequence to know about is that pitch has to be clamped against the
frame height — a tilt of eleven rows pushes the ground off a twenty-row
screen entirely.

**`fov` is a plane half-width, not an angle.** A ray is `dir + plane × camx`,
and expressing the field of view this way is what keeps the products inside a
16-bit integer on the Plus/4. See [`renderer.md`](renderer.md) §2.

## The three modes

`c` cycles them.

### Walk

Eye at 1.8 m. `w`/`s` forward and back, `a`/`d` strafe sideways, `q`/`e` or
←/→ rotate, ↑/↓ look.

Movement goes through `Camera::walk`, which resolves the two axes separately
so that running into a wall at an angle **slides you along it** rather than
pinning you to it. That is what makes a corner feel like a corner.

You cannot enter a building and you cannot leave the pavement — `z` is pinned
to eye height every frame.

### Drive

`w` throttle, `s` brake, `space` handbrake. Both `a`/`d` and `q`/`e` steer —
a car steers rather than rotating or strafing, so rather than guessing which
of the two the hands will reach for, both work.

The camera is not attached to the car; it is a chase boom, and two things
about it are deliberate.

**The heading lags the car's**, by a sixth of the remaining angle per frame.
A flick of the wheel swings the view a moment later, so a drift is watched
from the outside rather than from inside the spin.

**The boom shortens until it is out of a building.** A chase camera that
clips through a wall shows you the inside of the wall at exactly the moment
you most need to see the road, so the boom is tried at its full length and
retracted a quarter-cell at a time until the point behind the car is on
walkable ground.

The physics itself is in [`driving.md`](driving.md).

### Copter

`w`/`s` fly, `a`/`d` strafe, `q`/`e` rotate, `space` up and `z` down. It
starts above the tallest roof looking down.

`z` rather than Shift because a terminal cannot see a bare Shift: it sends no
bytes at all, and Shift with a letter is indistinguishable from the capital.

Flight ignores buildings horizontally — you are above them — but not the
floor of the mode, which keeps the camera over the roofline where the view is
worth having.

**How far down it looks is worked out, not chosen.** Ground `d` away is drawn
`eye × scale ÷ d` rows below the horizon, so the furthest thing the haze
allows — the draw distance — sits a fixed number of rows below it, and
everything between the horizon and there is too far away to draw at all. The
copter is pitched so that row is the *top* of the frame:

```
    horizon                        ← off the top, on purpose
    ─────────────────────────────
    row 0     the draw distance    ← nothing beyond here is drawn
      ...
    row h-1   directly below-ish
```

It was a fixed eight rows before, which is a walking camera's tilt. From the
roofline of the tallest building, at the default haze, the draw distance is
about twenty rows below the horizon and the bottom of a forty-row frame is
sixty: eight rows of tilt put three or four rows of city along the bottom
edge and forty rows of empty night above it. The mode looked broken and was
simply pointed at the sky.

The number depends on the height, the haze *and* the width of the frame —
the lens is fixed, so a wider frame is more rows per world unit — which is
why it is derived from the projection rather than kept as a constant.
`raycast::pitch_down` is the one that does it, and it shares
`raycast::scale` with the projection so the two cannot disagree.

**And the tilt limit is per mode.** Walking and driving clamp the pitch to a
third of the frame, which keeps the horizon on the screen: a view of nothing
but pavement is not a view. The copter is the opposite case — its horizon is
off the top on purpose — so clamping it to the walking rule was on its own
enough to point it back at the empty sky. It may tilt as far as its aim, and
one frame further; past that there is nothing new to see, only the same
ground stretched.

### A terminal cannot tell you a key was released

It sends a byte when a key goes *down* and nothing at all when it comes up.
So movement is edge-triggered: one step per press, repeating at the
terminal's own autorepeat rate, which is close enough to feel continuous.

Driving needs a held pedal rather than a nudge, so `Pedals` gives each
control a five-frame decay. Longer than the autorepeat interval and the car
stutters; much longer and it will not stop.

## The autopilot

`--demo` (or `--tour`), and `\` at any time, hands over to something that
reads the city and drives itself. Any movement key takes it back off — there
is no mode to leave, you just start driving.

There are two of them, and which you get is what `--demo` means:

| | what it does | where |
|---|---|---|
| `--demo` | the cab takes fares | `cabbie.rs` |
| `--demo --walk` | the camera walks the streets | `tour.rs` |

### The cabbie

The default, because it is what the thing is for and a camera walking past
parked cars does not show it. It picks up whatever fare the simulation is
offering, plans a route over the carriageway, drives it, and stops inside the
painted circle at the far end — at which point the simulation hands over the
passenger and issues another, so it runs indefinitely.

Two layers, and the split is the design. `City::drive_route` is a
breadth-first search over road cells, run **once per fare**: far too
expensive per frame, and the only thing that can be trusted to arrive, since
a greedy stepper cannot leave a U-shaped block and this grid is full of them.
The steering is then a pure function of that plan and the car's state.

It steers on two terms — how far the car is from the middle of its lane, and
how far it is from pointing along it — because a lane is a statement about
both. Aiming at a point some cells down the road, which is the obvious way to
follow one, has no term for lateral offset at all: a car parallel to its lane
but a lane and a half wide of it reports almost no error and stays there.

Two things about it do not work well yet and are in the backlog: its
preference for the right-hand lane measures between 25 and 63 per cent across
four cities, and about 40 per cent of travelling ticks have the car's centre
on a cell that is not carriageway.

### The walking tour

`--demo --walk`. It is in
[`crates/ascitty-core/src/tour.rs`](../crates/ascitty-core/src/tour.rs), and
it is not a scripted path: a path baked for one city is wrong for every
other seed. This one probes ahead, turns at junctions, keeps to the middle of
the street, and stops to look up at whatever is tallest nearby.

### Heading and gaze are separate

The single thing that makes it look like a person rather than a dolly.
`heading` is where the feet are going; the camera's `yaw` is `heading +
gaze`, and gaze wanders. So it can turn its head to watch a tower go past
without veering into it, and the movement never stops to let the look happen.

`Camera::slide` exists for exactly this: move by a world-space delta rather
than along the view direction.

### What it does

| Behaviour | What happens |
|---|---|
| `Strolling` | full pace, head drifting slowly to one side |
| `Admiring` | quarter pace, head turned towards the tallest thing within ten cells, tilted up in proportion to its height over its distance |
| `Turning` | half pace, at a junction, left or right if either is clear |
| `Waiting` | stopped, looking straight down the street |

Blockage is re-checked **every tick**, not only when a behaviour ends,
because a behaviour that outlasts the pavement walks you into a wall and
leaves you grinding against it for the rest of the take.

### Keeping to a lane

Both flanks are probed and the walker eases towards the open side — but not
to dead centre. It settles half a cell over, which is the middle of a lane.

Dead centre is the obvious target and the wrong one: it puts the camera
directly on top of the double yellow, so the nearest few rows of every frame
are a wall of centre line. Half a cell over, the line converges away from
you down the street instead of out from under you.

### Why it keeps off the walls at all

Both flanks are probed and the walker eases towards the open side. Without
it, it ends up hugging whichever wall it drifted onto — and a camera at eye
height pressed against a forty-storey building sees forty storeys of window
and nothing else.

The centring force is slightly *faster* than the walking pace, which looks
wrong written down and is right: it only reaches full strength with one
shoulder against a wall and eight clear cells on the other side, and below
that it is a nudge.

Two other numbers were tuned by looking at frames rather than at code.

**It looks three and a half cells ahead**, and that is not a "bigger is
safer" dial. Too short and it turns with its nose already against a
forty-storey facade, so the frame is one wall and nothing else. Too long and
it reacts to buildings it was never going to reach, which in a narrow street
means turning, turning back, and oscillating into the kerb — at four and a
half cells, four times as many frames were pressed against a wall as at
three and a half.

**It will not stop to admire anything closer than four cells.** A tower
twenty-five metres away is not something you crane your neck at, it is
something you are about to walk into, and framing it fills the screen with
windows and no sky.

### It is reproducible

Same city and same seed, same walk, every time. That is what makes a
recorded animation repeatable, and it is what lets the tests assert that the
walker never enters a building, never wedges, always looks up at something,
and ends up in about the same place at 30 Hz as at 60 Hz.

## Recording an animation

```sh
make cast                                   # build/tour.cast, 20 seconds
ascitty --record demo.cast --frames 1800    # a minute
ascitty --anim --frames 300                 # just play it, then exit
```

A `.cast` is an [asciicast v2](https://docs.asciinema.org/manual/asciicast/v2/)
file: a JSON header and one line per frame holding the bytes and when they
were written. An animation of a terminal program should *be* terminal output,
not a video of a terminal — it stays sharp at any size, and the frames in it
are the exact bytes the renderer produced.

Play it with `asciinema play demo.cast`, or upload it.

Size, for 20 seconds at 110×32:

| `--color` | raw | gzipped |
|---|---:|---:|
| `true` | 9.0 MB | 436 KB |
| `16` | 5.6 MB | 334 KB |
| `none` | 3.0 MB | 220 KB |

Truecolor costs about nineteen bytes an escape and a night city changes
colour every few cells, so most of the file is `\x1b[38;2;r;g;bm`. It
compresses about twenty to one; gzip before sending one.

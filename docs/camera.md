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

**It sits one car-length back and eleven tenths of a cell up.** Near and
high, which is one change and not two: from further back and lower down you
are a camera following a car, and from nearer and higher up you are looking
over its roof at the road. The two fight over one number — the car's foot
lands `eye × scale ÷ boom` rows below the horizon, so halving the boom
doubles that and raising the eye adds to it again, and at the height that
first looked right the cab's whole lower half was off the bottom of a
forty-row frame. What is left visible is the roof, the glass and the chequer band — the parts
that say it is a taxi — and the rest of the frame is street, which is what
the height is for.

**It stands twice as far off when you are reversing.** A close chase camera
looks over the boot at the road ahead, which is the wrong half of the world
when the car is going the other way; the only way to show more of what is
behind through a camera that stays behind is to stand further back. It is
continuous, off the car's own forward speed, so it draws back as the car
picks up reverse and comes in again as it stops.

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

### The frame is the window

There is no resolution setting. The frame is however many cells the terminal
has, and it follows a resize — which means asking, since there is no signal
to wait for without a libc. Two ways, settled at startup in the same round
trip as the keyboard handshake: a terminal that answers `CSI 18 t` reports
its size on the stream its keys already arrive on, and one that does not is
asked with `stty size`. Either way it is twice a second, not thirty times;
the second one is a fork and an exec and used to happen every frame.

### The arrow on the road

The one piece of interface that is *in* the picture: a yellow arrow lying on
the **ground plane** a few cells in front of the camera, running from the
bottom of the frame to about the middle of the taxi, pointing at whichever
end of the fare is current.

It is projected rather than drawn. Every cell below the horizon is turned
back into the piece of road it is a picture of — the distance is `eye ×
scale ÷ rows below the horizon`, the same expression the floor pass uses, and
the column gives the offset across at that distance — and that point is
rotated into the arrow's own frame and tested against a shaft and a triangle.
So it converges with the street it is lying on, and swinging it round the
compass sweeps it across the road the way a needle laid flat would.

The version before it was rotated in screen x and y and squashed by a
constant, and no amount of squashing fixed it: it read as a card held up in
front of the car, because a card has no perspective in it.

It carries a black outline — the same test, inflated, underneath — which is
what keeps it legible over a yellow cab on a yellow-lit street. And it is
drawn last, after the sprites and the weather, because a decal in the world
would disappear under the car at exactly the moment the car is what you are
looking at.

### The driver's head

The driving camera's pitch is not fixed. It carries a spring-damped head that
leans back under power and is thrown forward under braking, and that sits
still at any constant speed, because what it answers to is acceleration. It
is worth three rows of horizon under power and four under braking on a
forty-row frame, and it is underdamped on purpose so that lifting off swings
the view past level and back. The numbers and the reasoning are in
[driving.md](driving.md).

**Positive pitch is up.** The horizon is drawn at `h/2 + pitch`, row zero is
the top of the frame, and a horizon further down the frame has more sky above
it. `raycast::pitch_down` returns a large negative number for the same
reason. That sign was written down backwards once, in the one place that sets
the driving camera, and it put two things the wrong way round at the same
time: the chase camera looked slightly *up* rather than slightly down at the
road, and the head bobbed the wrong way — standing on the throttle threw the
view at the tarmac and standing on the brakes threw it at the sky. Both were
one minus sign. The driving camera now sits a seventh of the frame below
level, which on a forty-row frame puts the horizon six rows above the middle
and fills the bottom two thirds with street.

### Holding a key down

Every control is an analogue axis rather than a flag: a press winds the level
on over a fifth of a second, letting go winds it off over an eighth. That is
what makes a held key a pedal rather than a bit, and it is the same mechanism
walking, flying and driving — turning, strafing, tilting the head and the
throttle all read a level.

**`wasd` is the vehicle and the arrows are the view**, in every mode. What a
vehicle is changes — the wheel in the cab, a step sideways on foot and in the
air — and so does what a view is: behind the cab the arrows swing the camera
round the car, which is the driver looking about rather than the car turning,
while up and down stay on the pedals because the chase camera sets its own
pitch every frame. On foot and in the helicopter, `q` and `e` already turn
you, so left and right go to the other useful thing and step you sideways.

The pan is applied to the heading the chase camera is *chasing* rather than
to the camera itself, so the lag that already smooths a turn pans it round
smoothly, brings it back to centre when the key comes up, and swings the boom
with it: the camera orbits the cab rather than turning its back on it.

Whether a key is *down* is the harder half. A terminal sends a byte when a
key goes down, nothing when it comes up, and autorepeats the most recently
pressed key only — so two keys at once is not something the byte stream can
express. Terminals that speak the progressive keyboard protocol report
releases, this asks yours at startup, and where the answer is yes a held key
is genuinely held. Where it is no, a press stays live for half a second and
autorepeat renews it. The handshake and the trade-off are in
[driving.md](driving.md).

## The autopilot

It is on when the program starts, and `\` at any time hands it back. Any
movement key takes it off again — there is no mode to leave, you just start
driving. `--demo` and `--tour` ask for it explicitly, which is what it does
anyway; `--play` is the one that turns it off.

There are two of them, and which you get is which view you are in:

| | what it does | where |
|---|---|---|
| driving, the default | the cab takes fares | `cabbie.rs` |
| `--walk` | the camera walks the streets | `tour.rs` |

### The cabbie

The default, because it is what the thing is for and a camera walking past
parked cars does not show it. It picks up whatever fare the simulation is
offering, plans a route over the carriageway, drives it, and pulls up at the
kerb beside the circle at the far end — at which point the simulation hands
over the passenger and issues another, so it runs indefinitely. The circle
itself is on the pavement, because that is where the passenger is standing;
the cab stops beside it and the passenger walks the last step.

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

Both of those were measured for a long time and both were poor: the
right-hand lane about half the time, and a third of travelling ticks with the
car's centre off the carriageway. The fix was not in the lane target, which
is where the effort went, but in **what the car aims at when there is no lane
to hold**. Inside a junction — and the crossing of two arterials is fourteen
cells of junction — the controller had nothing to say, and the fallback
steered at the marker, so every junction was a stretch of driving at a point
on the far side of a block. Aiming a few cells up the route instead took the
lane split to 70–88 per cent, and the same change took one city from one fare
in five minutes to eight. Giving the engine an acceleration curve moved the
figures again — a car that takes a second and three quarters to get back up
to speed spends longer at the speeds where the cross-track term is divided by
a smaller number — and raising that gain by half settled them at 83, 79, 77
and 81.

The other half of it is a second stuck check. Wedged is not always
*stopped*: a car that has climbed a kerb and is grinding along a shop front
at a cell a second passes every speed test there is, and can do it for a
minute. Being off the carriageway for more than a second now backs the car
out the same way a stall does.

### The walking tour

`--walk`, which is also how you get out of the cab. It is in
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

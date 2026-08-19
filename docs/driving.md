# Driving

`crates/ascitty-core/src/drive.rs`.

## 1. This is not a simulation

There are no tyres, no weight transfer, no suspension and no engine curve,
and adding any of them would make the car worse.

Six properties are modelled and nothing else.

### The engine has a curve

Force is not constant. It is everything the engine has at a standstill and
tapers linearly to nothing at a quarter above the top speed, which is the
shape of a torque curve through a gearbox and, more to the point, the shape
that makes speed something the car *builds*. The approach is exponential, so
what the two constants really set is a time constant.

Measured from rest on open ground, flat out:

| | mph |
|---|---:|
| 0.25 s | 47 |
| 0.5 s | 82 |
| 1.0 s | 125 |
| 1.75 s | 154 |

The version this replaced used a constant force of twenty-six against a top
speed of seven units a second: it was at the clamp in 0.27 s, from any speed,
and every row of that table would read 154. There was nothing to hold the
throttle down *for*, which is most of what driving one of these is.

The ceiling is a quarter above the top speed rather than equal to it because
the engine has to out-pull the drag at the top of the range. With the two
equal, force and drag balance a little under the top speed and the clamp
never binds, so the car has a top speed it cannot quite reach.

### The car wants to go forwards, like a boat

Velocity is split into a longitudinal component and a lateral one; the
lateral one bleeds away every tick. How fast it bleeds is the entire
handling model.

### Turning the wheel does not turn the velocity

The heading is rotated **after** the velocity is recombined, so the car's
momentum carries on in the old direction for a few ticks. That gap between
where the car points and where it is going is the drift, and it is a
consequence of the update order rather than a special case.

```
    split v along the old heading   →   vf, vl
    apply engine and drag to vf
    bleed vl                            ← this is grip
    recombine into world velocity       ← still the old heading
    THEN rotate the heading             ← this is the drift
```

Swap the last two lines and the car is on rails.

### It is a car up to a point and a boat past it

Grip is interpolated between the parked figure and the flat-out one along a
cubic, so it is still nearly the parked figure at half the top speed and only
lets go over the last quarter of the range — and it gets there without a
threshold anyone can feel as a switch. Town corners track the nose; flat-out
ones do not. The handbrake removes grip at any speed, which is how you get
the car sideways on purpose.

Grip is quoted as the fraction of the slide that survives **one tick**, and
that is not a detail. A per-second figure has to be linearised to be spent a
tick at a time, and the linear form cannot remove more than one tick's worth
of anything: written as `(1 - keep) / 60` per tick, even a grip of *zero*
leaves `(1 - 1/60)^60` — 37% — of the slide alive a second later. Every
corner was a boat because no setting of the old constants could make one that
was not.

Taken at a held speed on open ground, a full-lock quarter turn:

| Entry | Radius | Peak slip | With the handbrake |
|---|---:|---:|---:|
| 28 km/h | 4 m | 0.10 | 0.89 |
| 65 km/h | 16 m | 0.08 | 0.93 |
| 100 km/h | 36 m | 0.06 | 0.90 |
| 150 km/h | 83 m | 0.15 | 0.84 |

The grip was raised twice. It let go far too early: at 150 km/h a full-lock
corner slipped 0.29 with nothing touching the handbrake, and a car that
drifts *without being asked to* is a car you cannot place between two rows of
buildings six metres apart. The slide is now something you ask for.

The version this replaced turned inside 14 m at 150 km/h and slipped between
0.85 and 0.93 at *every* speed in that table, handbrake or not. It drifted
identically at walking pace and flat out, which is another way of saying the
speed made no difference to the handling at all.

### The wheel stops working as the speed rises

Yaw rate climbs with speed while the steering lock is what limits it, and
falls away as `TURN_REF / speed` once the grip is. The corner the car can
take is therefore one of constant *force* rather than of a constant angle,
and its radius grows with the square of the speed — 5 m at 28 km/h, 18 m at
65, 87 m flat out. Going faster means going a great deal wider, and getting
the nose round a junction at speed means slowing down or hanging the tail
out.

This is the one piece of real vehicle behaviour in here. It is present
because without it a car with grip pivots on its own axis at 150 km/h, which
is what a tank does.

### Buildings are rigid and everything else is not

A wall stops the car, costs it speed and paint, and knocks it crooked — which
is most of why hitting one is exciting rather than merely a stop. The two
axes are resolved separately, so clipping a corner scrubs speed off one axis
and lets the other carry on.

A lamp post does not slow the car at all.

## 2. What is deliberately not modelled

- **A vertical axis.** The streets are flat and the car never leaves them.
- **Damage that matters.** It accumulates and it shows. A run that ends
  because the vehicle failed is a run that stopped being about pace.
- **Anything the player cannot feel.** Reality is not a goal. Pace is.

## 3. The tick rate is not a handling setting

Grip is per tick and drag is per second, and both are scaled to the rate they
are actually spent at. Neither used to be: at 30 Hz — the rate the autopilot
and the Plus/4 timings run at — the car kept twice as much of every slide and
had half the drag, so the machine that could least afford a loose car got the
loosest one. The same four seconds of full-lock cornering now ends within a
car's length of the same place at 30 Hz and at 60.

## 4. Cars hitting cars

The textbook two-body impulse:

```
    j = -(1 + e) × closing / (1/mₐ + 1/m_b)
```

with restitution **0.7** — far too bouncy for two tonnes of steel and exactly
right for what this is. Cars should go over like skittles.

Two things about this were wrong first time and are worth writing down.

**The normal has to be a unit vector before the dot product.** Taking the
closing speed against the raw separation scales the impulse by how far apart
the cars happen to be, which makes a hard contact — where they are closest —
the *gentlest* one, and lets the same pair collide forever because neither
gets pushed hard enough to separate.

**Mass must be applied once.** Splitting the impulse by mass *and* dividing
by mass inside the shove applies it twice. The symptom was a taxi at 40 mph
moving a parked car about a foot.

## 4a. The cab ploughs

The impulse is worked out with the cab weighing three times what it weighs
the rest of the time, which is not physics, it is the game. A taxi that loses
half its speed to every saloon it clips cannot cross town, and the fare is on
a clock. Heavy in the solve, the cab keeps most of its momentum and the other
car gets half again as much of a shove as an even exchange would.

The bus is tripled with it, because the bus is the thing the cab *cannot*
plough through and that only stays true if it keeps its lead - at its
ordinary weight against a ploughing cab, a saloon and a bus went nearly the
same distance when hit.

Separating them is asymmetric for the same reason. The car that is not the
cab is bounced back the whole overlap and a little further, so it visibly
recoils; the cab gives ground only if that car has nowhere to go, which on a
narrow street is a wall behind it.

Contact is three quarters of the sum of the half-lengths rather than all of
it. The bodies are drawn as boxes and collided as circles, and a circle round
a box twice as long as it is wide is a lot of empty air at the corners: at
the full length, two cars passing in opposite lanes of a two-cell street
clipped each other, which is not a near miss, it is a phantom.

## 4b. The boost, and the meter

A coin is worth three things: two seconds on the clock, a unit of money, and
three seconds of **boost** - twice the engine and twice the top speed, spent
only while the throttle is down, so a coin taken into a corner is still worth
something coming out of it.

Against that, the meter runs on petrol as well as on time: a unit of money a
second, for as long as the shift lasts. The fare pays a fixed amount for the
distance and the clock pays nothing at all, so without a running cost the
fastest route and the slowest are worth the same. With one, a trip is
profitable to the extent that it was quick and that you picked things up on
the way.

## 5. The other traffic

`crates/ascitty-core/src/sim.rs`, and the rules of the road it reads are in
`road.rs`.

Traffic used to be scenery with momentum: a car was dropped on a road cell,
pointed along it by a coin toss, given a fixed throttle and left alone. That
produces a street where half of everything is oncoming *in the lane
somebody else is using*, two cars a lane apart drive head-on at each other,
and nothing ever slows down for anything. Three things changed.

### The direction comes from the lane, not from a coin

`road::flow` reads which side of the crown of the carriageway a cell is on
and returns the one direction of travel that belongs there; `road::lane` is
its inverse — given a direction, where on the road a car should be. They are
tested against each other on every carriageway cell of four cities, because
the whole benefit is that the thing that *places* cars and the thing that
*steers* them agree.

A car is put down on the lane line, facing the way that lane goes. So the
cars on one side of the paint all go the same way, which is the difference
between a road and a car park with lines on it. Measured over four cities:
98 to 100 per cent of car-ticks on the correct side of the crown, against
about half before.

### Lane keeping is the same law the cabbie uses

An angle term to point the car down the street, and a cross-track term to
walk it onto the line, divided by speed so the correction that suits a crawl
does not snake the car at speed. The line it holds is the middle of the lane
it is already in — not the lane the rule would pick — unless that lane
belongs to the traffic going the other way, in which case it crosses back.
Otherwise every car on a fourteen-cell arterial files into one lane and
leaves the rest of it empty.

### Backing out of a shunt

A driver who has just been hit does not carry on as though nothing happened:
they reverse, with the wheel over, for a quarter of a shift. Fifteen seconds
is a long time to be backing up - long enough to get clear of whatever it
was, and long enough that on a street the player is still on, the car is
recycled somewhere ahead before it finishes. That is the intent: a shunt
clears itself off the road rather than becoming a permanent obstacle in the
middle of it. The lock alternates by index so a pile-up does not reverse in
formation.

### Giving way is two rules

- **Do not close on the car in front.** The speed a driver wants falls off
  with the gap to anything ahead in its own corridor, measured bumper to
  bumper, so a queue settles instead of concertinaing.
- **Give way to the right.** Anything crossing the nose from the right,
  within about four cells and actually moving, means wait. That is the
  junction rule everywhere that drives on the right, and it is enough to
  stop two cars arriving at the same crossroads from arriving in the same
  place.

And traffic now collides with *itself*, not only with the player. It used to
pass clean through, which is invisible until you are following a queue and
two of them occupy the same six metres of road. Measured over 1,800 ticks:
33 car-ticks spent inside another car with the drivers giving way, 366 with
them ignoring each other.

## 5a. The cab goes round things

The autopilot looks six cells up its own nose for something slower than
itself in a corridor a car and a bit wide, and leans past it - a third of a
turn of lock at most, on top of whatever the lane wants, strongest when the
gap is smallest - while lifting off if the gap is closing anyway.

Three things about it are load-bearing, and each of them was measured by
being got wrong first.

**The side is committed to.** Deciding it fresh every tick is the obvious
version and is worse than doing nothing: a car a little to the left is passed
on the right, which moves it to the right in the frame, which asks for a pass
on the left, and the cab drives up the middle of what it was avoiding at full
lock in alternating directions. The decision is held for most of a second.

**Dead ahead goes to the kerb side.** The tidier-looking choice is to pull
towards the crown, and it is wrong: on a road where the traffic keeps right,
the space to the left of the thing in front is the oncoming lane.

**It only pulls out where there is road to pull out onto, and only overtakes
on a narrow street for something that has stopped.** Dodging with no regard
for the kerb spent 43 per cent of one run's travelling ticks off the
carriageway against 2; overtaking merely-slower traffic on a two-cell street
- where the only room is the oncoming lane - took the right-hand-lane figure
from 85 per cent to 57.

## 6. Where a fare stands

Both ends of a job are on the **pedestrian** network — pavement, plaza or
park — and never on the carriageway. A marker in the middle of the road asks
the player to park in the traffic to earn it, and asks the autopilot to stop
dead on an avenue.

So a fare carries four places rather than two: where the passenger is, which
is where the circle is painted, and the kerb beside them, which is where a
car can actually be. The autopilot drives to the kerb; the handover happens
when the cab is within reach of the *person*, which is a stopping circle plus
the cell between them. The last step is the passenger's.

The coins are the route. They used to be a Manhattan L drawn between the two
ends with whichever of its cells happened to land on a road kept — which
between two ends of a bent street can be three coins, or none. Now they are
every third cell of the same breadth-first route the cabbie plans with, so
they are on the road, in order, joined up, and a player following them is
being shown the way rather than a bearing.

## 7. Masses and sizes

| | Length | Half-length | Mass | In a collision |
|---|---:|---:|---:|---:|
| Taxi | 12 m | 6 m | 10 | **30** |
| Traffic | 9.6 m | 4.8 m | 9 | 9 |
| Bus | 13.5 m | 6.75 m | 40 | **120** |

The cab is 5.4 m tall and the bus 8.1 m, which are not real dimensions and
are not meant to be: at forty rows a vehicle drawn honestly is three
characters and reads as a crate. What matters is the order and the ratios,
and that the cab's roof is low enough to see the road over - see
[`CarKind::hull`].

The bus was twice as big as this and lost a quarter twice. An eight-cell bus
is forty-eight metres, which is longer than most of the buildings it drives
past are wide, and it filled a two-cell street end to end.

## 8. No square roots

`speed()` uses the octagonal approximation `max + 3/8 × min`: two
comparisons and a shift, within 4%, which is well inside what a speedometer
in a game like this is for. `normalise` uses the same ruler, so "how far
apart are they" and "how fast is it going" are measured the same way.

`atan2_approx`, for the compass arrow, is the standard octant fold plus a
cubic — accurate to about a fifth of a degree, which is a fifth of a
character.

## 9. Holding a key down

A terminal sends a byte when a key goes down and nothing at all when it comes
up. So "is the accelerator pressed" is not a question the input stream can
answer, and worse: a terminal autorepeats **the most recently pressed key
only**. Hold `w`, then press `q`, and the `w` stops arriving entirely.
Accelerating through a corner — which is the whole of driving — was the one
thing the input could not express.

### Every control is an axis

Not a flag. A press winds the level on over a fifth of a second and letting
go winds it off over an eighth, which is what makes holding a key feel like
leaning on a pedal rather than like setting a bit. It also composes with the
engine curve above: the throttle ramps, and then the engine ramps.

### The keys

`wasd` is the car — throttle, brake, and the wheel on `a` and `d`, with `q`
and `e` on the wheel as well because that is where a walker's turn keys are
and getting into the cab should not move your hand. The arrows are the view:
left and right swing the camera round the cab and let go to centre it, up and
down are a second throttle and brake, because the chase camera sets its own
pitch and there is nothing up there to look at.

### Ask the terminal for releases

The progressive keyboard protocol — kitty's, implemented by ghostty, WezTerm,
foot and others — reports press, repeat and release as separate events. The
handshake is one round trip: query the flags with `CSI ? u` and send a
primary device attributes request straight after it. Every terminal ever made
answers the second, so a terminal that has answered *it* without answering
the first does not speak the protocol. That turns "wait and see if anything
comes back" into a definite answer, and the reply is consumed on the spot,
before the reader thread exists — an unread device-attributes report is a
handful of keystrokes as far as the decoder is concerned.

With the protocol, a held key is held and two of them are two.

### Without it, half a second of grace

A press stays live for half a second and every repeat renews it. Half,
because that is the *initial* delay before a keyboard starts repeating:
shorter and the first half second of holding a key is a dip, because the
terminal has sent one byte and is not yet sending more. Measured against an
emulated terminal at the system defaults — 500 ms to the first repeat, then
one every 33 ms — a quarter-second grace read 43 and 52 mph at the two
moments where half a second read 58 and 84.

The grace is also the whole of what makes two keys work without the protocol:
it is what keeps the throttle on for the half second after `q` steals the
autorepeat from `w`. The price is that a tap lingers, and the fix for that is
not a shorter grace — it is a terminal that reports releases.

## 10. The chase camera

Two things make it feel like a driving camera rather than a camera bolted to
a car.

The heading **lags** the car's, so a flick of the wheel swings the view a
moment later and a drift is watched from the outside rather than from inside
the spin.

And the boom **shortens until it is out of a building**, because a chase
camera that clips through a wall shows you the inside of the wall at exactly
the moment you most need to see the road.

### The driver's head

A head is on a neck, and a neck is a spring. Get on the throttle and the head
is left behind — the chin comes up and you see more sky than road. Stand on
the brake and it is thrown forward, and you see more road than sky. Hold a
speed, *any* speed, and it sits where it always sat.

That last part is the whole design: the lean answers to acceleration and to
nothing else. A hundred and fifty miles an hour down a straight looks exactly
like standing still, and the moment you lift off is the moment you feel.
Since the engine's force tapers as the speed comes up, the horizon settles
back on its own as the car runs out of acceleration — nothing arranges that,
it falls out of the two models meeting.

It is a second-order response rather than a number the camera is told. A
first-order one — pitch proportional to acceleration — moves the horizon the
instant the throttle does, which reads as the *picture* twitching rather than
as the driver moving. A spring has somewhere to be and takes time to get
there, so a stab of throttle sends the head back, past where it settles, and
down again over about half a second. Deliberately underdamped, at about four
tenths of critical: damped harder there is no bob at all, only a lean, and
damped less it wobbles for a second after every gearchange the car does not
have.

Measured on a forty-row frame, from rest:

| | rows of horizon |
|---|---:|
| Flat out from a standstill | 3 rows of sky |
| Lifting off at speed | 1 row of road, then back |
| Hard on the brakes | 4 rows of road |
| Any constant speed | none |

The travel is a ninth of the frame with a floor of four rows, so it is the
same gesture in a twenty-four row window as in a sixty row one, and never
small enough that the horizon flickers rather than moves. It is rounded to
whole rows rather than truncated: the renderer can only shear the horizon by
whole rows, and truncation is not symmetric about zero, so the car appeared
to dive less than it squatted.

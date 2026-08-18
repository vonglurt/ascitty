# Driving

`crates/ascitty-core/src/drive.rs`.

## 1. This is not a simulation

There are no tyres, no weight transfer, no suspension and no engine curve,
and adding any of them would make the car worse.

Four properties are modelled and nothing else.

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

### Grip falls off with speed, and the handbrake removes it

Interpolated between the parked figure and the flat-out one, so the car gets
loose as it gets fast without a threshold anyone can feel as a switch. Slow
corners are on rails; fast ones are not. The handbrake drops grip to almost
nothing, which is how you get the car sideways on purpose.

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

## 3. Cars hitting cars

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

## 4. Masses

| | Mass | Half-length |
|---|---:|---:|
| Taxi | 10 | 0.25 |
| Traffic | 9 | 0.25 |
| Bus | 40 | 0.5 |

A taxi sends a parked saloon spinning and barely moves a bus, which is the
point of there being a bus.

## 5. No square roots

`speed()` uses the octagonal approximation `max + 3/8 × min`: two
comparisons and a shift, within 4%, which is well inside what a speedometer
in a game like this is for. `normalise` uses the same ruler, so "how far
apart are they" and "how fast is it going" are measured the same way.

`atan2_approx`, for the compass arrow, is the standard octant fold plus a
cubic — accurate to about a fifth of a degree, which is a fifth of a
character.

## 6. A terminal cannot tell you a key was released

It sends a byte when a key goes down and nothing at all when it comes up, so
"is the accelerator pressed" is not a question the input stream can answer.
What it *can* answer is "was it pressed recently", and a short decay turns
the terminal's own autorepeat into something that reads as a held pedal.

Five frames. Longer than the autorepeat interval and the car stutters; much
longer and it will not stop.

## 7. The chase camera

Two things make it feel like a driving camera rather than a camera bolted to
a car.

The heading **lags** the car's, so a flick of the wheel swings the view a
moment later and a drift is watched from the outside rather than from inside
the spin.

And the boom **shortens until it is out of a building**, because a chase
camera that clips through a wall shows you the inside of the wall at exactly
the moment you most need to see the road.

//! Billboards: everything in the city that is not a building.
//!
//! Lamp posts, hydrants, trees, traffic, pedestrians, the fare markers and
//! the coins are all drawn the same way - as a flat card standing at a world
//! position, always facing the camera, clipped against the depth of the wall
//! behind it.
//!
//! # The shapes are ASCII art in the source
//!
//! A sprite is written as eight rows of eight characters, right here in the
//! file, and each character names a glyph from the catalogue.  That is not
//! laziness: the whole project is about what a shape looks like when it is
//! made of characters, and a sprite editor that is not itself made of
//! characters would be lying about the medium.  It also means adding a
//! mailbox is four lines of art rather than a data file and a loader.
//!
//! # Clipping
//!
//! The renderer leaves behind one distance per screen column - how far away
//! the nearest wall in that column was.  A billboard nearer than that is
//! drawn; further, it is not.  Per column rather than per cell, which is the
//! usual approximation and wrong only when something should be visible over
//! the top of a *near* building, which for a hydrant it never is.

use crate::atmos::Atmos;
use crate::camera::Camera;
use crate::catalog::{self, GlyphId};
use crate::fixed::{self, Fx, ONE};
use crate::frame::{Cel, Frame};
use crate::palette;
use crate::raycast::{Proj, CELL_ASPECT};
use crate::trig::{self, Ang};

/// What a billboard is a picture of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stamp {
    /// A street light: post, arm, lamp.
    LampPost,
    /// A traffic signal on a pole.
    Signal,
    /// A fire hydrant.
    Hydrant,
    /// A mailbox - a thing that exists to be knocked over.
    Mailbox,
    /// A street tree.
    Tree,
    /// A bollard.
    Bollard,
    /// A parking meter.
    Meter,
    /// A three-box saloon.
    Car,
    /// A boxy four-wheel-drive: short, tall, upright glass.
    Jeep,
    /// A land yacht of about 1972: very long, very low, most of it bonnet.
    Boat,
    /// The cab.  Longer than a saloon, with a checker band and a sign on
    /// the roof, because a taxi you are chasing has to be identifiable at a
    /// glance among a street full of traffic.
    Taxi,
    /// A car that has been hit hard enough to show it.
    Wreck,
    /// A bus.
    Bus,
    /// A person.
    Ped,
    /// A fare coin, spinning.
    Coin,
    /// The marker over a waiting passenger.
    Pickup,
    /// The marker over their destination.
    Dropoff,
    /// What is left of something you drove through.
    Debris,
}

/// One billboard in the world.
#[derive(Clone, Copy, Debug)]
pub struct Billboard {
    /// Position.
    pub x: Fx,
    /// Position.
    pub y: Fx,
    /// Height of the bottom of the card above the pavement.
    pub base: Fx,
    /// Width of the card, in world units.
    pub w: Fx,
    /// Height of the card, in world units.
    pub h: Fx,
    /// What it is a picture of.
    pub stamp: Stamp,
    /// Hue, where the stamp does not insist on its own.
    pub hue: u8,
    /// Animation phase - the spin of a coin, the stride of a walker.
    pub phase: u8,
    /// How far over it has been knocked, 0 upright to 8 flat.  Knocked-over
    /// things lean, which is cheaper and reads better than an animation.
    pub lean: u8,
    /// Which way round a vehicle is.  Ignored by everything else - a
    /// hydrant has no far side.
    pub view: Aspect,
}

impl Billboard {
    /// An upright thing on the pavement.
    pub fn upright(stamp: Stamp, x: Fx, y: Fx, w: Fx, h: Fx, hue: u8) -> Billboard {
        Billboard { x, y, base: 0, w, h, stamp, hue, phase: 0, lean: 0, view: Aspect::END_ON }
    }
}

// --- the art ---------------------------------------------------------------
//
// Eight rows, top first, eight columns.  A space is transparent.  Every
// other character names a catalogue glyph in `glyph_for` below, and the
// second character of the pair - in `LIT` - says how bright it is.

/// A street light: a mast, a lamp, and the glow around it.
///
/// The glow is drawn rather than simulated.  A point source with real
/// falloff would light the pavement under it, which needs a second pass over
/// the ground; a halo of two densities around the bulb costs two glyphs and
/// reads, at this size, as the same thing.  The bulb sits at the very top so
/// that the mast can be lengthened without the head growing with it.
#[rustfmt::skip]
const LAMP_POST_ART: [&str; 8] = [
    "  .:*:. ",
    "  :*O*: ",
    "  .:*:. ",
    "   ||   ",
    "   ||   ",
    "   ||   ",
    "   ||   ",
    "  ====  ",
];

#[rustfmt::skip]
const SIGNAL_ART: [&str; 8] = [
    "  ####  ",
    "  #r #  ",
    "  #y #  ",
    "  #g #  ",
    "  ####  ",
    "   ||   ",
    "   ||   ",
    "  ====  ",
];

#[rustfmt::skip]
const HYDRANT_ART: [&str; 8] = [
    "        ",
    "        ",
    "        ",
    "        ",
    "   #    ",
    "  ###   ",
    "  ###   ",
    " #####  ",
];

#[rustfmt::skip]
const MAILBOX_ART: [&str; 8] = [
    "        ",
    "        ",
    "  ####  ",
    " ###### ",
    " ###### ",
    "   ||   ",
    "   ||   ",
    "  ====  ",
];

#[rustfmt::skip]
const TREE_ART: [&str; 8] = [
    "  TTT   ",
    " TTTTT  ",
    "TTTTTTT ",
    "TTTTTTT ",
    " TTTTT  ",
    "  t t   ",
    "   t    ",
    "  ===   ",
];

#[rustfmt::skip]
const BOLLARD_ART: [&str; 8] = [
    "        ",
    "        ",
    "        ",
    "        ",
    "   ##   ",
    "   ##   ",
    "   ##   ",
    "  ====  ",
];

#[rustfmt::skip]
const METER_ART: [&str; 8] = [
    "        ",
    "        ",
    "   ##   ",
    "   o    ",
    "   |    ",
    "   |    ",
    "   |    ",
    "  ==    ",
];

/// A boxy four-wheel-drive, seen from the side: upright glass, no overhang
/// worth speaking of, wheels at the corners.
/// A long, low car of about 1972, seen from the side: most of it is bonnet,
/// the cabin is set well back, and the roofline runs into the boot.
/// A bus from the side: a box on wheels, and nothing else to say about it.
#[rustfmt::skip]
const PED_ART: [&str; 8] = [
    "        ",
    "        ",
    "   o    ",
    "   #    ",
    "  ###   ",
    "   #    ",
    "   |    ",
    "  = =   ",
];

#[rustfmt::skip]
const COIN_ART: [&str; 8] = [
    "        ",
    "  ####  ",
    " ###### ",
    " ###### ",
    " ###### ",
    "  ####  ",
    "        ",
    "        ",
];

#[rustfmt::skip]
const PICKUP_ART: [&str; 8] = [
    "  ####  ",
    " ###### ",
    "  ####  ",
    "   ##   ",
    "        ",
    "        ",
    "        ",
    "        ",
];

#[rustfmt::skip]
const DEBRIS_ART: [&str; 8] = [
    "        ",
    "        ",
    "        ",
    "        ",
    "        ",
    "        ",
    " .  . . ",
    " ...#.. ",
];

/// A blank card, for the stamps that are painted by a function rather than
/// sampled from art - see [`paint_car`].  Nothing ever reads it.
const NO_ART: [&str; 8] = ["        "; 8];

impl Stamp {
    /// The art for this stamp.
    ///
    /// The vehicles have none: they are painted by [`paint_car`] at whatever
    /// resolution they are drawn at, which is the whole point of that
    /// function, and eight rows of eight characters left lying about for
    /// them would rot the first time the painter changed.
    fn art(self) -> &'static [&'static str; 8] {
        match self {
            Stamp::LampPost => &LAMP_POST_ART,
            Stamp::Signal => &SIGNAL_ART,
            Stamp::Hydrant => &HYDRANT_ART,
            Stamp::Mailbox => &MAILBOX_ART,
            Stamp::Tree => &TREE_ART,
            Stamp::Bollard => &BOLLARD_ART,
            Stamp::Meter => &METER_ART,
            Stamp::Ped => &PED_ART,
            Stamp::Coin => &COIN_ART,
            Stamp::Pickup | Stamp::Dropoff => &PICKUP_ART,
            Stamp::Debris => &DEBRIS_ART,
            Stamp::Car
            | Stamp::Jeep
            | Stamp::Boat
            | Stamp::Taxi
            | Stamp::Wreck
            | Stamp::Bus => &NO_ART,
        }
    }

    /// Whether this belongs at the kerb rather than set back on the
    /// pavement.
    ///
    /// Street lighting and signals stand at the edge of the carriageway,
    /// which is where they are of use; a lamp in the middle of the pavement
    /// lights the wall.  Vegetation goes in the verge behind them.
    pub fn kerbside(self) -> bool {
        matches!(self, Stamp::LampPost | Stamp::Signal | Stamp::Hydrant | Stamp::Bollard)
    }

    /// Whether this is planted, and therefore belongs in the verge.
    pub fn planted(self) -> bool {
        matches!(self, Stamp::Tree)
    }

    /// Whether this is something a car can flatten.
    pub fn frangible(self) -> bool {
        matches!(
            self,
            Stamp::LampPost | Stamp::Mailbox | Stamp::Hydrant | Stamp::Bollard | Stamp::Meter | Stamp::Signal
        )
    }
}

/// One art character, as a glyph and a brightness, or `None` for transparent.
///
/// The characters chosen for the art are the ones that *look* like what they
/// mean in a fixed-width editor, so the art above is legible as art.  What
/// they map to is a different question and is answered here.
fn glyph_for(c: char, hue: u8, phase: u8) -> Option<(GlyphId, u8, u8)> {
    Some(match c {
        ' ' => return None,
        // A body panel.  A step darker than it was: a car is a thing with a
        // shape, and at the top of the ramp it was a lamp with a shape.
        '#' => (catalog::G_SOLID, hue, 5),
        '.' => (catalog::shade(3), hue, 3),
        '|' => (catalog::ST_POST, palette::H_WHITE, 3),
        '=' => (catalog::G_CORNICE + 3, palette::H_WHITE, 2),
        // The cab's own rear lamps.  Bit 1 of the phase is the brake, so
        // they sit there dim and come up when you lift off - which is the
        // only feedback in the frame that says the car heard you, and on a
        // chase camera it is the part of the car you are looking at.
        'b' => {
            if phase & 2 == 2 {
                (catalog::G_SOLID, palette::H_RED, 7)
            } else {
                (catalog::ST_LAMP, palette::H_RED, 4)
            }
        }
        // Lights, and which end of the car you are looking at.
        //
        // The low bit of the phase says the camera is in front of it.  White
        // means it is coming towards you and red means it is going away,
        // which is the only cue in the frame that says which way a car is
        // pointing - a box of body panels does not, and "am I about to hit
        // that" is a question about direction rather than about position.
        'o' => {
            if phase & 1 == 1 {
                (catalog::ST_LAMP, palette::H_WHITE, 7)
            } else if phase & 2 == 2 {
                // On the brakes, and you are behind it.
                (catalog::G_SOLID, palette::H_RED, 7)
            } else {
                (catalog::ST_LAMP, palette::H_RED, 5)
            }
        }
        // A street light, which is lit and stays lit.
        //
        // It used to share the cars' 'o' - the lamp that is white when it is
        // pointed at you and red when it is not - so a row of street lights
        // changed colour as you drove past them, and a lamp post that goes
        // white, red, white is a lamp post that is flashing.  A street light
        // has no front and no back: it is the same full-intensity lamp from
        // every side, all the time.
        'O' => (catalog::ST_LAMP, palette::H_YELLOW, 7),
        'T' => (catalog::FLORA_CANOPY, palette::H_GREEN, 4),
        't' => (catalog::FLORA_TRUNK, palette::H_BROWN, 3),
        'L' => (catalog::G_QUAD + 12 - 1, hue, 4), // lower half - the flank
        'R' => (catalog::G_QUAD + 12 - 1, hue, 4),
        // The checker band, and the sign on the roof.
        'k' => (catalog::G_QUAD + 6 - 1, palette::H_WHITE, 7),
        'S' => (catalog::G_SOLID, palette::H_YELLOW, 7),
        // Two densities of halo around a lamp.  Sparse dithers rather than
        // solid glyphs: a glow has no edge, and anything with an edge reads
        // as a lampshade.
        '*' => (catalog::G_HAZE + 3, palette::H_YELLOW, 6),
        ':' => (catalog::G_HAZE + 1, palette::H_YELLOW, 4),
        // The three signal aspects, only one of which is ever lit; which one
        // comes from the phase, so a junction full of them stays in step.
        'r' => (catalog::G_SOLID, palette::H_RED, if phase.is_multiple_of(3) { 7 } else { 1 }),
        'y' => (catalog::G_SOLID, palette::H_YELLOW, if phase % 3 == 1 { 7 } else { 1 }),
        'g' => (catalog::G_SOLID, palette::H_GREEN, if phase % 3 == 2 { 7 } else { 1 }),
        _ => (catalog::G_SOLID, hue, 5),
    })
}

/// How wide a box of `len` by `wid` looks, and whether you are seeing it
/// side-on.
///
/// `yaw` is the way the box is pointing; `(vx, vy)` is the direction from
/// the camera to it.  The silhouette of a rectangle is
///
/// ```text
///     width = len * |sin t| + wid * |cos t|
/// ```
///
/// where `t` is the angle between the two - so a car looked at down its own
/// length is as wide as a car is wide, broadside it is as wide as a car is
/// long, and everything between is the two corners.  That single expression
/// is the whole of what makes traffic read as boxes driving along a street
/// rather than as cards turning to face you: a car that passes across the
/// view stretches out and then shortens again as it goes.
///
/// Exact, and it costs two multiplies.  There is no approximation here to
/// justify - the projection of a rectangle onto a line really is this.
pub fn silhouette(len: Fx, wid: Fx, yaw: Ang, vx: Fx, vy: Fx) -> (Fx, bool) {
    let a = aspect(len, wid, yaw, vx, vy);
    (a.width, a.end < fixed::HALF)
}

/// Which way round a car is, as the card sees it.
///
/// See [`aspect`].
#[derive(Clone, Copy, Debug)]
pub struct Aspect {
    /// How wide the card is, in world units.
    pub width: Fx,
    /// What fraction of the card is the *end* of the car rather than its
    /// flank, from 0 (broadside) to 1 (dead astern or dead ahead).
    pub end: Fx,
    /// Whether that end band is on the left of the card.
    pub end_left: bool,
    /// Whether the end you can see is the front of the car.
    pub front: bool,
    /// Whether the car's nose points to the left of the card.  Well defined
    /// at every angle, including broadside, which is where the flank's
    /// bonnet and boot have to be told apart.
    pub nose_left: bool,
}

impl Aspect {
    /// Looked at down its own length: what everything that is not a vehicle
    /// gets, and what a vehicle gets before anybody has worked out where the
    /// camera is.
    pub const END_ON: Aspect =
        Aspect { width: 0, end: ONE, end_left: false, front: false, nose_left: false };
}

/// Where the camera is standing relative to a car, in the terms the painter
/// needs.
///
/// # Why a fraction and not eight sprites
///
/// A car used to be drawn from one of two pictures - end-on or broadside -
/// chosen by which of the two was larger, so a car at any angle at all was
/// drawn as whichever extreme it was nearer.  At forty-five degrees, where
/// half of what you can see is the flank and half is the boot, it flipped
/// between them: the same car crossing a junction turned from a rear view
/// into a side view in one frame, at exactly the moment you were watching
/// it.  The silhouette was already exact - it is the projection of a
/// rectangle and always has been - so the *width* was right and the picture
/// inside it was not.
///
/// Eight fixed aspects at forty-five degrees apart is the usual fix and is
/// still a fix for a problem that does not need one.  The card is painted by
/// a function, so it can simply be told how much of each view to draw: the
/// end band is `wid * |cos t|` of the width and the flank is `len * |sin t|`
/// of it, which are the two terms the width is already the sum of.  At
/// forty-five degrees that is a boot and a flank side by side in the
/// proportions a boot and a flank appear in, and it moves continuously
/// through every angle rather than in eight steps.
///
/// Which side the end band goes on is the one thing left, and it is a sign:
/// the end is on the left of the card when the car's nose points to the
/// right of the view, which is a cross product.
pub fn aspect(len: Fx, wid: Fx, yaw: Ang, vx: Fx, vy: Fx) -> Aspect {
    // The car's own axis, and the axis across it.
    let (ax, ay) = (trig::cos(yaw), trig::sin(yaw));
    // Normalise the view direction, cheaply: the octagonal norm is within
    // six per cent, and six per cent of a car's length is a tenth of a
    // character at any distance you can see one.
    let (a, b) = (fixed::abs(vx), fixed::abs(vy));
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    let n = hi + fixed::mul(lo, fixed::ratio(3, 8));
    if n <= 0 {
        return Aspect::END_ON;
    }
    let (ux, uy) = (fixed::div(vx, n), fixed::div(vy, n));
    // The signed view along the car's axis and across it.  `vx, vy` runs
    // from the camera to the car, so a positive `along` means the car is
    // pointing away and you are looking at its back.
    let along = fixed::mul(ux, ax) + fixed::mul(uy, ay);
    let across = fixed::mul(ux, -ay) + fixed::mul(uy, ax);
    let (fa, fc) = (fixed::abs(along), fixed::abs(across));
    let end_w = fixed::mul(wid, fa);
    let side_w = fixed::mul(len, fc);
    let width = end_w + side_w;
    // Which way the car's nose points across the screen.
    //
    // The camera's right-hand axis is `(-uy, ux)` - see `Camera::plane` -
    // and the car's nose dotted into it is `-across`, so the nose points to
    // the left of the card exactly when `across` is positive.  This is
    // well defined everywhere, including broadside, where `along` is nearly
    // zero and its sign is noise.
    let nose_left = across > 0;
    // The visible end is the nose if you are in front of it and the tail if
    // you are behind it, so which side the end band goes on is the nose's
    // side or the other one.  It used to be the nose's side unconditionally,
    // which is right in front of a car and wrong behind one - and behind one
    // is where the chase camera lives, so the boot was drawn on the wrong
    // corner of the cab for every frame of the game.
    let front = along < 0;
    Aspect { width, end: if width > 0 { fixed::div(end_w, width) } else { ONE }, end_left: nose_left == front, front, nose_left }
}


// --- cars, as a function ---------------------------------------------------
//
// Everything else in this file is eight rows of eight characters, sampled at
// whatever size the thing ends up on screen.  That is the right shape for a
// hydrant.  It is the wrong shape for the car you are looking at for the
// whole game: a cab fourteen rows tall drawn from an eight-row picture is an
// eight-row picture with fat pixels, and no amount of redrawing the eight
// rows fixes it.
//
// So a car is a *function* of where you are on the card, evaluated at the
// resolution it is actually drawn at - the same trick the font uses, for the
// same reason.  Twenty rows of cab get twenty rows of detail.

/// What kind of thing is being painted, and from which side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Body {
    /// Somebody else's car: a three-box saloon.
    Saloon,
    /// Short, tall and square: a four-wheel-drive.
    Jeep,
    /// Long and low: the land yacht.
    Boat,
    /// The one you are driving: chequer band, roof sign.
    Taxi,
    /// Longer, taller, flatter.
    Bus,
    /// A saloon that has been in the wars.
    Wreck,
}

impl Body {
    /// Which body a stamp is.
    ///
    /// No longer "and from which side": which side you are looking from is
    /// [`Aspect`], it varies continuously, and a stamp that named one of two
    /// fixed views is exactly the thing that made a car flip from a rear
    /// view to a side view in a single frame.
    fn of(stamp: Stamp) -> Option<Body> {
        Some(match stamp {
            Stamp::Taxi => Body::Taxi,
            Stamp::Car => Body::Saloon,
            Stamp::Jeep => Body::Jeep,
            Stamp::Boat => Body::Boat,
            Stamp::Bus => Body::Bus,
            Stamp::Wreck => Body::Wreck,
            _ => return None,
        })
    }
}

/// The top of the body along the flank, as a `v` down the card.
///
/// This is the three-box profile, and it is the single most recognisable
/// thing about a car seen from the side: bonnet, cabin, boot.  Without it a
/// side view is a slab of glass the whole length of the vehicle with a roof
/// on top - which is what this drew for a long time, and which reads as a
/// van whatever colour it is painted.
///
/// `u` runs nose-first: 0 is the front bumper and 1 is the back one, so the
/// caller flips it when the car is pointing the other way.
fn flank_top(body: Body, u: Fx) -> Fx {
    // A bus is a box and has no bonnet worth drawing at this size.
    if body == Body::Bus {
        return 0;
    }
    let waist = waist_of(body, true);
    // Where the cabin starts and stops, and how far down the bonnet and the
    // boot sit.  The cabin of a land yacht is set a long way back - most of
    // the car is in front of the driver - and a jeep is nearly all cabin.
    let (front, back, bonnet, boot) = match body {
        Body::Jeep => (
            fixed::ratio(16, 100),
            fixed::ratio(92, 100),
            fixed::mul(waist, fixed::ratio(70, 100)),
            fixed::mul(waist, fixed::ratio(55, 100)),
        ),
        Body::Boat => (
            fixed::ratio(46, 100),
            fixed::ratio(80, 100),
            fixed::mul(waist, fixed::ratio(92, 100)),
            fixed::mul(waist, fixed::ratio(80, 100)),
        ),
        _ => (
            fixed::ratio(32, 100),
            fixed::ratio(76, 100),
            fixed::mul(waist, fixed::ratio(88, 100)),
            fixed::mul(waist, fixed::ratio(76, 100)),
        ),
    };
    // The screen pillars are the slopes, and they are what make a car look
    // fast or slow: a short steep one is upright and formal, a long shallow
    // one is a fastback.
    let rake = fixed::ratio(10, 100);
    if u >= front && u <= back {
        return 0;
    }
    if u < front {
        let t = fixed::div((front - u).min(rake), rake);
        return fixed::mul(bonnet, t.clamp(0, ONE));
    }
    let t = fixed::div((u - back).min(rake), rake);
    fixed::mul(boot, t.clamp(0, ONE))
}

/// Whether a point along the flank is in the glasshouse rather than in the
/// bonnet or the boot.
///
/// The same cabin `flank_top` uses, brought in by a pillar's width at each
/// end so the windscreen and the backlight have something to sit in.
fn flank_glass(body: Body, u: Fx, pillar: Fx) -> bool {
    if body == Body::Bus {
        return u > pillar && u < ONE - pillar;
    }
    // Where the roofline is flat is where the cabin is, and a step in from
    // each end of it is where the glass is.
    let flat = flank_top(body, u) <= 0;
    flat
        && flank_top(body, (u - pillar).max(0)) <= 0
        && flank_top(body, (u + pillar).min(ONE)) <= 0
}

/// The last of the width, drawn as the sky rather than as the car.
///
/// Where the panel has turned furthest from you it is at a *grazing* angle,
/// and a painted panel at a grazing angle is a mirror: what you see along
/// the edge of a car is not its colour, it is the sky.  That is the fix for
/// the edge as well as a highlight - the silhouette used to end on a
/// character boundary in the body's own colour, which is a staircase of
/// solid blocks with the city showing through the steps.
///
/// So the last of the width is drawn in the sky's hue, with a *dither*
/// rather than a solid glyph, so the cell is only partly covered and the
/// background comes through it.  A car now ends rather than stopping, and it
/// ends in the colour of what is behind it, which is what makes an edge
/// disappear.
fn rim(edge: Fx, u: Fx, key: Fx, sky: (u8, u8)) -> (GlyphId, u8, u8) {
    let (sh, sl) = sky;
    let t = fixed::div(edge - RIM, ONE - RIM).clamp(0, ONE);
    // Solid at the inner side of the rim and thinning to a quarter covered
    // at the silhouette, so the fade is in *coverage* as well as in colour.
    let cover = 8 - fixed::floor(fixed::mul(t, fixed::from_int(6))).clamp(0, 6);
    // The rim is the sky, lifted where the sun is behind that edge: the
    // bright side of a car has a bright edge and the dark side has a dark
    // one, or the rim reads as a wire round the outside.
    let side_key = if u > fixed::HALF { key } else { -key };
    let up = fixed::floor(fixed::mul(side_key.max(0), fixed::from_int(2)));
    let l = (sl as i32 + up).clamp(1, 7) as u8;
    (catalog::shade(cover as u8), sh, l)
}

/// Half the width of the body at a given height down the card.
///
/// This is the whole silhouette in one expression: a roof narrower than the
/// waist, a waist that runs most of the height, and a slight tuck at the
/// bottom where the sills are.  `v` is 0 at the top of the card and 1 at the
/// ground.
fn body_half(body: Body, side: bool, v: Fx) -> Fx {
    let (roof_top, shoulder, sill) = match (body, side) {
        // A bus is a box: it is nearly the same width all the way up.
        (Body::Bus, _) => (fixed::ratio(42, 100), fixed::ratio(48, 100), fixed::ratio(46, 100)),
        // A jeep is a smaller box.  Upright glass, no tumblehome, and the
        // roof as wide as the waist - which is the whole of what tells a
        // four-wheel-drive from a saloon at thirty characters.
        (Body::Jeep, _) => (fixed::ratio(40, 100), fixed::ratio(47, 100), fixed::ratio(45, 100)),
        // Along its length a car is very nearly a rectangle: the sills, the
        // waist and the roof are all close to the full length, and what
        // shortens the roof is the bonnet and the boot in front of and
        // behind it rather than a taper.  That is `flank_top`'s job, and
        // doing it here as well used to do it twice - a roof that was
        // shorter *and* a cabin that was shorter, so a saloon in profile had
        // a greenhouse a third of its length.
        (_, true) => (fixed::ratio(50, 100), fixed::ratio(50, 100), fixed::ratio(48, 100)),
        // End on, it is nearly as wide at the roof as at the waist.
        (_, false) => (fixed::ratio(30, 100), fixed::ratio(46, 100), fixed::ratio(42, 100)),
    };
    let waist = waist_of(body, side);
    if v < waist {
        // The greenhouse: widening from the roof to the shoulder.
        let t = fixed::div(v, waist).clamp(0, ONE);
        fixed::lerp(roof_top, shoulder, t)
    } else {
        // The body: the shoulder tucking in a little towards the sills.
        let t = fixed::div(v - waist, ONE - waist).clamp(0, ONE);
        fixed::lerp(shoulder, sill, t)
    }
}

/// Where the glass stops and the doors start, down the card.
/// Where the lamps stop and the bumper starts, down the card.
///
/// The lamps and the front wheels want the same corner of the card, and on a
/// car the lamp is the higher of the two: a front view is lamp and bumper
/// across the bottom with the tyres showing beneath.  Sampled at six rows a
/// car gets one row in the lamp band and one below it, which is exactly the
/// arrangement, and it survives all the way down to the smallest car drawn.
const BUMPER: Fx = fixed::ratio(82, 100);

/// How far across the half-width the bodywork runs before the rim starts.
///
/// Five sixths.  The last sixth of a car's width, seen from anywhere, is
/// panel that has turned nearly edge-on to you - which on a real car is the
/// part that shows you the sky instead of the paint.  See the rim in
/// [`paint_face`].
const RIM: Fx = fixed::ratio(83, 100);

/// How far down from the top edge of the body the same fade reaches, as a
/// fraction of the card.
///
/// A twenty-fifth, which on the sizes a car is drawn at is between half a
/// row and two rows: enough to soften a roofline against a bright sky and
/// not enough to eat the roof.
const RIM_TOP: Fx = fixed::ratio(4, 100);

/// How far up the card the wheel arches are cut, from the ground line.
///
/// A fifth of the card: about half the height of the body below the waist,
/// which is where a wheel arch reaches on a car that is not a hot rod.
const ARCH: Fx = fixed::ratio(20, 100);

fn waist_of(body: Body, side: bool) -> Fx {
    let _ = side;
    match body {
        Body::Bus => fixed::ratio(55, 100),
        // A jeep is glass down to the waist and the waist is high, so it is
        // mostly window; a land yacht is the opposite - a low roof over a
        // great deal of flank.
        Body::Jeep => fixed::ratio(58, 100),
        // One figure for both faces of a body, not two.  A waist line is
        // the bottom of the glass and it runs *round* a car: two faces that
        // put it at different heights meet at the corner with a step in it,
        // and a step in the waist reads as a crease down the wing.
        Body::Boat => fixed::ratio(33, 100),
        _ => fixed::ratio(40, 100),
    }
}

/// How much of the top of the cab's card the roof sign occupies.
///
/// A fifth.  A real one is about a foot tall on a car five feet high, which
/// is a tenth - but a tenth of a card that is often twelve rows is one row,
/// and one row cannot hold a bracket, a box and four letters.  A fifth is
/// the smallest band that can be *read* as a roof sign rather than as a
/// lump, and the cab is the one vehicle in the city that has to be
/// identified at a glance from behind, in traffic, at night.
const SIGN_BAND: Fx = fixed::ratio(20, 100);

/// The box itself, as a fraction of the card: where it starts and stops
/// across, and where it stops down.
///
/// The bracket legs live in the gap between the bottom of the box and the
/// roof, and they are what make it a sign *mounted on* a car rather than a
/// yellow brick lying on one.  Anybody who has looked at a cab has looked at
/// those two little legs without noticing them; take them away and the sign
/// reads as part of the roof.
const SIGN_HALF: Fx = fixed::ratio(15, 100);
const SIGN_BOX: Fx = fixed::ratio(12, 100);

/// A 3x3 alphabet, four letters wide, spelling one word.
///
/// Three rows of a `u16`, three bits a letter, most significant bit first,
/// reading `T A X I` left to right with a blank column between each pair.
/// Fifteen bits of the sixteen, which is the whole font: this is the only
/// word the program ever writes.
///
/// Three rows rather than the five a legible alphabet wants, because three
/// is what there is.  The sign box is about an eighth of the cab's card, and
/// a cab filling half the height of a fifty-row frame gives that eighth four
/// rows: a five-row glyph sampled at four rows is not small type, it is a
/// different pattern, and `T` comes out as a bar.  At three rows every
/// letter here survives being sampled at three, which is the only size that
/// matters.
// The grouping is the letters, not a number's digits: three bits a glyph
// with a blank column between.  Clippy would rather they were nibbles.
#[allow(clippy::unusual_byte_groupings)]
#[rustfmt::skip]
const TAXI_ROWS: [u16; 3] = [
    //  T      A      X      I
    0b111_0_111_0_101_0_111,
    0b010_0_111_0_010_0_010,
    0b010_0_101_0_101_0_111,
];

/// The rows and columns the sign box needs before the word is set in it.
///
/// One row and one column per feature, and a font this coarse has three of
/// each per letter with a gap between: fifteen columns and three rows is the
/// point below which the letters stop being letters.  Below it the sign
/// still says something - see the bar in [`roof_sign`] - it just stops
/// pretending to be readable, which is what a lit sign does at any distance
/// at all.
const TYPE_ROWS: i32 = 3;
const TYPE_COLS: i32 = 15;

/// The lit box, the legs it stands on, and the word in it.
///
/// Returns `None` where the sign is not, which is most of the band: the
/// caller treats the whole top of the card as the sign's and draws nothing
/// where this declines.
fn roof_sign(u: Fx, v: Fx, sky: (u8, u8), rows: i32, cols: i32) -> Option<(GlyphId, u8, u8)> {
    if v >= SIGN_BAND {
        return None;
    }
    let mid = fixed::abs(u - fixed::HALF);
    if v < SIGN_BOX {
        if mid > SIGN_HALF {
            return None;
        }
        // The rim of the box, which is what gives it a thickness: a lit
        // panel with no darker edge is a hole in the picture rather than an
        // object in front of it.
        let rim = fixed::ratio(3, 100);
        if mid > SIGN_HALF - rim || v < rim || v > SIGN_BOX - rim {
            return Some((catalog::G_SOLID, palette::H_BROWN, 2));
        }
        // Inside it: the word, in the dark, on a lit ground.  The lettering
        // is measured across the *inner* box rather than the whole card, so
        // it stays centred whatever the rim costs.
        let inner_half = SIGN_HALF - rim;
        let lu = fixed::div(u - (fixed::HALF - inner_half), inner_half * 2);
        let lv = fixed::div(v - rim, SIGN_BOX - rim * 2);
        // A margin round the type, so the letters never touch the rim.
        //
        // ...and only if the box is tall enough to hold the alphabet.  Five
        // rows of type sampled at three rows is not small type, it is a
        // different pattern - the `T` comes out as a bar and the word reads
        // as a picket fence.  Below that the sign is simply lit, which is
        // what a roof sign at forty metres is anyway.
        let (mx, my) = (fixed::ratio(8, 100), fixed::ratio(18, 100));
        let inside = lu > mx && lu < ONE - mx && lv > my && lv < ONE - my;
        let box_rows = fixed::floor(fixed::mul(SIGN_BOX, fixed::from_int(rows)));
        let box_cols = fixed::floor(fixed::mul(inner_half * 2, fixed::from_int(cols)));
        if inside && box_rows >= TYPE_ROWS && box_cols >= TYPE_COLS {
            let col = fixed::floor(fixed::mul(
                fixed::div(lu - mx, ONE - mx * 2),
                fixed::from_int(TYPE_COLS),
            ))
            .clamp(0, TYPE_COLS - 1);
            let row = fixed::floor(fixed::mul(
                fixed::div(lv - my, ONE - my * 2),
                fixed::from_int(TAXI_ROWS.len() as i32),
            ))
            .clamp(0, TAXI_ROWS.len() as i32 - 1);
            if TAXI_ROWS[row as usize] >> (TYPE_COLS - 1 - col) & 1 == 1 {
                return Some((catalog::G_SOLID, palette::H_BLACK, 0));
            }
        } else if inside && lv > fixed::ratio(35, 100) && lv < fixed::ratio(65, 100) {
            // Too small to set the word, so the word becomes what it looks
            // like from across the street: a dark bar in a lit box.  Which
            // is not a placeholder - it is what writing on an illuminated
            // sign resolves to before you can read it, and a sign that goes
            // blank at range reads as a lamp rather than as a sign.
            return Some((catalog::G_SOLID, palette::H_BLACK, 0));
        }
        // ...and the ground it is on, which is the one thing on the car
        // that is a *lamp*: it is at full luminance whatever the sky is
        // doing, because a roof sign is lit from inside.
        return Some((catalog::G_SOLID, palette::H_YELLOW, 7));
    }
    // The bracket: two legs, inset from the ends of the box, standing on the
    // roof.  Dark, and taking a little of the sky the way a chromed bar
    // does, so it is a bar in the air rather than a gap in the sign.
    let leg = fixed::ratio(4, 100);
    let stand = SIGN_HALF - fixed::ratio(6, 100);
    if fixed::abs(mid - stand) < leg / 2 {
        let (sh, sl) = sky;
        return Some((catalog::G_SOLID, sh, sl.saturating_sub(3).max(1)));
    }
    None
}

/// Paint one point of a car, from wherever you happen to be standing.
///
/// `u` and `v` are where on the card you are, from 0 to 1, left to right and
/// top to bottom.  `view` says how much of the card is the end of the car
/// and how much is its flank - see [`aspect`] - and this function's only job
/// beyond painting is to split the card between the two and hand each half
/// to [`paint_face`] with its own local `u`.
///
/// That split is the three-quarter view.  At forty-five degrees the card is
/// a boot and a flank side by side in the proportions a boot and a flank
/// appear in, with the boot on whichever side the car's nose is turned away
/// from, and the seam between them is the corner of the car.  It moves
/// continuously: a car turning across a junction rolls from one view into
/// the other rather than cutting between two pictures.
#[allow(clippy::too_many_arguments)]
pub fn paint_car(
    body: Body,
    view: Aspect,
    u: Fx,
    v: Fx,
    hue: u8,
    phase: u8,
    sky: (u8, u8),
    key: Fx,
    rows: i32,
    cols: i32,
) -> Option<(GlyphId, u8, u8)> {
    // A band narrower than this is not worth a seam: at a hair off dead
    // astern the flank is a fraction of a character wide, and drawing it
    // costs a pillar down the edge of every car in the city.
    const SLIVER: Fx = fixed::ratio(12, 100);
    let end = view.end.clamp(0, ONE);
    if end >= ONE - SLIVER {
        return paint_face(body, false, u, v, hue, phase, sky, key, rows, cols);
    }
    if end <= SLIVER {
        return paint_face(body, true, u, v, hue, phase, sky, key, rows, cols);
    }
    let (in_end, local) = if view.end_left {
        (u < end, if u < end { fixed::div(u, end) } else { fixed::div(u - end, ONE - end) })
    } else {
        let seam = ONE - end;
        (u >= seam, if u >= seam { fixed::div(u - seam, end) } else { fixed::div(u, seam) })
    };
    // Which side of *this* face the corner of the car is on.
    //
    // Without it the two faces each taper to their own silhouette and the
    // card comes out as two separate cars with a hole between them: at
    // forty-five degrees there was a column of daylight down the corner of
    // every vehicle in the city, with a rim highlight on both sides of it,
    // which reads as a car that has been cut in half.  A body does not end
    // at its own corner - it turns.  So the face nearer the seam runs
    // straight out to it, and only the two outside edges get an edge.
    let seam: i8 = if view.end_left == in_end { 1 } else { -1 };
    // The key light is in *card* space, so it does not care which face this
    // point belongs to - the two faces of a three-quarter view are lit by
    // the same sun from the same side, and a light that flipped at the seam
    // would draw the corner of the car as a crease.
    paint_face_at(
        body,
        !in_end,
        local.clamp(0, ONE),
        v,
        hue,
        phase,
        sky,
        key,
        rows,
        cols,
        seam,
        view.nose_left,
    )
}

/// Paint one point of one face of a car - the end of it, or the flank.
///
/// `side` is true for the flank.  `phase` is the two bits every car carries:
/// bit 0 that you are looking at its front, bit 1 that it is braking.
#[allow(clippy::too_many_arguments)]
pub fn paint_face(
    body: Body,
    side: bool,
    u: Fx,
    v: Fx,
    hue: u8,
    phase: u8,
    sky: (u8, u8),
    key: Fx,
    rows: i32,
    cols: i32,
) -> Option<(GlyphId, u8, u8)> {
    paint_face_at(body, side, u, v, hue, phase, sky, key, rows, cols, 0, true)
}

/// [`paint_face`], plus which side of this face the corner of the car is on:
/// 0 for neither, -1 for the left of the card, +1 for the right.
///
/// A face with a seam does not end on that side - it runs out to it, and the
/// face on the other side of the seam carries on from there.
#[allow(clippy::too_many_arguments)]
pub fn paint_face_at(
    body: Body,
    side: bool,
    u: Fx,
    v: Fx,
    hue: u8,
    phase: u8,
    sky: (u8, u8),
    key: Fx,
    rows: i32,
    cols: i32,
    seam: i8,
    nose_left: bool,
) -> Option<(GlyphId, u8, u8)> {
    // The cab wears its sign above its roof, so the top of its card is not
    // its roof - see `roof_sign`.  Everything below here works in body
    // coordinates, with the sign's band already taken off the top.
    if body == Body::Taxi {
        if let Some(paint) = roof_sign(u, v, sky, rows, cols) {
            return Some(paint);
        }
        if v < SIGN_BAND {
            return None;
        }
    }
    let v = if body == Body::Taxi {
        fixed::div(v - SIGN_BAND, ONE - SIGN_BAND)
    } else {
        v
    };
    let mid = fixed::abs(u - fixed::HALF);
    // Where the wheels start, and where the lamps do.
    //
    // The bands have to be wide enough to survive being sampled at the size
    // a car is actually drawn: a cab six rows tall samples this function at
    // v = 0.08, 0.25, 0.42, 0.58, 0.75 and 0.92, and a band narrower than
    // the gap between two of those is a band that some cars simply do not
    // have.  The brake lights were 0.80 to 0.91 and vanished on anything
    // under eight rows, which is most of the traffic.
    let ground = fixed::ratio(88, 100);
    let lamps = fixed::ratio(70, 100);

    let half = body_half(body, side, v);

    // Lamps, at the bottom corners of the end you are looking at.
    //
    // Before the wheels, because they share that corner of the card and the
    // lamp is the smaller of the two: a car seen end-on is mostly lamp and
    // bumper down there, with the tyres showing between them.
    if !side
        && v > lamps
        && v < BUMPER
        && mid <= half
        && mid > half - fixed::ratio(16, 100)
    {
        return Some(if phase & 1 == 1 {
            (catalog::G_SOLID, palette::H_WHITE, 7)
        } else if phase & 2 == 2 {
            (catalog::G_SOLID, palette::H_RED, 7)
        } else {
            (catalog::G_SOLID, palette::H_RED, 4)
        });
    }

    // Wheels and the arches they sit in.
    //
    // The wheels used to be a dark grey bar under the sills, which at a
    // distance is a shadow: the car had no visible running gear at all, and
    // side-on it read as a brick.  A wheel is black, and what makes it look
    // like a wheel rather than a stripe is the *arch* - a black bite taken
    // out of the bodywork above it, which is the one part of a car's
    // silhouette that says where its wheels are from any angle.
    //
    // The arch is a parabola rather than a semicircle, because it costs two
    // multiplies and no square root and nobody has ever looked at a car and
    // said the wheel arch was the wrong conic.
    let (near, far) = if side {
        (fixed::ratio(22, 100), fixed::ratio(46, 100))
    } else {
        (fixed::ratio(26, 100), fixed::ratio(44, 100))
    };
    if mid > near && mid < far {
        let hub = (near + far) / 2;
        let span = (far - near) / 2;
        // 1 at the middle of the wheel, 0 at its edges.
        let t = fixed::div(mid - hub, span.max(1));
        let lift = (ONE - fixed::mul(t, t)).clamp(0, ONE);
        let arch = ground - fixed::mul(ARCH, lift);
        if v > arch {
            // A lighter hub, so a big wheel reads as a wheel rather than as
            // a hole.  It is smaller than one character at the sizes a car
            // in traffic is drawn at, which is the right way round: near
            // cars get the detail and far ones get a black wheel.
            let up = fixed::div(ground - v, ARCH.max(1));
            let spoke = fixed::abs(up - fixed::ratio(35, 100)) < fixed::ratio(12, 100)
                && fixed::abs(t) < fixed::ratio(30, 100);
            return Some(if spoke {
                (catalog::G_SOLID, palette::H_WHITE, 2)
            } else {
                (catalog::G_SOLID, palette::H_BLACK, 0)
            });
        }
    }

    // Off the end of the card, or outside the body - unless this is the
    // side the corner is on, where the body carries on into the next face.
    let towards_seam = (seam > 0 && u > fixed::HALF) || (seam < 0 && u < fixed::HALF);
    if v > ground || (mid > half && !towards_seam) {
        return None;
    }
    // ...and above the bonnet or the boot, which on a flank is sky.  `u`
    // runs nose-first, so it is flipped when the car points the other way.
    let along_u = if nose_left { u } else { ONE - u };
    let top = if side { flank_top(body, along_u) } else { 0 };
    if v < top {
        return None;
    }

    let waist = waist_of(body, side);
    let roof = fixed::mul(waist, fixed::ratio(35, 100));

    // The rim, before anything that is painted on the body, because a
    // grazing angle shows you the sky whatever panel is behind it - and
    // because the chequer band used to reach the silhouette and put a hard
    // black-and-white staircase down the edge of the cab.
    let edge = if towards_seam { 0 } else { fixed::div(mid, half.max(1)) };
    if edge > RIM {
        return Some(rim(edge, u, key, sky));
    }
    // The same thing along the top.  A roofline, and the slope of a bonnet
    // or a boot, are silhouette edges exactly as the sides are, and they
    // were the ones still ending on a character boundary: the wedge of a
    // three-box profile came to a staircase of solid blocks against the sky.
    // Expressed as a fraction of the same rim so the two agree at the
    // corner where they meet.
    let above = v - top.max(0);
    if above < RIM_TOP {
        let t = ONE - fixed::div(above, RIM_TOP);
        return Some(rim(fixed::lerp(RIM, ONE, t.clamp(0, ONE)), u, key, sky));
    }

    // The roof: body colour, lifted, because it is the panel pointed at the
    // sky.
    if v < roof {
        return Some((catalog::G_SOLID, hue, 6));
    }

    // The glass.  It takes the *sky's* hue rather than the car's, which is
    // what a window does: a windscreen is a dark mirror pointed upwards, so
    // it is blue in the afternoon and gold at sunrise without being told
    // what time it is.  Inset from the body's edge by a pillar's width - and
    // on a flank, inset from the bonnet and the boot as well, because glass
    // that ran to the ends of the car is what made a saloon a van.
    let pillar = fixed::ratio(6, 100);
    let glassy = !side || (top <= 0 && flank_glass(body, along_u, pillar));
    if glassy && v < waist && mid < half - pillar {
        let (sh, sl) = sky;
        // A vertical shade across the glass, brighter at the top where more
        // of the sky is in it.
        let t = fixed::div(v - roof, (waist - roof).max(1)).clamp(0, ONE);
        let luma = sl.saturating_add(1).saturating_sub(fixed::floor(fixed::mul(t, ONE)) as u8);
        return Some((catalog::G_SOLID, sh, luma.clamp(1, 7)));
    }

    // The chequer band, which is the whole of what makes a taxi a taxi.
    //
    // Along the flank only, because that is where it is on the car this one
    // is dressed as: a checker cab wears the band down each side and the
    // ends are plain.  Drawing it round the back as well put a chequered
    // stripe across the boot, and in a three-quarter view - where the card
    // is a boot and a flank side by side - the two halves of it met at the
    // corner at slightly different heights and read as a crease.
    if body == Body::Taxi && side {
        let band = fixed::ratio(55, 100);
        if v > band && v < fixed::ratio(70, 100) {
            let square = fixed::floor(fixed::mul(u, fixed::from_int(10)));
            let white = square.rem_euclid(2) == 0;
            return Some((
                catalog::G_SOLID,
                if white { palette::H_WHITE } else { palette::H_BLACK },
                if white { 7 } else { 0 },
            ));
        }
    }

    // The body.
    //
    // Two terms, and the second is the one that makes it a car rather than a
    // shape cut out of paper.
    //
    // Down the card, it darkens towards the sills, because the lower the
    // panel the more of the road it is facing and the less of the sky.
    let down = fixed::div(v - waist, (ONE - waist).max(1)).clamp(0, ONE);
    let mut luma = 6 - fixed::floor(fixed::mul(down, fixed::from_int(2))).clamp(0, 2) as i32;

    // Across the card, it turns away from you.  A car's flank is a *curved*
    // panel: it faces you in the middle of the card and faces sideways at
    // the edges, so the light lands on one side of it and not the other, and
    // which side is the sun's business - see `key_light`.  That is the whole
    // of the volume, and it costs one multiply.
    //
    // `turn` is how far round the panel has gone, signed: -1 at the left
    // edge of the card, 0 down the middle, +1 at the right.  A panel whose
    // normal points at the sun is a step and a half brighter than one facing
    // straight at the camera, and one pointing away is a step and a half
    // darker, so the two sides of a car are three steps apart.  On an
    // eight-level ramp that is as much as it can be without the dark side
    // going black.
    let turn = fixed::clamp(fixed::div(u - fixed::HALF, half.max(1)), -ONE, ONE);
    let lit = fixed::mul(turn, key);
    luma += fixed::floor(fixed::mul(lit, fixed::ratio(3, 2)) + fixed::HALF);

    let dented = body == Body::Wreck && (fixed::floor(fixed::mul(u + v, fixed::from_int(9))) & 1) == 0;
    if dented {
        luma -= 2;
    }
    Some((catalog::G_SOLID, hue, luma.clamp(1, 7) as u8))
}

/// Where the sun is, across the view, from -1 (hard left) to +1 (hard
/// right).
///
/// The component of the sun's bearing along the camera's *right* axis.  Sun
/// behind you or in front of you gives nothing, which is correct: the card
/// has no way to show a light that is not across it, and a car lit from
/// straight behind the camera is evenly lit, which is what a flat card
/// already looks like.
fn key_light(atmos: &Atmos, cam: &Camera) -> Fx {
    let a = atmos.sun_az();
    let (sx, sy) = (trig::cos(a), trig::sin(a));
    let (dx, dy) = cam.dir();
    // The camera's right-hand axis, which is its direction turned a quarter.
    let (rx, ry) = (-dy, dx);
    // The light comes *from* the sun, so a sun on the right lights the right
    // of everything, and the sign is the dot product as it stands.
    fixed::clamp(fixed::mul(sx, rx) + fixed::mul(sy, ry), -ONE, ONE)
}

/// Draw one billboard.
///
/// Returns false if it was entirely off screen or entirely hidden, which the
/// caller can use to skip the work of animating things nobody can see.
pub fn draw(f: &mut Frame, depth: &[Fx], cam: &Camera, atmos: &Atmos, p: &Proj, b: &Billboard) -> bool {
    let (dx, dy) = cam.dir();
    let (px, py) = cam.plane();
    // Which side of the card the light is on.
    //
    // A billboard always faces the camera, so the only part of the sun's
    // direction the card can express is the part *across* the view - and
    // that part is exactly what makes a car look like a volume rather than a
    // painted rectangle.  The sun rises in the east and sets in the west, so
    // this swings from one side of every car in the city to the other over
    // the course of a day and takes the shading with it: the highlight walks
    // round the bodywork and the shadow stays opposite it.
    //
    // One dot product a billboard, not one a cell.
    let key = key_light(atmos, cam);

    // Into camera space.  The 2x2 matrix [plane | dir] takes camera space to
    // world space, so its inverse takes the sprite the other way; `ty` comes
    // out as the perpendicular distance, in the same units the wall depths
    // are in, which is what makes the comparison below meaningful.
    let (rx, ry) = (b.x - cam.x, b.y - cam.y);
    let det = fixed::mul(px, dy) - fixed::mul(dx, py);
    if det == 0 {
        return false;
    }
    let tx = fixed::div(fixed::mul(dy, rx) - fixed::mul(dx, ry), det);
    let ty = fixed::div(fixed::mul(px, ry) - fixed::mul(py, rx), det);
    if ty < fixed::ratio(1, 8) {
        return false; // behind the camera, or on top of it
    }
    if ty > fixed::from_int(crate::atmos::draw_distance(atmos.haze)) {
        return false;
    }

    // Screen position and size.
    let centre = fixed::mul(
        fixed::from_int(p.w / 2),
        ONE + fixed::div(tx, ty),
    );
    let cx = fixed::floor(centre);
    // Columns per world unit is rows per world unit times the cell aspect.
    let half_w = fixed::floor(fixed::div(
        fixed::mul(fixed::mul(b.w, p.proj), fixed::from_int(CELL_ASPECT)),
        fixed::mul(ty, fixed::from_int(2)),
    ))
    .max(0);
    let rows = fixed::floor(fixed::div(fixed::mul(b.h, p.proj), ty)).max(0);
    if rows == 0 || half_w == 0 {
        return false;
    }
    // The foot of the card, and the top, in screen rows.
    //
    // `p.eye` is the camera's height *above the ground*, which is the number
    // the ground plane is drawn with - see `raycast::projection`.  This used
    // `cam.z`, the camera's absolute height, and the two are not the same
    // thing anywhere the terrain has risen: measured across four places in
    // one city, an eye height of 0.71 to 0.80 against a `cam.z` of 1.17 to
    // 1.83.  Sprites were therefore drawn with an eye height twice to two
    // and a half times too big, which pushes their feet that much further
    // below the horizon - and because the error scales with `1/ty` like
    // everything else in the projection, a car ten cells away sat eleven
    // rows too low while a distant one was almost right.  That is what
    // "the cars are in the wrong place and drift as they recede" was.
    let foot = p.horizon + fixed::floor(fixed::div(fixed::mul(p.eye - b.base, p.proj), ty));
    let top = foot - rows;

    // Leaning: a knocked-over lamp post pivots about its foot.  Shearing the
    // card is not a rotation and does not pretend to be - but at eight
    // characters tall the difference between a shear and a rotation is
    // roughly one character, and a shear is two additions.
    let shear = b.lean as i32;

    let mut drawn = false;
    let art = b.stamp.art();
    // A car is painted by a function at whatever resolution it is drawn at;
    // everything else is eight rows of eight characters.  See `paint_car`.
    let painted = Body::of(b.stamp);
    let sky = atmos.sky_colour();
    for sy in top.max(0)..=(foot - 1).min(p.h - 1) {
        if sy < 0 {
            continue;
        }
        let v = ((sy - top) * 8 / rows.max(1)).clamp(0, 7) as usize;
        let vf = fixed::div(fixed::from_int(sy - top) + fixed::HALF, fixed::from_int(rows.max(1)));
        // Rows nearer the top lean further over.
        let lean_px = shear * (7 - v as i32) * half_w / 8;
        let left = cx - half_w + lean_px;
        let span = fixed::from_int((2 * half_w).max(1));
        for sx in left.max(0)..=(cx + half_w + lean_px).min(p.w - 1) {
            let col = sx as usize;
            if col >= depth.len() || depth[col] < ty {
                continue; // a building is in the way
            }
            let paint = match painted {
                Some(body) => {
                    let uf = fixed::div(fixed::from_int(sx - left) + fixed::HALF, span);
                    paint_car(body, b.view, uf, vf, b.hue, b.phase, sky, key, rows, 2 * half_w)
                }
                None => {
                    let u = ((sx - left) * 8 / (2 * half_w).max(1)).clamp(0, 7) as usize;
                    let ch = art[v].as_bytes().get(u).copied().unwrap_or(b' ') as char;
                    glyph_for(ch, b.hue, b.phase)
                }
            };
            let Some((g, hue, luma)) = paint else {
                continue;
            };
            f.put(sx, sy, Cel { glyph: g, color: atmos.shade(hue, luma, ty) });
            drawn = true;
        }
    }
    drawn
}

/// Draw a list of billboards, furthest first so nearer ones win.
pub fn draw_all(
    f: &mut Frame,
    depth: &[Fx],
    cam: &Camera,
    atmos: &Atmos,
    p: &Proj,
    boards: &mut [Billboard],
    order: &mut Vec<(Fx, usize)>,
) {
    order.clear();
    for (i, b) in boards.iter().enumerate() {
        let (rx, ry) = (b.x - cam.x, b.y - cam.y);
        // Squared distance would overflow Q16.16 at city scale; the
        // octagonal norm does not, and sorting only needs an ordering.
        let (a, c) = (fixed::abs(rx), fixed::abs(ry));
        let (hi, lo) = if a > c { (a, c) } else { (c, a) };
        order.push((hi + lo * 3 / 8, i));
    }
    order.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    for &(_, i) in order.iter() {
        draw(f, depth, cam, atmos, p, &boards[i]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raycast;
    use crate::trig;
    use crate::world::City;

    fn setup() -> (City, Camera, Atmos, Frame, Vec<Fx>, Proj) {
        let city = City::generate(12);
        let mut cam = Camera::spawn(&city, 48, 48);
        cam.yaw = 0;
        let atmos = Atmos { ..Default::default() };
        let mut f = Frame::new(100, 36);
        let mut depth = Vec::new();
        raycast::render_to(&city, &cam, &atmos, &mut f, &mut depth);
        let p = raycast::projection(&city, &cam, &f);
        (city, cam, atmos, f, depth, p)
    }

    #[test]
    fn every_stamp_has_eight_rows_of_eight() {
        for s in [
            Stamp::LampPost, Stamp::Signal, Stamp::Hydrant, Stamp::Mailbox, Stamp::Tree,
            Stamp::Bollard, Stamp::Meter, Stamp::Ped, Stamp::Coin, Stamp::Pickup,
            Stamp::Debris,
        ] {
            let art = s.art();
            assert_eq!(art.len(), 8, "{s:?} has the wrong number of rows");
            for (i, row) in art.iter().enumerate() {
                assert_eq!(row.len(), 8, "{s:?} row {i} is {:?}", row);
                assert!(row.is_ascii(), "{s:?} row {i} is not ASCII");
            }
        }
    }

    #[test]
    fn something_right_in_front_of_the_camera_gets_drawn() {
        let (_c, cam, atmos, mut f, depth, p) = setup();
        let (dx, dy) = cam.dir();
        let b = Billboard::upright(
            Stamp::LampPost,
            cam.x + fixed::mul(dx, fixed::from_int(2)),
            cam.y + fixed::mul(dy, fixed::from_int(2)),
            fixed::ratio(1, 2),
            fixed::from_int(2),
            palette::H_WHITE,
        );
        assert!(draw(&mut f, &depth, &cam, &atmos, &p, &b), "nothing was drawn");
    }

    #[test]
    fn something_behind_the_camera_is_not_drawn() {
        let (_c, cam, atmos, mut f, depth, p) = setup();
        let (dx, dy) = cam.dir();
        let b = Billboard::upright(
            Stamp::Car,
            cam.x - fixed::mul(dx, fixed::from_int(4)),
            cam.y - fixed::mul(dy, fixed::from_int(4)),
            ONE,
            ONE,
            palette::H_YELLOW,
        );
        assert!(!draw(&mut f, &depth, &cam, &atmos, &p, &b));
    }

    #[test]
    fn something_beyond_the_draw_distance_is_not_drawn() {
        let (_c, cam, atmos, mut f, depth, p) = setup();
        let (dx, dy) = cam.dir();
        let far = fixed::from_int(crate::atmos::draw_distance(atmos.haze) + 20);
        let b = Billboard::upright(
            Stamp::Car,
            cam.x + fixed::mul(dx, far),
            cam.y + fixed::mul(dy, far),
            ONE,
            ONE,
            palette::H_YELLOW,
        );
        assert!(!draw(&mut f, &depth, &cam, &atmos, &p, &b));
    }

    #[test]
    fn a_sprite_behind_a_wall_is_hidden() {
        let (_c, cam, atmos, mut f, depth, p) = setup();
        // Put it past the nearest wall in the middle column.
        let mid = depth[p.w as usize / 2];
        if mid >= fixed::from_int(crate::atmos::draw_distance(atmos.haze)) {
            return; // nothing in the way to test against
        }
        let (dx, dy) = cam.dir();
        let d = mid + fixed::from_int(3);
        let b = Billboard::upright(
            Stamp::Car,
            cam.x + fixed::mul(dx, d),
            cam.y + fixed::mul(dy, d),
            ONE,
            ONE,
            palette::H_YELLOW,
        );
        assert!(!draw(&mut f, &depth, &cam, &atmos, &p, &b), "drew a car through a building");
    }

    #[test]
    fn a_sprite_gets_smaller_as_it_gets_further_away() {
        let (_c, cam, atmos, mut f, depth, p) = setup();
        let (dx, dy) = cam.dir();
        let filled = |d: i32, f: &mut Frame| {
            f.clear();
            let b = Billboard::upright(
                Stamp::Bus,
                cam.x + fixed::mul(dx, fixed::from_int(d)),
                cam.y + fixed::mul(dy, fixed::from_int(d)),
                ONE,
                ONE,
                palette::H_YELLOW,
            );
            draw(f, &depth, &cam, &atmos, &p, &b);
            f.cels.iter().filter(|c| **c != Cel::EMPTY).count()
        };
        let near = filled(2, &mut f);
        let far = filled(6, &mut f);
        assert!(near > far, "a bus at 2 units ({near}) is no bigger than one at 6 ({far})");
        assert!(far > 0, "the far bus vanished entirely");
    }

    #[test]
    fn a_sprite_off_to_the_side_lands_off_to_the_side() {
        let (_c, mut cam, atmos, mut f, depth, p) = setup();
        cam.yaw = 0;
        let put = |ang: f64, f: &mut Frame| -> Option<i32> {
            f.clear();
            let a = trig::from_degrees(ang);
            let b = Billboard::upright(
                Stamp::Coin,
                cam.x + fixed::mul(trig::cos(a), fixed::from_int(4)),
                cam.y + fixed::mul(trig::sin(a), fixed::from_int(4)),
                ONE,
                ONE,
                palette::H_YELLOW,
            );
            draw(f, &depth, &cam, &atmos, &p, &b);
            (0..p.w).find(|&x| (0..p.h).any(|y| f.get(x, y) != Cel::EMPTY))
        };
        let left = put(-15.0, &mut f);
        let right = put(15.0, &mut f);
        if let (Some(l), Some(r)) = (left, right) {
            assert!(l < r, "a sprite to the left ({l}) drew right of one to the right ({r})");
        }
    }

    /// A painted car is a car at any size it is drawn.
    ///
    /// The bands have to survive being sampled coarsely - see `paint_car` -
    /// so this asks for every part of one at the sizes cars actually get.
    ///
    /// The row counts are the *body's*, and the card is a quarter taller
    /// again because the cab's roof sign is drawn on the same card and takes
    /// a fifth off the top of it - see `SIGN_BAND` and `CarKind::hull`.  A
    /// six-row body is what is being defended here, and a six-row body on a
    /// cab arrives as a seven-row card.
    #[test]
    fn a_painted_car_has_all_of_its_parts_at_any_size() {
        let sky = (palette::H_BLUE, 5);
        for body_rows in [6, 7, 8, 10, 12, 16, 20, 30, 40] {
            let rows = body_rows * 5 / 4;
            let mut glass = 0;
            let mut lamp = 0;
            let mut wheel = 0;
            let mut band = 0;
            for r in 0..rows {
                let v = fixed::div(fixed::from_int(r) + fixed::HALF, fixed::from_int(rows));
                for c in 0..rows * 2 {
                    let u = fixed::div(fixed::from_int(c) + fixed::HALF, fixed::from_int(rows * 2));
                    // Both faces, because they carry different parts: the
                    // lamps are on the ends and the chequer band runs along
                    // the flanks, exactly as they do on the car.
                    for side in [false, true] {
                    let Some((_, hue, luma)) =
                        paint_face(Body::Taxi, side, u, v, palette::H_YELLOW, 2, sky, 0, rows, rows * 2)
                    else {
                        continue;
                    };
                    if hue == sky.0 {
                        glass += 1;
                    }
                    if hue == palette::H_RED && luma == 7 {
                        lamp += 1;
                    }
                    // Black low down is a wheel or its arch; black higher up
                    // is the dark square of the chequer band.  The two are
                    // the same colour and are told apart by where they are,
                    // which is also how you tell them apart looking at it.
                    //
                    // Measured down the *body* rather than down the card:
                    // the two are the same thing on every vehicle except
                    // this one, and on this one the sign band has pushed
                    // everything a fifth of a card lower.
                    let bv = fixed::div(v - SIGN_BAND, ONE - SIGN_BAND);
                    if hue == palette::H_BLACK && bv > fixed::ratio(70, 100) {
                        wheel += 1;
                    }
                    if hue == palette::H_BLACK && bv > 0 && bv < fixed::ratio(70, 100) {
                        band += 1;
                    }
                    }
                }
            }
            assert!(glass > 0, "{body_rows} rows: no windscreen");
            assert!(lamp > 0, "{body_rows} rows: no brake lights");
            assert!(wheel > 0, "{body_rows} rows: no wheels");
            assert!(band > 0, "{body_rows} rows: no chequer band");
        }
    }

    /// Side-on, a car has black wheels sitting in black arches.
    ///
    /// The two are the same colour and are told apart by shape: the arch is
    /// a bite out of the bodywork, so above the ground line there is black
    /// at the wheels and body colour between them.  A car with a plain black
    /// bar along its bottom passes a test for "black low down" and looks
    /// like a brick, so what is asserted is the *gap*.
    #[test]
    fn a_car_seen_side_on_has_wheels_in_arches() {
        let sky = (palette::H_BLUE, 5);
        let rows = 24;
        let row = |v: Fx| {
            let mut black = 0;
            let mut body = 0;
            for c in 0..rows * 2 {
                let u = fixed::div(fixed::from_int(c) + fixed::HALF, fixed::from_int(rows * 2));
                match paint_face(Body::Saloon, true, u, v, palette::H_RED, 0, sky, 0, 24, 48) {
                    Some((_, palette::H_BLACK, _)) => black += 1,
                    Some((_, h, _)) if h == palette::H_RED => body += 1,
                    _ => {}
                }
            }
            (black, body)
        };
        // Just above the ground line: wheels and arches, with sill between.
        let (black, body) = row(fixed::ratio(85, 100));
        assert!(black >= 4, "only {black} cells of wheel across the bottom of the car");
        assert!(body > 0, "the bottom of the car is all wheel - no sill between the arches");
        // Halfway up the doors: no black at all.
        let (black, _) = row(fixed::ratio(60, 100));
        assert_eq!(black, 0, "black in the middle of the door");
    }

    /// The windows take the sky's colour, whatever the sky is doing.
    #[test]
    fn the_windows_reflect_the_sky() {
        for hue in [palette::H_BLUE, palette::H_ORANGE, palette::H_GREEN] {
            let mut found = false;
            for r in 0..16 {
                let v = fixed::div(fixed::from_int(r) + fixed::HALF, fixed::from_int(16));
                let got = paint_face(Body::Saloon, false, fixed::HALF, v, palette::H_RED, 0, (hue, 5), 0, 24, 48);
                if let Some((_, h, _)) = got {
                    if h == hue {
                        found = true;
                    }
                    // ...and the rest of it is still the car's own colour.
                    assert!(
                        h == hue || h == palette::H_RED || h == palette::H_WHITE || h == palette::H_BLACK,
                        "a car painted in {hue} has a {h} panel"
                    );
                }
            }
            assert!(found, "no window took the sky's {hue}");
        }
    }

    #[test]
    fn drawing_a_crowd_never_panics() {
        let (_c, cam, atmos, mut f, depth, p) = setup();
        let mut boards: Vec<Billboard> = (0..200)
            .map(|i| {
                let mut b = Billboard::upright(
                    Stamp::Ped,
                    cam.x + fixed::ratio(i % 37 - 18, 2),
                    cam.y + fixed::ratio(i / 37 - 2, 2),
                    fixed::ratio(1, 3),
                    ONE,
                    palette::H_PINK,
                );
                b.lean = (i % 9) as u8;
                b.phase = (i % 3) as u8;
                b
            })
            .collect();
        let mut order = Vec::new();
        draw_all(&mut f, &depth, &cam, &atmos, &p, &mut boards, &mut order);
        assert_eq!(order.len(), 200);
    }
}

#[cfg(test)]
mod silhouette_tests {
    use super::*;
    use crate::fixed;

    /// A box looked at down its own length is as wide as the box is wide,
    /// and broadside it is as wide as the box is long.
    #[test]
    fn a_car_is_as_long_from_the_side_as_it_is_wide_from_behind() {
        let (len, wid) = (fixed::from_int(2), fixed::ratio(6, 5));
        // Pointing east, seen from the west: end-on.
        let (w, side) = silhouette(len, wid, 0, ONE, 0);
        assert_eq!(w, wid, "a car seen down its own length should be its width");
        assert!(!side);
        // Pointing east, seen from the north: broadside.
        let (w, side) = silhouette(len, wid, 0, 0, ONE);
        assert_eq!(w, len, "a car seen broadside should be its length");
        assert!(side);
    }

    /// ...and between those two at every angle from every heading, give or
    /// take the octagonal norm.
    ///
    /// The tolerance is not slack.  The view direction is normalised with
    /// `max + 3/8 min` rather than a square root, which is short by up to
    /// about three per cent at forty-five degrees, and the error lands on
    /// both terms of the silhouette.  Three per cent of a car is a fiftieth
    /// of a character at any distance you can see one, and a square root per
    /// vehicle per frame is not worth it - but the bound is real and is
    /// stated here rather than hidden in a fudge factor.
    #[test]
    fn the_silhouette_is_between_the_width_and_the_length() {
        let (len, wid) = (fixed::from_int(2), fixed::ratio(6, 5));
        let slack = fixed::mul(len, fixed::ratio(1, 20));
        for yaw in (0..65_536).step_by(1_021) {
            for view in (0..65_536).step_by(997) {
                let (vx, vy) = (trig::cos(view as Ang), trig::sin(view as Ang));
                let (w, side) = silhouette(len, wid, yaw as Ang, vx, vy);
                assert!(
                    w >= wid - slack && w <= len + wid,
                    "yaw {yaw} view {view} gave {}",
                    fixed::to_f32(w)
                );
                // Side-on means the long dimension dominates, so the
                // silhouette has to be at least as wide as the car is.
                if side {
                    assert!(w > wid - slack, "yaw {yaw} view {view} claims side-on at {}", fixed::to_f32(w));
                }
            }
        }
    }
    /// Three quarters of the way round a car you see three quarters of it.
    ///
    /// The property that matters is not any one number: it is that the card
    /// changes *continuously* with the angle.  What it replaced flipped
    /// between two pictures at forty-five degrees, so the test is that
    /// nothing flips.
    #[test]
    fn a_car_is_seen_from_wherever_you_are_standing() {
        let (len, wid) = (fixed::from_int(2), fixed::from_int(1));
        // The car points east; walk the camera round it.
        let mut last: Option<Fx> = None;
        let mut worst = 0;
        for deg in 0..360 {
            let a = trig::from_degrees(deg as f64);
            // From the car to the camera is the opposite of camera to car.
            let (vx, vy) = (-trig::cos(a), -trig::sin(a));
            let asp = aspect(len, wid, 0, vx, vy);
            assert!(asp.end >= 0 && asp.end <= ONE, "{deg}: end fraction {}", fixed::to_f32(asp.end));
            if let Some(p) = last {
                worst = worst.max(fixed::abs(asp.end - p));
            }
            last = Some(asp.end);
        }
        // A degree of camera movement never moves the seam by more than a
        // tenth of the card.  It moves fastest near dead ahead, where a
        // degree of turn is `len/wid` degrees' worth of flank appearing, and
        // that is the honest rate; a *flip* moves it by the whole card, so
        // the bar is ten times the real motion and a tenth of the failure.
        assert!(
            worst < ONE / 10,
            "the view jumped by {} of a card in one degree",
            fixed::to_f32(worst)
        );
    }

    /// Dead astern is all boot, broadside is all flank, and the corner is
    /// where the two meet.
    #[test]
    fn the_seam_is_where_the_corner_of_the_car_is() {
        let (len, wid) = (fixed::from_int(2), fixed::from_int(1));
        let at = |deg: f64| {
            let a = trig::from_degrees(deg);
            aspect(len, wid, 0, -trig::cos(a), -trig::sin(a))
        };
        // The car points east, so a camera due east of it is looking at its
        // nose: all end, and the end is the front.
        let front = at(0.0);
        assert!(front.end > ONE - ONE / 20, "ahead is {} end", fixed::to_f32(front.end));
        assert!(front.front, "standing in front reports the back");
        // Due west, behind it: all end, and the end is the back.
        let back = at(180.0);
        assert!(back.end > ONE - ONE / 20, "astern is {} end", fixed::to_f32(back.end));
        assert!(!back.front, "standing behind reports the front");
        // Broadside: no end at all.
        let side = at(90.0);
        assert!(side.end < ONE / 20, "broadside is {} end", fixed::to_f32(side.end));
        // And three-quarter rear: a real share of each, with the end band
        // on the side the nose is turned away from.
        let q = at(45.0);
        assert!(q.end > ONE / 5 && q.end < ONE * 3 / 5, "three-quarters is {} end", fixed::to_f32(q.end));
        let other = at(-45.0);
        assert_ne!(q.end_left, other.end_left, "the boot is on the same side from both quarters");
    }

    /// The highlight walks round the car as the sun does.
    ///
    /// The sun rises in the east and sets in the west, so a car watched from
    /// one place is lit from one side in the morning and the other in the
    /// evening, with the shade always opposite.  That is the whole of what
    /// makes the card read as a volume rather than as a painted rectangle,
    /// and it is one dot product.
    #[test]
    fn the_lit_side_of_a_car_follows_the_sun() {
        // Looking north, so east is to one side of the frame and west the
        // other.  The camera's position does not matter; only its heading.
        let cam = Camera { yaw: trig::QUARTER, ..Default::default() };
        let sun = |deg: f64| {
            // The same dot product `key_light` takes, with the bearing given
            // rather than read off the clock: what is being defended is that
            // the shading answers to where the sun *is*, and `Atmos::sun_az`
            // has its own test for where that is at a given hour.
            let (sx, sy) = (trig::cos(trig::from_degrees(deg)), trig::sin(trig::from_degrees(deg)));
            let (dx, dy) = cam.dir();
            fixed::clamp(fixed::mul(sx, -dy) + fixed::mul(sy, dx), -ONE, ONE)
        };
        // East is bearing zero here - see `Atmos::sun_az` - and the camera
        // faces north, so a sunrise is on one side and a sunset the other.
        let dawn = sun(0.0);
        let dusk = sun(180.0);
        assert!(dawn != 0, "the morning sun is edge on to the camera");
        assert!(
            (dawn > 0) != (dusk > 0),
            "the sun set on the same side it rose: {} then {}",
            fixed::to_f32(dawn),
            fixed::to_f32(dusk)
        );

        // ...and the painter answers to it.  Mean brightness of the left
        // half of a card against the right, with the light from each side.
        let sky = (palette::H_BLUE, 5u8);
        let lit = |key: Fx| -> (i32, i32) {
            let (mut l, mut r) = (0, 0);
            for row in 4..20 {
                let v = fixed::ratio(row, 24);
                for col in 4..44 {
                    let u = fixed::ratio(col, 48);
                    let Some((g, _, luma)) =
                        paint_face(Body::Saloon, false, u, v, palette::H_RED, 0, sky, key, 24, 48)
                    else {
                        continue;
                    };
                    // The rim is the sky rather than the car, so it is not
                    // part of what the body's shading is being asked about.
                    if g != catalog::G_SOLID {
                        continue;
                    }
                    if u < fixed::HALF { l += luma as i32 } else { r += luma as i32 }
                }
            }
            (l, r)
        };
        let (fl, fr) = lit(fixed::ratio(4, 5));
        let (nl, nr) = lit(-fixed::ratio(4, 5));
        assert!(fr > fl, "the sun on the right did not light the right: {fl} against {fr}");
        assert!(nl > nr, "the sun on the left did not light the left: {nl} against {nr}");
    }

    /// A car ends in the colour of what is behind it.
    ///
    /// The silhouette used to stop dead on a character boundary in the
    /// body's own colour, which is a staircase of solid blocks with the city
    /// showing through the steps.  The outermost of the bodywork is now the
    /// sky, and it is drawn with a dither so the cell is only partly
    /// covered - the fade is in coverage as well as in colour.
    #[test]
    fn a_car_fades_into_the_sky_at_its_edges() {
        let sky = (palette::H_BLUE, 5u8);
        let at = |u: Fx| paint_face(Body::Saloon, false, u, fixed::ratio(60, 100), palette::H_RED, 0, sky, 0, 24, 48);
        // Down the middle it is the car.
        let (mg, mh, _) = at(fixed::HALF).expect("no car in the middle of the card");
        assert_eq!(mh, palette::H_RED, "the middle of the car is not the car's colour");
        assert_eq!(mg, catalog::G_SOLID, "the middle of the car is see-through");
        // Walking out to the edge, the last of it is the sky, and it is not
        // solid.
        let mut rim_seen = 0;
        let mut thinnest = 8;
        for i in 0..48 {
            let u = fixed::ratio(i, 48);
            let Some((g, h, _)) = at(u) else { continue };
            if h == sky.0 {
                rim_seen += 1;
                let cover = if g == catalog::G_SOLID { 8 } else { g - catalog::G_DITHER + 1 };
                thinnest = thinnest.min(cover);
            }
        }
        assert!(rim_seen >= 2, "only {rim_seen} cells of rim on a whole card");
        assert!(thinnest < 8, "the rim is solid all the way to the silhouette");
    }

    /// The cab wears a lit box on a bracket, above its roof.
    ///
    /// Three things, and all three have to be there or it is a lump: a box
    /// with a rim so it has thickness, something written in it, and a gap
    /// underneath with legs in it so it is mounted on the car rather than
    /// part of it.
    #[test]
    fn the_cab_has_a_roof_sign_on_a_bracket() {
        let sky = (palette::H_BLUE, 5u8);
        // Big enough to set the word in: see `TYPE_ROWS` and `TYPE_COLS`.
        let (rows, cols) = (60, 240);
        let (mut lit, mut ink, mut leg, mut air) = (0, 0, 0, 0);
        for r in 0..rows {
            let v = fixed::div(fixed::from_int(r) + fixed::HALF, fixed::from_int(rows));
            if v >= SIGN_BAND {
                break;
            }
            for c in 0..cols {
                let u = fixed::div(fixed::from_int(c) + fixed::HALF, fixed::from_int(cols));
                match paint_face(Body::Taxi, false, u, v, palette::H_YELLOW, 0, sky, 0, rows, cols) {
                    None => air += 1,
                    Some((_, h, l)) if h == palette::H_YELLOW && l == 7 => lit += 1,
                    Some((_, h, _)) if h == palette::H_BLACK => ink += 1,
                    Some((_, h, _)) if h == sky.0 => leg += 1,
                    Some(_) => {}
                }
            }
        }
        assert!(lit > 0, "the sign is not lit");
        assert!(ink > 0, "nothing is written on the sign");
        assert!(leg > 0, "the sign has no bracket under it");
        assert!(air > 0, "the sign fills the whole band; there is no sky beside it");

        // And a saloon has none of it.
        for r in 0..rows {
            let v = fixed::div(fixed::from_int(r) + fixed::HALF, fixed::from_int(rows));
            if v >= SIGN_BAND {
                break;
            }
            let got = paint_face(Body::Saloon, false, fixed::HALF, v, palette::H_RED, 0, sky, 0, rows, cols);
            assert!(
                !matches!(got, Some((_, h, 7)) if h == palette::H_YELLOW),
                "a saloon is wearing a taxi sign"
            );
        }
    }

    /// A jeep, a saloon and a land yacht are different shapes.
    ///
    /// Not different colours - the traffic is already every colour there is
    /// and it does not help at forty columns.  What has to differ is the
    /// silhouette, so this measures the one number that carries it: how far
    /// down the card the glass stops.
    #[test]
    fn the_three_bodies_have_three_silhouettes() {
        let sky = (palette::H_BLUE, 5u8);
        let glass_ends = |body: Body| -> Fx {
            let mut last = 0;
            for i in 0..100 {
                let v = fixed::ratio(i, 100);
                if let Some((_, hue, _)) = paint_face(body, true, fixed::HALF, v, palette::H_RED, 0, sky, 0, 24, 48) {
                    if hue == sky.0 {
                        last = v;
                    }
                }
            }
            last
        };
        let (jeep, saloon, boat) = (glass_ends(Body::Jeep), glass_ends(Body::Saloon), glass_ends(Body::Boat));
        assert!(jeep > saloon, "a jeep is not glassier than a saloon: {} against {}", fixed::to_f32(jeep), fixed::to_f32(saloon));
        assert!(saloon > boat, "a land yacht is not lower than a saloon: {} against {}", fixed::to_f32(saloon), fixed::to_f32(boat));
    }

}


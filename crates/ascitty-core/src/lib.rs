//! ASCITTY - a raytraced ASCII city, for terminals and for a Commodore
//! Plus/4.
//!
//! This crate is the whole renderer.  It owns the number system, the city,
//! the camera, the glyph catalogue and the per-frame cast; the binaries
//! around it only decide where the pixels go.  Nothing in here touches a
//! terminal, a file or a clock, which is what lets the same code answer for
//! both targets:
//!
//! ```text
//!   ascitty-core  ──┬──►  ascitty-tty    a colour terminal, 60 fps
//!                   └──►  ascitty-bake  ──►  C headers  ──►  cc65  ──►  .prg
//! ```
//!
//! The 6502 does not run this code - it runs a C transcription of it
//! against tables this crate generated.  What crosses the gap is the
//! *arithmetic*: Q16.16 here, Q8.8 there, same operations in the same order,
//! so a frame that disagrees between the two is a bug rather than a
//! difference of opinion about floating point.
//!
//! # Where to start
//!
//! - [`world`] - what a city is, and how one gets built
//! - [`raycast`] - the height-field walk that turns it into a frame
//! - [`font`] and [`catalog`] - the procedural block font, and the 128
//!   shapes it produces
//! - [`glyph`] - how a shape becomes a character you can actually print
//! - [`atmos`] - rain, moon, stars and haze

#![warn(missing_docs)]

pub mod arch;
pub mod atmos;
pub mod camera;
pub mod catalog;
pub mod fixed;
pub mod font;
pub mod frame;
pub mod glyph;
pub mod palette;
pub mod raycast;
pub mod rng;
pub mod trig;
pub mod world;

/// The version of this crate, for the status line and the build manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

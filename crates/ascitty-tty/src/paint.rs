//! Turning a [`Frame`] into bytes a terminal will show.
//!
//! One string per frame, one write per frame.  The expensive mistake here is
//! emitting a colour escape per cell: at 100x34 that is 3400 escapes of
//! about 19 bytes each, 65 KB a frame, 4 MB a second - enough to make the
//! terminal the bottleneck rather than the renderer.  So the painter tracks
//! the colour it last emitted and only says anything when it changes.  A
//! night city is mostly black, so in practice most rows emit one escape.

use ascitty_core::frame::Frame;
use ascitty_core::glyph::Mode;
use ascitty_core::palette::{self, Color};

/// How much colour the terminal can be trusted with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Depth {
    /// 24-bit colour.  The Plus/4 palette is reproduced exactly.
    #[default]
    True,
    /// The eight ANSI colours and their bright forms.
    Ansi16,
    /// No colour at all - the glyph carries everything.
    Mono,
}

impl Depth {
    /// Parse a depth name, for the command line.
    pub fn parse(s: &str) -> Option<Depth> {
        match s {
            "true" | "24" | "truecolor" => Some(Depth::True),
            "16" | "ansi" | "ansi16" => Some(Depth::Ansi16),
            "none" | "mono" | "0" => Some(Depth::Mono),
            _ => None,
        }
    }

    /// Guess from the environment, the way every other terminal program does.
    pub fn detect() -> Depth {
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        if colorterm.contains("truecolor") || colorterm.contains("24bit") {
            return Depth::True;
        }
        let term = std::env::var("TERM").unwrap_or_default();
        if term.contains("256color") || term.contains("direct") {
            return Depth::True;
        }
        if term.is_empty() || term == "dumb" {
            return Depth::Mono;
        }
        Depth::Ansi16
    }
}

/// Append the escape that selects a colour.
fn set_color(out: &mut String, c: Color, depth: Depth) {
    match depth {
        Depth::Mono => {}
        Depth::Ansi16 => {
            out.push_str("\x1b[");
            push_num(out, palette::to_ansi16(c) as u32);
            out.push('m');
        }
        Depth::True => {
            let (r, g, b) = palette::rgb_table()[(c & 0x7f) as usize];
            out.push_str("\x1b[38;2;");
            push_num(out, r as u32);
            out.push(';');
            push_num(out, g as u32);
            out.push(';');
            push_num(out, b as u32);
            out.push('m');
        }
    }
}

/// `write!` would pull in the formatting machinery for three digits.
fn push_num(out: &mut String, n: u32) {
    if n >= 100 {
        out.push((b'0' + (n / 100) as u8) as char);
    }
    if n >= 10 {
        out.push((b'0' + (n / 10 % 10) as u8) as char);
    }
    out.push((b'0' + (n % 10) as u8) as char);
}

/// Paint a frame into `out`, which is cleared first and reused between
/// frames so that a steady state does not allocate.
pub fn paint(f: &Frame, mode: Mode, depth: Depth, out: &mut String) {
    out.clear();
    out.push_str("\x1b[H"); // cursor home; the frame is always fully redrawn
    let mut last: Option<Color> = None;
    for y in 0..f.h {
        if y > 0 {
            out.push_str("\r\n");
        }
        for x in 0..f.w {
            let cel = f.cels[y * f.w + x];
            // A blank is a blank in every colour, so it never forces an
            // escape - which is most of the sky and most of the saving.
            let ch = mode.glyph(cel.glyph);
            if ch != ' ' && last != Some(cel.color) {
                set_color(out, cel.color, depth);
                last = Some(cel.color);
            }
            out.push(ch);
        }
    }
    out.push_str("\x1b[0m");
}

/// Paint a frame as plain text, no colour and no escapes at all - for
/// screenshots in documentation, for `--shot`, and for golden-frame tests.
pub fn plain(f: &Frame, mode: Mode) -> String {
    let mut s = String::with_capacity((f.w + 1) * f.h);
    for y in 0..f.h {
        for x in 0..f.w {
            s.push(mode.glyph(f.cels[y * f.w + x].glyph));
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use ascitty_core::frame::Cel;

    #[test]
    fn a_blank_frame_emits_no_colour_escapes() {
        let f = Frame::new(10, 3);
        let mut s = String::new();
        paint(&f, Mode::Ascii, Depth::True, &mut s);
        assert!(!s.contains("38;2"), "an empty sky emitted colour escapes");
    }

    #[test]
    fn a_run_of_one_colour_emits_one_escape() {
        let mut f = Frame::new(20, 1);
        for x in 0..20 {
            f.put(x, 0, Cel { glyph: 8, color: 0x33 });
        }
        let mut s = String::new();
        paint(&f, Mode::Ascii, Depth::True, &mut s);
        assert_eq!(s.matches("\x1b[38;2;").count(), 1);
    }

    #[test]
    fn mono_emits_no_escapes_but_the_reset() {
        let mut f = Frame::new(4, 2);
        f.put(0, 0, Cel { glyph: 8, color: 0x33 });
        let mut s = String::new();
        paint(&f, Mode::Ascii, Depth::Mono, &mut s);
        assert!(!s.contains("38;2"));
        assert!(!s.contains("\x1b[9"));
    }

    #[test]
    fn plain_output_is_exactly_w_by_h() {
        let f = Frame::new(7, 3);
        let s = plain(&f, Mode::Ascii);
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|l| l.chars().count() == 7));
    }

    #[test]
    fn numbers_render_without_the_formatting_machinery() {
        let mut s = String::new();
        push_num(&mut s, 0);
        push_num(&mut s, 7);
        push_num(&mut s, 42);
        push_num(&mut s, 255);
        assert_eq!(s, "0742255");
    }

    #[test]
    fn depth_names_parse() {
        assert_eq!(Depth::parse("true"), Some(Depth::True));
        assert_eq!(Depth::parse("16"), Some(Depth::Ansi16));
        assert_eq!(Depth::parse("mono"), Some(Depth::Mono));
        assert_eq!(Depth::parse("plaid"), None);
    }
}

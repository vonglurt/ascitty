//! Turning a [`Frame`] into bytes a terminal will show.
//!
//! One string per frame, one write per frame.  The expensive mistake here is
//! emitting a colour escape per cell: at 100x34 that is 3400 escapes of
//! about 19 bytes each, 65 KB a frame, 4 MB a second - enough to make the
//! terminal the bottleneck rather than the renderer.  So the painter tracks
//! the colour it last emitted and only says anything when it changes.  A
//! night city is mostly black, so in practice most rows emit one escape.

use ascitty_core::frame::{Cel, Frame};
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

/// A painter that remembers what is already on the screen.
///
/// # Why the whole frame is not sent every time
///
/// [`paint`] emits every cell, and at the sizes people actually run this it
/// is too much: 250x98 is 122 KB a frame, which at thirty frames a second is
/// 3.7 MB a second of escape codes.  The renderer produces that frame in
/// two thirds of a millisecond and the terminal needs twenty to draw it, so
/// the terminal is what you are watching - and what you are watching it do
/// is fall behind, which looks like bands of the picture updating on
/// different frames.  Vertical bands, because a frame is written in row
/// order and a row is one long run: a terminal that gives up part way
/// through leaves the right of the screen showing the frame before.
///
/// So the painter keeps the last frame and sends only what changed, with a
/// cursor move to skip the rest.
///
/// # Why the runs are joined
///
/// A cursor move costs about eight bytes and a character costs one, so
/// skipping a gap shorter than that costs more than repainting it.  Runs
/// separated by less than [`Painter::JOIN`] unchanged cells are therefore
/// merged, which also keeps the escape count down on a frame that has
/// changed in a hundred scattered places.
#[derive(Default)]
pub struct Painter {
    prev: Vec<Cel>,
    w: usize,
    h: usize,
    mode: Option<Mode>,
    depth: Option<Depth>,
}

impl Painter {
    /// How many unchanged cells are worth repainting rather than skipping.
    const JOIN: usize = 8;

    /// Throw away what the screen is believed to hold, so the next frame is
    /// sent in full.  For a resize, or anything else that has scribbled on
    /// the terminal.
    pub fn forget(&mut self) {
        self.prev.clear();
    }

    /// Paint `f`, sending only what has changed since the last call.
    ///
    /// Returns the bytes appended, which is what the caller measures.
    pub fn paint(&mut self, f: &Frame, mode: Mode, depth: Depth, out: &mut String) {
        out.clear();
        let stale = self.prev.len() != f.cels.len()
            || self.w != f.w
            || self.h != f.h
            || self.mode != Some(mode)
            || self.depth != Some(depth);
        if stale {
            // Nothing known about the screen: send all of it, the way
            // `paint` does, and remember it.
            paint(f, mode, depth, out);
            self.prev.clear();
            self.prev.extend_from_slice(&f.cels);
            self.w = f.w;
            self.h = f.h;
            self.mode = Some(mode);
            self.depth = Some(depth);
            return;
        }

        // A cell is the same to *look at* if its glyph is a blank in both,
        // whatever colour is attached: a blank is a blank in every colour,
        // which is the same rule `paint` uses to skip escapes.
        let same = |a: Cel, b: Cel| -> bool {
            let (ca, cb) = (mode.glyph(a.glyph), mode.glyph(b.glyph));
            ca == cb && (ca == ' ' || a.color == b.color)
        };

        let mut last: Option<Color> = None;
        for y in 0..f.h {
            let row = y * f.w;
            let mut x = 0;
            while x < f.w {
                if same(f.cels[row + x], self.prev[row + x]) {
                    x += 1;
                    continue;
                }
                // The run: from here to the last change, joining over gaps
                // too short to be worth a cursor move.
                let start = x;
                let mut end = x + 1;
                let mut probe = end;
                while probe < f.w {
                    if !same(f.cels[row + probe], self.prev[row + probe]) {
                        end = probe + 1;
                    } else if probe >= end + Self::JOIN {
                        break;
                    }
                    probe += 1;
                }
                out.push_str("\x1b[");
                push_num(out, y as u32 + 1);
                out.push(';');
                push_num(out, start as u32 + 1);
                out.push('H');
                for i in start..end {
                    let cel = f.cels[row + i];
                    let ch = mode.glyph(cel.glyph);
                    if ch != ' ' && last != Some(cel.color) {
                        set_color(out, cel.color, depth);
                        last = Some(cel.color);
                    }
                    out.push(ch);
                }
                x = end;
            }
        }
        if !out.is_empty() {
            out.push_str("\x1b[0m");
        }
        self.prev.copy_from_slice(&f.cels);
    }
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
    use ascitty_core::catalog;
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
    /// A model of the part of a terminal this program uses.
    ///
    /// Enough to replay what the painter emits and say what would be on the
    /// screen: cursor home, absolute cursor moves, `\r\n`, the colour
    /// escapes, and printable characters.  Nothing else is ever sent.
    ///
    /// It exists so that the *diff* painter can be checked against the full
    /// one by the only measure that matters - what ends up on the screen -
    /// rather than by inspecting the escapes it chose.
    struct Screen {
        w: usize,
        h: usize,
        cells: Vec<(char, Option<Color>)>,
        cur: (usize, usize),
        color: Option<Color>,
    }

    impl Screen {
        fn new(w: usize, h: usize) -> Screen {
            Screen { w, h, cells: vec![(' ', None); w * h], cur: (0, 0), color: None }
        }

        fn feed(&mut self, s: &str) {
            let b: Vec<char> = s.chars().collect();
            let mut i = 0;
            while i < b.len() {
                match b[i] {
                    '\r' => {
                        self.cur.0 = 0;
                        i += 1;
                    }
                    '\n' => {
                        self.cur.1 += 1;
                        i += 1;
                    }
                    '\x1b' => {
                        assert_eq!(b[i + 1], '[', "only CSI is ever sent");
                        let mut j = i + 2;
                        let mut args = String::new();
                        while j < b.len() && !b[j].is_ascii_alphabetic() {
                            args.push(b[j]);
                            j += 1;
                        }
                        let nums: Vec<u32> =
                            args.split(';').filter_map(|n| n.parse().ok()).collect();
                        match b[j] {
                            'H' => {
                                let r = *nums.first().unwrap_or(&1) as usize - 1;
                                let c = *nums.get(1).unwrap_or(&1) as usize - 1;
                                self.cur = (c, r);
                            }
                            'm' => {
                                self.color = match nums.as_slice() {
                                    [0] | [] => None,
                                    // The painter writes the palette's own
                                    // RGB, so the colour comes back by
                                    // looking it up rather than by matching.
                                    [38, 2, r, g, bl] => (0..128u8)
                                        .find(|&c| {
                                            palette::to_rgb(c)
                                                == (*r as u8, *g as u8, *bl as u8)
                                        })
                                        .or(self.color),
                                    _ => self.color,
                                };
                            }
                            c => panic!("unexpected escape {c}"),
                        }
                        i = j + 1;
                    }
                    ch => {
                        if self.cur.1 < self.h && self.cur.0 < self.w {
                            let at = self.cur.1 * self.w + self.cur.0;
                            // A blank carries no colour, exactly as the
                            // painter assumes when it skips the escape.
                            self.cells[at] = (ch, if ch == ' ' { None } else { self.color });
                        }
                        self.cur.0 += 1;
                        i += 1;
                    }
                }
            }
        }
    }

    /// Ten frames of a moving picture, painted both ways, land the same.
    ///
    /// The diff painter is only worth having if it is invisible, and the way
    /// to know that is to replay what each one emits into a model terminal
    /// and compare the screens - not to compare the escapes, which are
    /// deliberately different.
    #[test]
    fn painting_only_what_changed_puts_the_same_thing_on_the_screen() {
        let (w, h) = (48, 20);
        let mut full = Screen::new(w, h);
        let mut diffed = Screen::new(w, h);
        let mut painter = Painter::default();
        let mut a = String::new();
        let mut b = String::new();
        let mut saved = 0usize;
        let mut sent = 0usize;
        for t in 0..10u32 {
            // A city, roughly: a lot of it holds still from frame to frame
            // and some of it moves.  That is the shape of the saving as well
            // as of the picture - a frame where every cell changes cannot be
            // sent in less than every cell, and is not what this is for.
            let mut f = Frame::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let lit = (x * 5 + y * 3) % 17 == 0;
                    let moving = x.abs_diff((t as usize * 3) % w) < 2;
                    f.cels[y * w + x] = if lit || moving {
                        Cel {
                            glyph: catalog::G_SOLID,
                            color: palette::rgb_index(
                                ((x + y) % 16) as u8,
                                (if moving { 7 } else { (x + y) % 8 }) as u8,
                            ),
                        }
                    } else {
                        Cel::EMPTY
                    };
                }
            }
            paint(&f, Mode::Unicode, Depth::True, &mut a);
            painter.paint(&f, Mode::Unicode, Depth::True, &mut b);
            full.feed(&a);
            diffed.feed(&b);
            assert_eq!(
                full.cells, diffed.cells,
                "frame {t}: the diffed screen is not the painted one"
            );
            if t > 0 {
                saved += a.len();
                sent += b.len();
            }
        }
        // ...and it is worth having.  Half, on a pattern whose colour
        // changes at nearly every lit cell, which is the worst case for a
        // run-based painter; on an actual city frame it is a great deal
        // better, and `--bench` reports that figure.
        assert!(sent * 2 < saved, "the diff sent {sent} bytes against {saved}");
    }

    /// Forgetting sends the whole frame again.
    ///
    /// What a resize needs, and what anything that has scribbled on the
    /// terminal needs.
    #[test]
    fn a_painter_that_has_forgotten_sends_everything() {
        let f = Frame::new(20, 8);
        let mut painter = Painter::default();
        let mut s = String::new();
        painter.paint(&f, Mode::Unicode, Depth::True, &mut s);
        let first = s.len();
        painter.paint(&f, Mode::Unicode, Depth::True, &mut s);
        assert!(s.len() < first / 4, "an unchanged frame sent {} bytes", s.len());
        painter.forget();
        painter.paint(&f, Mode::Unicode, Depth::True, &mut s);
        assert_eq!(s.len(), first, "forgetting did not send the whole frame");
    }

}

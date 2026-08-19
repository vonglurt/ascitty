//! Terminal plumbing, with no dependencies.
//!
//! Everything here could have been a crate.  It is not one, because the
//! whole program is a renderer that has to be transcribed to a 6502 later,
//! and a dependency tree is a thing that has to be understood before it can
//! be transcribed.  Raw mode is four `stty` flags; the alternate screen is
//! two escape sequences; reading keys is a thread and a channel.  The cost
//! of writing that down once is lower than the cost of carrying a
//! terminal-handling library through a port to a machine with 64 KB.

use std::io::{Read, Write};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// Set once the terminal has been put into raw mode, so the restore runs
/// exactly once however the program ends.
static RAW: AtomicBool = AtomicBool::new(false);
/// Set when the terminal has been asked for key *release* events and agreed.
/// The restore has to pop the mode it pushed, and only if it pushed one.
static KEYS_HELD: AtomicBool = AtomicBool::new(false);

/// Ask the terminal to report key releases, and say whether it will.
///
/// # Why this exists
///
/// A terminal sends a byte when a key goes down and nothing at all when it
/// comes up.  So "is the accelerator pressed" is not a question the input
/// stream can answer, and for most of this program's life it was answered by
/// guessing: a press stayed live for a few frames, and holding a key was
/// really the terminal's own autorepeat arriving fast enough to keep the
/// guess alive.  That guess cannot express two keys at once, because a
/// terminal autorepeats *the last key pressed only* - hold `w`, then press
/// `q`, and the `w` stops arriving entirely.  Accelerating through a corner,
/// which is the whole of driving, was the one thing it could not do.
///
/// The progressive keyboard protocol, which kitty introduced and ghostty,
/// WezTerm, foot and others implement, reports press, repeat and release as
/// distinct events.  With it, holding two keys is two keys held.
///
/// # The handshake
///
/// Query the current flags with `CSI ? u` and follow it immediately with a
/// primary device attributes request.  Every terminal ever made answers the
/// second; a terminal that has answered it *without* answering the first
/// does not speak the protocol, which turns "wait and see if anything comes
/// back" into a definite answer that arrives in one round trip.
///
/// The reply is consumed here rather than left in the buffer, which is why
/// this runs before the reader thread starts: an unread device-attributes
/// report is a handful of keystrokes as far as the decoder is concerned.
fn handshake() -> (bool, bool) {
    let mut out = std::io::stdout();
    // Three questions in one round trip: what keyboard protocol do you
    // speak, how big are you, and - as the fence every terminal answers -
    // what are you.
    if out.write_all(b"\x1b[?u\x1b[18t\x1b[c").is_err() || out.flush().is_err() {
        return (false, false);
    }
    // Non-blocking-ish: return after a tenth of a second whether or not
    // anything arrived, so a terminal that answers neither costs a tenth of
    // a second at startup and nothing else.
    let _ = stty(&["-icanon", "min", "0", "time", "1"]);
    let mut reply = Vec::new();
    let mut chunk = [0u8; 64];
    let mut stdin = std::io::stdin();
    for _ in 0..5 {
        match stdin.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => reply.extend_from_slice(&chunk[..n]),
        }
        // The fence: a primary device attributes reply is `CSI ? ... c`.
        if reply.contains(&b'c') {
            break;
        }
    }
    let _ = stty(&["raw", "-echo"]);
    // A terminal that answered `CSI 18 t` will answer it again, which is
    // how the frame size is followed without forking a process to ask.
    let sized = size_reply(&reply).is_some();
    if !has_kitty_reply(&reply) {
        return (false, sized);
    }
    // Push the flags we want: 1 disambiguate, 2 report event types, 8 report
    // every key as an escape code.  The third is what makes an ordinary
    // letter arrive as an event with a press and a release rather than as a
    // bare byte with neither.
    if out.write_all(b"\x1b[>11u").is_err() || out.flush().is_err() {
        return (false, sized);
    }
    KEYS_HELD.store(true, Ordering::SeqCst);
    (true, sized)
}

/// The size out of a `CSI 8 ; rows ; cols t` report, as `(cols, rows)`.
fn size_reply(b: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i + 4 < b.len() {
        if b[i] == 0x1b && b[i + 1] == b'[' && b[i + 2] == b'8' && b[i + 3] == b';' {
            let mut j = i + 4;
            while j < b.len() && b[j] != b't' {
                j += 1;
            }
            if j < b.len() {
                let mut it = b[i + 4..j].split(|&c| c == b';');
                let rows = std::str::from_utf8(it.next()?).ok()?.trim().parse().ok()?;
                let cols = std::str::from_utf8(it.next()?).ok()?.trim().parse().ok()?;
                return Some((cols, rows));
            }
        }
        i += 1;
    }
    None
}

/// Ask the terminal how big it is.  The answer arrives on the key stream as
/// [`Key::Size`], which is where a terminal puts everything it says.
pub fn ask_size() {
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x1b[18t");
    let _ = out.flush();
}

/// Whether a terminal's reply to `CSI ? u` is in there: `CSI ? <digits> u`.
fn has_kitty_reply(b: &[u8]) -> bool {
    let mut i = 0;
    while i + 3 < b.len() {
        if b[i] == 0x1b && b[i + 1] == b'[' && b[i + 2] == b'?' {
            let mut j = i + 3;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 3 && j < b.len() && b[j] == b'u' {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Puts the terminal into raw mode and restores it on drop - including on a
/// panic, which is the case that actually matters: a renderer that panics
/// with the cursor hidden and echo off leaves a shell nobody can type into.
pub struct Term {
    /// Whether this terminal reports key releases - see [`handshake`].  The
    /// driving controls read it: with it, a held key is held; without it,
    /// they fall back to keeping a press alive for a few frames and letting
    /// autorepeat top it up.
    pub holds_keys: bool,
    /// Whether this terminal answers `CSI 18 t` with its size.
    ///
    /// The frame is whatever size the window is, and following a resize
    /// means asking something.  A terminal that answers this question
    /// answers it on the stream we are already reading, for the price of
    /// six bytes; one that does not leaves `stty size`, which is a fork and
    /// an exec and was being done *every frame*.
    pub reports_size: bool,
}

impl Term {
    /// Enter raw mode, the alternate screen, and hide the cursor.
    pub fn enter() -> std::io::Result<Term> {
        stty(&["raw", "-echo"])?;
        RAW.store(true, Ordering::SeqCst);
        let mut out = std::io::stdout();
        // 1049: alternate screen buffer.  25l: hide cursor.
        out.write_all(b"\x1b[?1049h\x1b[?25l\x1b[2J")?;
        out.flush()?;

        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            prev(info);
        }));
        // Before the reader thread exists, because the handshake reads the
        // terminal's replies off stdin itself.
        let (holds_keys, reports_size) = handshake();
        Ok(Term { holds_keys, reports_size })
    }

    /// The terminal size in character cells, or a sane default.
    pub fn size() -> (usize, usize) {
        if let Ok(o) = Command::new("stty").arg("size").stdin(std::process::Stdio::inherit()).output() {
            let s = String::from_utf8_lossy(&o.stdout);
            let mut it = s.split_whitespace();
            if let (Some(r), Some(c)) = (it.next(), it.next()) {
                if let (Ok(r), Ok(c)) = (r.parse::<usize>(), c.parse::<usize>()) {
                    if r > 2 && c > 2 {
                        return (c, r);
                    }
                }
            }
        }
        (100, 34)
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        restore();
    }
}

/// Undo everything [`Term::enter`] did.  Safe to call more than once.
pub fn restore() {
    if !RAW.swap(false, Ordering::SeqCst) {
        return;
    }
    let mut out = std::io::stdout();
    // Pop the keyboard mode before anything else, and only if one was
    // pushed: leaving a shell in a mode where every key arrives as an escape
    // sequence is a worse thing to leave behind than no cursor.
    if KEYS_HELD.swap(false, Ordering::SeqCst) {
        let _ = out.write_all(b"\x1b[<u");
    }
    let _ = out.write_all(b"\x1b[0m\x1b[?25h\x1b[?1049l");
    let _ = out.flush();
    let _ = stty(&["sane"]);
}

fn stty(args: &[&str]) -> std::io::Result<()> {
    Command::new("stty")
        .args(args)
        .stdin(std::process::Stdio::inherit())
        .status()
        .map(|_| ())
}

/// A key press, already decoded from whatever bytes the terminal sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A printable character, lowercased.
    Char(char),
    /// An arrow key.
    Up,
    /// An arrow key.
    Down,
    /// An arrow key.
    Left,
    /// An arrow key.
    Right,
    /// Escape, or Ctrl-C.
    Quit,
    /// Not a key at all: the terminal answering how big it is, in columns
    /// and rows.  It arrives on the key stream because that is the only
    /// stream a terminal has.
    Size(usize, usize),
}

/// What just happened to a key.
///
/// Only a terminal speaking the progressive keyboard protocol can tell the
/// three apart; everything else reports [`Edge::Press`] for every byte it
/// sends, including the ones its own autorepeat sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// It went down.
    Press,
    /// It is still down and the terminal is repeating it.
    Repeat,
    /// It came up.
    Release,
}

/// A key and what happened to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stroke {
    /// Which key.
    pub key: Key,
    /// Down, still down, or up.
    pub edge: Edge,
}

/// Non-blocking keyboard input.
///
/// Reading stdin blocks, and a renderer must not block, so the read lives on
/// its own thread and posts to a channel.  The thread is detached and dies
/// with the process; there is nothing to join, because there is nothing it
/// owns that outlives the program.
pub struct Keys {
    rx: Receiver<Stroke>,
}

impl Keys {
    /// Start reading.
    pub fn start() -> Keys {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 32];
            let mut stdin = std::io::stdin();
            loop {
                let n = match stdin.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                let mut i = 0;
                while i < n {
                    let (key, used) = decode(&buf[i..n]);
                    i += used.max(1);
                    if let Some(k) = key {
                        if tx.send(k).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Keys { rx }
    }

    /// Every key event since the last call.
    pub fn drain(&self) -> Vec<Stroke> {
        let mut v = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(k) => v.push(k),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return v,
            }
        }
    }
}

/// Decode one key event from the front of a byte slice.  Returns the event
/// and how many bytes it consumed.
///
/// Two grammars at once, because both arrive: the legacy one, where a letter
/// is a byte and an arrow is three, and the progressive one, where
/// everything is `CSI number ; modifiers : event u` and a release is as
/// ordinary as a press.  Which is in use depends on whether the terminal
/// agreed to the handshake, and nothing below needs to know: a legacy byte
/// has no event field and is a press.
fn decode(b: &[u8]) -> (Option<Stroke>, usize) {
    let press = |k: Key| (Some(Stroke { key: k, edge: Edge::Press }), 1);
    match b {
        [] => (None, 1),
        [0x1b, b'[', ..] => csi(b),
        // A lone Escape is a quit, but only if nothing follows it in the
        // same read - which is how a terminal distinguishes the two, and it
        // is as reliable here as anywhere.
        [0x1b, ..] if b.len() == 1 => press(Key::Quit),
        [0x1b, ..] => (None, 2),
        [3, ..] | [4, ..] => press(Key::Quit), // Ctrl-C, Ctrl-D
        [c, ..] if c.is_ascii_graphic() || *c == b' ' => {
            press(Key::Char((*c as char).to_ascii_lowercase()))
        }
        _ => (None, 1),
    }
}

/// Decode one CSI sequence: `ESC [ parameters final`.
///
/// The whole sequence is consumed whether or not it means anything here,
/// which is the part that matters.  A device-attributes report or a mouse
/// event that is only partly eaten leaves its tail in the stream, and the
/// tail of an escape sequence is a handful of letters - which, in a program
/// where letters are the controls, is the car driving itself into a wall.
fn csi(b: &[u8]) -> (Option<Stroke>, usize) {
    // Parameters are digits, ';' and ':'; the sequence ends at the first
    // byte in 0x40..=0x7e.
    let mut end = 2;
    while end < b.len() && !(0x40..=0x7e).contains(&b[end]) {
        end += 1;
    }
    if end >= b.len() {
        // Incomplete: wait for the rest rather than eating half of it.
        return (None, b.len());
    }
    let final_byte = b[end];
    let params = &b[2..end];
    let used = end + 1;

    // `params` is groups separated by ';', each of sub-parameters separated
    // by ':'.  Only two groups matter: the first is the key, and the second
    // is the modifiers, whose second sub-parameter is the event type.
    let mut groups = params.split(|&c| c == b';');
    let first = groups.next().unwrap_or(b"");
    let second = groups.next().unwrap_or(b"");
    let num = |g: &[u8]| -> Option<u32> {
        let d = g.split(|&c| c == b':').next().unwrap_or(b"");
        std::str::from_utf8(d).ok()?.parse().ok()
    };
    let event = second
        .split(|&c| c == b':')
        .nth(1)
        .and_then(|d| std::str::from_utf8(d).ok())
        .and_then(|d| d.parse::<u32>().ok())
        .unwrap_or(1);
    let edge = match event {
        2 => Edge::Repeat,
        3 => Edge::Release,
        _ => Edge::Press,
    };
    // Modifiers are a bitmask plus one: 1 shift, 2 alt, 4 ctrl.
    let ctrl = num(second).unwrap_or(1).saturating_sub(1) & 4 != 0;

    let key = match final_byte {
        // The terminal telling us how big it is.
        b't' => {
            let mut it = params.split(|&c| c == b';');
            let what = it.next().and_then(|d| std::str::from_utf8(d).ok()?.parse::<u32>().ok());
            let rows = it.next().and_then(|d| std::str::from_utf8(d).ok()?.parse::<usize>().ok());
            let cols = it.next().and_then(|d| std::str::from_utf8(d).ok()?.parse::<usize>().ok());
            match (what, rows, cols) {
                (Some(8), Some(r), Some(c)) if r > 2 && c > 2 => Some(Key::Size(c, r)),
                _ => None,
            }
        }
        b'A' => Some(Key::Up),
        b'B' => Some(Key::Down),
        b'C' => Some(Key::Right),
        b'D' => Some(Key::Left),
        b'u' => match num(first) {
            // Escape, and Ctrl-C or Ctrl-D, which is how a quit arrives once
            // every key is an escape sequence.
            Some(27) => Some(Key::Quit),
            Some(c) if ctrl && (c == 'c' as u32 || c == 'd' as u32) => Some(Key::Quit),
            Some(c) => char::from_u32(c)
                .filter(|c| c.is_ascii_graphic() || *c == ' ')
                .map(|c| Key::Char(c.to_ascii_lowercase())),
            None => None,
        },
        _ => None,
    };
    (key.map(|key| Stroke { key, edge }), used)
}

/// Write a whole frame in one syscall.
pub fn present(s: &str) -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    out.write_all(s.as_bytes())?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(k: Key) -> Option<Stroke> {
        Some(Stroke { key: k, edge: Edge::Press })
    }

    #[test]
    fn arrows_decode_and_consume_three_bytes() {
        assert_eq!(decode(b"\x1b[A"), (press(Key::Up), 3));
        assert_eq!(decode(b"\x1b[D"), (press(Key::Left), 3));
    }

    #[test]
    fn a_lone_escape_quits_but_a_prefix_does_not() {
        assert_eq!(decode(b"\x1b"), (press(Key::Quit), 1));
        assert_eq!(decode(b"\x1bO").0, None);
    }

    #[test]
    fn letters_arrive_lowercased() {
        assert_eq!(decode(b"W"), (press(Key::Char('w')), 1));
        assert_eq!(decode(b"w"), (press(Key::Char('w')), 1));
    }

    #[test]
    fn ctrl_c_quits() {
        assert_eq!(decode(&[3]), (press(Key::Quit), 1));
    }

    #[test]
    fn a_burst_of_keys_all_decode() {
        let mut got = Vec::new();
        let b = b"ww\x1b[Cd";
        let mut i = 0;
        while i < b.len() {
            let (k, used) = decode(&b[i..]);
            i += used;
            if let Some(k) = k {
                got.push(k.key);
            }
        }
        assert_eq!(
            got,
            vec![Key::Char('w'), Key::Char('w'), Key::Right, Key::Char('d')]
        );
    }

    /// The whole point of the protocol: a key that goes down and comes up.
    #[test]
    fn a_progressive_key_reports_down_repeat_and_up() {
        assert_eq!(decode(b"\x1b[119u"), (press(Key::Char('w')), 6));
        assert_eq!(
            decode(b"\x1b[119;1:2u"),
            (Some(Stroke { key: Key::Char('w'), edge: Edge::Repeat }), 10)
        );
        assert_eq!(
            decode(b"\x1b[119;1:3u"),
            (Some(Stroke { key: Key::Char('w'), edge: Edge::Release }), 10)
        );
    }

    /// Arrows keep their old encoding and gain an event type.
    #[test]
    fn a_progressive_arrow_is_still_an_arrow() {
        assert_eq!(decode(b"\x1b[1;1:3A"), (Some(Stroke { key: Key::Up, edge: Edge::Release }), 8));
        assert_eq!(decode(b"\x1b[1;1:1D"), (press(Key::Left), 8));
    }

    /// With every key an escape sequence, this is how a quit arrives.
    #[test]
    fn escape_and_ctrl_c_survive_the_protocol() {
        assert_eq!(decode(b"\x1b[27u"), (press(Key::Quit), 5));
        assert_eq!(decode(b"\x1b[99;5u"), (press(Key::Quit), 7));
        // ...and an unmodified `c` is still the copter key.
        assert_eq!(decode(b"\x1b[99u"), (press(Key::Char('c')), 5));
    }

    /// A sequence this program has no use for is consumed whole.
    ///
    /// The failure this guards against is not cosmetic: half a
    /// device-attributes report left in the stream decodes as letters, and
    /// in this program letters are the controls.
    #[test]
    fn an_unknown_sequence_is_eaten_whole() {
        let (k, used) = decode(b"\x1b[?62;1;6c");
        assert_eq!(k, None);
        assert_eq!(used, 10);
        // And a partial one is left alone until the rest of it arrives.
        assert_eq!(decode(b"\x1b[?62;").0, None);
    }

    #[test]
    fn the_keyboard_query_reply_is_recognised() {
        assert!(has_kitty_reply(b"\x1b[?0u\x1b[?62;c"));
        assert!(has_kitty_reply(b"\x1b[?11u"));
        // A terminal that answered only the device attributes has not got it.
        assert!(!has_kitty_reply(b"\x1b[?62;1;6c"));
        assert!(!has_kitty_reply(b""));
    }
}

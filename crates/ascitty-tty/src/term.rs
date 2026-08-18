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

/// Puts the terminal into raw mode and restores it on drop - including on a
/// panic, which is the case that actually matters: a renderer that panics
/// with the cursor hidden and echo off leaves a shell nobody can type into.
pub struct Term;

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
        Ok(Term)
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
}

/// Non-blocking keyboard input.
///
/// Reading stdin blocks, and a renderer must not block, so the read lives on
/// its own thread and posts to a channel.  The thread is detached and dies
/// with the process; there is nothing to join, because there is nothing it
/// owns that outlives the program.
pub struct Keys {
    rx: Receiver<Key>,
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

    /// Every key pressed since the last call.
    pub fn drain(&self) -> Vec<Key> {
        let mut v = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(k) => v.push(k),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return v,
            }
        }
    }
}

/// Decode one key from the front of a byte slice.  Returns the key and how
/// many bytes it consumed.
fn decode(b: &[u8]) -> (Option<Key>, usize) {
    match b {
        [] => (None, 1),
        // CSI sequences for the arrows.  A lone Escape is a quit, but only
        // if nothing follows it in the same read - which is how a terminal
        // distinguishes the two, and it is as reliable here as anywhere.
        [0x1b, b'[', c, ..] => {
            let k = match c {
                b'A' => Some(Key::Up),
                b'B' => Some(Key::Down),
                b'C' => Some(Key::Right),
                b'D' => Some(Key::Left),
                _ => None,
            };
            (k, 3)
        }
        [0x1b, ..] if b.len() == 1 => (Some(Key::Quit), 1),
        [0x1b, ..] => (None, 2),
        [3, ..] | [4, ..] => (Some(Key::Quit), 1), // Ctrl-C, Ctrl-D
        [c, ..] if c.is_ascii_graphic() || *c == b' ' => {
            (Some(Key::Char((*c as char).to_ascii_lowercase())), 1)
        }
        _ => (None, 1),
    }
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

    #[test]
    fn arrows_decode_and_consume_three_bytes() {
        assert_eq!(decode(b"\x1b[A"), (Some(Key::Up), 3));
        assert_eq!(decode(b"\x1b[D"), (Some(Key::Left), 3));
    }

    #[test]
    fn a_lone_escape_quits_but_a_prefix_does_not() {
        assert_eq!(decode(b"\x1b"), (Some(Key::Quit), 1));
        assert_eq!(decode(b"\x1bO").0, None);
    }

    #[test]
    fn letters_arrive_lowercased() {
        assert_eq!(decode(b"W"), (Some(Key::Char('w')), 1));
        assert_eq!(decode(b"w"), (Some(Key::Char('w')), 1));
    }

    #[test]
    fn ctrl_c_quits() {
        assert_eq!(decode(&[3]), (Some(Key::Quit), 1));
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
                got.push(k);
            }
        }
        assert_eq!(
            got,
            vec![Key::Char('w'), Key::Char('w'), Key::Right, Key::Char('d')]
        );
    }
}

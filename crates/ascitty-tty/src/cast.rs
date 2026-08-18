//! Recording a tour as an asciinema cast.
//!
//! An animation of a terminal program should be *terminal output*, not a
//! video of a terminal. A `.cast` file is a JSON header and one line per
//! frame holding the bytes and when they were written, so it stays sharp at
//! any size, it is a few hundred kilobytes rather than a few hundred
//! megabytes, and the frames in it are the exact bytes the renderer
//! produced.
//!
//! Play one with `asciinema play`, or upload it, or `cat` it if you are
//! curious - it is text.
//!
//! Format: <https://docs.asciinema.org/manual/asciicast/v2/>

use std::io::{BufWriter, Write};
use std::path::Path;

/// Writes an asciicast v2 file, one frame at a time.
pub struct Recorder {
    out: BufWriter<std::fs::File>,
    t: f64,
    dt: f64,
    frames: u32,
}

impl Recorder {
    /// Start a recording of a `w` by `h` terminal at `fps`.
    pub fn create(path: &Path, w: usize, h: usize, fps: u32) -> std::io::Result<Recorder> {
        let file = std::fs::File::create(path)?;
        let mut out = BufWriter::new(file);
        // The timestamp is only metadata - players show it as the recording
        // date - so a clock failure is not worth failing the recording for.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        writeln!(
            out,
            "{{\"version\":2,\"width\":{w},\"height\":{h},\"timestamp\":{stamp},\
             \"env\":{{\"TERM\":\"xterm-256color\",\"SHELL\":\"/bin/sh\"}},\
             \"title\":\"ASCITTY\"}}"
        )?;
        Ok(Recorder { out, t: 0.0, dt: 1.0 / fps.max(1) as f64, frames: 0 })
    }

    /// Record one frame's worth of output.
    pub fn frame(&mut self, data: &str) -> std::io::Result<()> {
        write!(self.out, "[{:.6},\"o\",\"", self.t)?;
        escape(&mut self.out, data)?;
        writeln!(self.out, "\"]")?;
        self.t += self.dt;
        self.frames += 1;
        Ok(())
    }

    /// Finish, returning how many frames and how long the recording runs.
    pub fn finish(mut self) -> std::io::Result<(u32, f64)> {
        self.out.flush()?;
        Ok((self.frames, self.t))
    }
}

/// JSON string escaping.
///
/// Hand-written because the only characters this ever has to deal with are
/// the escape character, the quote, the backslash and printable text - and
/// pulling in a JSON crate to emit four cases would be the only dependency
/// in the repository.
fn escape(out: &mut impl Write, s: &str) -> std::io::Result<()> {
    let mut plain = 0;
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        let esc: &[u8] = match b {
            b'"' => b"\\\"",
            b'\\' => b"\\\\",
            b'\n' => b"\\n",
            b'\r' => b"\\r",
            0x1b => b"\\u001b",
            0x00..=0x1f => b"",
            _ => continue,
        };
        out.write_all(&bytes[plain..i])?;
        if esc.is_empty() {
            write!(out, "\\u{b:04x}")?;
        } else {
            out.write_all(esc)?;
        }
        plain = i + 1;
    }
    out.write_all(&bytes[plain..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn esc(s: &str) -> String {
        let mut v: Vec<u8> = Vec::new();
        escape(&mut v, s).unwrap();
        String::from_utf8(v).unwrap()
    }

    #[test]
    fn plain_text_passes_through_untouched() {
        assert_eq!(esc("hello city"), "hello city");
    }

    #[test]
    fn escapes_and_quotes_are_escaped() {
        assert_eq!(esc("\x1b[H"), "\\u001b[H");
        assert_eq!(esc("say \"x\""), "say \\\"x\\\"");
        assert_eq!(esc("a\\b"), "a\\\\b");
        assert_eq!(esc("a\r\nb"), "a\\r\\nb");
    }

    #[test]
    fn other_control_characters_become_unicode_escapes() {
        assert_eq!(esc("\x07"), "\\u0007");
    }

    #[test]
    fn block_elements_survive() {
        // Multi-byte UTF-8 must not be split or escaped.
        assert_eq!(esc("▀▄█░"), "▀▄█░");
    }

    #[test]
    fn a_recording_is_a_header_and_a_line_per_frame() {
        let dir = std::env::temp_dir().join("ascitty-cast-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.cast");
        let mut r = Recorder::create(&path, 80, 24, 30).unwrap();
        r.frame("\x1b[Hone").unwrap();
        r.frame("\x1b[Htwo").unwrap();
        let (n, secs) = r.finish().unwrap();
        assert_eq!(n, 2);
        assert!((secs - 2.0 / 30.0).abs() < 1e-9);

        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("{\"version\":2,\"width\":80,\"height\":24"));
        assert!(lines[1].starts_with("[0.000000,\"o\",\""));
        assert!(lines[1].contains("\\u001b[Hone"));
        assert!(lines[2].starts_with("[0.033333,\"o\","));
        std::fs::remove_file(&path).ok();
    }
}

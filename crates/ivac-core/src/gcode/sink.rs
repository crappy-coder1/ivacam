//! Output sink for post-processor line emission — the write-through seam
//! for the streaming-gcode evolution (`ivac-3j1p`).
//!
//! Centralizes the line-buffer contract every post-processor and the per-op
//! pipeline cache share: append a line, count lines, clone a line range,
//! extend with a batch, and finish into the joined program. Historically
//! each post carried a bare `out: Vec<String>` and re-implemented these
//! operations inline (`linuxcnc`, `hpgl`, and `grbl` via its embedded
//! `linuxcnc::Post`); this type is the single place they now live.
//!
//! Today it is backed by an in-memory `Vec<String>`, so behavior is
//! **byte-identical** to the field it replaces. It exists as a distinct type
//! so the streaming evolution has exactly one seam to plug into: a future
//! append-only [`std::io::Write`]-backed variant makes peak memory O(1) in
//! program size instead of O(total lines). [`GcodeSink::write_to`] is that
//! primitive today — it streams the finished program without materializing
//! the monolithic joined `String`.
//!
//! The random-access operations ([`GcodeSink::clone_from`] /
//! [`GcodeSink::extend_from_slice`]) that the per-op cache relies on are the
//! part a pure append-only streaming sink cannot serve. Reconciling that —
//! capturing each op's contribution as byte offsets, or moving the snapshot
//! to a higher level — is the design the streaming variant blocks on, tracked
//! as a follow-up to this first increment.

use std::io::{self, Write};

/// Accumulates a post-processor's emitted g-code lines.
///
/// Each stored string is one line **without** its trailing newline — the
/// newline is the line *separator*, materialized by [`GcodeSink::finish`] /
/// [`GcodeSink::write_to`].
#[derive(Debug, Default, Clone)]
pub(crate) struct GcodeSink {
    lines: Vec<String>,
}

impl GcodeSink {
    /// Append one already-rendered line.
    pub(crate) fn push(&mut self, line: String) {
        self.lines.push(line);
    }

    /// Number of buffered lines. The per-op cache slices an op's
    /// contribution by the count captured before/after the op runs.
    pub(crate) fn len(&self) -> usize {
        self.lines.len()
    }

    /// Clone the buffered lines from `start` (inclusive); empty when
    /// `start >= len()`. How the per-op cache captures an op's output range.
    pub(crate) fn clone_from(&self, start: usize) -> Vec<String> {
        if start >= self.lines.len() {
            Vec::new()
        } else {
            self.lines[start..].to_vec()
        }
    }

    /// Append a pre-rendered batch verbatim — the op-cache hit replay path.
    pub(crate) fn extend_from_slice(&mut self, lines: &[String]) {
        self.lines.extend_from_slice(lines);
    }

    /// The finished program as one `String`: lines joined by `\n` with a
    /// trailing `\n`. Byte-identical to the historical `out.join("\n") +
    /// "\n"`. Derived from [`GcodeSink::write_to`] so there is a single
    /// canonical output path; the `from_utf8` cannot fail (every byte came
    /// from a `&str` line or the `\n` separator).
    pub(crate) fn finish(&self) -> String {
        let mut buf = Vec::new();
        self.write_to(&mut buf)
            .expect("writing to a Vec<u8> is infallible");
        String::from_utf8(buf).expect("g-code lines are valid UTF-8")
    }

    /// Stream the finished program to `w` without materializing the joined
    /// `String` — the write-through primitive the streaming evolution is
    /// built on. Emits the lines separated by `\n` with a trailing `\n`
    /// (so an empty program is a lone `\n`, matching the historical
    /// `join("\n") + "\n"`), letting a transport write g-code straight to a
    /// file/socket at O(1) extra memory instead of buffering a second full
    /// copy of the program.
    pub(crate) fn write_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0 {
                w.write_all(b"\n")?;
            }
            w.write_all(line.as_bytes())?;
        }
        w.write_all(b"\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sink_of(lines: &[&str]) -> GcodeSink {
        let mut s = GcodeSink::default();
        for l in lines {
            s.push((*l).to_string());
        }
        s
    }

    #[test]
    fn push_and_len_track() {
        let mut s = GcodeSink::default();
        assert_eq!(s.len(), 0);
        s.push("G0 X0".into());
        s.push("G1 X1".into());
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn finish_joins_with_trailing_newline() {
        assert_eq!(sink_of(&["G0", "G1"]).finish(), "G0\nG1\n");
        assert_eq!(sink_of(&["G0"]).finish(), "G0\n");
        // Empty program still ends in a lone newline — matches the historical
        // `[].join("\n") + "\n"`.
        assert_eq!(sink_of(&[]).finish(), "\n");
    }

    #[test]
    fn clone_from_slices_the_tail() {
        let s = sink_of(&["a", "b", "c"]);
        assert_eq!(s.clone_from(1), vec!["b".to_string(), "c".to_string()]);
        assert_eq!(s.clone_from(3), Vec::<String>::new());
        assert_eq!(s.clone_from(99), Vec::<String>::new());
    }

    #[test]
    fn extend_appends_a_batch() {
        let mut s = sink_of(&["a"]);
        s.extend_from_slice(&["b".to_string(), "c".to_string()]);
        assert_eq!(s.finish(), "a\nb\nc\n");
    }

    #[test]
    fn write_to_emits_the_expected_bytes() {
        // Pin the streaming primitive to literal bytes (not to finish(),
        // which is derived from it) — the join("\n") + "\n" semantics,
        // including the empty-program lone-newline edge case.
        let cases: [(&[&str], &str); 3] = [
            (&[], "\n"),
            (&["only"], "only\n"),
            (&["G0 X0", "G1 X1 Y2", "M2"], "G0 X0\nG1 X1 Y2\nM2\n"),
        ];
        for (lines, expected) in cases {
            let mut buf = Vec::new();
            sink_of(lines).write_to(&mut buf).unwrap();
            assert_eq!(buf, expected.as_bytes(), "arity {}", lines.len());
        }
    }
}

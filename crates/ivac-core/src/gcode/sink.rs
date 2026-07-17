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
//! [`GcodeSink`] is backed by an in-memory `Vec<String>`, so its behavior
//! is **byte-identical** to the field it replaces (peak memory O(total
//! lines)). [`StreamingGcodeSink`] is its append-only [`std::io::Write`]-backed
//! counterpart: it writes each line straight through and keeps only the
//! current op's lines in memory, so peak memory is O(largest single op).
//! [`GcodeSink::write_to`] is the shared write-through primitive both build
//! on — it streams the finished program without materializing the monolithic
//! joined `String`.
//!
//! The random-access operations ([`GcodeSink::clone_from`] /
//! [`GcodeSink::extend_from_slice`]) that the per-op cache relies on are the
//! part a pure append-only stream cannot serve once bytes are flushed away.
//! [`StreamingGcodeSink`] reconciles that with a bounded per-op tee (see its
//! docs); wiring it through the posts and transports so a real program
//! streams is the remaining increment (`ivac-3j1p.3`).

use std::io::{self, Write};

/// Write one program line to `w`, prefixing the `\n` *separator* for
/// every line after the first (`first == false`). The single trailing
/// newline is emitted separately by the caller (after the last line), so
/// both the buffered [`GcodeSink::write_to`] and the streaming
/// [`StreamingGcodeSink::push`] agree byte-for-byte on the historical
/// `join("\n") + "\n"`. One definition of the separator semantics so the
/// buffered and streaming paths cannot drift.
fn write_separated_line<W: Write>(w: &mut W, first: bool, line: &str) -> io::Result<()> {
    if !first {
        w.write_all(b"\n")?;
    }
    w.write_all(line.as_bytes())
}

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

    /// Borrow the buffered lines as a slice. Used by a post whose
    /// `finish()` re-derives its program text from the raw lines rather
    /// than the canonical `join("\n")` [`GcodeSink::finish`] — the HPGL
    /// post splits each buffered entry on `;` so every plotter statement
    /// lands on its own output line. Reads the same backing store as
    /// [`GcodeSink::finish`] without cloning it. Like the random-access
    /// operations above, this whole-buffer read is a thing an append-only
    /// streaming sink cannot serve — a `finish()` transform is in the same
    /// boat as the op-cache range ops, reconciled by the streaming variant.
    pub(crate) fn lines(&self) -> &[String] {
        &self.lines
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
            write_separated_line(w, i == 0, line)?;
        }
        w.write_all(b"\n")
    }
}

/// Append-only, [`std::io::Write`]-backed g-code sink — the streaming
/// counterpart to [`GcodeSink`] and the core of increment 3 of the
/// streaming-gcode evolution (`ivac-3j1p`).
///
/// Where [`GcodeSink`] holds the whole program in a `Vec<String>` (peak
/// memory O(total program size)), this variant **writes each line straight
/// through to `writer`** the moment it is emitted and keeps only the
/// *current operation's* lines in memory. Peak memory is therefore
/// O(largest single op) rather than O(program) — the win the epic is
/// after for unbounded raster / huge programs.
///
/// # Reconciling the per-op cache (the blocker this increment resolves)
///
/// The per-op pipeline cache captures each op's contribution with three
/// random-access line operations — [`out_lines_count`] to mark the op's
/// start, [`out_lines_clone_from`] to read the op's body back at its end,
/// and [`out_extend_lines`] to splice a cached body in on a hit — which a
/// pure append-only stream cannot serve once bytes are flushed away.
///
/// The reconciliation (design option (c): a bounded tee) rests on an
/// invariant of the emit loop: `clone_from` is *only ever* called with the
/// marker captured at the **start of the current op** (`pipeline.rs`
/// captures `body_marker = out_lines_count()` just before the op body and
/// reads `out_lines_clone_from(body_marker)` just after it). It is never
/// an arbitrary historical range — always the tail of what was just
/// emitted. So this sink tees only the lines emitted **since the last
/// [`checkpoint`]** into `tail`; the driver loop calls [`checkpoint`] at
/// each op boundary (the same point it reads `out_lines_count`), dropping
/// the previous op's tee (already written through to `writer`). `tail` is
/// thus bounded by one op's output, and `clone_from` / `extend_from_slice`
/// stay correct and byte-identical to the buffered sink.
///
/// [`out_lines_count`]: crate::gcode::PostProcessor::out_lines_count
/// [`out_lines_clone_from`]: crate::gcode::PostProcessor::out_lines_clone_from
/// [`out_extend_lines`]: crate::gcode::PostProcessor::out_extend_lines
/// [`checkpoint`]: StreamingGcodeSink::checkpoint
///
/// # Not yet wired
///
/// The posts still embed the buffered [`GcodeSink`]; threading a `Write`
/// through the `PostProcessor` construction and the cli/server/tauri/wasm
/// transports (so a real program streams to a file/socket) is increment 4
/// (`ivac-3j1p.3`). This type lands the primitive + the op-cache design
/// ahead of that consumer, exactly as increment 1 landed
/// [`GcodeSink::write_to`]. Hence `#[allow(dead_code)]`: it is exercised by
/// the module tests (which replay the emit loop's exact call sequence) and
/// adopted for real in `ivac-3j1p.3`.
///
/// # Known limitation (`ivac-3j1p.2` follow-up)
///
/// A single pathologically large op — e.g. one laser-raster op that emits
/// the entire program as one `G1`-per-pixel body — still buffers that op's
/// whole tee, so peak stays O(that op). True O(1) there needs a cache
/// **bypass** for oversized ops (stream straight through, skip the
/// snapshot) or a seek-based re-read (design option (a), needs `W: Read +
/// Seek`). Tracked as a follow-up; out of scope for the common many-ops
/// case this increment makes O(1).
#[allow(dead_code)] // consumed by ivac-3j1p.3 transport wiring; see doc above.
pub(crate) struct StreamingGcodeSink<W: Write> {
    /// The write-through destination. Each [`push`](Self::push) hits it
    /// immediately; the sink never holds a second full copy of the program.
    writer: W,
    /// Total logical lines emitted so far — what [`len`](Self::len)
    /// returns and the marker the op cache captures via `out_lines_count`.
    total: usize,
    /// The lines emitted since the last [`checkpoint`](Self::checkpoint):
    /// the current op's contribution, the only range the op cache ever
    /// clones. Everything before `tail_start` has been written through to
    /// `writer` and dropped, bounding memory to one op's output.
    tail: Vec<String>,
    /// Logical index of `tail[0]` — `total` as of the last checkpoint.
    /// [`clone_from`](Self::clone_from) translates an absolute marker into
    /// a `tail` offset by subtracting this.
    tail_start: usize,
    /// Whether any line has reached `writer` yet, driving the `\n`
    /// separator so the stream is byte-identical to `join("\n") + "\n"`.
    wrote_any: bool,
}

#[allow(dead_code)] // consumed by ivac-3j1p.3 transport wiring; see type doc.
impl<W: Write> StreamingGcodeSink<W> {
    /// Wrap a writer. The writer is expected to be buffered by the caller
    /// (e.g. a [`std::io::BufWriter`]) — this sink issues one `write_all`
    /// per line and does not batch.
    pub(crate) fn new(writer: W) -> Self {
        Self {
            writer,
            total: 0,
            tail: Vec::new(),
            tail_start: 0,
            wrote_any: false,
        }
    }

    /// Emit one line: write it straight through to `writer` (with the
    /// leading `\n` separator for every line after the first) and tee it
    /// into the current-op `tail`. The [`GcodeSink::push`] analogue, but
    /// O(1) in program size rather than growing an unbounded `Vec`.
    pub(crate) fn push(&mut self, line: String) -> io::Result<()> {
        write_separated_line(&mut self.writer, !self.wrote_any, &line)?;
        self.wrote_any = true;
        self.total += 1;
        self.tail.push(line);
        Ok(())
    }

    /// Append a pre-rendered batch verbatim — the op-cache HIT replay path
    /// ([`out_extend_lines`](crate::gcode::PostProcessor::out_extend_lines)).
    /// Streams each line through like [`push`](Self::push).
    pub(crate) fn extend_from_slice(&mut self, lines: &[String]) -> io::Result<()> {
        for line in lines {
            self.push(line.clone())?;
        }
        Ok(())
    }

    /// Total lines emitted so far. The marker the op cache captures via
    /// `out_lines_count` and later hands back to [`clone_from`](Self::clone_from).
    pub(crate) fn len(&self) -> usize {
        self.total
    }

    /// Clone the emitted lines from absolute index `start` — the op-cache
    /// body-capture path. `start` must be `>= the last checkpoint`
    /// (`tail_start`): the emit loop only ever clones the current op's tail
    /// (see the type doc), so earlier lines are already streamed out and
    /// gone. Returns empty for `start >= len()`, matching
    /// [`GcodeSink::clone_from`].
    pub(crate) fn clone_from(&self, start: usize) -> Vec<String> {
        if start >= self.total {
            return Vec::new();
        }
        debug_assert!(
            start >= self.tail_start,
            "streaming sink can only clone the current op's tail \
             (start {start} < checkpoint {}); the op cache never clones \
             before the active op boundary",
            self.tail_start,
        );
        let off = start.saturating_sub(self.tail_start);
        self.tail
            .get(off..)
            .map(<[String]>::to_vec)
            .unwrap_or_default()
    }

    /// Mark an op boundary: drop the previous op's tee (its lines are
    /// already written through to `writer`) and start a fresh tail at the
    /// current position. Called by the driver loop at the same point it
    /// captures `out_lines_count` for the next op, keeping `tail` bounded
    /// to a single op. `len()` is unaffected — the logical count is
    /// monotonic across checkpoints.
    pub(crate) fn checkpoint(&mut self) {
        self.tail_start = self.total;
        self.tail.clear();
    }

    /// Finish the program: emit the single trailing newline (so the stream
    /// ends in `join("\n") + "\n"`, and an empty program is a lone `\n`),
    /// flush, and hand the writer back to the caller.
    pub(crate) fn finish(mut self) -> io::Result<W> {
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(self.writer)
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
    fn lines_borrows_the_backing_buffer() {
        // The read accessor HPGL's finish() reads to split each buffered
        // entry on `;`. Borrows the same lines finish() would join —
        // no clone, verbatim order.
        let s = sink_of(&["IN;SP1;", "PA0,0;"]);
        assert_eq!(s.lines(), &["IN;SP1;".to_string(), "PA0,0;".to_string()]);
        assert!(sink_of(&[]).lines().is_empty());
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

    /// Push every line through a fresh streaming sink over a `Vec<u8>`,
    /// `finish`, and return the bytes — the streaming analogue of
    /// `sink_of(lines).finish()`.
    fn stream_bytes(lines: &[&str]) -> Vec<u8> {
        let mut s = StreamingGcodeSink::new(Vec::<u8>::new());
        for l in lines {
            s.push((*l).to_string()).unwrap();
        }
        s.finish().unwrap()
    }

    #[test]
    fn streaming_output_is_byte_identical_to_buffered() {
        // The whole point: a write-through stream must produce exactly the
        // same bytes the buffered join("\n") + "\n" would, including the
        // empty-program lone-newline and single-line edge cases.
        for lines in [
            &[][..],
            &["only"][..],
            &["G0 X0", "G1 X1 Y2", "M2"][..],
            &["", "G1", ""][..], // blank lines survive verbatim
        ] {
            let streamed = stream_bytes(lines);
            let buffered = sink_of(lines).finish();
            assert_eq!(
                streamed,
                buffered.as_bytes(),
                "streaming vs buffered mismatch for {lines:?}",
            );
        }
    }

    #[test]
    fn streaming_len_counts_all_emitted_lines() {
        let mut s = StreamingGcodeSink::new(Vec::<u8>::new());
        assert_eq!(s.len(), 0);
        s.push("a".into()).unwrap();
        s.push("b".into()).unwrap();
        assert_eq!(s.len(), 2);
        s.extend_from_slice(&["c".into(), "d".into()]).unwrap();
        assert_eq!(s.len(), 4);
        // checkpoint does not reset the logical count.
        s.checkpoint();
        assert_eq!(s.len(), 4);
    }

    #[test]
    fn streaming_clone_from_returns_the_current_op_tail() {
        // Replays the emit loop's per-op sequence: checkpoint at the op
        // boundary, capture the marker via len(), emit the body, then read
        // it back with clone_from(marker) exactly as store_op_cache does.
        let mut s = StreamingGcodeSink::new(Vec::<u8>::new());

        // Op A.
        s.checkpoint();
        let marker_a = s.len();
        s.push("A0".into()).unwrap();
        s.push("A1".into()).unwrap();
        assert_eq!(
            s.clone_from(marker_a),
            vec!["A0".to_string(), "A1".to_string()],
            "clone_from(marker) must return op A's body",
        );

        // Op B — after B's checkpoint, A's lines are dropped from the tee
        // (already streamed through); only B's tail is clonable.
        s.checkpoint();
        let marker_b = s.len();
        assert_eq!(marker_b, 2);
        s.push("B0".into()).unwrap();
        assert_eq!(s.clone_from(marker_b), vec!["B0".to_string()]);
        // The tee is bounded to op B: it no longer holds A0/A1.
        assert_eq!(s.tail, vec!["B0".to_string()]);
    }

    #[test]
    fn streaming_clone_from_past_end_is_empty() {
        // Mirrors GcodeSink::clone_from: start >= len() yields nothing
        // (a no-output op captured into the cache).
        let mut s = StreamingGcodeSink::new(Vec::<u8>::new());
        s.push("a".into()).unwrap();
        assert!(s.clone_from(1).is_empty());
        assert!(s.clone_from(99).is_empty());
    }

    #[test]
    fn streaming_op_cache_replay_matches_buffered_program() {
        // End-to-end: drive the streaming sink through a miss (emit +
        // clone_from), a hit (extend_from_slice of a cached body), and a
        // no-output op, and assert the streamed bytes equal the buffered
        // program the same emissions would have produced.
        let mut s = StreamingGcodeSink::new(Vec::<u8>::new());

        // Op A: cache MISS — driver emits, store_op_cache clones the body.
        s.checkpoint();
        let m = s.len();
        s.push("A0".into()).unwrap();
        s.push("A1".into()).unwrap();
        let cached_a = s.clone_from(m); // what store_op_cache would keep

        // Op B: cache HIT — no driver run; apply_cached_op splices a body.
        s.checkpoint();
        s.extend_from_slice(&["B0".into(), "B1".into()]).unwrap();

        // Op C: HIT replaying op A's cached body verbatim.
        s.checkpoint();
        s.extend_from_slice(&cached_a).unwrap();

        assert_eq!(s.len(), 6);
        let streamed = s.finish().unwrap();
        let buffered = sink_of(&["A0", "A1", "B0", "B1", "A0", "A1"]).finish();
        assert_eq!(streamed, buffered.as_bytes());
    }
}

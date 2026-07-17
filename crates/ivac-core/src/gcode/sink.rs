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
//! [`GcodeSink`] has two modes behind one infallible API so a post can be
//! constructed either way without any driver-code change:
//!
//!   * **Buffered** (the default) — backed by an in-memory `Vec<String>`,
//!     byte-identical to the field it replaced (peak memory O(total lines)).
//!     [`GcodeSink::finish`] joins it into the program `String`.
//!   * **Streaming** — backed by a [`std::io::Write`]: each line is written
//!     straight through the moment it is emitted and only the *current op's*
//!     lines are kept in memory, so peak memory is O(largest single op).
//!     Finalized with [`GcodeSink::finish_stream`] (flush + trailing newline)
//!     rather than a `String`.
//!
//! [`GcodeSink::write_to`] is the shared write-through primitive the buffered
//! `finish()` and the streaming `push` both agree with byte-for-byte — it
//! streams the finished program without materializing the monolithic joined
//! `String`.
//!
//! # Reconciling the per-op cache (what the streaming mode resolves)
//!
//! The per-op pipeline cache captures each op's contribution with three
//! random-access line operations — [`len`](GcodeSink::len) to mark the op's
//! start, [`clone_from`](GcodeSink::clone_from) to read the op's body back at
//! its end, and [`extend_from_slice`](GcodeSink::extend_from_slice) to splice
//! a cached body in on a hit — which a pure append-only stream cannot serve
//! once bytes are flushed away.
//!
//! The reconciliation (design option (c): a bounded tee) rests on an
//! invariant of the emit loop: `clone_from` is *only ever* called with the
//! marker captured at the **start of the current op** (`pipeline.rs` captures
//! `body_marker = out_lines_count()` just before the op body and reads
//! `out_lines_clone_from(body_marker)` just after it). It is never an
//! arbitrary historical range — always the tail of what was just emitted. So
//! the streaming mode tees only the lines emitted **since the last
//! [`checkpoint`](GcodeSink::checkpoint)** into `tail`; the driver loop calls
//! [`checkpoint`](GcodeSink::checkpoint) at each op boundary (the same point
//! it reads `out_lines_count`), dropping the previous op's tee (already
//! written through to the writer). `tail` is thus bounded by one op's output,
//! and `clone_from` / `extend_from_slice` stay correct and byte-identical to
//! the buffered mode.
//!
//! # Infallible emit, deferred io-error
//!
//! Posts emit through an infallible API (`raw` / `move_to` / … all return
//! `()`), with dozens of call sites per driver. To keep that API unchanged,
//! the streaming mode **defers** the first write error into `err` instead of
//! propagating per line; [`finish_stream`](GcodeSink::finish_stream) surfaces
//! it once at the end. Line bookkeeping (`len` / `tail`) advances regardless,
//! so the op cache stays consistent even after a write fails.
//!
//! # Wired through the pipeline
//!
//! `ivac-3j1p.3` step 4a made the posts *constructible* streaming
//! ([`linuxcnc::Post::streaming`](crate::gcode::linuxcnc) /
//! [`grbl::Post::streaming`](crate::gcode::grbl)); step 4b threads a `Write`
//! through the pipeline emit loop (the loop calls
//! [`PostProcessor::checkpoint`](crate::gcode::PostProcessor::checkpoint) at
//! each op boundary and finalizes a streaming post with `finish_stream`
//! instead of `finish`), exposed as
//! [`stream_gcode_to_writer`](crate::pipeline::stream_gcode_to_writer) — a
//! real Generate can stream straight to a file. The buffered `finish() ->
//! String` path stays the default for interactive Generate (the frontend
//! g-code panel + 3D preview still want the whole program). HPGL stays
//! buffered-only: its `finish()` re-derives the program by splitting each
//! buffered entry on `;`, which needs the whole buffer the streaming mode
//! does not retain.
//!
//! # Known limitation (`ivac-3j1p.4` follow-up)
//!
//! A single pathologically large op — e.g. one laser-raster op that emits the
//! entire program as one `G1`-per-pixel body — still buffers that op's whole
//! tee, so peak stays O(that op). True O(1) there needs a cache **bypass** for
//! oversized ops (stream straight through, skip the snapshot) or a seek-based
//! re-read. Tracked as a follow-up; out of scope for the common many-ops case
//! this mode makes O(1).

use std::io::{self, Write};

/// Write one program line to `w`, prefixing the `\n` *separator* for
/// every line after the first (`first == false`). The single trailing
/// newline is emitted separately by the caller (after the last line), so
/// both the buffered [`GcodeSink::write_to`] and the streaming push agree
/// byte-for-byte on the historical `join("\n") + "\n"`. One definition of
/// the separator semantics so the buffered and streaming paths cannot drift.
fn write_separated_line<W: Write>(w: &mut W, first: bool, line: &str) -> io::Result<()> {
    if !first {
        w.write_all(b"\n")?;
    }
    w.write_all(line.as_bytes())
}

/// Write-through streaming state — the append-only counterpart to a
/// `Vec<String>`. Keeps only the current op's lines in memory (see the
/// module docs' bounded-tee reconciliation of the per-op cache).
struct StreamState {
    /// The write-through destination. Each pushed line hits it immediately;
    /// the state never holds a second full copy of the program. Boxed +
    /// `Send` so the pipeline's background emit thread can own it.
    writer: Box<dyn Write + Send>,
    /// Total logical lines emitted so far — what [`GcodeSink::len`] returns
    /// and the marker the op cache captures via `out_lines_count`.
    total: usize,
    /// The lines emitted since the last [`GcodeSink::checkpoint`]: the
    /// current op's contribution, the only range the op cache ever clones.
    /// Everything before `tail_start` has been written through and dropped,
    /// bounding memory to one op's output.
    tail: Vec<String>,
    /// Logical index of `tail[0]` — `total` as of the last checkpoint.
    /// [`GcodeSink::clone_from`] translates an absolute marker into a `tail`
    /// offset by subtracting this.
    tail_start: usize,
    /// Whether any line has reached `writer` yet, driving the `\n` separator
    /// so the stream is byte-identical to `join("\n") + "\n"`.
    wrote_any: bool,
    /// First deferred write error, surfaced by [`GcodeSink::finish_stream`].
    /// Once set, further `write_all`s are skipped but line bookkeeping still
    /// advances so the op cache stays consistent.
    err: Option<io::Error>,
}

impl StreamState {
    /// Emit one line: write it straight through (with the leading `\n`
    /// separator for every line after the first) and tee it into the
    /// current-op `tail`. On the first io error, record it and stop writing
    /// (bookkeeping still advances).
    fn push(&mut self, line: String) {
        if self.err.is_none() {
            if let Err(e) = write_separated_line(&mut self.writer, !self.wrote_any, &line) {
                self.err = Some(e);
            }
        }
        self.wrote_any = true;
        self.total += 1;
        self.tail.push(line);
    }
}

/// Accumulates a post-processor's emitted g-code lines, buffered in memory
/// or streamed straight through a writer (see the module docs).
///
/// Each stored/emitted string is one line **without** its trailing newline —
/// the newline is the line *separator*, materialized by [`GcodeSink::finish`]
/// / [`GcodeSink::write_to`] (buffered) or interleaved by the streaming push
/// + [`GcodeSink::finish_stream`] (streaming).
pub(crate) struct GcodeSink {
    mode: Mode,
}

enum Mode {
    /// In-memory buffer — byte-identical to the historical `out: Vec<String>`.
    Buffered(Vec<String>),
    /// Write-through stream — O(largest op) peak memory.
    Streaming(StreamState),
}

impl Default for GcodeSink {
    /// Buffered, empty — the historical `out: Vec<String>::new()` behavior,
    /// so `#[derive(Default)]` posts are unchanged.
    fn default() -> Self {
        Self {
            mode: Mode::Buffered(Vec::new()),
        }
    }
}

impl std::fmt::Debug for GcodeSink {
    // A `Box<dyn Write>` isn't `Debug`; summarize the mode + counts instead
    // (the posts derive `Debug`, so the sink must be `Debug`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.mode {
            Mode::Buffered(v) => f
                .debug_struct("GcodeSink")
                .field("mode", &"buffered")
                .field("lines", &v.len())
                .finish(),
            Mode::Streaming(s) => f
                .debug_struct("GcodeSink")
                .field("mode", &"streaming")
                .field("total", &s.total)
                .field("tail", &s.tail.len())
                .field("errored", &s.err.is_some())
                .finish(),
        }
    }
}

impl GcodeSink {
    /// Construct a streaming sink that writes each line straight through
    /// `writer`. The writer is expected to be buffered by the caller (e.g. a
    /// [`std::io::BufWriter`]) — the sink issues one `write_all` per line and
    /// does not batch. Finalize with [`finish_stream`](Self::finish_stream).
    pub(crate) fn streaming(writer: Box<dyn Write + Send>) -> Self {
        Self {
            mode: Mode::Streaming(StreamState {
                writer,
                total: 0,
                tail: Vec::new(),
                tail_start: 0,
                wrote_any: false,
                err: None,
            }),
        }
    }

    /// Append one already-rendered line. Buffered: pushes to the `Vec`.
    /// Streaming: writes straight through + tees into the current-op tail.
    pub(crate) fn push(&mut self, line: String) {
        match &mut self.mode {
            Mode::Buffered(v) => v.push(line),
            Mode::Streaming(s) => s.push(line),
        }
    }

    /// Number of logical lines emitted. The per-op cache slices an op's
    /// contribution by the count captured before/after the op runs.
    pub(crate) fn len(&self) -> usize {
        match &self.mode {
            Mode::Buffered(v) => v.len(),
            Mode::Streaming(s) => s.total,
        }
    }

    /// Clone the emitted lines from `start` (inclusive); empty when
    /// `start >= len()`. How the per-op cache captures an op's output range.
    ///
    /// In streaming mode `start` must be `>= the last checkpoint`
    /// (`tail_start`): the emit loop only ever clones the current op's tail
    /// (see the module docs), so earlier lines are already streamed out and
    /// gone.
    pub(crate) fn clone_from(&self, start: usize) -> Vec<String> {
        match &self.mode {
            Mode::Buffered(v) => {
                if start >= v.len() {
                    Vec::new()
                } else {
                    v[start..].to_vec()
                }
            }
            Mode::Streaming(s) => {
                if start >= s.total {
                    return Vec::new();
                }
                debug_assert!(
                    start >= s.tail_start,
                    "streaming sink can only clone the current op's tail \
                     (start {start} < checkpoint {}); the op cache never clones \
                     before the active op boundary",
                    s.tail_start,
                );
                let off = start.saturating_sub(s.tail_start);
                s.tail
                    .get(off..)
                    .map(<[String]>::to_vec)
                    .unwrap_or_default()
            }
        }
    }

    /// Append a pre-rendered batch verbatim — the op-cache hit replay path.
    /// Streaming: writes each line through like [`push`](Self::push).
    pub(crate) fn extend_from_slice(&mut self, lines: &[String]) {
        match &mut self.mode {
            Mode::Buffered(v) => v.extend_from_slice(lines),
            Mode::Streaming(s) => {
                for line in lines {
                    s.push(line.clone());
                }
            }
        }
    }

    /// Mark an op boundary (streaming only): drop the previous op's tee (its
    /// lines are already written through) and start a fresh tail at the
    /// current position, keeping `tail` bounded to a single op. `len()` is
    /// unaffected — the logical count is monotonic across checkpoints. A
    /// no-op in buffered mode (the whole program is retained anyway), so the
    /// emit loop can call it unconditionally.
    ///
    /// The pipeline emit loop calls this (via [`PostProcessor::checkpoint`]
    /// (crate::gcode::PostProcessor::checkpoint)) at each op boundary — the
    /// same point it captures `out_lines_count` as the op's `body_marker` —
    /// so the tee stays bounded to a single op's output.
    pub(crate) fn checkpoint(&mut self) {
        if let Mode::Streaming(s) = &mut self.mode {
            s.tail_start = s.total;
            s.tail.clear();
        }
    }

    /// Borrow the buffered lines as a slice. Used by a post whose `finish()`
    /// re-derives its program text from the raw lines rather than the
    /// canonical `join("\n")` (the HPGL post splits each buffered entry on
    /// `;`). Buffered-only: the streaming mode does not retain the whole
    /// program, which is exactly why HPGL is never constructed streaming.
    pub(crate) fn lines(&self) -> &[String] {
        match &self.mode {
            Mode::Buffered(v) => v,
            Mode::Streaming(_) => {
                debug_assert!(
                    false,
                    "lines() is buffered-only; streaming keeps only the current-op tail"
                );
                &[]
            }
        }
    }

    /// The finished program as one `String`: lines joined by `\n` with a
    /// trailing `\n`. Byte-identical to the historical `out.join("\n") +
    /// "\n"`. Derived from [`GcodeSink::write_to`] so there is a single
    /// canonical output path; the `from_utf8` cannot fail (every byte came
    /// from a `&str` line or the `\n` separator). Buffered-only — a streaming
    /// post has already written its program out and finalizes via
    /// [`finish_stream`](Self::finish_stream).
    pub(crate) fn finish(&self) -> String {
        match &self.mode {
            Mode::Buffered(_) => {
                let mut buf = Vec::new();
                self.write_to(&mut buf)
                    .expect("writing to a Vec<u8> is infallible");
                String::from_utf8(buf).expect("g-code lines are valid UTF-8")
            }
            Mode::Streaming(_) => {
                debug_assert!(
                    false,
                    "finish() is buffered-only; streaming posts use finish_stream()"
                );
                String::new()
            }
        }
    }

    /// Stream the finished program to `w` without materializing the joined
    /// `String` — the write-through primitive the buffered `finish()` builds
    /// on. Emits the lines separated by `\n` with a trailing `\n` (so an
    /// empty program is a lone `\n`, matching the historical `join("\n") +
    /// "\n"`). Buffered-only; the streaming mode writes through as it goes.
    pub(crate) fn write_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        match &self.mode {
            Mode::Buffered(lines) => {
                for (i, line) in lines.iter().enumerate() {
                    write_separated_line(w, i == 0, line)?;
                }
                w.write_all(b"\n")
            }
            Mode::Streaming(_) => {
                debug_assert!(false, "write_to() is buffered-only");
                Ok(())
            }
        }
    }

    /// Finalize a streaming sink: emit the single trailing newline (so the
    /// stream ends in `join("\n") + "\n"`, and an empty program is a lone
    /// `\n`), flush the writer, and surface the first deferred write error if
    /// any occurred. A no-op returning `Ok` in buffered mode — a buffered
    /// post's program is retrieved via [`finish`](Self::finish) instead — so
    /// callers can invoke it unconditionally.
    pub(crate) fn finish_stream(&mut self) -> io::Result<()> {
        let Mode::Streaming(s) = &mut self.mode else {
            return Ok(());
        };
        if s.err.is_none() {
            if let Err(e) = s.writer.write_all(b"\n") {
                s.err = Some(e);
            }
        }
        if s.err.is_none() {
            if let Err(e) = s.writer.flush() {
                s.err = Some(e);
            }
        }
        match s.err.take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

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

    /// An in-memory `Write + Send` whose bytes stay readable after the sink
    /// has moved the writer in — the streaming analogue of inspecting a
    /// buffered `finish()`. `Arc<Mutex<..>>` satisfies the sink's `Send`
    /// bound and lets the test hold a second handle to read back.
    #[derive(Clone)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Push every line through a fresh streaming sink over a shared buffer,
    /// `finish_stream`, and return the bytes — the streaming analogue of
    /// `sink_of(lines).finish()`.
    fn stream_bytes(lines: &[&str]) -> Vec<u8> {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let mut s = GcodeSink::streaming(Box::new(SharedBuf(buf.clone())));
        for l in lines {
            s.push((*l).to_string());
        }
        s.finish_stream().unwrap();
        let out = buf.lock().unwrap().clone();
        out
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
        let buf = Arc::new(Mutex::new(Vec::new()));
        let mut s = GcodeSink::streaming(Box::new(SharedBuf(buf)));
        assert_eq!(s.len(), 0);
        s.push("a".into());
        s.push("b".into());
        assert_eq!(s.len(), 2);
        s.extend_from_slice(&["c".into(), "d".into()]);
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
        let buf = Arc::new(Mutex::new(Vec::new()));
        let mut s = GcodeSink::streaming(Box::new(SharedBuf(buf)));

        // Op A.
        s.checkpoint();
        let marker_a = s.len();
        s.push("A0".into());
        s.push("A1".into());
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
        s.push("B0".into());
        assert_eq!(s.clone_from(marker_b), vec!["B0".to_string()]);
    }

    #[test]
    fn streaming_clone_from_past_end_is_empty() {
        // Mirrors buffered clone_from: start >= len() yields nothing
        // (a no-output op captured into the cache).
        let buf = Arc::new(Mutex::new(Vec::new()));
        let mut s = GcodeSink::streaming(Box::new(SharedBuf(buf)));
        s.push("a".into());
        assert!(s.clone_from(1).is_empty());
        assert!(s.clone_from(99).is_empty());
    }

    #[test]
    fn streaming_op_cache_replay_matches_buffered_program() {
        // End-to-end: drive the streaming sink through a miss (emit +
        // clone_from), a hit (extend_from_slice of a cached body), and a
        // replay of a cached body, and assert the streamed bytes equal the
        // buffered program the same emissions would have produced.
        let buf = Arc::new(Mutex::new(Vec::new()));
        let mut s = GcodeSink::streaming(Box::new(SharedBuf(buf.clone())));

        // Op A: cache MISS — driver emits, store_op_cache clones the body.
        s.checkpoint();
        let m = s.len();
        s.push("A0".into());
        s.push("A1".into());
        let cached_a = s.clone_from(m); // what store_op_cache would keep

        // Op B: cache HIT — no driver run; apply_cached_op splices a body.
        s.checkpoint();
        s.extend_from_slice(&["B0".into(), "B1".into()]);

        // Op C: HIT replaying op A's cached body verbatim.
        s.checkpoint();
        s.extend_from_slice(&cached_a);

        assert_eq!(s.len(), 6);
        s.finish_stream().unwrap();
        let streamed = buf.lock().unwrap().clone();
        let buffered = sink_of(&["A0", "A1", "B0", "B1", "A0", "A1"]).finish();
        assert_eq!(streamed, buffered.as_bytes());
    }

    /// A `Write` that fails on the Nth write — to exercise deferred io-error.
    struct FailAfter {
        writes_left: usize,
    }

    impl Write for FailAfter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.writes_left == 0 {
                return Err(io::Error::new(io::ErrorKind::Other, "boom"));
            }
            self.writes_left -= 1;
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn streaming_defers_the_first_write_error_to_finish() {
        // The infallible push API cannot return an error, so the sink defers
        // it: bookkeeping keeps advancing and finish_stream surfaces it once.
        let mut s = GcodeSink::streaming(Box::new(FailAfter { writes_left: 1 }));
        s.push("ok".into()); // consumes the one allowed write
        s.push("boom".into()); // separator write fails; deferred
        s.push("still-counted".into());
        // Bookkeeping is unaffected by the io error — the op cache stays
        // consistent even past a failed write.
        assert_eq!(s.len(), 3);
        let err = s.finish_stream().expect_err("deferred error must surface");
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[test]
    fn finish_stream_on_buffered_is_ok_noop() {
        // Buffered posts have nothing to flush; callers can invoke it
        // unconditionally.
        let mut s = sink_of(&["G0", "G1"]);
        assert!(s.finish_stream().is_ok());
        // The buffered program is still retrievable via finish().
        assert_eq!(s.finish(), "G0\nG1\n");
    }
}

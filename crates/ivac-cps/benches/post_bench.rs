//! Post-execution throughput baseline.
//!
//! Measures whole-program runs (prelude + post eval, then dispatch of N
//! motion records) through the bundled grbl post at 10k / 100k / 300k
//! records. Records-per-second is the number that matters: boa is
//! ~50-100x slower than V8, so this is the tripwire for "is the JS
//! runtime fast enough for real programs on desktop?".
//!
//!   cargo bench -p ivac-cps
//!
//! Measured 2026-07-29 (this tree, release): 10k records 1.39 s
//! (7.2k rec/s), 100k 15.2 s (6.6k rec/s), 300k 46.0 s (6.5k rec/s) —
//! ~150 µs per record, flat with program size. Profiling put
//! ~60% of that in `FormatNumber.format`, and the cheap half of that
//! (a per-call regex + repeated `Math.pow`) is already gone: end-to-end
//! throughput improved 20-24% at scale (42% at 10k) with byte-identical
//! output, proven by the V8↔boa differential and the FANUC goldens.
//!
//! What remains is boa's per-property-read cost, which no JS-level
//! change removes. The next lever, if large programs ever demand it, is
//! nativizing the format pipeline as boa host functions behind the same
//! JS names (post-visible behavior identical; the differential suite is
//! the proof). Bigger practical win first: whole-run memoization so a
//! repeat Generate never enters boa at all (ivac-rnsg).

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ivac_cps::ir::{
    codes, Header, ParamValue, Parameter, Position, Program, Record, Section, ToolSpec, IR_VERSION,
};

fn program_with(records: usize) -> Program {
    let mut motion = Vec::with_capacity(records);
    for i in 0..records {
        // A serpentine cut: every record moves, so nothing is
        // delta-suppressed away and the format/variable pipeline runs
        // at full cost per line.
        #[allow(clippy::cast_precision_loss)]
        let t = i as f64;
        motion.push(Record::Linear {
            x: (t * 0.37) % 200.0,
            y: (t * 0.11) % 100.0,
            z: -1.0 - (t * 0.001) % 2.0,
            feed: 800.0,
            movement: codes::MOVEMENT_CUTTING,
        });
    }
    Program {
        version: IR_VERSION,
        header: Header {
            program_name: "1001".into(),
            program_comment: "bench".into(),
            tolerance_mm: 0.01,
            parameters: vec![],
        },
        sections: vec![Section {
            id: 1,
            strategy: "contour2d".into(),
            tool: ToolSpec {
                number: 1,
                description: "6mm flat".into(),
                diameter: 6.0,
                corner_radius: 0.0,
                taper_angle: 0.0,
                flutes: 2,
                tool_type: codes::TOOL_MILLING_END_FLAT,
                coolant: codes::COOLANT_DISABLED,
            },
            spindle_rpm: 18000.0,
            spindle_clockwise: true,
            work_offset: 1,
            work_plane: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            parameters: vec![Parameter {
                name: "operation-comment".into(),
                value: ParamValue::Text("Bench".into()),
            }],
            initial_position: Position {
                x: 0.0,
                y: 0.0,
                z: 15.0,
            },
            final_position: Position {
                x: 0.0,
                y: 0.0,
                z: 15.0,
            },
            records: motion,
        }],
        machine: None,
    }
}

fn bench_post(c: &mut Criterion) {
    let script = ivac_cps::library::bundled("grbl")
        .expect("bundled grbl")
        .source;
    let overrides = serde_json::json!({});
    let mut group = c.benchmark_group("run_post");
    // A 300k-record run takes seconds; keep the sample count low so the
    // whole bench stays in the minutes range.
    group.sample_size(10);
    for records in [10_000usize, 100_000, 300_000] {
        let program = program_with(records);
        group.throughput(Throughput::Elements(records as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(records),
            &program,
            |b, program| {
                b.iter(|| {
                    ivac_cps::run_post(script, "grbl.cps", program, &overrides).expect("post runs")
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_post);
criterion_main!(benches);

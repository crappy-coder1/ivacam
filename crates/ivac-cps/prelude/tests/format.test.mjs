// format.test.mjs — hand-pinned FormatNumber expectations. The broad
// matrix lives in fixtures/format_cases.json (differential-checked
// against boa); these are the human-verified anchors from real FANUC
// usage — if one of these changes, the change is wrong.

import { test } from "node:test";
import assert from "node:assert/strict";
import { createRuntime, readFixture, computeExpected } from "./harness.mjs";

function fmt(spec) {
  const rt = createRuntime();
  return (v) => rt.eval(`createFormat(${spec}).format(${v})`);
}

test("gFormat: width 2, zeropad, decimals 1 (FANUC G-words)", () => {
  const rt = createRuntime();
  rt.eval(`var g = createFormat({prefix:"G", width:2, zeropad:true, decimals:1});`);
  assert.equal(rt.eval("g.format(0)"), "G00");
  assert.equal(rt.eval("g.format(1)"), "G01");
  assert.equal(rt.eval("g.format(5.1)"), "G05.1");
  assert.equal(rt.eval("g.format(17)"), "G17");
  assert.equal(rt.eval("g.format(90)"), "G90");
  assert.equal(rt.eval("g.format(53.1)"), "G53.1");
});

test("xyzFormat: 3 decimals, forceDecimal, trailing-zero trim", () => {
  const f = fmt(`{decimals:3, forceDecimal:true}`);
  assert.equal(f("10.5"), "10.5");
  assert.equal(f("10"), "10.");
  assert.equal(f("0"), "0.");
  assert.equal(f("-0.25"), "-0.25");
  assert.equal(f("0.0005"), "0.001"); // half away from zero
  assert.equal(f("-0.0005"), "-0.001");
  assert.equal(f("2.6745"), "2.675"); // representation-error tie
  assert.equal(f("-2.6745"), "-2.675");
  assert.equal(f("0.0004"), "0."); // rounds to zero: no sign
  assert.equal(f("-0.0004"), "0.");
  assert.equal(f("0.1 + 0.2"), "0.3"); // binary noise never leaks
});

test("feedFormat: 0 decimals, forceDecimal (FANUC F-words)", () => {
  const f = fmt(`{decimals:0, forceDecimal:true}`);
  assert.equal(f("500"), "500.");
  assert.equal(f("499.5"), "500.");
  assert.equal(f("499.4"), "499.");
});

test("abcFormat: scale DEG converts radians to degrees", () => {
  const rt = createRuntime();
  rt.eval(`var abc = createFormat({decimals:3, forceDecimal:true, scale:DEG});`);
  assert.equal(rt.eval("abc.format(Math.PI)"), "180.");
  assert.equal(rt.eval("abc.format(Math.PI / 6)"), "30.");
  assert.equal(rt.eval("abc.format(-Math.PI / 2)"), "-90.");
});

test("oFormat: width 4 zeropad (program numbers)", () => {
  const f = fmt(`{width:4, zeropad:true, decimals:0}`);
  assert.equal(f("1"), "0001");
  assert.equal(f("1001"), "1001");
  assert.equal(f("12345"), "12345"); // width never truncates
});

test("cyclic mapping: sign 1 wraps into [0, limit) before scaling", () => {
  const rt = createRuntime();
  rt.eval(
    `var c = createFormat({decimals:3, forceDecimal:true, cyclicLimit:Math.PI*2, cyclicSign:1, scale:DEG});`
  );
  assert.equal(rt.eval("c.format(Math.PI * 2 + 1)"), rt.eval("c.format(1)"));
  assert.equal(rt.eval("c.format(-Math.PI / 2)"), "270.");
});

test("clamping applies after rounding", () => {
  const f = fmt(`{decimals:2, forceDecimal:true, minimum:-10, maximum:10}`);
  assert.equal(f("11"), "10.");
  assert.equal(f("-11"), "-10.");
  assert.equal(f("9.996"), "10.");
});

test("trimLeadZero renders FANUC-style bare fractions", () => {
  const f = fmt(`{decimals:3, trimLeadZero:true, forceDecimal:true}`);
  assert.equal(f("0.5"), ".5");
  assert.equal(f("-0.5"), "-.5");
  assert.equal(f("1.5"), "1.5");
  assert.equal(f("0"), "0.");
});

test("forceSign marks positives, never zero", () => {
  const f = fmt(`{decimals:2, forceDecimal:true, forceSign:true}`);
  assert.equal(f("5"), "+5.");
  assert.equal(f("-5"), "-5.");
  assert.equal(f("0"), "0.");
});

test("getResultingValue/areDifferent compare on the output grid", () => {
  const rt = createRuntime();
  rt.eval(`var f = createFormat({decimals:3, forceDecimal:true});`);
  assert.equal(rt.eval("f.getResultingValue(1.0004)"), 1);
  assert.equal(rt.eval("f.getResultingValue(1.0005)"), 1.001);
  assert.equal(rt.eval("f.areDifferent(1.0004, 1.0)"), false);
  assert.equal(rt.eval("f.areDifferent(1.0006, 1.0)"), true);
  assert.equal(rt.eval("f.isSignificant(0.0004)"), false);
  assert.equal(rt.eval("f.isSignificant(0.0006)"), true);
});

test("differential fixtures are fresh (committed == recomputed)", () => {
  const expected = computeExpected();
  assert.deepEqual(
    expected.formats,
    JSON.parse(readFixture("format_expected.json")),
    "format_expected.json is stale — node prelude/tests/run.mjs --update"
  );
  assert.deepEqual(
    expected.scenarios,
    JSON.parse(readFixture("scenarios_expected.json")),
    "scenarios_expected.json is stale — node prelude/tests/run.mjs --update"
  );
});

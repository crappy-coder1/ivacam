// variables.test.mjs — Variable/Modal/Reference suppression semantics,
// pinned by hand against FANUC post behavior.

import { test } from "node:test";
import assert from "node:assert/strict";
import { createRuntime } from "./harness.mjs";

test("variable suppresses repeats on the OUTPUT grid, not raw values", () => {
  const rt = createRuntime();
  rt.eval(`
    var x = createVariable({prefix:"X"}, createFormat({decimals:3, forceDecimal:true}));
  `);
  assert.equal(rt.eval("x.format(10.5)"), "X10.5");
  assert.equal(rt.eval("x.format(10.5)"), "");
  assert.equal(rt.eval("x.format(10.5004)"), ""); // same after rounding
  assert.equal(rt.eval("x.format(10.501)"), "X10.501");
  assert.equal(rt.eval("x.getCurrent()"), 10.501);
});

test("reset forces the next output; force outputs every time", () => {
  const rt = createRuntime();
  rt.eval(`
    var a = createVariable({prefix:"A"}, createFormat({decimals:1, forceDecimal:true}));
    var s = createVariable({prefix:"S", force:true}, createFormat({decimals:0}));
  `);
  assert.equal(rt.eval("a.format(1)"), "A1.");
  rt.eval("a.reset()");
  assert.equal(rt.eval("a.format(1)"), "A1.");
  assert.equal(rt.eval("s.format(12000)"), "S12000");
  assert.equal(rt.eval("s.format(12000)"), "S12000");
});

test("onchange fires exactly when text is produced (FANUC retracted)", () => {
  const rt = createRuntime();
  rt.eval(`
    var retracted = true;
    var z = createVariable({onchange:function() {retracted = false;}, prefix:"Z"},
                           createFormat({decimals:3, forceDecimal:true}));
  `);
  assert.equal(rt.eval("z.format(15)"), "Z15.");
  assert.equal(rt.eval("retracted"), false);
  rt.eval("retracted = true;");
  assert.equal(rt.eval("z.format(15)"), ""); // suppressed → no onchange
  assert.equal(rt.eval("retracted"), true);
  assert.equal(rt.eval("z.format(-1)"), "Z-1.");
  assert.equal(rt.eval("retracted"), false);
});

test("ReferenceVariable: 2-arg format suppresses against the reference", () => {
  const rt = createRuntime();
  rt.eval(`
    var i = createReferenceVariable({prefix:"I"}, createFormat({decimals:3, forceDecimal:true}));
  `);
  assert.equal(rt.eval("i.format(5, 0)"), "I5.");
  assert.equal(rt.eval("i.format(0, 0)"), "");
  assert.equal(rt.eval("i.format(0.0004, 0)"), ""); // rounds onto reference
  assert.equal(rt.eval("i.format(-2.5, 0)"), "I-2.5");
  assert.equal(rt.eval("i.format(7)"), "I7."); // no reference → always out
});

test("modal carries the format's address letter and getCurrent", () => {
  const rt = createRuntime();
  rt.eval(`
    var gMotion = createModal({}, createFormat({prefix:"G", width:2, zeropad:true, decimals:1}));
  `);
  assert.equal(rt.eval("gMotion.format(0)"), "G00");
  assert.equal(rt.eval("gMotion.format(0)"), "");
  assert.equal(rt.eval("gMotion.format(1)"), "G01");
  assert.equal(rt.eval("gMotion.getCurrent()"), 1);
  rt.eval("gMotion.reset()");
  assert.equal(rt.eval("gMotion.format(1)"), "G01");
});

test("modal onchange resets a sibling modal (gPlane → gMotion pattern)", () => {
  const rt = createRuntime();
  rt.eval(`
    var gFormat = createFormat({prefix:"G", width:2, zeropad:true, decimals:1});
    var gMotion = createModal({}, gFormat);
    var gPlane = createModal({onchange:function() {gMotion.reset();}}, gFormat);
  `);
  assert.equal(rt.eval("gMotion.format(1)"), "G01");
  assert.equal(rt.eval("gPlane.format(17)"), "G17");
  assert.equal(rt.eval("gMotion.format(1)"), "G01"); // reset by plane change
  assert.equal(rt.eval("gPlane.format(17)"), ""); // suppressed → no reset
  assert.equal(rt.eval("gMotion.format(1)"), "");
});

test("createOutputVariable: control + incremental type", () => {
  const rt = createRuntime();
  rt.eval(`
    var fmt = createFormat({decimals:3, forceDecimal:true});
    var abs = createOutputVariable({prefix:"X"}, fmt);
    var forced = createOutputVariable({prefix:"S", control:CONTROL_FORCE}, createFormat({decimals:0}));
    var inc = createOutputVariable({prefix:"U", type:TYPE_INCREMENTAL}, fmt);
  `);
  assert.equal(rt.eval("abs.format(5)"), "X5.");
  assert.equal(rt.eval("abs.format(5)"), "");
  assert.equal(rt.eval("forced.format(100)"), "S100");
  assert.equal(rt.eval("forced.format(100)"), "S100");
  assert.equal(rt.eval("inc.format(5)"), "U5.");
  assert.equal(rt.eval("inc.format(7.5)"), "U2.5");
  assert.equal(rt.eval("inc.format(7.5)"), "");
});

test("ModalGroup enforces per-group exclusivity", () => {
  const rt = createRuntime();
  rt.eval(`
    var mg = new ModalGroup();
    mg.setFormatNumber(createFormat({prefix:"G", width:2, zeropad:true, decimals:1}));
    var motion = mg.createGroup();
    mg.addCode(motion, 0); mg.addCode(motion, 1);
    var plane = mg.createGroup();
    mg.addCode(plane, 17); mg.addCode(plane, 18);
  `);
  assert.equal(rt.eval("mg.format(0)"), "G00");
  assert.equal(rt.eval("mg.format(17)"), "G17"); // other group unaffected
  assert.equal(rt.eval("mg.format(0)"), "");
  assert.equal(rt.eval("mg.format(1)"), "G01");
  assert.equal(rt.eval("mg.getActiveCode(1)"), 1);
  assert.equal(rt.eval("mg.inSameGroup(0, 1)"), true);
  assert.equal(rt.eval("mg.inSameGroup(1, 17)"), false);
});

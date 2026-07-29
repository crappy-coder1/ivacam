// words.test.mjs — writeWords/writeWords2/formatWords flattening and the
// redirection sink stack.

import { test } from "node:test";
import assert from "node:assert/strict";
import { createRuntime } from "./harness.mjs";

test("formatWords flattens arguments objects and nested arrays", () => {
  const rt = createRuntime();
  assert.equal(rt.eval(`formatWords("G1", "X10.5", "F500")`), "G1 X10.5 F500");
  assert.equal(rt.eval(`formatWords(["G1", ["X1", "Y2"], "Z3"])`), "G1 X1 Y2 Z3");
  assert.equal(rt.eval(`formatWords("A", undefined, null, "", "B")`), "A B");
  assert.equal(
    rt.eval(`(function () { return formatWords(arguments); })("G0", "X0")`),
    "G0 X0"
  );
  // The FANUC writeBlock shape: arguments object forwarded whole.
  assert.equal(
    rt.eval(`
      (function writeBlockish() {
        return formatWords("N10", arguments);
      })("G1", ["X5", "Y6"])
    `),
    "N10 G1 X5 Y6"
  );
});

test("writeWords drops empty lines entirely", () => {
  const rt = createRuntime();
  rt.eval(`writeWords("G1", "X5");`);
  rt.eval(`writeWords("", undefined);`); // nothing
  rt.eval(`writeWords("M30");`);
  assert.equal(rt.output(), "G1 X5\nM30\n");
});

test("writeWords2 outputs only when args 2+ produce text", () => {
  const rt = createRuntime();
  rt.eval(`writeWords2("N10", "G1", "X5");`);
  rt.eval(`writeWords2("N20");`);
  rt.eval(`writeWords2("N30", "", undefined);`);
  rt.eval(`writeWords2("/", "N40", "G0");`);
  assert.equal(rt.output(), "N10 G1 X5\n/ N40 G0\n");
});

test("word separator and EOL are honored", () => {
  const rt = createRuntime();
  rt.eval(`setWordSeparator(""); writeWords("G1", "X5", "F100");`);
  rt.eval(`setWordSeparator(" "); setEOL("\\r\\n"); writeWords("M30");`);
  assert.equal(rt.output(), "G1X5F100\nM30\r\n");
});

test("redirection captures words; close returns to normal output", () => {
  const rt = createRuntime();
  rt.eval(`writeln("before");`);
  rt.eval(`redirectToBuffer();`);
  rt.eval(`writeWords("G0", "X1");`);
  assert.equal(rt.eval("isRedirecting()"), true);
  assert.equal(rt.eval("getRedirectionBuffer()"), "G0 X1\n");
  rt.eval(`closeRedirection();`);
  assert.equal(rt.eval("isRedirecting()"), false);
  rt.eval(`writeln("after");`);
  assert.equal(rt.output(), "before\nafter\n");
});

test("nested redirection pops innermost first", () => {
  const rt = createRuntime();
  rt.eval(`
    redirectToBuffer();
    writeln("outer1");
    redirectToBuffer();
    writeln("inner");
    var innerText = getRedirectionBuffer();
    closeRedirection();
    writeln("outer2");
    var outerText = getRedirectionBuffer();
    closeRedirection();
  `);
  assert.equal(rt.eval("innerText"), "inner\n");
  assert.equal(rt.eval("outerText"), "outer1\nouter2\n");
  assert.equal(rt.output(), "");
});

test("text helpers: subst/conditional/filterText/strict parsers", () => {
  const rt = createRuntime();
  assert.equal(rt.eval(`subst("Tool %1 of %2", 3, 8)`), "Tool 3 of 8");
  assert.equal(rt.eval(`conditional(1 == 1, "F100")`), "F100");
  assert.equal(rt.eval(`conditional(0, "Q1")`), "");
  assert.equal(
    rt.eval(
      `filterText("HELLO, WORLD! #5", " ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.,=_-")`
    ),
    "HELLO, WORLD 5"
  );
  assert.equal(rt.eval(`getAsInt("1001")`), 1001);
  assert.equal(rt.eval(`getAsFloat("-12.5")`), -12.5);
  assert.equal(
    rt.eval(`(function () { try { getAsInt("O1001"); return "no"; } catch (e) { return "yes"; } })()`),
    "yes"
  );
});

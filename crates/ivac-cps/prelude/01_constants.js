// 01_constants.js — kernel constants.
//
// cps.3 seeds only what the output-exactness core (04-07) needs; cps.4
// completes the full ~330-constant table (COMMAND_*, COOLANT_*,
// MOVEMENT_*, TOOL_*, CAPABILITY_*, PLANE_*, ...) and adds the Rust↔JS
// consistency test against ivac-cps/src/ir.rs::codes.
//
// Posts compare these by NAME, so the requirement is that recorder
// (Rust) and prelude (JS) agree — not that the numbers match Autodesk's
// undocumented internals.

// Units (the driver assigns the active `unit` global from these).
var IN = 1;
var MM = 2;

// Scale helper for createFormat({scale:DEG}) — radians → degrees.
var DEG = 180.0 / Math.PI;

// FormatNumber.setType() values.
var FORMAT_REAL = 0;
var FORMAT_INTEGER = 1;
var FORMAT_LZS = 2; // leading-zero suppression (".5")
var FORMAT_TZS = 3; // trailing-zero suppression

// OutputVariable.setControl() values.
var CONTROL_CHANGED = 0;
var CONTROL_FORCE = 1;
var CONTROL_NONZERO = 2;

// OutputVariable.setType() values.
var TYPE_ABSOLUTE = 0;
var TYPE_INCREMENTAL = 1;
var TYPE_DIRECTIONAL = 2;

// Convenience text constants (globals.d.ts declares them).
var EOL = "\n";
var SP = " ";
var CR = "\r";
var LF = "\n";
var TAB = "\t";

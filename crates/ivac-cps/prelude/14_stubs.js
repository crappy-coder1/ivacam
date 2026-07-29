// 14_stubs.js — the completeness layer.
//
// Everything `globals.d.ts` declares that the earlier modules don't
// already provide lands here, in one of two forms:
//
//  * REAL — cheap, correct implementations (arc geometry read off the
//    circular state, record-cursor queries, tool-table helpers, unit
//    math). Preferred: a working answer beats a diagnostic.
//  * WARN-ONCE STUB — features ivacam genuinely has no data for
//    (probing, polar mode, subprogram files, turning, additive). Each
//    emits ONE named diagnostic and returns a benign value, so a
//    third-party post touching an unsupported corner produces an
//    actionable field report instead of `TypeError: not a callable
//    function`.
//
// `tests/symbol_audit.rs` fails when a declared symbol is missing here,
// so a refs/ update surfaces new API immediately.

var __ivacWarnedOnce = {};

/// One diagnostic per distinct message, however often it's hit.
function warningOnce(message) {
  var key = String(message);
  if (__ivacWarnedOnce[key]) {
    return;
  }
  __ivacWarnedOnce[key] = true;
  warning(key);
}

/// Names the unsupported feature once, then lets the post continue.
function __ivacUnsupported(feature) {
  warningOnce(
    subst(localize("%1 is not supported by ivaCAM CPS v1"), String(feature))
  );
}

// ---- environment / identity ----

function getVersion() {
  return "ivaCAM CPS runtime";
}
function getPlatform() {
  return "ivacam";
}
/// Security level 0 = no filesystem access, which is exactly true here.
function getSecurityLevel() {
  return 0;
}
function getConfigurationPath() {
  return "";
}
function getOutputPath() {
  return "";
}
function getPostProcessorPath() {
  return "";
}
function getCachePath() {
  return "";
}
function getTempFolder() {
  return "";
}
function getTempFile() {
  return "";
}
function include(path) {
  __ivacUnsupported("include(" + String(path) + ")");
}
function getProgramNameAsInt() {
  return getAsInt(programName);
}
function getGlobalParameterCount() {
  return 0;
}

// ---- small helpers the kernel exposes ----

/// Integer sequence [start, end) — Autodesk's array helper.
function range(start, end, step) {
  var out = [];
  var from = end === undefined ? 0 : start;
  var to = end === undefined ? start : end;
  var by = step === undefined ? 1 : step;
  if (by === 0) {
    return out;
  }
  for (var v = from; by > 0 ? v < to : v > to; v += by) {
    out.push(v);
  }
  return out;
}

/// Recursively flatten nested arrays / array-likes into one array.
function flatten(value) {
  var out = [];
  __flattenWords(out, value);
  return out;
}

/// Inverse-time feed word (F = distance / feed, in the requested unit).
function getInverseTime(distance, feed) {
  if (distance <= 1e-12 || feed <= 1e-12) {
    return 0;
  }
  return feed / distance;
}

// ---- motion / state queries ----

var feedrate = 0;
var end;
var initialCyclePosition;
var spindleAxis;

function getFeedrate() {
  return feedrate;
}
function getEnd() {
  return end === undefined ? getCurrentPosition() : end;
}
function getWCSPosition() {
  return getCurrentPosition();
}
function getCurrentToolAxis() {
  return getCurrentDirection();
}
function getRadiusCompensation() {
  return radiusCompensation;
}
function getPower() {
  // Laser/plasma power rides the spindle-speed word by convention.
  return spindleSpeed;
}
function getMovement() {
  return movement;
}
function isAxialCenterDrilling() {
  return false;
}
function isNewWorkPlane() {
  return isFirstSection();
}
function isToolChangeNeeded() {
  if (isFirstSection()) {
    return true;
  }
  return getPreviousSection().getTool().number !== tool.number;
}
function isProbingCycle() {
  return false;
}
function isWellKnownCommand(command) {
  return __ivacCommandNames[command] !== undefined;
}
function invokeOnSpindleSpeed(rpm) {
  spindleSpeed = rpm;
  __ivacCallOptional("onSpindleSpeed", [rpm]);
}

// ---- record cursor ----

function hasPreviousRecord() {
  return __ivacRun.recordIndex > 0;
}
function getRecord(index) {
  var section = __ivacRun.sections[__ivacRun.sectionIndex];
  if (!section) {
    return __ivacRecordWrapper(undefined);
  }
  var i = index === undefined ? __ivacRun.recordIndex : index;
  return __ivacRecordWrapper(section.__ir.records[i]);
}
function getNumberOfRecords() {
  var section = __ivacRun.sections[__ivacRun.sectionIndex];
  return section ? section.__ir.records.length : 0;
}

// ---- circular geometry (read off the live circular state) ----

function getCircularStartRadius() {
  return getCircularRadius();
}
function getCircularArcLength() {
  return getCircularRadius() * getCircularSweep();
}
function getHelicalDistance() {
  var c = __ivacCircular;
  return Math.abs(c.end.z - c.start.z);
}
function getISOPlane() {
  var plane = getCircularPlane();
  if (plane === PLANE_XY) {
    return 17;
  }
  if (plane === PLANE_ZX) {
    return 18;
  }
  if (plane === PLANE_YZ) {
    return 19;
  }
  return -1;
}
/// Quadrant (1-4) of the arc's start point about its center.
function getQuadrant() {
  var c = __ivacCircular;
  var dx = c.start.x - c.center.x;
  var dy = c.start.y - c.center.y;
  if (dx >= 0 && dy >= 0) {
    return 1;
  }
  if (dx < 0 && dy >= 0) {
    return 2;
  }
  if (dx < 0 && dy < 0) {
    return 3;
  }
  return 4;
}

// Circular state mirrors the d.ts declares as bare globals. Kept in
// sync by the driver's arc dispatch.
var circularCenter;
var circularNormal;
var circularRadius = 0;
var circularSweep = 0;
var circularChordLength = 0;
var circularHelicalDistance = 0;
var circularFullCircle = false;
var circularSpiral = false;
var circularMergeTolerance = 1e-6;

// ---- tools ----

function getFirstTool() {
  var table = getToolTable();
  return table.getNumberOfTools() > 0 ? table.getTool(0) : undefined;
}
function getToolList() {
  var table = getToolTable();
  var out = [];
  for (var i = 0; i < table.getNumberOfTools(); ++i) {
    out.push(table.getTool(i));
  }
  return out;
}
function getNumberOfTools() {
  return getToolTable().getNumberOfTools();
}
/// Z range across every section using `toolNumber`.
function toolZRange(toolNumber) {
  var range_ = new Range();
  for (var i = 0; i < getNumberOfSections(); ++i) {
    var section = getSection(i);
    if (section.getTool().number === toolNumber) {
      range_.expandToRange(section.getGlobalZRange());
    }
  }
  return range_;
}
function getMaterialName() {
  return "";
}
function getFixture() {
  __ivacUnsupported("fixture queries");
  return undefined;
}

// ---- multi-axis helpers (real API lands with the kinematics work) ----

function getMultiAxisMoveLength(_x, _y, _z, _a, _b, _c) {
  return new MoveLength();
}
function MoveLength() {
  this.tool = 0;
  this.linear = 0;
  this.rotary = 0;
  this.abc = new Vector(0, 0, 0);
}
function VectorPair(first, second) {
  this.first = first === undefined ? new Vector(0, 0, 0) : first;
  this.second = second === undefined ? new Vector(0, 0, 0) : second;
}
function loadMachineConfiguration(path) {
  __ivacUnsupported("loadMachineConfiguration(" + String(path) + ")");
  return machineConfiguration;
}
var eulerConvention = EULER_ZXZ_R;
var machineParameters;
function MachineParameters() {}

// ---- polar mode (turning/mill-turn: no ivacam data source) ----

function activatePolarMode() {
  __ivacUnsupported("polar mode");
}
function activateAutoPolarMode() {
  __ivacUnsupported("polar mode");
}
function deactivatePolarMode() {
  __ivacUnsupported("polar mode");
}
function isPolarModeActive() {
  return false;
}

// ---- properties / lifecycle ----

function validatePropertyDefinitions() {
  // The property sheet is validated on the Rust side (inspect_post);
  // nothing for the post to do here.
}
var __ivacTerminationHandlers = [];
function registerTerminationHandler(handler) {
  if (typeof handler === "function") {
    __ivacTerminationHandlers.push(handler);
  }
}

// ---- declared globals with kernel defaults ----

var abortOnDeprecation = false;
var allowFeedPerRevolutionDrilling = false;
var allowMachineChangeOnSection = false;
var bufferRotaryMoves = false;
var mapToWCS = true;
var mapWorkOrigin = true;
var preventPost = false;
var probeMultipleFeatures = false;
var supportedFeatures = 0;
var filename = "";

// ---- class placeholders ----
//
// Posts RECEIVE these (from getSection/getTool/getRecord/…), they don't
// construct them; the constructors exist so `instanceof` and `typeof`
// probes behave, and warn if a post really tries to build one.

function Section() {
  __ivacUnsupported("constructing Section directly");
}
function Tool() {
  __ivacUnsupported("constructing Tool directly");
}
function ToolTable() {
  __ivacUnsupported("constructing ToolTable directly");
}
function Record() {
  __ivacUnsupported("constructing Record directly");
}
function CircularMotion() {
  __ivacUnsupported("constructing CircularMotion directly");
}
function StringSubstitution() {
  __ivacUnsupported("StringSubstitution");
}
function FileSystem() {
  __ivacUnsupported("filesystem access");
}
function TextFile() {
  __ivacUnsupported("file output");
}
function Simulation() {
  __ivacUnsupported("machine simulation");
}
var simulation;

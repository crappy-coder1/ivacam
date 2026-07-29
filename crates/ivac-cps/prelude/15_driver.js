// 15_driver.js — load sequence, program model, and the dispatch driver.
//
// Eval order contract: everything a post touches at TOP LEVEL (unit,
// spatial(), toRad(), constants, factories) is live before the host
// evaluates the .cps file; the kernel reads the post's config globals
// back only when __ivacExecute runs. error() sets a halt flag WITHOUT
// throwing (posts do `error(...); return;`), and the driver stops
// after the callback returns.

// Machine singleton — created HERE, not in 09_machine.js, to dodge a
// boa 0.21.1 same-script new-after-prototype-assignment bug (see the
// note in 09_machine.js).
machineConfiguration = new MachineConfiguration();

// ---- config globals (kernel defaults; the post's top level overrides) ----

var unit = MM;
var programName = "";
var programComment = "";

var description = "";
var vendor = "";
var vendorUrl = "";
var legal = "";
var certificationLevel = 2;
var minimumRevision = 0;
var longDescription = "";
var extension = "nc";
var programNameIsInteger = false;
var capabilities = 0;
var tolerance = 0.02;

var minimumChordLength = 0.25;
var minimumCircularRadius = 0.01;
var maximumCircularRadius = 1000;
var minimumCircularSweep = toRad(0.01);
var maximumCircularSweep = toRad(180);
var allowHelicalMoves = true;
var allowSpiralMoves = false;
var allowedCircularPlanes; // undefined is meaningful: any plane

var highFeedrate = 0;
var highFeedMapping = HIGH_FEED_NO_MAPPING;

// ---- driver state ----

var currentSection = undefined;
var tool = undefined;
var spindleSpeed = 0;
var movement = MOVEMENT_RAPID;
var radiusCompensation = RADIUS_COMPENSATION_OFF;

// Cycle state placeholders — cps.5 owns the real cycle engine; the
// names exist so posts' `typeof` probes see a coherent kernel.
var cycleType = undefined;
var cycle = undefined;
var cycleExpanded = false;

var __ivacRun = {
  sections: [],
  program: undefined,
  sectionIndex: -1,
  recordIndex: -1,
  errorFlag: false,
  errorMessages: [],
  warnedRapidMachine: false,
  warnedCycles: false,
  recordsSinceAbortCheck: 0,
};

// ---- diagnostics / control ----

function error(message) {
  __ivacRun.errorFlag = true;
  __ivacRun.errorMessages.push(String(message));
  __ivac.diag("error", String(message));
}

function warning(message) {
  __ivac.diag("warning", String(message));
}

function alert(_title, message) {
  __ivac.diag("warning", String(message));
}

function log(message) {
  __ivac.log(String(message));
}

function validate(condition, message) {
  if (!condition) {
    throw new Error(message !== undefined ? String(message) : "validate() failed");
  }
}

function __ivacCheckAbort() {
  __ivacRun.recordsSinceAbortCheck += 1;
  if (__ivacRun.recordsSinceAbortCheck >= 1024) {
    __ivacRun.recordsSinceAbortCheck = 0;
    if (__ivac.abortCheck()) {
      throw new Error("__IVAC_CANCELLED__");
    }
  }
}

// ---- command plumbing ----

var __ivacCommandNames = (function () {
  // Reverse map COMMAND_* value → canonical name, first name wins so
  // aliases can't shadow the primary id.
  var names = [
    "COMMAND_STOP", "COMMAND_OPTIONAL_STOP", "COMMAND_END",
    "COMMAND_SPINDLE_CLOCKWISE", "COMMAND_SPINDLE_COUNTERCLOCKWISE",
    "COMMAND_START_SPINDLE", "COMMAND_STOP_SPINDLE", "COMMAND_ORIENTATE_SPINDLE",
    "COMMAND_LOAD_TOOL", "COMMAND_COOLANT_ON", "COMMAND_COOLANT_OFF",
    "COMMAND_ACTIVATE_SPEED_FEED_SYNCHRONIZATION",
    "COMMAND_DEACTIVATE_SPEED_FEED_SYNCHRONIZATION",
    "COMMAND_LOCK_MULTI_AXIS", "COMMAND_UNLOCK_MULTI_AXIS", "COMMAND_EXACT_STOP",
    "COMMAND_START_CHIP_TRANSPORT", "COMMAND_STOP_CHIP_TRANSPORT",
    "COMMAND_OPEN_DOOR", "COMMAND_CLOSE_DOOR", "COMMAND_BREAK_CONTROL",
    "COMMAND_TOOL_MEASURE", "COMMAND_CALIBRATE", "COMMAND_VERIFY", "COMMAND_CLEAN",
    "COMMAND_ALARM", "COMMAND_ALERT", "COMMAND_CHANGE_PALLET", "COMMAND_POWER_ON",
    "COMMAND_POWER_OFF", "COMMAND_MAIN_CHUCK_OPEN", "COMMAND_MAIN_CHUCK_CLOSE",
    "COMMAND_SECONDARY_CHUCK_OPEN", "COMMAND_SECONDARY_CHUCK_CLOSE",
    "COMMAND_SECONDARY_SPINDLE_SYNCHRONIZATION_ACTIVATE",
    "COMMAND_SECONDARY_SPINDLE_SYNCHRONIZATION_DEACTIVATE",
    "COMMAND_SYNC_CHANNELS", "COMMAND_PROBE_ON", "COMMAND_PROBE_OFF",
  ];
  var map = {};
  for (var i = 0; i < names.length; ++i) {
    map[i] = names[i];
  }
  return map;
})();

function getCommandStringId(command) {
  var name = __ivacCommandNames[command];
  return name === undefined ? "COMMAND_UNKNOWN_" + command : name;
}

// Kernel defaults a post may override with its own declarations.
function onUnsupportedCommand(command) {
  warning("unsupported command: " + getCommandStringId(command));
}
function onImpliedCommand(_command) {}

// ---- section navigation ----

function getNumberOfSections() {
  return __ivacRun.sections.length;
}
function getSection(i) {
  return __ivacRun.sections[i];
}
function getCurrentSectionId() {
  return __ivacRun.sectionIndex;
}
function isFirstSection() {
  return __ivacRun.sectionIndex <= 0;
}
function isLastSection() {
  return __ivacRun.sectionIndex >= __ivacRun.sections.length - 1;
}
function hasNextSection() {
  return __ivacRun.sectionIndex + 1 < __ivacRun.sections.length;
}
function getPreviousSection() {
  return __ivacRun.sections[Math.max(__ivacRun.sectionIndex - 1, 0)];
}
function getNextSection() {
  return __ivacRun.sections[
    Math.min(__ivacRun.sectionIndex + 1, __ivacRun.sections.length - 1)
  ];
}
function getToolTable() {
  return __ivacToolTable(__ivacRun.sections);
}
/** The next DIFFERENT tool used after the current section, if any. */
function getNextTool(number) {
  for (var i = __ivacRun.sectionIndex + 1; i < __ivacRun.sections.length; ++i) {
    var t = __ivacRun.sections[i].getTool();
    if (t.number !== number) {
      return t;
    }
  }
  return undefined;
}

function hasParameter(name) {
  return currentSection !== undefined && currentSection.hasParameter(name);
}
function getParameter(name, defaultValue) {
  return currentSection === undefined
    ? defaultValue
    : currentSection.getParameter(name, defaultValue);
}
function hasGlobalParameter(name) {
  return __ivacRun.globalParams !== undefined && __ivacRun.globalParams[name] !== undefined;
}
function getGlobalParameter(name, defaultValue) {
  var v = __ivacRun.globalParams === undefined ? undefined : __ivacRun.globalParams[name];
  return v === undefined ? defaultValue : v;
}

function isProbeOperation() {
  return tool !== undefined && tool.type === TOOL_PROBE;
}
function isInspectionOperation() {
  return (
    currentSection !== undefined &&
    currentSection.hasParameter("operation-strategy") &&
    currentSection.getParameter("operation-strategy") === "inspectSurface"
  );
}
function isDrillingCycle() {
  return cycleType !== undefined;
}
function writeSectionNotes() {
  if (hasParameter("notes")) {
    writeln(String(getParameter("notes")));
  }
}

// ---- record lookahead ----

function __ivacRecordWrapper(record) {
  var kind = record === undefined ? "" : record.kind;
  return {
    isMotion: function () {
      return (
        kind === "rapid" ||
        kind === "linear" ||
        kind === "circular" ||
        kind === "rapid5d" ||
        kind === "linear5d"
      );
    },
    getType: function () {
      switch (kind) {
        case "linear":
          return RECORD_LINEAR;
        case "rapid":
          return RECORD_LINEAR;
        case "circular":
          return RECORD_CIRCULAR;
        case "dwell":
          return RECORD_DWELL;
        case "cycle":
          return RECORD_CYCLE;
        case "cycleEnd":
          return RECORD_CYCLE_OFF;
        case "comment":
          return RECORD_COMMENT;
        case "passThrough":
          return RECORD_PASS_THROUGH;
        default:
          return RECORD_INVALID;
      }
    },
  };
}

function hasNextRecord() {
  var section = __ivacRun.sections[__ivacRun.sectionIndex];
  return (
    section !== undefined && __ivacRun.recordIndex + 1 < section.__ir.records.length
  );
}
function getNextRecord() {
  var section = __ivacRun.sections[__ivacRun.sectionIndex];
  var record =
    section === undefined
      ? undefined
      : section.__ir.records[__ivacRun.recordIndex + 1];
  return __ivacRecordWrapper(record);
}

// ---- re-dispatch helpers (resolve CURRENT bindings at call time) ----

function __ivacCallOptional(name, args) {
  var fn = this[name] !== undefined ? this[name] : undefined;
  if (typeof fn === "function") {
    fn.apply(undefined, args || []);
    return true;
  }
  return false;
}

function invokeOnRapid(x, y, z) {
  if (typeof onRapid === "function") {
    onRapid(x, y, z);
  }
  setCurrentPosition(new Vector(x, y, z));
}
function invokeOnLinear(x, y, z, feed) {
  if (typeof onLinear === "function") {
    onLinear(x, y, z, feed);
  }
  setCurrentPosition(new Vector(x, y, z));
}
function invokeOnRapid5D(x, y, z, a, b, c) {
  if (typeof onRapid5D === "function") {
    onRapid5D(x, y, z, a, b, c);
  }
  setCurrentPosition(new Vector(x, y, z));
}
function invokeOnLinear5D(x, y, z, a, b, c, feed) {
  if (typeof onLinear5D === "function") {
    onLinear5D(x, y, z, a, b, c, feed);
  }
  setCurrentPosition(new Vector(x, y, z));
}

// Cycle engine placeholders — replaced by the real implementation in
// prelude/12_cycles.js (cps.5).
function expandCyclePoint(_x, _y, _z) {
  error("expandCyclePoint: canned-cycle support is not available yet");
}
function cycleNotSupported() {
  error("Canned cycle is not supported: " + String(cycleType));
}

// ---- the dispatch driver ----

function __ivacExecute(program, overrides) {
  __ivacRun.program = program;
  __ivacRun.errorFlag = false;
  __ivacRun.errorMessages = [];
  __ivacRun.sectionIndex = -1;
  __ivacRun.recordIndex = -1;

  // Config readback happens implicitly (the post's top level already
  // overwrote the globals above). Freeze the unit scale for the
  // Section/Tool wrappers and dispatch scaling.
  __ivacScale = unit === IN ? 1 / 25.4 : 1;

  programName = program.header.programName;
  programComment = program.header.programComment || "";

  var globalParams = {};
  var headerParams = program.header.parameters || [];
  for (var i = 0; i < headerParams.length; ++i) {
    globalParams[headerParams[i].name] = headerParams[i].value;
  }
  __ivacRun.globalParams = globalParams;

  var sections = [];
  for (var s = 0; s < program.sections.length; ++s) {
    sections.push(__ivacSection(program.sections[s], s, program));
  }
  __ivacRun.sections = sections;

  if (overrides) {
    __ivacApplyPropertyOverrides(overrides);
  }

  function halted() {
    return __ivacRun.errorFlag;
  }

  __ivacCallOptional("onMachine");
  if (!halted()) {
    for (var g = 0; g < headerParams.length; ++g) {
      __ivacCallOptional("onParameter", [
        headerParams[g].name,
        headerParams[g].value,
      ]);
      if (halted()) {
        break;
      }
    }
  }
  if (!halted()) {
    __ivacCallOptional("onOpen");
  }

  for (var si = 0; si < sections.length && !halted(); ++si) {
    __ivacRun.sectionIndex = si;
    __ivacRun.recordIndex = -1;
    currentSection = sections[si];
    tool = currentSection.getTool();
    spindleSpeed = currentSection.__ir.spindleRpm;
    movement = MOVEMENT_RAPID;
    radiusCompensation = RADIUS_COMPENSATION_OFF;
    __ivacState.skipSection = false;
    setCurrentPosition(currentSection.getInitialPosition());
    setCurrentDirection(new Vector(0, 0, 1));

    // Parameter stream BEFORE onSection (Autodesk order).
    var params = currentSection.__ir.parameters || [];
    for (var p = 0; p < params.length; ++p) {
      __ivacCallOptional("onParameter", [params[p].name, params[p].value]);
      if (halted()) {
        break;
      }
    }
    if (halted()) {
      break;
    }

    __ivacCallOptional("onSection");

    var records = currentSection.__ir.records;
    for (var r = 0; r < records.length && !halted(); ++r) {
      if (__ivacState.skipSection) {
        break;
      }
      __ivacCheckAbort();
      __ivacRun.recordIndex = r;
      var record = records[r];
      switch (record.kind) {
        case "rapid":
          movement = MOVEMENT_RAPID;
          if (typeof onRapid === "function") {
            onRapid(
              record.x * __ivacScale,
              record.y * __ivacScale,
              record.z * __ivacScale
            );
          }
          setCurrentPosition(
            new Vector(
              record.x * __ivacScale,
              record.y * __ivacScale,
              record.z * __ivacScale
            )
          );
          break;
        case "linear":
          movement = record.movement;
          if (typeof onLinear === "function") {
            onLinear(
              record.x * __ivacScale,
              record.y * __ivacScale,
              record.z * __ivacScale,
              record.feed * __ivacScale
            );
          }
          setCurrentPosition(
            new Vector(
              record.x * __ivacScale,
              record.y * __ivacScale,
              record.z * __ivacScale
            )
          );
          break;
        case "circular":
          movement = MOVEMENT_CUTTING;
          __ivacDispatchCircular(
            record.clockwise,
            new Vector(
              record.center.x * __ivacScale,
              record.center.y * __ivacScale,
              record.center.z * __ivacScale
            ),
            new Vector(
              record.end.x * __ivacScale,
              record.end.y * __ivacScale,
              record.end.z * __ivacScale
            ),
            record.feed * __ivacScale
          );
          break;
        case "rapidMachine":
          if (!__ivacRun.warnedRapidMachine) {
            __ivacRun.warnedRapidMachine = true;
            warning(
              "machine-frame rapids (G53 tool-change staging) have no .cps representation — omitted"
            );
          }
          break;
        case "cycle":
        case "cycleEnd":
          if (!__ivacRun.warnedCycles) {
            __ivacRun.warnedCycles = true;
            warning(
              "canned drill cycles are not dispatched yet (cps.5) — cycle records omitted"
            );
          }
          break;
        case "dwell":
          __ivacCallOptional("onDwell", [record.seconds]);
          break;
        case "command":
          __ivacCallOptional("onCommand", [record.command]);
          break;
        case "spindleSpeed":
          spindleSpeed = record.rpm;
          __ivacCallOptional("onSpindleSpeed", [record.rpm]);
          break;
        case "coolant":
          // No dedicated kernel entry point; posts that care define
          // onCoolant (section-level coolant travels on the tool).
          __ivacCallOptional("onCoolant", [record.mode]);
          break;
        case "comment":
          __ivacCallOptional("onComment", [record.text]);
          break;
        case "passThrough":
          if (!__ivacCallOptional("onPassThrough", [record.text])) {
            writeln(record.text);
          }
          break;
        default:
          break;
      }
    }
    if (halted()) {
      break;
    }
    __ivacCallOptional("onSectionEnd");
    currentSection = undefined;
  }

  if (!halted()) {
    __ivacCallOptional("onClose");
  }
  __ivacCallOptional("onTerminate");

  return {
    ok: !__ivacRun.errorFlag,
    errors: __ivacRun.errorMessages,
    extension: extension,
    unit: unit === IN ? "in" : "mm",
    programName: String(programName),
    description: String(description),
  };
}

/** Top-level metadata extraction for inspect_post (no program run). */
function __ivacInspect() {
  return {
    description: String(description),
    vendor: String(vendor),
    extension: String(extension),
    capabilities: capabilities | 0,
    properties: __ivacDescribeProperties(),
  };
}

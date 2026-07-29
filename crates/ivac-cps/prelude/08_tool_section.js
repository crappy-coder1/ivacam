// 08_tool_section.js — Section / Tool / ToolTable wrappers over the
// injected IR JSON (lazy objects the driver constructs per program).
//
// Field names mirror the Autodesk classes posts read; everything is
// millimetres/radians here — the driver scales values at DISPATCH
// (onRapid/onLinear arguments), while `tool.*`/section getters expose
// unit-scaled values via __ivacScale (set from the post's `unit`).

// Active unit scale: 1 for MM posts, 1/25.4 for IN posts. The driver
// sets it after reading the post's `unit` back.
var __ivacScale = 1;

function __ivacLen(v) {
  return v * __ivacScale;
}

function __ivacTool(toolIr, sectionIr) {
  var t = {
    number: toolIr.number,
    description: toolIr.description || "",
    comment: "",
    vendor: "",
    productId: "",
    type: toolIr.toolType,
    coolant: toolIr.coolant,
    numberOfFlutes: toolIr.flutes,
    taperAngle: toolIr.taperAngle,
    // Offsets default to the tool number — the FANUC convention for
    // simple machines (H<t>/D<t>).
    lengthOffset: toolIr.number,
    diameterOffset: toolIr.number,
    spindleRPM: sectionIr.spindleRpm,
    clockwise: sectionIr.spindleClockwise,
    material: MATERIAL_UNSPECIFIED,
    breakControl: false,
    manualToolChange: false,
    liveTool: true,
    // Body geometry — unknown to ivacam's library; zeros are what
    // Fusion reports for undefined holders too.
    bodyLength: 0,
    holderLength: 0,
    shaftDiameter: 0,
    threadPitch: 0,
  };
  Object.defineProperty(t, "diameter", {
    get: function () {
      return __ivacLen(toolIr.diameter);
    },
  });
  Object.defineProperty(t, "cornerRadius", {
    get: function () {
      return __ivacLen(toolIr.cornerRadius);
    },
  });
  Object.defineProperty(t, "fluteLength", {
    get: function () {
      return 0;
    },
  });
  t.getSpindleRPM = function () {
    return t.spindleRPM;
  };
  return t;
}

function __ivacSection(sectionIr, index, program) {
  var s = {};
  s.__ir = sectionIr;
  s.__index = index;
  s.__program = program;

  var paramMap = {};
  for (var i = 0; i < (sectionIr.parameters || []).length; ++i) {
    paramMap[sectionIr.parameters[i].name] = sectionIr.parameters[i].value;
  }
  s.__params = paramMap;

  s.strategy = sectionIr.strategy;
  s.workOffset = sectionIr.workOffset;
  // The formatted WCS word (FANUC family): 1-6 → G54..G59, beyond →
  // extended G54.1 P<n>. 0 means "not set" — posts see G54.
  var offset = sectionIr.workOffset;
  s.wcs = offset > 6 ? "G54.1 P" + (offset - 6) : "G" + (53 + Math.max(offset, 1));

  var tool = __ivacTool(sectionIr.tool, sectionIr);
  s.getTool = function () {
    return tool;
  };

  var workPlane = Matrix.fromRows(sectionIr.workPlane);
  s.workPlane = workPlane;
  s.getWorkPlane = function () {
    return workPlane;
  };

  s.getId = function () {
    return index;
  };
  s.getParameter = function (name, defaultValue) {
    var value = paramMap[name];
    return value === undefined ? defaultValue : value;
  };
  s.hasParameter = function (name) {
    return paramMap[name] !== undefined;
  };
  s.getInitialPosition = function () {
    var p = sectionIr.initialPosition;
    return new Vector(__ivacLen(p.x), __ivacLen(p.y), __ivacLen(p.z));
  };
  s.getFinalPosition = function () {
    var p = sectionIr.finalPosition;
    return new Vector(__ivacLen(p.x), __ivacLen(p.y), __ivacLen(p.z));
  };
  s.getGlobalInitialToolAxis = function () {
    return workPlane.getForward();
  };
  s.getGlobalFinalToolAxis = function () {
    return workPlane.getForward();
  };
  s.getGlobalZRange = function () {
    var range = new Range();
    range.expandTo(__ivacLen(sectionIr.initialPosition.z));
    for (var i = 0; i < sectionIr.records.length; ++i) {
      var r = sectionIr.records[i];
      if (r.z !== undefined && r.z !== null && r.kind !== "rapidMachine") {
        range.expandTo(__ivacLen(r.z));
      }
      if (r.kind === "cycle") {
        for (var j = 0; j < r.points.length; ++j) {
          range.expandTo(__ivacLen(r.points[j].z));
        }
      }
    }
    return range;
  };
  s.getBoundingBox = function () {
    var box = null;
    for (var i = 0; i < sectionIr.records.length; ++i) {
      var r = sectionIr.records[i];
      if (r.x !== undefined && r.kind !== "rapidMachine") {
        var v = new Vector(__ivacLen(r.x), __ivacLen(r.y), __ivacLen(r.z));
        if (!box) {
          box = new BoundingBox(
            new Vector(v.x, v.y, v.z),
            new Vector(v.x, v.y, v.z)
          );
        } else {
          box.expandTo(v);
        }
      }
    }
    return box || new BoundingBox();
  };
  s.getMovements = function () {
    var mask = 0;
    for (var i = 0; i < sectionIr.records.length; ++i) {
      var r = sectionIr.records[i];
      if (r.kind === "linear") {
        mask |= 1 << r.movement;
      } else if (r.kind === "rapid") {
        mask |= 1 << MOVEMENT_RAPID;
      }
    }
    return mask;
  };
  s.getNumberOfCyclePoints = function () {
    var n = 0;
    for (var i = 0; i < sectionIr.records.length; ++i) {
      if (sectionIr.records[i].kind === "cycle") {
        n += sectionIr.records[i].points.length;
      }
    }
    return n;
  };
  s.hasAnyCycle = function () {
    for (var i = 0; i < sectionIr.records.length; ++i) {
      if (sectionIr.records[i].kind === "cycle") {
        return true;
      }
    }
    return false;
  };
  s.isMultiAxis = function () {
    return false; // v1 IR is 3-axis only
  };
  s.isOptimizedForMachine = function () {
    return false;
  };
  s.isOptional = function () {
    return false;
  };
  s.isPatterned = function () {
    return false;
  };
  s.getPatternId = function () {
    return 0;
  };
  s.getForceToolChange = function () {
    return false;
  };
  s.getInitialToolAxisABC = function () {
    return new Vector(0, 0, 0);
  };
  s.getFinalToolAxisABC = function () {
    return new Vector(0, 0, 0);
  };
  s.getOptimizedTCPMode = function () {
    return 0;
  };
  s.getNextTool = function () {
    return undefined;
  };
  s.optimizeMachineAnglesByMachine = function (_machine, _mode) {
    // 3-axis sections have nothing to optimize (cps.6 wires the real
    // path together with multi-axis records).
  };
  s.getFeedrates = function () {
    return [];
  };
  s.getMaximumFeedrate = function () {
    var max = 0;
    for (var i = 0; i < sectionIr.records.length; ++i) {
      var r = sectionIr.records[i];
      if (r.feed !== undefined && r.feed > max) {
        max = r.feed;
      }
    }
    return __ivacLen(max);
  };
  return s;
}

function __ivacToolTable(sections) {
  var byNumber = {};
  var order = [];
  for (var i = 0; i < sections.length; ++i) {
    var tool = sections[i].getTool();
    if (!byNumber[tool.number]) {
      byNumber[tool.number] = tool;
      order.push(tool);
    }
  }
  return {
    getNumberOfTools: function () {
      return order.length;
    },
    getTool: function (index) {
      return order[index];
    },
  };
}

/** Display name for a TOOL_* code (used in FANUC tool-list comments). */
function getToolTypeName(toolType) {
  var names = {};
  names[TOOL_UNSPECIFIED] = "unspecified";
  names[TOOL_DRILL] = "drill";
  names[TOOL_DRILL_CENTER] = "center drill";
  names[TOOL_DRILL_SPOT] = "spot drill";
  names[TOOL_DRILL_BLOCK] = "drill block";
  names[TOOL_MILLING_END_FLAT] = "flat end mill";
  names[TOOL_MILLING_END_BALL] = "ball end mill";
  names[TOOL_MILLING_END_BULLNOSE] = "bullnose end mill";
  names[TOOL_MILLING_CHAMFER] = "chamfer mill";
  names[TOOL_MILLING_FACE] = "face mill";
  names[TOOL_MILLING_SLOT] = "slot mill";
  names[TOOL_MILLING_RADIUS] = "radius mill";
  names[TOOL_MILLING_DOVETAIL] = "dovetail mill";
  names[TOOL_MILLING_TAPERED] = "tapered mill";
  names[TOOL_MILLING_LOLLIPOP] = "lollipop mill";
  names[TOOL_MILLING_FORM] = "form mill";
  names[TOOL_MILLING_THREAD] = "thread mill";
  names[TOOL_TAP_RIGHT_HAND] = "right hand tap";
  names[TOOL_TAP_LEFT_HAND] = "left hand tap";
  names[TOOL_REAMER] = "reamer";
  names[TOOL_BORING_BAR] = "boring bar";
  names[TOOL_COUNTER_BORE] = "counter bore";
  names[TOOL_COUNTER_SINK] = "counter sink";
  names[TOOL_HOLDER_ONLY] = "holder";
  names[TOOL_PROBE] = "probe";
  names[TOOL_WIRE] = "wire";
  names[TOOL_WATER_JET] = "water jet";
  names[TOOL_LASER_CUTTER] = "laser cutter";
  names[TOOL_PLASMA_CUTTER] = "plasma cutter";
  names[TOOL_WELDER] = "welder";
  names[TOOL_GRINDER] = "grinder";
  names[TOOL_MARKER] = "marker";
  var name = names[toolType];
  return name === undefined ? "unspecified" : name;
}

/**
  ivaCAM bundled post: GRBL / grblHAL (metric).

  Written from scratch for the ivaCAM .cps runtime — NOT derived from
  any Autodesk post. GPL-3.0-or-later, same as the ivaCAM sources.

  GRBL notes: no tool changer (tool changes pause with M0 by default),
  no canned drill cycles (cycles expand to G0/G1), no tool-length
  offsets (G43 is not emitted), arcs in the XY plane with IJ centers.
*/

description = "GRBL (ivaCAM)";
vendor = "ivaCAM";
vendorUrl = "https://github.com/aalarchiv/ivacam";
legal = "GPL-3.0-or-later";
extension = "gcode";
capabilities = CAPABILITY_MILLING;

tolerance = spatial(0.01, MM);
minimumChordLength = spatial(0.25, MM);
minimumCircularRadius = spatial(0.01, MM);
maximumCircularRadius = spatial(1000, MM);
minimumCircularSweep = toRad(0.01);
maximumCircularSweep = toRad(180);
allowHelicalMoves = true;
allowedCircularPlanes = 1 << PLANE_XY; // GRBL arcs stay in G17 here

properties = {
  pauseOnToolChange: {
    title      : "Pause on tool change",
    description: "Emit M0 so the operator can swap the bit and press cycle start.",
    type       : "boolean",
    value      : true,
    scope      : "post"
  },
  useM30: {
    title      : "End with M30",
    description: "End the program with M30 instead of M2.",
    type       : "boolean",
    value      : true,
    scope      : "post"
  },
  spindleWarmupSeconds: {
    title      : "Spindle warm-up (s)",
    description: "Dwell after starting the spindle so it reaches speed.",
    type       : "number",
    value      : 0,
    scope      : "post"
  }
};

var gFormat = createFormat({prefix:"G", decimals:1});
var mFormat = createFormat({prefix:"M", decimals:0});
var xyzFormat = createFormat({decimals:3, forceDecimal:true});
var feedFormat = createFormat({decimals:0});
var rpmFormat = createFormat({decimals:0});
var secFormat = createFormat({decimals:3});

var xOutput = createVariable({prefix:"X"}, xyzFormat);
var yOutput = createVariable({prefix:"Y"}, xyzFormat);
var zOutput = createVariable({prefix:"Z"}, xyzFormat);
var feedOutput = createVariable({prefix:"F"}, feedFormat);
var sOutput = createVariable({prefix:"S", force:true}, rpmFormat);
var iOutput = createReferenceVariable({prefix:"I"}, xyzFormat);
var jOutput = createReferenceVariable({prefix:"J"}, xyzFormat);

var gMotionModal = createModal({}, gFormat); // G0-G3
var gUnitModal = createModal({}, gFormat); // G20-21
var gAbsIncModal = createModal({}, gFormat); // G90-91
var gFeedModeModal = createModal({}, gFormat); // G93-94
var gPlaneModal = createModal({}, gFormat); // G17-19

var permittedCommentChars = " abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.,=_-:#";

function writeBlock() {
  writeWords(arguments);
}

function writeComment(text) {
  writeln("(" + filterText(String(text), permittedCommentChars) + ")");
}

function forceXYZ() {
  xOutput.reset();
  yOutput.reset();
  zOutput.reset();
}

function onOpen() {
  if (programComment) {
    writeComment(programComment);
  }
  writeBlock(gUnitModal.format(unit == IN ? 20 : 21));
  writeBlock(gAbsIncModal.format(90), gFeedModeModal.format(94), gPlaneModal.format(17));
}

function onComment(message) {
  writeComment(message);
}

var pendingToolNumber;

function onSection() {
  var insertToolCall =
    isFirstSection() || tool.number != getPreviousSection().getTool().number;

  if (hasParameter("operation-comment")) {
    writeComment(getParameter("operation-comment"));
  }

  if (insertToolCall) {
    writeBlock(gMotionModal.format(0), zOutput.format(currentSection.getInitialPosition().z));
    if (!isFirstSection()) {
      writeBlock(mFormat.format(5));
    }
    writeComment("tool " + tool.number + ": " + getToolTypeName(tool.type) + " D=" + xyzFormat.format(tool.diameter));
    if (getProperty("pauseOnToolChange") && !isFirstSection()) {
      writeBlock(mFormat.format(0));
    }
  }

  var start = currentSection.getInitialPosition();
  writeBlock(sOutput.format(spindleSpeed), mFormat.format(tool.clockwise ? 3 : 4));
  var warmup = getProperty("spindleWarmupSeconds");
  if (warmup > 0) {
    onDwell(warmup);
  }
  if (tool.coolant == COOLANT_FLOOD) {
    writeBlock(mFormat.format(8));
  } else if (tool.coolant == COOLANT_MIST) {
    writeBlock(mFormat.format(7));
  }
  forceXYZ();
  writeBlock(gMotionModal.format(0), xOutput.format(start.x), yOutput.format(start.y));
  writeBlock(gMotionModal.format(0), zOutput.format(start.z));
}

// Mid-section coolant change records.
function onCoolant(mode) {
  if (mode == COOLANT_FLOOD) {
    writeBlock(mFormat.format(8));
  } else if (mode == COOLANT_MIST) {
    writeBlock(mFormat.format(7));
  } else {
    writeBlock(mFormat.format(9));
  }
}

function onDwell(seconds) {
  writeBlock(gFormat.format(4), "P" + secFormat.format(seconds));
}

function onSpindleSpeed(rpm) {
  writeBlock(sOutput.format(rpm));
}

function onRapid(x, y, z) {
  var xw = xOutput.format(x);
  var yw = yOutput.format(y);
  var zw = zOutput.format(z);
  if (xw || yw || zw) {
    writeBlock(gMotionModal.format(0), xw, yw, zw);
    feedOutput.reset();
  }
}

function onLinear(x, y, z, feed) {
  var xw = xOutput.format(x);
  var yw = yOutput.format(y);
  var zw = zOutput.format(z);
  var fw = feedOutput.format(feed);
  if (xw || yw || zw) {
    writeBlock(gMotionModal.format(1), xw, yw, zw, fw);
  } else if (fw) {
    writeBlock(gMotionModal.format(1), fw);
  }
}

function onCircular(clockwise, cx, cy, cz, x, y, z, feed) {
  if (getCircularPlane() != PLANE_XY) {
    linearize(tolerance);
    return;
  }
  var start = getCurrentPosition();
  if (isFullCircle()) {
    writeBlock(
      gMotionModal.format(clockwise ? 2 : 3),
      iOutput.format(cx - start.x, 0),
      jOutput.format(cy - start.y, 0),
      feedOutput.format(feed)
    );
  } else {
    writeBlock(
      gMotionModal.format(clockwise ? 2 : 3),
      xOutput.format(x),
      yOutput.format(y),
      zOutput.format(z),
      iOutput.format(cx - start.x, 0),
      jOutput.format(cy - start.y, 0),
      feedOutput.format(feed)
    );
  }
}

// GRBL has no canned cycles — expand every cycle point.
function onCyclePoint(x, y, z) {
  expandCyclePoint(x, y, z);
}

function onCycleEnd() {
  feedOutput.reset();
}

function onCommand(command) {
  switch (command) {
  case COMMAND_STOP:
    writeBlock(mFormat.format(0));
    return;
  case COMMAND_OPTIONAL_STOP:
    writeBlock(mFormat.format(1));
    return;
  case COMMAND_SPINDLE_CLOCKWISE:
    writeBlock(sOutput.format(spindleSpeed), mFormat.format(3));
    return;
  case COMMAND_SPINDLE_COUNTERCLOCKWISE:
    writeBlock(sOutput.format(spindleSpeed), mFormat.format(4));
    return;
  case COMMAND_START_SPINDLE:
    onCommand(tool.clockwise ? COMMAND_SPINDLE_CLOCKWISE : COMMAND_SPINDLE_COUNTERCLOCKWISE);
    return;
  case COMMAND_STOP_SPINDLE:
    writeBlock(mFormat.format(5));
    return;
  case COMMAND_COOLANT_ON:
    writeBlock(mFormat.format(8));
    return;
  case COMMAND_COOLANT_OFF:
    writeBlock(mFormat.format(9));
    return;
  default:
    // Everything else has no GRBL representation.
    return;
  }
}

function onSectionEnd() {
  forceXYZ();
  feedOutput.reset();
}

function onClose() {
  writeBlock(mFormat.format(5));
  writeBlock(mFormat.format(9));
  writeBlock(mFormat.format(getProperty("useM30") ? 30 : 2));
}

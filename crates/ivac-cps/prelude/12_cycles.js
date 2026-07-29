// 12_cycles.js — canned-cycle state, dispatch, and the kernel
// expansion library (`expandCyclePoint`).
//
// Contract (FANUC oracle): one Cycle record becomes onCycle() → one
// onCyclePoint(x,y,z) per hole → onCycleEnd(). `cycle` holds the
// CycleParameters bag (post units), `cycleType` the Autodesk name.
// A post may bail into `expandCyclePoint(x, y, z)` at any point — the
// kernel then replays the cycle's motion through the post's CURRENT
// onRapid/onLinear/onDwell/onCommand bindings (host → guest → host
// re-entrancy), and `cycleExpanded` stays true for the cycle's
// remaining points.

var __ivacCycle = {
  pointIndex: -1,
  pointCount: 0,
};

function isFirstCyclePoint() {
  return __ivacCycle.pointIndex === 0;
}
function isLastCyclePoint() {
  return __ivacCycle.pointIndex === __ivacCycle.pointCount - 1;
}
function getNumberOfCyclePoints() {
  return __ivacCycle.pointCount;
}
function isExpanding() {
  return !!cycleExpanded;
}

var __ivacWellKnownCycles = [
  "drilling",
  "counter-boring",
  "chip-breaking",
  "deep-drilling",
  "gun-drilling",
  "tapping",
  "left-tapping",
  "right-tapping",
  "tapping-with-chip-breaking",
  "left-tapping-with-chip-breaking",
  "right-tapping-with-chip-breaking",
  "reaming",
  "boring",
  "stop-boring",
  "fine-boring",
  "back-boring",
  "bore-milling",
  "thread-milling",
  "circular-pocket-milling",
];

function isWellKnownCycle() {
  return __ivacWellKnownCycles.indexOf(cycleType) >= 0;
}

/** Reposition to the cycle clearance plane before the first point —
 * the kernel helper posts call at cycle entry. */
function repositionToCycleClearance(_cycle, _x, _y, _z) {
  var position = getCurrentPosition();
  if (position.z < _cycle.clearance - 1e-9) {
    invokeOnRapid(position.x, position.y, _cycle.clearance);
  }
}

/** Kernel-side expansion of the current cycle at one point. Motion is
 * replayed through the post's CURRENT bindings; `cycleExpanded` stays
 * set for the rest of the cycle. */
function expandCyclePoint(x, y, z) {
  cycleExpanded = true;
  var c = cycle;
  var clearance = c.clearance !== undefined ? c.clearance : c.retract;
  var retract = c.retract !== undefined ? c.retract : clearance;
  var bottom = z;
  var feed = c.feedrate;
  var retractFeed = c.retractFeedrate;

  function dwellIfSet(seconds) {
    if (seconds && seconds > 0) {
      if (typeof onDwell === "function") {
        onDwell(seconds);
      }
    }
  }

  // Entry: rapid over the hole at clearance, then down to the retract
  // plane when it sits below.
  invokeOnRapid(x, y, clearance);
  if (retract < clearance - 1e-9) {
    invokeOnRapid(x, y, retract);
  }

  switch (cycleType) {
    case "drilling":
      invokeOnLinear(x, y, bottom, feed);
      invokeOnRapid(x, y, clearance);
      break;
    case "counter-boring":
      invokeOnLinear(x, y, bottom, feed);
      dwellIfSet(c.dwell);
      invokeOnRapid(x, y, clearance);
      break;
    case "reaming":
      invokeOnLinear(x, y, bottom, feed);
      dwellIfSet(c.dwell);
      invokeOnLinear(x, y, retract, retractFeed || feed);
      invokeOnRapid(x, y, clearance);
      break;
    case "boring":
      invokeOnLinear(x, y, bottom, feed);
      dwellIfSet(c.dwell);
      invokeOnLinear(x, y, retract, retractFeed || feed);
      invokeOnRapid(x, y, clearance);
      break;
    case "stop-boring":
      invokeOnLinear(x, y, bottom, feed);
      dwellIfSet(c.dwell);
      if (typeof onCommand === "function") {
        onCommand(COMMAND_STOP_SPINDLE);
      }
      invokeOnRapid(x, y, retract);
      if (typeof onCommand === "function") {
        onCommand(COMMAND_START_SPINDLE);
      }
      invokeOnRapid(x, y, clearance);
      break;
    case "deep-drilling":
    case "chip-breaking": {
      var q = c.incrementalDepth !== undefined ? Math.abs(c.incrementalDepth) : 0;
      if (q < 1e-9) {
        q = Math.abs(retract - bottom);
      }
      var reduction = c.incrementalDepthReduction || 0;
      var minimumQ = c.minimumIncrementalDepth || 0;
      var breakDistance = c.chipBreakDistance !== undefined ? c.chipBreakDistance : 0.5;
      var accumulated =
        c.accumulatedDepth !== undefined && c.accumulatedDepth > 0
          ? c.accumulatedDepth
          : Number.POSITIVE_INFINITY;
      var fullRetractEvery = cycleType === "deep-drilling" ? 0 : accumulated;
      var current = retract;
      var sinceFullRetract = 0;
      var step = q;
      for (;;) {
        var next = Math.max(current - step, bottom);
        invokeOnLinear(x, y, next, feed);
        dwellIfSet(c.dwell);
        var cut = current - next;
        current = next;
        sinceFullRetract += cut;
        if (current <= bottom + 1e-9) {
          break;
        }
        var fullRetract =
          cycleType === "deep-drilling" || sinceFullRetract >= fullRetractEvery - 1e-9;
        if (fullRetract) {
          invokeOnRapid(x, y, retract);
          var reEntry = Math.min(current + 0.5, retract);
          invokeOnRapid(x, y, reEntry);
          invokeOnLinear(x, y, current, feed);
          sinceFullRetract = 0;
        } else {
          var breakZ = Math.min(current + breakDistance, retract);
          invokeOnRapid(x, y, breakZ);
          invokeOnLinear(x, y, current, feed);
        }
        if (reduction > 0) {
          step -= reduction;
          var floor = minimumQ > 0 ? minimumQ : 1e-3;
          if (step < floor) {
            step = floor;
          }
        }
      }
      invokeOnRapid(x, y, clearance);
      break;
    }
    case "tapping":
    case "right-tapping":
    case "left-tapping": {
      // Non-rigid expansion: feed in, reverse, feed out, restore. The
      // feed follows pitch × rpm when the cycle didn't carry one.
      var left = cycleType === "left-tapping" || (tool && tool.type === TOOL_TAP_LEFT_HAND);
      var tapFeed = feed;
      invokeOnLinear(x, y, bottom, tapFeed);
      dwellIfSet(c.dwell);
      if (typeof onCommand === "function") {
        onCommand(left ? COMMAND_SPINDLE_CLOCKWISE : COMMAND_SPINDLE_COUNTERCLOCKWISE);
      }
      invokeOnLinear(x, y, retract, tapFeed);
      if (typeof onCommand === "function") {
        onCommand(left ? COMMAND_SPINDLE_COUNTERCLOCKWISE : COMMAND_SPINDLE_CLOCKWISE);
      }
      invokeOnRapid(x, y, clearance);
      break;
    }
    default:
      cycleNotSupported();
      return;
  }
}

/** Kernel error for a cycle the post cannot output nor expand. */
function cycleNotSupported() {
  error(subst(localize("Canned cycle is not supported: %1"), String(cycleType)));
}

/** Driver-side dispatch of one IR Cycle record (values pre-scaled to
 * the post's unit). */
function __ivacDispatchCycle(cycleTypeName, params, points) {
  cycleType = cycleTypeName;
  cycle = params;
  cycleExpanded = false;
  __ivacCycle.pointCount = points.length;
  __ivacCycle.pointIndex = -1;

  if (typeof onCycle === "function") {
    onCycle();
  }
  for (var i = 0; i < points.length; ++i) {
    __ivacCycle.pointIndex = i;
    var p = points[i];
    if (typeof onCyclePoint === "function") {
      onCyclePoint(p.x, p.y, p.z);
    } else {
      expandCyclePoint(p.x, p.y, p.z);
    }
    if (__ivacRun.errorFlag) {
      return;
    }
    // Kernel position after a canned point: over the hole at the
    // clearance plane (G98 return-to-initial semantics).
    setCurrentPosition(
      new Vector(p.x, p.y, cycle.clearance !== undefined ? cycle.clearance : p.z)
    );
  }
  __ivacCycle.pointIndex = -1;
}

/** Driver-side dispatch of an IR CycleEnd record. */
function __ivacDispatchCycleEnd() {
  if (cycleType === undefined) {
    return;
  }
  if (typeof onCycleEnd === "function") {
    onCycleEnd();
  }
  cycleType = undefined;
  cycle = undefined;
  cycleExpanded = false;
  __ivacCycle.pointCount = 0;
  __ivacCycle.pointIndex = -1;
}

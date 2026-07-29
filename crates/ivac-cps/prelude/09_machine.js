// 09_machine.js — MachineConfiguration / Axis and the rotary kinematics.
//
// Posts run their machine-activation code on EVERY program, 3-axis
// included (the FANUC activateMachine() does), so this surface has to
// be real even when ivacam records no rotary motion. The kinematics
// half below (getABC / getOrientation / remapABC / remapToABC /
// getPreferredABC / isABCSupported, plus Axis.remapToRange2) is the
// full multi-axis API: it solves real wrist configurations, so a post's
// setWorkPlane / getWorkPlaneMachineABC logic exercises genuine math
// rather than a stub.

function Axis(spec) {
  this.__enabled = false;
  this.__coordinate = 0;
  this.__axis = new Vector(0, 0, 1);
  this.__table = true;
  this.__cyclic = false;
  this.__range = undefined;
  this.__preference = 0;
  this.__tcp = false;
  this.__resolution = 0;
  this.__offset = new Vector(0, 0, 0);
  if (spec) {
    this.__enabled = true;
    if (spec.coordinate !== undefined) {
      this.__coordinate = spec.coordinate;
    }
    if (spec.axis !== undefined) {
      this.__axis = new Vector(spec.axis[0], spec.axis[1], spec.axis[2]);
    }
    if (spec.table !== undefined) {
      this.__table = !!spec.table;
    }
    if (spec.cyclic !== undefined) {
      this.__cyclic = !!spec.cyclic;
    }
    if (spec.range !== undefined) {
      this.__range = new Range(toRad(spec.range[0]), toRad(spec.range[1]));
    }
    if (spec.preference !== undefined) {
      this.__preference = spec.preference;
    }
    if (spec.tcp !== undefined) {
      this.__tcp = !!spec.tcp;
    }
    if (spec.resolution !== undefined) {
      this.__resolution = spec.resolution;
    }
  }
}
Axis.prototype.isEnabled = function () {
  return this.__enabled;
};
Axis.prototype.isTable = function () {
  return this.__table;
};
Axis.prototype.isHead = function () {
  return !this.__table;
};
Axis.prototype.isTCPEnabled = function () {
  return this.__tcp;
};
Axis.prototype.isCyclic = function () {
  return this.__cyclic;
};
Axis.prototype.getAxis = function () {
  return new Vector(this.__axis.x, this.__axis.y, this.__axis.z);
};
Axis.prototype.getCoordinate = function () {
  return this.__coordinate;
};
Axis.prototype.getRange = function () {
  return this.__range === undefined ? new Range() : this.__range;
};
Axis.prototype.getPreference = function () {
  return this.__preference;
};
Axis.prototype.getResolution = function () {
  return this.__resolution;
};
Axis.prototype.getOffset = function () {
  return this.__offset;
};

function createAxis(spec) {
  return new Axis(spec);
}

function MachineConfiguration() {
  this.__axes = [];
  for (var i = 0; i < arguments.length; ++i) {
    this.__axes.push(arguments[i]);
  }
  this.__vendor = "";
  this.__model = "";
  this.__description = "";
  this.__received = false;
  this.__homeX = undefined;
  this.__homeY = undefined;
  this.__retractPlane = undefined;
  this.__virtualTooltip = false;
  this.__toolLength = 0;
  this.__rewinds = false;
}

MachineConfiguration.prototype.getAxisU = function () {
  return this.__axes[0] || new Axis();
};
MachineConfiguration.prototype.getAxisV = function () {
  return this.__axes[1] || new Axis();
};
MachineConfiguration.prototype.getAxisW = function () {
  return this.__axes[2] || new Axis();
};
MachineConfiguration.prototype.getNumberOfAxes = function () {
  var n = 3;
  for (var i = 0; i < this.__axes.length; ++i) {
    if (this.__axes[i].isEnabled()) {
      n += 1;
    }
  }
  return n;
};
MachineConfiguration.prototype.isMultiAxisConfiguration = function () {
  return this.getNumberOfAxes() > 3;
};
MachineConfiguration.prototype.isHeadConfiguration = function () {
  for (var i = 0; i < this.__axes.length; ++i) {
    if (this.__axes[i].isEnabled() && this.__axes[i].isHead()) {
      return true;
    }
  }
  return false;
};
/** True if rotary coordinate i (0=A,1=B,2=C) exists on this machine. */
MachineConfiguration.prototype.isMachineCoordinate = function (coordinate) {
  for (var i = 0; i < this.__axes.length; ++i) {
    if (this.__axes[i].isEnabled() && this.__axes[i].getCoordinate() === coordinate) {
      return true;
    }
  }
  return false;
};
MachineConfiguration.prototype.getVendor = function () {
  return this.__vendor;
};
MachineConfiguration.prototype.setVendor = function (vendor) {
  this.__vendor = vendor;
};
MachineConfiguration.prototype.getModel = function () {
  return this.__model;
};
MachineConfiguration.prototype.setModel = function (model) {
  this.__model = model;
};
MachineConfiguration.prototype.getDescription = function () {
  return this.__description;
};
MachineConfiguration.prototype.setDescription = function (description) {
  this.__description = description;
};
MachineConfiguration.prototype.isReceived = function () {
  return this.__received;
};
MachineConfiguration.prototype.performRewinds = function () {
  return this.__rewinds;
};
MachineConfiguration.prototype.enableMachineRewinds = function () {
  this.__rewinds = true;
};
MachineConfiguration.prototype.setVirtualTooltip = function (v) {
  this.__virtualTooltip = !!v;
};
MachineConfiguration.prototype.setToolLength = function (length) {
  this.__toolLength = length;
};
MachineConfiguration.prototype.setRewindStockExpansion = function (_expansion) {};
MachineConfiguration.prototype.setHomePositionX = function (x) {
  this.__homeX = x;
};
MachineConfiguration.prototype.setHomePositionY = function (y) {
  this.__homeY = y;
};
MachineConfiguration.prototype.hasHomePositionX = function () {
  return this.__homeX !== undefined;
};
MachineConfiguration.prototype.hasHomePositionY = function () {
  return this.__homeY !== undefined;
};
MachineConfiguration.prototype.getHomePositionX = function () {
  return this.__homeX === undefined ? 0 : this.__homeX;
};
MachineConfiguration.prototype.getHomePositionY = function () {
  return this.__homeY === undefined ? 0 : this.__homeY;
};
MachineConfiguration.prototype.setRetractPlane = function (z) {
  this.__retractPlane = z;
};
MachineConfiguration.prototype.getRetractPlane = function () {
  return this.__retractPlane === undefined ? 0 : this.__retractPlane;
};
MachineConfiguration.prototype.hasRetractPlane = function () {
  return this.__retractPlane !== undefined;
};
/** Multi-axis feed configuration. `__ivacMultiAxisFeed` (below) turns
 * it into the F word for a 5D move. */
MachineConfiguration.prototype.setMultiAxisFeedrate = function (
  mode,
  maximum,
  type,
  tolerance,
  ratio
) {
  this.__multiAxisFeedrate = {
    mode: mode,
    maximum: maximum,
    type: type,
    tolerance: tolerance,
    ratio: ratio,
  };
};

// Global machine state. DELIBERATELY only declared here and
// instantiated in 15_driver.js: boa 0.21.1 mis-evaluates a `new C()`
// executed during the top-level run of the SAME script that assigned
// `C.prototype.m = function () {...}` members (the new-expression
// yields the last assigned method instead of the instance; see the
// `boa_new_after_prototype_assignment_bug` spike test). Instantiating
// from a later prelude file sidesteps it.
var machineConfiguration;
var __ivacReceivedMachine = false;

function setMachineConfiguration(configuration) {
  machineConfiguration = configuration;
}
function getMachineConfiguration() {
  return machineConfiguration;
}

// ---- kinematics ----
//
// Rotary solution for a two-axis (or single-axis) wrist: find the axis
// angles that bring the machine's spindle direction onto the work
// plane's tool axis.
//
// Frames: a TABLE axis rotates the PART, so the orientation the part
// sees is the inverse of the axis rotation — tables therefore
// contribute their transpose to the composed orientation, heads their
// direct rotation. Candidate angles are always VERIFIED against
// `getOrientation`, so a new axis layout needs no new algebra and a
// wrong candidate can never escape the solver.

/// Rotation matrix of one axis at `angle` about its own direction.
function __ivacAxisRotation(axis, angle) {
  return Matrix.getAxisRotation(axis.getAxis(), angle);
}

/// Compose the machine orientation for an (A, B, C) triple, in axis
/// order U → V → W.
/// HEAD axes carry the tool, so their rotations stack in axis order.
/// TABLE axes rotate the PART: what the tool sees is the INVERSE of the
/// part's rotation, and inverting a product reverses it — so table
/// transposes multiply in REVERSE axis order. Getting this backwards
/// makes the outer table axis unable to influence the tool direction at
/// all (the classic AC-table symptom).
MachineConfiguration.prototype.getOrientation = function (abc) {
  var heads = Matrix.getIdentity();
  var tables = Matrix.getIdentity();
  for (var i = 0; i < this.__axes.length; ++i) {
    var axis = this.__axes[i];
    if (!axis.isEnabled()) {
      continue;
    }
    var rotation = __ivacAxisRotation(axis, abc.getCoordinate(axis.getCoordinate()));
    if (axis.isTable()) {
      // (Ru·Rv)^T = Rv^T·Ru^T — prepend to reverse the order.
      tables = rotation.getTransposed().multiply(tables);
    } else {
      heads = heads.multiply(rotation);
    }
  }
  // Returned in the ROW convention the rest of the API speaks: a
  // matrix's `forward` ROW is its tool axis (that's what posts compare
  // against `workPlane.forward`). The composition above builds the
  // rotation column-wise, so transpose on the way out.
  return tables.multiply(heads).getTransposed();
};

/// Tool direction the machine reaches at `abc` — the orientation's
/// `forward` row, matching how posts read `workPlane.forward`.
MachineConfiguration.prototype.getDirection = function (abc) {
  return this.getOrientation(abc).getForward();
};

/// What orientation remains once the rotaries have taken `abc` out of
/// the requested work plane `m`: R = O(abc)⁻¹ · m. Non-TCP posts feed
/// this to setRotation.
MachineConfiguration.prototype.getRemainingOrientation = function (abc, m) {
  // In the row convention, R = O⁻¹·m comes out as m·Oᵀ — and its
  // `forward` row is then Oᵀ·(m's tool axis), which is ẑ exactly when
  // the machine can reach that axis. That is the property a post
  // depends on when it hands the result to setRotation.
  return m.multiply(this.getOrientation(abc).getTransposed());
};

/// Solve for the axis angles orienting the spindle along
/// `orientation.forward`.
///
/// Two rotaries give a ±tilt solution pair; both are generated and
/// picked by (1) axis preference, (2) range validity, (3) least tilt.
/// One rotary reduces to a single angle about its axis. An unreachable
/// orientation throws — the FANUC post catches that and reports
/// "Machine angles not supported".
MachineConfiguration.prototype.getABC = function (orientation) {
  var target = orientation.getForward().getNormalized();
  var enabled = __ivacEnabledAxes(this);
  if (enabled.length === 0) {
    // 3-axis: only +Z is reachable, and it needs no rotation.
    if (!isSameDirection(target, new Vector(0, 0, 1))) {
      throw new Error("machine cannot reach orientation");
    }
    return new Vector(0, 0, 0);
  }

  var candidates = __ivacSolveABC(this, enabled, target);
  if (candidates.length === 0) {
    throw new Error("machine cannot reach orientation");
  }
  var best;
  var bestScore;
  for (var c = 0; c < candidates.length; ++c) {
    var abc = candidates[c];
    var score = [
      __ivacPreferenceScore(enabled, abc),
      __ivacInRange(enabled, abc) ? 0 : 1,
      Math.abs(abc.x) + Math.abs(abc.y) + Math.abs(abc.z),
    ];
    if (
      bestScore === undefined ||
      score[0] < bestScore[0] ||
      (score[0] === bestScore[0] &&
        (score[1] < bestScore[1] ||
          (score[1] === bestScore[1] && score[2] < bestScore[2] - 1e-12)))
    ) {
      best = abc;
      bestScore = score;
    }
  }
  return best;
};

function __ivacEnabledAxes(machine) {
  var out = [];
  for (var i = 0; i < machine.__axes.length; ++i) {
    if (machine.__axes[i].isEnabled()) {
      out.push(machine.__axes[i]);
    }
  }
  return out;
}

/// Candidate angle sets whose composed orientation reproduces `target`.
function __ivacSolveABC(machine, enabled, target) {
  var out = [];
  var TWO_PI = Math.PI * 2;

  function accept(abc) {
    if (isSameDirection(machine.getDirection(abc), target)) {
      out.push(abc);
    }
  }

  if (enabled.length === 1) {
    var axis = enabled[0];
    var single = __ivacAngleCandidates(machine, axis, target);
    for (var i = 0; i < single.length; ++i) {
      var one = new Vector(0, 0, 0);
      one.setCoordinate(axis.getCoordinate(), single[i]);
      accept(one);
    }
    return out;
  }

  // Two rotaries. Which axis is the OUTER one (applied last to the tool
  // vector, hence the one whose dot product with the direction is
  // invariant) depends on the mount: tables compose reversed, so the
  // second axis acts last, while heads stack in order, so the first
  // does. Solve BOTH assignments and let verification keep whichever
  // produced real solutions — no per-layout case analysis, and a wrong
  // guess simply yields nothing.
  __ivacSolvePair(machine, enabled[0], enabled[1], target, accept);
  __ivacSolvePair(machine, enabled[1], enabled[0], target, accept);
  return out;
}

/// Solve a two-axis wrist for one (inner, outer) assignment, feeding
/// every verified candidate to `accept`.
function __ivacSolvePair(machine, tilt, swing, target, accept) {
  var TWO_PI = Math.PI * 2;
  var tilts = __ivacInnerAngleCandidates(machine, tilt, swing, target);
  for (var t = 0; t < tilts.length; ++t) {
    var abc = new Vector(0, 0, 0);
    abc.setCoordinate(tilt.getCoordinate(), tilts[t]);
    var magnitude = __ivacSolveSwing(machine, swing, abc, target);
    // A table axis rotates the PART, so the tool sweeps the OPPOSITE
    // way, and a wrist can also reach a direction from the far side.
    // Rather than case-splitting on mount type and axis handedness,
    // offer all four and let `accept`'s verification decide — the check
    // is exact, so a wrong sign simply never survives.
    var branches = [magnitude, -magnitude, magnitude + Math.PI, -magnitude + Math.PI];
    for (var b = 0; b < branches.length; ++b) {
      var angle = branches[b];
      while (angle > Math.PI) {
        angle -= TWO_PI;
      }
      while (angle <= -Math.PI) {
        angle += TWO_PI;
      }
      var candidate = new Vector(abc.x, abc.y, abc.z);
      candidate.setCoordinate(swing.getCoordinate(), angle);
      accept(candidate);
    }
  }
}

/// Candidate angles for the INNER axis of a two-axis wrist.
///
/// The outer axis rotates about its own direction, so the dot product
/// between the tool direction and that direction is INVARIANT under it.
/// Solving the inner angle therefore means solving
///
///     dot(D(a), u_outer) = dot(target, u_outer)
///
/// where `D(a)` is the direction the machine reaches with only the
/// inner axis moved. `D(a)·u` traces a circle, so it has the form
/// `P + Q·cos a + S·sin a`; sampling f at 0, π/2 and π recovers P/Q/S
/// exactly, and the equation then solves in closed form with the usual
/// ± pair. Sampling (rather than per-layout algebra) keeps this correct
/// for ANY axis directions and for head or table mounts alike.
function __ivacInnerAngleCandidates(machine, inner, outer, target) {
  var u = outer.getAxis().getNormalized();
  var wanted = Vector.dot(target, u);

  function f(angle) {
    var probe = new Vector(0, 0, 0);
    probe.setCoordinate(inner.getCoordinate(), angle);
    return Vector.dot(machine.getDirection(probe), u);
  }

  var f0 = f(0);
  var fq = f(Math.PI / 2);
  var fp = f(Math.PI);
  var p = (f0 + fp) / 2;
  var q = (f0 - fp) / 2;
  var sTerm = fq - p;
  var amplitude = Math.sqrt(q * q + sTerm * sTerm);
  if (amplitude < 1e-12) {
    // The inner axis cannot change this dot product (its axis is
    // parallel to the outer one): any angle is as good, so offer the
    // canonical pair and let verification decide.
    return [0, Math.PI];
  }
  var cos = clamp(-1, (wanted - p) / amplitude, 1);
  var phase = Math.atan2(sTerm, q);
  var delta = Math.acos(cos);
  return [phase + delta, phase - delta];
}

/// Single-rotary candidates: the angle between home and target measured
/// in the plane perpendicular to the axis (± since either sign of a
/// wrist tilt reaches the same direction).
function __ivacAngleCandidates(machine, axis, target) {
  var home = machine.getDirection(new Vector(0, 0, 0));
  var a = axis.getAxis().getNormalized();
  var homePerp = Vector.diff(home, Vector.product(a, Vector.dot(home, a)));
  var targetPerp = Vector.diff(target, Vector.product(a, Vector.dot(target, a)));
  if (homePerp.isZero() || targetPerp.isZero()) {
    return [0, Math.PI];
  }
  var cos = clamp(-1, Vector.dot(homePerp.getNormalized(), targetPerp.getNormalized()), 1);
  var angle = Math.acos(cos);
  if (angle < 1e-12) {
    return [0];
  }
  return [angle, -angle];
}

/// Swing MAGNITUDE for a fixed tilt: the angle between the tilt-only
/// direction and the target, measured about the swing axis. The caller
/// tries both signs (and the π-shifted pair) because the sense depends
/// on mount type and axis handedness.
function __ivacSolveSwing(machine, swing, abc, target) {
  var a = swing.getAxis().getNormalized();
  var probe = new Vector(abc.x, abc.y, abc.z);
  probe.setCoordinate(swing.getCoordinate(), 0);
  var base = machine.getDirection(probe);
  var basePerp = Vector.diff(base, Vector.product(a, Vector.dot(base, a)));
  var targetPerp = Vector.diff(target, Vector.product(a, Vector.dot(target, a)));
  if (basePerp.isZero() || targetPerp.isZero()) {
    return 0; // already aligned about this axis
  }
  var bn = basePerp.getNormalized();
  var tn = targetPerp.getNormalized();
  return Math.atan2(Vector.dot(a, Vector.cross(bn, tn)), clamp(-1, Vector.dot(bn, tn), 1));
}

function __ivacPreferenceScore(enabled, abc) {
  var score = 0;
  for (var i = 0; i < enabled.length; ++i) {
    var axis = enabled[i];
    var preference = axis.getPreference();
    if (preference === 0) {
      continue;
    }
    var value = abc.getCoordinate(axis.getCoordinate());
    // Wrong side of the preference costs one point per axis.
    if ((preference > 0 && value < -1e-9) || (preference < 0 && value > 1e-9)) {
      score += 1;
    }
  }
  return score;
}

function __ivacInRange(enabled, abc) {
  for (var i = 0; i < enabled.length; ++i) {
    var axis = enabled[i];
    if (axis.isCyclic()) {
      continue;
    }
    var range = axis.getRange();
    if (!range.isNonEmpty()) {
      continue;
    }
    var value = abc.getCoordinate(axis.getCoordinate());
    if (value < range.getMinimum() - 1e-9 || value > range.getMaximum() + 1e-9) {
      return false;
    }
  }
  return true;
}

/// Nudge each cyclic angle toward its axis preference — same
/// orientation, preferred sign.
MachineConfiguration.prototype.getPreferredABC = function (abc) {
  var out = new Vector(abc.x, abc.y, abc.z);
  var TWO_PI = Math.PI * 2;
  for (var i = 0; i < this.__axes.length; ++i) {
    var axis = this.__axes[i];
    if (!axis.isEnabled() || !axis.isCyclic()) {
      continue;
    }
    var preference = axis.getPreference();
    if (preference === 0) {
      continue;
    }
    var coordinate = axis.getCoordinate();
    var value = out.getCoordinate(coordinate);
    if (preference > 0 && value < -1e-9) {
      value += TWO_PI;
    } else if (preference < 0 && value > 1e-9) {
      value -= TWO_PI;
    }
    out.setCoordinate(coordinate, value);
  }
  return out;
};

/// Bring `abc` inside every axis range. Cyclic axes wrap by full turns;
/// a ranged axis that cannot reach the angle THROWS (the FANUC post
/// catches it and reports "Machine angles not supported").
MachineConfiguration.prototype.remapABC = function (abc) {
  var out = new Vector(abc.x, abc.y, abc.z);
  for (var i = 0; i < this.__axes.length; ++i) {
    var axis = this.__axes[i];
    if (!axis.isEnabled()) {
      continue;
    }
    var coordinate = axis.getCoordinate();
    var value = axis.remapToRange2(out.getCoordinate(coordinate));
    if (value === undefined) {
      throw new Error("machine angles out of range");
    }
    out.setCoordinate(coordinate, value);
  }
  return out;
};

/// Remap to the solution CLOSEST to `reference` — shortest rotary
/// travel from where the machine already stands.
MachineConfiguration.prototype.remapToABC = function (abc, reference) {
  var TWO_PI = Math.PI * 2;
  var out = new Vector(abc.x, abc.y, abc.z);
  for (var i = 0; i < this.__axes.length; ++i) {
    var axis = this.__axes[i];
    if (!axis.isEnabled() || !axis.isCyclic()) {
      continue;
    }
    var coordinate = axis.getCoordinate();
    var value = out.getCoordinate(coordinate);
    var from = reference.getCoordinate(coordinate);
    while (value - from > Math.PI) {
      value -= TWO_PI;
    }
    while (from - value > Math.PI) {
      value += TWO_PI;
    }
    out.setCoordinate(coordinate, value);
  }
  return out;
};

/// True when every rotary angle in `abc` is reachable. Called with no
/// argument it answers "is this a plain 3-axis machine?".
MachineConfiguration.prototype.isABCSupported = function (abc) {
  if (abc === undefined) {
    return !this.isMultiAxisConfiguration();
  }
  var enabled = __ivacEnabledAxes(this);
  return enabled.length === 0 ? true : __ivacInRange(enabled, abc);
};

/// Wrap/clamp one angle into this axis's range; `undefined` when the
/// angle cannot be reached.
Axis.prototype.remapToRange2 = function (angle) {
  var TWO_PI = Math.PI * 2;
  var value = angle;
  if (this.isCyclic()) {
    while (value <= -Math.PI - 1e-12) {
      value += TWO_PI;
    }
    while (value > Math.PI + 1e-12) {
      value -= TWO_PI;
    }
    return value;
  }
  var range = this.getRange();
  if (!range.isNonEmpty()) {
    return value;
  }
  // A full turn either way may land inside a wide range (e.g. ±360°).
  var options = [value, value + TWO_PI, value - TWO_PI];
  for (var i = 0; i < options.length; ++i) {
    if (options[i] >= range.getMinimum() - 1e-9 && options[i] <= range.getMaximum() + 1e-9) {
      return options[i];
    }
  }
  return undefined;
};

/// Per-section machine-angle preprocessing. With multi-axis records the
/// driver would precompute each section's ABC here; the v1 IR is
/// 3-axis, so this records the requested mode and `isOptimizedForMachine`
/// reports true only when the machine actually has rotaries.
var __ivacMachineOptimizeType = 0;

function optimizeMachineAngles2(optimizeType) {
  __ivacMachineOptimizeType = optimizeType;
}

function isOptimizedForMachine() {
  return (
    __ivacMachineOptimizeType !== OPTIMIZE_NONE &&
    machineConfiguration.isMultiAxisConfiguration()
  );
}

/// Multi-axis feed for a 5D move: inverse-time (F = 1/minutes, or 60/
/// minutes for INVERSE_SECONDS) or degrees-per-minute, per
/// `setMultiAxisFeedrate`. Plain FEED_PER_MINUTE passes through.
function __ivacMultiAxisFeed(distance, rotaryDelta, feed) {
  var config = machineConfiguration.__multiAxisFeedrate;
  if (!config || config.mode === FEED_PER_MINUTE) {
    return feed;
  }
  var minutes = distance > 1e-12 && feed > 1e-12 ? distance / feed : 0;
  if (config.mode === FEED_INVERSE_TIME) {
    if (minutes <= 1e-12) {
      return config.maximum;
    }
    var inverse = config.type === INVERSE_SECONDS ? 60 / minutes : 1 / minutes;
    return Math.min(inverse, config.maximum);
  }
  if (config.mode === FEED_DPM) {
    if (minutes <= 1e-12) {
      return config.maximum;
    }
    return Math.min(toDeg(Math.abs(rotaryDelta)) / minutes, config.maximum);
  }
  return feed;
}


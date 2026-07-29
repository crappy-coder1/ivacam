// 11_circular.js — circular-motion state, helpers, and the KERNEL
// POLICY: decide per arc record whether the post's onCircular runs at
// all, gets a split arc, or the kernel auto-linearizes through
// onLinear (per the post's minimum/maximum circular config globals).
//
// All values here are in the POST's unit system (the driver scales IR
// mm before handing arcs over).

var __ivacCircular = {
  active: false,
  clockwise: false,
  start: new Vector(0, 0, 0),
  center: new Vector(0, 0, 0),
  end: new Vector(0, 0, 0),
  normal: new Vector(0, 0, 1),
  feed: 0,
};

function isFullCircle() {
  var c = __ivacCircular;
  return Vector.diff(c.start, c.end).length < 1e-9;
}

function getCircularPlane() {
  var n = __ivacCircular.normal;
  if (Math.abs(Math.abs(n.z) - 1) < 1e-9) {
    return PLANE_XY;
  }
  if (Math.abs(Math.abs(n.y) - 1) < 1e-9) {
    return PLANE_ZX;
  }
  if (Math.abs(Math.abs(n.x) - 1) < 1e-9) {
    return PLANE_YZ;
  }
  return -1;
}

function getCircularCenter() {
  var c = __ivacCircular.center;
  return new Vector(c.x, c.y, c.z);
}

function getCircularRadius() {
  var c = __ivacCircular;
  // In-plane distance start→center (helical arcs keep the plane
  // radius, not the 3D distance).
  var d = Vector.diff(c.start, c.center);
  var plane = getCircularPlane();
  if (plane === PLANE_XY) {
    return Math.sqrt(d.x * d.x + d.y * d.y);
  }
  if (plane === PLANE_ZX) {
    return Math.sqrt(d.x * d.x + d.z * d.z);
  }
  if (plane === PLANE_YZ) {
    return Math.sqrt(d.y * d.y + d.z * d.z);
  }
  return d.length;
}

// Start/end angles in the arc plane; sweep is positive, measured in
// the arc's own direction of travel. Full circles sweep 2π.
function __ivacArcAngles() {
  var c = __ivacCircular;
  var plane = getCircularPlane();
  var toAngle;
  if (plane === PLANE_ZX) {
    toAngle = function (p) {
      return Math.atan2(p.x - c.center.x, p.z - c.center.z);
    };
  } else if (plane === PLANE_YZ) {
    toAngle = function (p) {
      return Math.atan2(p.z - c.center.z, p.y - c.center.y);
    };
  } else {
    toAngle = function (p) {
      return Math.atan2(p.y - c.center.y, p.x - c.center.x);
    };
  }
  var a0 = toAngle(c.start);
  var a1 = toAngle(c.end);
  // Direction of increasing angle vs travel direction: for a +normal
  // plane, CCW travel increases the angle; CW decreases it. A flipped
  // normal flips the relationship.
  var positiveIsCcw =
    (plane === PLANE_XY && c.normal.z > 0) ||
    (plane === PLANE_ZX && c.normal.y > 0) ||
    (plane === PLANE_YZ && c.normal.x > 0) ||
    plane === -1;
  var travelsPositive = positiveIsCcw ? !c.clockwise : c.clockwise;
  var sweep = travelsPositive ? a1 - a0 : a0 - a1;
  var TAU = Math.PI * 2;
  sweep %= TAU;
  if (sweep < 1e-12) {
    sweep += TAU; // start==end or wrap → full circle / crossing zero
  }
  return { start: a0, sweep: sweep, travelsPositive: travelsPositive };
}

function getCircularSweep() {
  return __ivacArcAngles().sweep;
}

function isHelical() {
  var c = __ivacCircular;
  var plane = getCircularPlane();
  var delta;
  if (plane === PLANE_ZX) {
    delta = c.end.y - c.start.y;
  } else if (plane === PLANE_YZ) {
    delta = c.end.x - c.start.x;
  } else {
    delta = c.end.z - c.start.z;
  }
  return Math.abs(delta) > 1e-9;
}

function isSpiral() {
  var c = __ivacCircular;
  var rEnd = (function () {
    var d = Vector.diff(c.end, c.center);
    var plane = getCircularPlane();
    if (plane === PLANE_XY) {
      return Math.sqrt(d.x * d.x + d.y * d.y);
    }
    if (plane === PLANE_ZX) {
      return Math.sqrt(d.x * d.x + d.z * d.z);
    }
    if (plane === PLANE_YZ) {
      return Math.sqrt(d.y * d.y + d.z * d.z);
    }
    return d.length;
  })();
  return Math.abs(rEnd - getCircularRadius()) > 1e-9;
}

/** Point on the arc at parameter u ∈ [0, 1] (helix-aware). */
function getPositionU(u) {
  var c = __ivacCircular;
  var angles = __ivacArcAngles();
  var theta = angles.start + (angles.travelsPositive ? 1 : -1) * angles.sweep * u;
  var r = getCircularRadius();
  var plane = getCircularPlane();
  if (plane === PLANE_ZX) {
    return new Vector(
      c.center.x + r * Math.sin(theta),
      c.start.y + (c.end.y - c.start.y) * u,
      c.center.z + r * Math.cos(theta)
    );
  }
  if (plane === PLANE_YZ) {
    return new Vector(
      c.start.x + (c.end.x - c.start.x) * u,
      c.center.y + r * Math.cos(theta),
      c.center.z + r * Math.sin(theta)
    );
  }
  return new Vector(
    c.center.x + r * Math.cos(theta),
    c.center.y + r * Math.sin(theta),
    c.start.z + (c.end.z - c.start.z) * u
  );
}

function canLinearize() {
  return true;
}

/** Chord count that keeps the sagitta within `tol`. */
function getNumberOfSegments(tol) {
  var r = getCircularRadius();
  var sweep = getCircularSweep();
  var t = Math.max(tol, 1e-6);
  if (t >= r) {
    return Math.max(2, Math.ceil(sweep / (Math.PI / 2)));
  }
  var maxStep = 2 * Math.acos(1 - t / r);
  return Math.max(2, Math.ceil(sweep / maxStep));
}

/** Kernel linearization: replays the arc as chords through the post's
 * CURRENT onLinear binding, updating the position per chord. */
function linearize(tol) {
  var n = getNumberOfSegments(tol);
  var feed = __ivacCircular.feed;
  for (var k = 1; k <= n; ++k) {
    var p = getPositionU(k / n);
    invokeOnLinear(p.x, p.y, p.z, feed);
  }
}

/** Driver-side arc dispatch: applies the post's circular-capability
 * config, then either linearizes, splits, or calls onCircular. */
function __ivacDispatchCircular(clockwise, center, end, feed) {
  var start = getCurrentPosition();
  __ivacCircular = {
    active: true,
    clockwise: clockwise,
    start: start,
    center: center,
    end: end,
    normal: new Vector(0, 0, 1), // v1 IR records XY-plane arcs
    feed: feed,
  };

  var radius = getCircularRadius();
  var sweep = getCircularSweep();
  var chord = Vector.diff(end, start).length;

  var planeAllowed = true;
  if (allowedCircularPlanes !== undefined) {
    planeAllowed =
      getCircularPlane() >= 0 &&
      (allowedCircularPlanes & (1 << getCircularPlane())) !== 0;
  }

  var mustLinearize =
    radius < minimumCircularRadius ||
    radius > maximumCircularRadius ||
    (!isFullCircle() && chord < minimumChordLength) ||
    sweep < minimumCircularSweep ||
    !planeAllowed ||
    (isHelical() && !allowHelicalMoves) ||
    (isSpiral() && !(typeof allowSpiralMoves !== "undefined" && allowSpiralMoves));

  if (mustLinearize) {
    linearize(tolerance);
    __ivacCircular.active = false;
    setCurrentPosition(end);
    return;
  }

  if (sweep > maximumCircularSweep + 1e-12) {
    // Split into equal pieces, each within the post's sweep limit.
    // Boundary points MUST be computed up front: dispatching a piece
    // replaces the circular state getPositionU reads from.
    var pieces = Math.ceil(sweep / maximumCircularSweep);
    var targets = [];
    for (var k = 1; k < pieces; ++k) {
      targets.push(getPositionU(k / pieces));
    }
    targets.push(end);
    var prev = start;
    for (var t = 0; t < targets.length; ++t) {
      __ivacSingleCircular(clockwise, center, prev, targets[t], feed);
      prev = targets[t];
    }
    setCurrentPosition(end);
    return;
  }

  __ivacSingleCircular(clockwise, center, start, end, feed);
  setCurrentPosition(end);
}

function __ivacSingleCircular(clockwise, center, start, end, feed) {
  __ivacCircular = {
    active: true,
    clockwise: clockwise,
    start: new Vector(start.x, start.y, start.z),
    center: new Vector(center.x, center.y, center.z),
    end: new Vector(end.x, end.y, end.z),
    normal: new Vector(0, 0, 1),
    feed: feed,
  };
  setCurrentPosition(start);
  if (typeof onCircular === "function") {
    onCircular(clockwise, center.x, center.y, center.z, end.x, end.y, end.z, feed);
  } else {
    linearize(tolerance);
  }
  __ivacCircular.active = false;
  setCurrentPosition(end);
}

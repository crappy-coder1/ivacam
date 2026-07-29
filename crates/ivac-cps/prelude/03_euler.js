// 03_euler.js — the 24-convention Euler engine.
//
// One table-driven implementation covers all 24 orderings, following
// Shoemake, "Euler Angle Conversion" (Graphics Gems IV): a convention
// is described by (inner axis i, parity, repetition, frame), and every
// extraction/composition is the same code with those four knobs.
//
//   * i          — index of the FIRST rotation axis (0=X, 1=Y, 2=Z)
//   * parity     — 0 when (i, j, k) is an even permutation of (X, Y, Z),
//                  1 when odd (j/k swap roles)
//   * repetition — 1 for the "symmetric" orderings that repeat the first
//                  axis (XYX, ZXZ, …), 0 for the three-distinct-axis
//                  ones (XYZ, ZYX, …)
//   * frame      — 0 = _S (static / extrinsic: rotations about FIXED
//                  world axes), 1 = _R (rotating / intrinsic: each
//                  rotation about the axis the previous one moved).
//                  The two differ only by reversing the angle order,
//                  which is exactly how the table handles them.
//
// EULER_* constants are declared in 01_constants.js; this table is
// indexed by them, so the two files must stay in the same order.

// [i, parity, repetition, frame] per convention. Static block first
// (12), then rotating (12), matching the constant order.
var __ivacEulerTable = [
  [0, 0, 1, 0], // EULER_XYX_S
  [0, 0, 0, 0], // EULER_XYZ_S
  [0, 1, 1, 0], // EULER_XZX_S
  [0, 1, 0, 0], // EULER_XZY_S
  [1, 1, 1, 0], // EULER_YXY_S
  [1, 1, 0, 0], // EULER_YXZ_S
  [1, 0, 0, 0], // EULER_YZX_S
  [1, 0, 1, 0], // EULER_YZY_S
  [2, 0, 0, 0], // EULER_ZXY_S
  [2, 0, 1, 0], // EULER_ZXZ_S
  [2, 1, 0, 0], // EULER_ZYX_S
  [2, 1, 1, 0], // EULER_ZYZ_S
  [0, 0, 1, 1], // EULER_XYX_R
  [0, 0, 0, 1], // EULER_XYZ_R
  [0, 1, 1, 1], // EULER_XZX_R
  [0, 1, 0, 1], // EULER_XZY_R
  [1, 1, 1, 1], // EULER_YXY_R
  [1, 1, 0, 1], // EULER_YXZ_R
  [1, 0, 0, 1], // EULER_YZX_R
  [1, 0, 1, 1], // EULER_YZY_R
  [2, 0, 0, 1], // EULER_ZXY_R
  [2, 0, 1, 1], // EULER_ZXZ_R
  [2, 1, 0, 1], // EULER_ZYX_R
  [2, 1, 1, 1], // EULER_ZYZ_R
];

/// Axis-order triple (i, j, k) for a convention's parity.
function __ivacEulerAxes(i, parity) {
  var next = [1, 2, 0, 1];
  var j = next[i + parity];
  var k = next[i + 1 - parity];
  return [i, j, k];
}

/// Row-major element access of a Matrix (row/col in 0..3).
function __ivacMatrixAt(m, row, col) {
  var v = row === 0 ? m.right : row === 1 ? m.up : m.forward;
  return col === 0 ? v.x : col === 1 ? v.y : v.z;
}

var __IVAC_EULER_SINGULARITY = 16 * 2.220446049250313e-16;

/// Extract Euler angles for `convention` from a rotation matrix.
/// Returns a Vector of the three angles IN CONVENTION ORDER.
function __ivacGetEuler(m, convention) {
  var spec = __ivacEulerTable[convention];
  if (spec === undefined) {
    throw new Error("unknown Euler convention: " + String(convention));
  }
  var i = spec[0];
  var parity = spec[1];
  var repetition = spec[2];
  var frame = spec[3];
  var axes = __ivacEulerAxes(i, parity);
  var j = axes[1];
  var k = axes[2];

  var ax;
  var ay;
  var az;
  if (repetition) {
    var sy = Math.sqrt(
      __ivacMatrixAt(m, i, j) * __ivacMatrixAt(m, i, j) +
        __ivacMatrixAt(m, i, k) * __ivacMatrixAt(m, i, k)
    );
    if (sy > __IVAC_EULER_SINGULARITY) {
      ax = Math.atan2(__ivacMatrixAt(m, i, j), __ivacMatrixAt(m, i, k));
      ay = Math.atan2(sy, __ivacMatrixAt(m, i, i));
      az = Math.atan2(__ivacMatrixAt(m, j, i), -__ivacMatrixAt(m, k, i));
    } else {
      // Gimbal lock: the first and third rotations act about the same
      // world axis, so only their sum is observable — attribute it all
      // to the first and zero the third.
      ax = Math.atan2(-__ivacMatrixAt(m, j, k), __ivacMatrixAt(m, j, j));
      ay = Math.atan2(sy, __ivacMatrixAt(m, i, i));
      az = 0;
    }
  } else {
    var cy = Math.sqrt(
      __ivacMatrixAt(m, i, i) * __ivacMatrixAt(m, i, i) +
        __ivacMatrixAt(m, j, i) * __ivacMatrixAt(m, j, i)
    );
    if (cy > __IVAC_EULER_SINGULARITY) {
      ax = Math.atan2(__ivacMatrixAt(m, k, j), __ivacMatrixAt(m, k, k));
      ay = Math.atan2(-__ivacMatrixAt(m, k, i), cy);
      az = Math.atan2(__ivacMatrixAt(m, j, i), __ivacMatrixAt(m, i, i));
    } else {
      ax = Math.atan2(-__ivacMatrixAt(m, j, k), __ivacMatrixAt(m, j, j));
      ay = Math.atan2(-__ivacMatrixAt(m, k, i), cy);
      az = 0;
    }
  }
  if (parity) {
    ax = -ax;
    ay = -ay;
    az = -az;
  }
  if (frame) {
    // Rotating (intrinsic) frame: same numbers, reversed order.
    var t = ax;
    ax = az;
    az = t;
  }
  return new Vector(ax, ay, az);
}

/// Compose a rotation matrix from Euler angles in `convention` order —
/// the exact inverse of [`__ivacGetEuler`].
function __ivacEulerToMatrix(angles, convention) {
  var spec = __ivacEulerTable[convention];
  if (spec === undefined) {
    throw new Error("unknown Euler convention: " + String(convention));
  }
  var i = spec[0];
  var parity = spec[1];
  var repetition = spec[2];
  var frame = spec[3];
  var axes = __ivacEulerAxes(i, parity);
  var j = axes[1];
  var k = axes[2];

  var ax = angles.x;
  var ay = angles.y;
  var az = angles.z;
  if (frame) {
    var t = ax;
    ax = az;
    az = t;
  }
  if (parity) {
    ax = -ax;
    ay = -ay;
    az = -az;
  }

  var si = Math.sin(ax);
  var sj = Math.sin(ay);
  var sh = Math.sin(az);
  var ci = Math.cos(ax);
  var cj = Math.cos(ay);
  var ch = Math.cos(az);
  var cc = ci * ch;
  var cs = ci * sh;
  var sc = si * ch;
  var ss = si * sh;

  var rows = [
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
  ];
  if (repetition) {
    rows[i][i] = cj;
    rows[i][j] = sj * si;
    rows[i][k] = sj * ci;
    rows[j][i] = sj * sh;
    rows[j][j] = -cj * ss + cc;
    rows[j][k] = -cj * cs - sc;
    rows[k][i] = -sj * ch;
    rows[k][j] = cj * sc + cs;
    rows[k][k] = cj * cc - ss;
  } else {
    rows[i][i] = cj * ch;
    rows[i][j] = sj * sc - cs;
    rows[i][k] = sj * cc + ss;
    rows[j][i] = cj * sh;
    rows[j][j] = sj * ss + cc;
    rows[j][k] = sj * cs - sc;
    rows[k][i] = -sj;
    rows[k][j] = cj * si;
    rows[k][k] = cj * ci;
  }
  return Matrix.fromRows(rows);
}

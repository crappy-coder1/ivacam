// kinematics.test.mjs — the 24-convention Euler engine and the rotary
// solver. These are the correctness proofs the multi-axis API rests on:
// an Euler round-trip that misses by a degree, or a getABC that returns
// angles pointing the spindle elsewhere, would silently produce
// wrong-orientation NC.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRuntime } from './harness.mjs';

const CONVENTIONS = [
  'EULER_XYX_S', 'EULER_XYZ_S', 'EULER_XZX_S', 'EULER_XZY_S',
  'EULER_YXY_S', 'EULER_YXZ_S', 'EULER_YZX_S', 'EULER_YZY_S',
  'EULER_ZXY_S', 'EULER_ZXZ_S', 'EULER_ZYX_S', 'EULER_ZYZ_S',
  'EULER_XYX_R', 'EULER_XYZ_R', 'EULER_XZX_R', 'EULER_XZY_R',
  'EULER_YXY_R', 'EULER_YXZ_R', 'EULER_YZX_R', 'EULER_YZY_R',
  'EULER_ZXY_R', 'EULER_ZXZ_R', 'EULER_ZYX_R', 'EULER_ZYZ_R',
];

/// Deterministic pseudo-random rotations (no Math.random: the boa side
/// of the differential must see identical inputs).
const SETUP = `
  function rot(seed) {
    // Three angles from a simple LCG, then a composed rotation.
    var a = ((seed * 1103515245 + 12345) % 1000) / 1000 * Math.PI - Math.PI / 2;
    var b = ((seed * 69069 + 1) % 1000) / 1000 * Math.PI - Math.PI / 2;
    var c = ((seed * 22695477 + 1) % 1000) / 1000 * Math.PI - Math.PI / 2;
    return Matrix.getZRotation(c)
      .multiply(Matrix.getYRotation(b))
      .multiply(Matrix.getXRotation(a));
  }
  function maxAbsDiff(m, n) {
    var rows = [['right', 'x'], ['right', 'y'], ['right', 'z'],
                ['up', 'x'], ['up', 'y'], ['up', 'z'],
                ['forward', 'x'], ['forward', 'y'], ['forward', 'z']];
    var worst = 0;
    for (var i = 0; i < rows.length; ++i) {
      var d = Math.abs(m[rows[i][0]][rows[i][1]] - n[rows[i][0]][rows[i][1]]);
      if (d > worst) { worst = d; }
    }
    return worst;
  }
`;

test('every Euler convention round-trips a rotation matrix', () => {
  const rt = createRuntime();
  rt.eval(SETUP);
  for (const convention of CONVENTIONS) {
    for (let seed = 1; seed <= 6; ++seed) {
      const worst = rt.eval(`
        (function () {
          var m = rot(${seed});
          var angles = m.getEuler2(${convention});
          var back = Matrix.getEulerRotation(angles, ${convention});
          return maxAbsDiff(m, back);
        })()
      `);
      assert.ok(
        worst < 1e-9,
        `${convention} seed ${seed}: round-trip error ${worst}`,
      );
    }
  }
});

test('Euler extraction handles the gimbal-lock singularity', () => {
  const rt = createRuntime();
  rt.eval(SETUP);
  // ZXZ with the middle angle at 0 is the classic symmetric
  // singularity; XYZ with pitch at ±90° the asymmetric one. Both must
  // yield angles that still rebuild the matrix.
  const cases = [
    ['EULER_ZXZ_R', 'Matrix.getIdentity()'],
    ['EULER_ZXZ_R', 'Matrix.getZRotation(0.4)'],
    ['EULER_XYZ_S', 'Matrix.getYRotation(Math.PI / 2)'],
    ['EULER_XYZ_S', 'Matrix.getYRotation(-Math.PI / 2)'],
  ];
  for (const [convention, expr] of cases) {
    const worst = rt.eval(`
      (function () {
        var m = ${expr};
        var angles = m.getEuler2(${convention});
        return maxAbsDiff(m, Matrix.getEulerRotation(angles, ${convention}));
      })()
    `);
    assert.ok(worst < 1e-9, `${convention} / ${expr}: error ${worst}`);
  }
});

test('rotation factories agree with axis-angle', () => {
  const rt = createRuntime();
  rt.eval(SETUP);
  const worst = rt.eval(`
    (function () {
      var a = 0.7;
      var checks = [
        maxAbsDiff(Matrix.getXRotation(a), Matrix.getAxisRotation(new Vector(1,0,0), a)),
        maxAbsDiff(Matrix.getYRotation(a), Matrix.getAxisRotation(new Vector(0,1,0), a)),
        maxAbsDiff(Matrix.getZRotation(a), Matrix.getAxisRotation(new Vector(0,0,1), a)),
      ];
      return Math.max(checks[0], Math.max(checks[1], checks[2]));
    })()
  `);
  assert.ok(worst < 1e-12, `factory mismatch ${worst}`);
});

/// The identity that makes the whole solver trustworthy: whatever
/// getABC returns must actually point the spindle at the request.
const AC_TABLE = `
  var aAxis = createAxis({coordinate:0, table:true, axis:[1, 0, 0], range:[-120, 120], preference:1, tcp:true});
  var cAxis = createAxis({coordinate:2, table:true, axis:[0, 0, 1], range:[-360, 360], preference:0, tcp:true});
  machineConfiguration = new MachineConfiguration(aAxis, cAxis);
`;
const BC_TABLE = `
  var bAxis = createAxis({coordinate:1, table:true, axis:[0, 1, 0], range:[-120, 120], preference:1, tcp:true});
  var cAxis = createAxis({coordinate:2, table:true, axis:[0, 0, 1], cyclic:true, preference:0, tcp:true});
  machineConfiguration = new MachineConfiguration(bAxis, cAxis);
`;
const AB_HEAD = `
  var aAxis = createAxis({coordinate:0, table:false, axis:[1, 0, 0], range:[-90, 90], preference:0});
  var bAxis = createAxis({coordinate:1, table:false, axis:[0, 1, 0], range:[-90, 90], preference:0});
  machineConfiguration = new MachineConfiguration(aAxis, bAxis);
`;

const DIRECTIONS = [
  '[0, 0, 1]',
  '[0, 0.5, 0.866025403784]',
  '[0.5, 0, 0.866025403784]',
  '[0.353553390593, 0.353553390593, 0.866025403784]',
  '[0, 0.707106781187, 0.707106781187]',
  '[-0.5, 0.5, 0.707106781187]',
];

for (const [name, config] of [
  ['AC table', AC_TABLE],
  ['BC table', BC_TABLE],
  ['AB head', AB_HEAD],
]) {
  test(`${name}: getOrientation(getABC(m)).forward === m.forward`, () => {
    const rt = createRuntime();
    rt.eval(config);
    for (const dir of DIRECTIONS) {
      const result = rt.eval(`
        (function () {
          var d = new Vector(${dir}[0], ${dir}[1], ${dir}[2]).getNormalized();
          var m = new Matrix(new Vector(1,0,0), new Vector(0,1,0), d);
          try {
            var abc = machineConfiguration.getABC(m);
            var reached = machineConfiguration.getDirection(abc);
            return JSON.stringify({
              ok: true,
              err: Math.max(
                Math.abs(reached.x - d.x),
                Math.max(Math.abs(reached.y - d.y), Math.abs(reached.z - d.z)),
              ),
            });
          } catch (e) {
            return JSON.stringify({ ok: false, message: String(e.message) });
          }
        })()
      `);
      const parsed = JSON.parse(result);
      assert.ok(parsed.ok, `${name} ${dir}: ${parsed.message}`);
      assert.ok(parsed.err < 1e-9, `${name} ${dir}: reached direction off by ${parsed.err}`);
    }
  });
}

test('unreachable orientation throws', () => {
  const rt = createRuntime();
  rt.eval(AB_HEAD);
  // Straight down (-Z) needs 180° of tilt; both head axes stop at 90°.
  const outcome = rt.eval(`
    (function () {
      var m = new Matrix(new Vector(1,0,0), new Vector(0,1,0), new Vector(0, 0, -1));
      try {
        var abc = machineConfiguration.getABC(m);
        machineConfiguration.remapABC(abc);
        return 'reached';
      } catch (e) { return 'threw'; }
    })()
  `);
  assert.equal(outcome, 'threw');
});

test('remapABC wraps cyclic axes and rejects out-of-range ones', () => {
  const rt = createRuntime();
  rt.eval(BC_TABLE);
  // C is cyclic: 3π wraps into (-π, π].
  const wrapped = rt.eval(`
    machineConfiguration.remapABC(new Vector(0, 0, Math.PI * 3)).z
  `);
  assert.ok(Math.abs(wrapped - Math.PI) < 1e-9, `wrapped to ${wrapped}`);
  // B is ranged ±120°: 150° cannot be reached.
  const outcome = rt.eval(`
    (function () {
      try {
        machineConfiguration.remapABC(new Vector(0, toRad(150), 0));
        return 'reached';
      } catch (e) { return 'threw'; }
    })()
  `);
  assert.equal(outcome, 'threw');
});

test('remapToABC takes the shortest path from the current angles', () => {
  const rt = createRuntime();
  rt.eval(BC_TABLE);
  // Standing at 170°, a target of -170° is 20° away the short way.
  const value = rt.eval(`
    machineConfiguration.remapToABC(
      new Vector(0, 0, toRad(-170)),
      new Vector(0, 0, toRad(170)),
    ).z
  `);
  assert.ok(
    Math.abs(value - Math.PI * (190 / 180)) < 1e-9,
    `expected 190°, got ${(value * 180) / Math.PI}°`,
  );
});

test('getPreferredABC honors a positive axis preference', () => {
  const rt = createRuntime();
  rt.eval(BC_TABLE);
  // C is cyclic with preference 0 → untouched; make a preferring axis.
  rt.eval(`
    var cPref = createAxis({coordinate:2, table:true, axis:[0,0,1], cyclic:true, preference:1});
    machineConfiguration = new MachineConfiguration(cPref);
  `);
  const value = rt.eval(`machineConfiguration.getPreferredABC(new Vector(0, 0, -1)).z`);
  assert.ok(value > 0, `expected a positive angle, got ${value}`);
});

test('getRemainingOrientation leaves nothing when the machine can reach it', () => {
  const rt = createRuntime();
  rt.eval(AC_TABLE);
  const worst = rt.eval(`
    (function () {
      var d = new Vector(0.3, 0.2, 0.9).getNormalized();
      var m = new Matrix(new Vector(1,0,0), new Vector(0,1,0), d);
      var abc = machineConfiguration.getABC(m);
      var remaining = machineConfiguration.getRemainingOrientation(abc, m);
      // The leftover rotation must not tilt the tool any further: its own
      // tool axis (the forward row, same convention posts read) has to
      // be the untilted spindle axis.
      var f = remaining.getForward();
      return Math.max(Math.abs(f.x), Math.abs(f.y));
    })()
  `);
  assert.ok(worst < 1e-9, `remaining orientation still tilts by ${worst}`);
});

test('isABCSupported reflects axis ranges', () => {
  const rt = createRuntime();
  rt.eval(AC_TABLE);
  assert.equal(rt.eval(`machineConfiguration.isABCSupported(new Vector(toRad(90), 0, 0))`), true);
  assert.equal(rt.eval(`machineConfiguration.isABCSupported(new Vector(toRad(150), 0, 0))`), false);
});

test('multi-axis feed: inverse time and DPM', () => {
  const rt = createRuntime();
  rt.eval(`
    machineConfiguration = new MachineConfiguration(
      createAxis({coordinate:0, table:true, axis:[1,0,0], range:[-120,120]}),
    );
    machineConfiguration.setMultiAxisFeedrate(FEED_INVERSE_TIME, 9999.99, INVERSE_MINUTES, 0.5, 1.0);
  `);
  // 10 mm at 500 mm/min = 0.02 min → F = 1/0.02 = 50.
  assert.equal(rt.eval(`__ivacMultiAxisFeed(10, 0, 500)`), 50);
  // Clamped at the configured maximum for a zero-length move.
  assert.equal(rt.eval(`__ivacMultiAxisFeed(0, 0, 500)`), 9999.99);
  rt.eval(`
    machineConfiguration.setMultiAxisFeedrate(FEED_DPM, 9999.99, DPM_STANDARD, 0.5, 1.0);
  `);
  // 10 mm at 500 mm/min = 0.02 min while rotating 0.2 rad (11.459°)
  // → 572.96 deg/min.
  const dpm = rt.eval(`__ivacMultiAxisFeed(10, 0.2, 500)`);
  assert.ok(Math.abs(dpm - 572.9577951308232) < 1e-6, `got ${dpm}`);
});

// 02_vector_matrix.js — Vector / Matrix / Range / BoundingBox.
//
// cps.4 ships the working core (what 3-axis posts touch: vector
// algebra, identity work planes, Z ranges). cps.6 extends Matrix with
// the 24-convention Euler engine and rotation factories for the full
// multi-axis API.

function Vector(x, y, z) {
  this.x = Number(x) || 0;
  this.y = Number(y) || 0;
  this.z = Number(z) || 0;
}

Object.defineProperty(Vector.prototype, "length", {
  get: function () {
    return Math.sqrt(this.x * this.x + this.y * this.y + this.z * this.z);
  },
});

Vector.prototype.getNormalized = function () {
  var len = this.length;
  if (len < 1e-15) {
    return new Vector(0, 0, 0);
  }
  return new Vector(this.x / len, this.y / len, this.z / len);
};
Vector.prototype.getNegated = function () {
  return new Vector(-this.x, -this.y, -this.z);
};
Vector.prototype.getAbsolute = function () {
  return new Vector(Math.abs(this.x), Math.abs(this.y), Math.abs(this.z));
};
Vector.prototype.isZero = function () {
  return this.length < 1e-15;
};
Vector.prototype.isNonZero = function () {
  return !this.isZero();
};
Vector.prototype.normalize = function () {
  var len = this.length;
  if (len >= 1e-15) {
    this.x /= len;
    this.y /= len;
    this.z /= len;
  }
  return this;
};
Vector.prototype.getCoordinate = function (i) {
  return i === 0 ? this.x : i === 1 ? this.y : this.z;
};
Vector.prototype.setCoordinate = function (i, value) {
  if (i === 0) {
    this.x = value;
  } else if (i === 1) {
    this.y = value;
  } else {
    this.z = value;
  }
};
Vector.prototype.abs = Vector.prototype.getAbsolute;
Vector.prototype.negate = function () {
  this.x = -this.x;
  this.y = -this.y;
  this.z = -this.z;
  return this;
};
Vector.prototype.toString = function () {
  return "(" + this.x + ", " + this.y + ", " + this.z + ")";
};

Vector.diff = function (a, b) {
  return new Vector(a.x - b.x, a.y - b.y, a.z - b.z);
};
Vector.sum = function (a, b) {
  return new Vector(a.x + b.x, a.y + b.y, a.z + b.z);
};
Vector.product = function (a, factor) {
  return new Vector(a.x * factor, a.y * factor, a.z * factor);
};
Vector.dot = function (a, b) {
  return a.x * b.x + a.y * b.y + a.z * b.z;
};
Vector.cross = function (a, b) {
  return new Vector(
    a.y * b.z - a.z * b.y,
    a.z * b.x - a.x * b.z,
    a.x * b.y - a.y * b.x
  );
};
Vector.lerp = function (a, b, t) {
  return new Vector(
    a.x + (b.x - a.x) * t,
    a.y + (b.y - a.y) * t,
    a.z + (b.z - a.z) * t
  );
};

// Matrix: row-major 3×3. Rows are exposed the Autodesk way — right/up/
// forward accessors plus getEuler2 (cps.6 delivers all 24 conventions;
// the identity case posts hit for 3-axis work is exact today).
function Matrix(right, up, forward) {
  if (right === undefined) {
    this.right = new Vector(1, 0, 0);
    this.up = new Vector(0, 1, 0);
    this.forward = new Vector(0, 0, 1);
  } else {
    this.right = new Vector(right.x, right.y, right.z);
    this.up = new Vector(up.x, up.y, up.z);
    this.forward = new Vector(forward.x, forward.y, forward.z);
  }
}

Matrix.getIdentity = function () {
  return new Matrix();
};

/** Builds a matrix from a row-major nested array (the IR work plane). */
Matrix.fromRows = function (rows) {
  return new Matrix(
    new Vector(rows[0][0], rows[0][1], rows[0][2]),
    new Vector(rows[1][0], rows[1][1], rows[1][2]),
    new Vector(rows[2][0], rows[2][1], rows[2][2])
  );
};

Matrix.prototype.getRight = function () {
  return new Vector(this.right.x, this.right.y, this.right.z);
};
Matrix.prototype.getUp = function () {
  return new Vector(this.up.x, this.up.y, this.up.z);
};
Matrix.prototype.getForward = function () {
  return new Vector(this.forward.x, this.forward.y, this.forward.z);
};
Matrix.prototype.isIdentity = function () {
  return (
    Math.abs(this.right.x - 1) < 1e-12 &&
    Math.abs(this.up.y - 1) < 1e-12 &&
    Math.abs(this.forward.z - 1) < 1e-12 &&
    Math.abs(this.right.y) < 1e-12 &&
    Math.abs(this.right.z) < 1e-12 &&
    Math.abs(this.up.x) < 1e-12 &&
    Math.abs(this.up.z) < 1e-12 &&
    Math.abs(this.forward.x) < 1e-12 &&
    Math.abs(this.forward.y) < 1e-12
  );
};
Matrix.prototype.getTransposed = function () {
  return new Matrix(
    new Vector(this.right.x, this.up.x, this.forward.x),
    new Vector(this.right.y, this.up.y, this.forward.y),
    new Vector(this.right.z, this.up.z, this.forward.z)
  );
};
Matrix.prototype.multiply = function (other) {
  var t = other.getTransposed();
  return new Matrix(
    new Vector(Vector.dot(this.right, t.right), Vector.dot(this.right, t.up), Vector.dot(this.right, t.forward)),
    new Vector(Vector.dot(this.up, t.right), Vector.dot(this.up, t.up), Vector.dot(this.up, t.forward)),
    new Vector(Vector.dot(this.forward, t.right), Vector.dot(this.forward, t.up), Vector.dot(this.forward, t.forward))
  );
};
Matrix.prototype.transform = function (v) {
  return new Vector(
    Vector.dot(this.right, v),
    Vector.dot(this.up, v),
    Vector.dot(this.forward, v)
  );
};
/** Euler extraction — the identity fast path is exact; the general
 * table-driven engine lands with the kinematics work (cps.6). */
Matrix.prototype.getEuler2 = function (_convention) {
  if (this.isIdentity()) {
    return new Vector(0, 0, 0);
  }
  throw new Error(
    "Matrix.getEuler2: non-identity work planes need the cps.6 kinematics module"
  );
};

function Range(minimum, maximum) {
  this.minimum = minimum === undefined ? Number.POSITIVE_INFINITY : minimum;
  this.maximum = maximum === undefined ? Number.NEGATIVE_INFINITY : maximum;
}
Range.prototype.getMinimum = function () {
  return this.minimum;
};
Range.prototype.getMaximum = function () {
  return this.maximum;
};
Range.prototype.isNonEmpty = function () {
  return this.minimum <= this.maximum;
};
Range.prototype.expandTo = function (value) {
  if (value < this.minimum) {
    this.minimum = value;
  }
  if (value > this.maximum) {
    this.maximum = value;
  }
};
Range.prototype.expandToRange = function (range) {
  this.expandTo(range.minimum);
  this.expandTo(range.maximum);
};

function BoundingBox(lower, upper) {
  this.lower = lower || new Vector(0, 0, 0);
  this.upper = upper || new Vector(0, 0, 0);
}
BoundingBox.prototype.getXRange = function () {
  return new Range(this.lower.x, this.upper.x);
};
BoundingBox.prototype.getYRange = function () {
  return new Range(this.lower.y, this.upper.y);
};
BoundingBox.prototype.getZRange = function () {
  return new Range(this.lower.z, this.upper.z);
};
BoundingBox.prototype.expandTo = function (v) {
  this.lower.x = Math.min(this.lower.x, v.x);
  this.lower.y = Math.min(this.lower.y, v.y);
  this.lower.z = Math.min(this.lower.z, v.z);
  this.upper.x = Math.max(this.upper.x, v.x);
  this.upper.y = Math.max(this.upper.y, v.y);
  this.upper.z = Math.max(this.upper.z, v.z);
};

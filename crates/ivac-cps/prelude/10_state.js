// 10_state.js — kernel state: position tracking, frame transform,
// unit helpers.

// Current position in the POST's unit system (the driver seeds it per
// section and updates it after every motion dispatch).
var __ivacState = {
  position: new Vector(0, 0, 0),
  direction: new Vector(0, 0, 1),
  abc: new Vector(0, 0, 0),
  rotation: Matrix.getIdentity(),
  translation: new Vector(0, 0, 0),
  skipSection: false,
};

function getCurrentPosition() {
  var p = __ivacState.position;
  return new Vector(p.x, p.y, p.z);
}
function setCurrentPosition(position) {
  __ivacState.position = new Vector(position.x, position.y, position.z);
}
function getCurrentDirection() {
  var d = __ivacState.direction;
  return new Vector(d.x, d.y, d.z);
}
function setCurrentDirection(direction) {
  __ivacState.direction = new Vector(direction.x, direction.y, direction.z);
}
function getCurrentABC() {
  var a = __ivacState.abc;
  return new Vector(a.x, a.y, a.z);
}
function setCurrentABC(abc) {
  __ivacState.abc = new Vector(abc.x, abc.y, abc.z);
}

// Frame transform (setRotation/setTranslation) — posts apply the
// section work plane here; getFramePosition maps a model position into
// the working frame. Identity for 3-axis work.
function setRotation(matrix) {
  __ivacState.rotation = matrix;
}
function getRotation() {
  return __ivacState.rotation;
}
function setTranslation(vector) {
  __ivacState.translation = new Vector(vector.x, vector.y, vector.z);
}
function getTranslation() {
  return __ivacState.translation;
}
function cancelTransformation() {
  __ivacState.rotation = Matrix.getIdentity();
  __ivacState.translation = new Vector(0, 0, 0);
}
function getFramePosition(position) {
  var rotated = __ivacState.rotation.transform(position);
  return Vector.sum(rotated, __ivacState.translation);
}
function getFrameDirection(direction) {
  return __ivacState.rotation.transform(direction);
}

// Unit helpers. `spatial(value, unit)` converts a length given in
// `unit` into the POST's active unit; toPreciseUnit is its alias with
// documentation semantics (no rounding here either way).
function spatial(value, valueUnit) {
  if (valueUnit === unit) {
    return value;
  }
  return valueUnit === MM ? value / 25.4 : value * 25.4;
}
function toUnit(value, valueUnit) {
  return spatial(value, valueUnit);
}
function toPreciseUnit(value, valueUnit) {
  return spatial(value, valueUnit);
}
function toRad(degrees) {
  return (degrees * Math.PI) / 180.0;
}
function toDeg(radians) {
  return (radians * 180.0) / Math.PI;
}
function clamp(minimum, value, maximum) {
  return Math.min(Math.max(value, minimum), maximum);
}

/** True when every enabled section is plain 3-axis (v1 IR: always). */
function is3D() {
  return true;
}
function isTurning() {
  return false;
}

function isSameDirection(a, b) {
  return Vector.dot(a, b) > 1.0 - 1e-4;
}

/** Post asks to skip the rest of the current section's records —
 * onSectionEnd still fires. */
function skipRemainingSection() {
  __ivacState.skipSection = true;
}

function getOutputUnit() {
  return unit;
}
function getDogLeg() {
  return false;
}
function getLangId() {
  return "en";
}
function getCodePage() {
  return 0;
}
function setCodePage(_codePage) {}

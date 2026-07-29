// 09_machine.js — MachineConfiguration / Axis, the surface a 3-axis
// program exercises. Posts run their machine-activation code even
// without rotary axes (the FANUC activateMachine() runs on every
// program), so the API must be REAL for the 3-axis paths today; the
// kinematics core (getABC/remapABC/getOrientation and real Axis
// ranges) lands with cps.6, which extends these same objects.

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
MachineConfiguration.prototype.isABCSupported = function (_abc) {
  return !this.isMultiAxisConfiguration();
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
/** Multi-axis feed configuration — stored; the 5D dispatch (cps.6)
 * computes inverse-time feeds from it. */
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

/** Per-section machine-angle preprocessing — a no-op until the v1 IR
 * carries multi-axis records (cps.6). */
function optimizeMachineAngles2(_optimizeType) {}

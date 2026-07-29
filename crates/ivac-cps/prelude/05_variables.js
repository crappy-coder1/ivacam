// 05_variables.js — createVariable / createModal / createReferenceVariable /
// createIncrementalVariable / createOutputVariable / ModalGroup.
//
// Suppression contract (what makes NC output modal): a variable's
// format(v) returns "" when the RESULTING value (post-map, rounded)
// equals the last output one — unless forced, reset, or disabled.
// `onchange` closures supplied by the post fire exactly when text is
// actually produced (FANUC's zOutput uses this to clear `retracted`).

function Variable(specifiers, format) {
  this.__fmt = format;
  this.__prefix = "";
  this.__force = false;
  this.__onchange = undefined;
  this.__enabled = true;
  this.__current = undefined;
  if (specifiers) {
    if (specifiers.prefix !== undefined) {
      this.__prefix = specifiers.prefix;
    }
    if (specifiers.force !== undefined) {
      this.__force = !!specifiers.force;
    }
    if (specifiers.onchange !== undefined) {
      this.__onchange = specifiers.onchange;
    }
  }
}

Variable.prototype.format = function (value) {
  if (!this.__enabled) {
    return "";
  }
  var rv = this.__fmt.getResultingValue(value);
  if (!this.__force && this.__current !== undefined && rv === this.__current) {
    return "";
  }
  this.__current = rv;
  if (typeof this.__onchange === "function") {
    this.__onchange();
  }
  return this.__prefix + this.__fmt.format(value);
};
Variable.prototype.getCurrent = function () {
  return this.__current;
};
Variable.prototype.setCurrent = function (value) {
  this.__current = this.__fmt.getResultingValue(value);
};
Variable.prototype.reset = function () {
  this.__current = undefined;
};
Variable.prototype.disable = function () {
  this.__enabled = false;
};
Variable.prototype.enable = function () {
  this.__enabled = true;
};
Variable.prototype.isEnabled = function () {
  return this.__enabled;
};
Variable.prototype.getPrefix = function () {
  return this.__prefix;
};
Variable.prototype.setPrefix = function (prefix) {
  this.__prefix = prefix;
};
Variable.prototype.getFormat = function () {
  return this.__fmt;
};
Variable.prototype.setFormat = function (format) {
  this.__fmt = format;
};

function createVariable(specifiers, format) {
  return new Variable(specifiers, format);
}

// Modal: identical suppression logic over codes, plus a suffix. The
// FormatNumber usually carries the address letter (gFormat's "G").
function Modal(specifiers, format) {
  Variable.call(this, specifiers, format);
  this.__suffix = "";
  if (specifiers && specifiers.suffix !== undefined) {
    this.__suffix = specifiers.suffix;
  }
}
Modal.prototype = Object.create(Variable.prototype);
Modal.prototype.constructor = Modal;
Modal.prototype.format = function (value) {
  if (!this.__enabled) {
    return "";
  }
  var rv = this.__fmt.getResultingValue(value);
  if (!this.__force && this.__current !== undefined && rv === this.__current) {
    return "";
  }
  this.__current = rv;
  if (typeof this.__onchange === "function") {
    this.__onchange();
  }
  return this.__prefix + this.__fmt.format(value) + this.__suffix;
};
Modal.prototype.getSuffix = function () {
  return this.__suffix;
};
Modal.prototype.setSuffix = function (suffix) {
  this.__suffix = suffix;
};

function createModal(specifiers, format) {
  return new Modal(specifiers, format);
}

// ReferenceVariable: 2-arg format — suppress iff the value formats the
// same as the reference (arc IJK against the start point). Stateless
// between calls apart from getCurrent bookkeeping.
function ReferenceVariable(specifiers, format) {
  Variable.call(this, specifiers, format);
}
ReferenceVariable.prototype = Object.create(Variable.prototype);
ReferenceVariable.prototype.constructor = ReferenceVariable;
ReferenceVariable.prototype.format = function (value, reference) {
  if (!this.__enabled) {
    return "";
  }
  var rv = this.__fmt.getResultingValue(value);
  if (reference !== undefined && !this.__force) {
    if (rv === this.__fmt.getResultingValue(reference)) {
      return "";
    }
  }
  this.__current = rv;
  if (typeof this.__onchange === "function") {
    this.__onchange();
  }
  return this.__prefix + this.__fmt.format(value);
};

function createReferenceVariable(specifiers, format) {
  return new ReferenceVariable(specifiers, format);
}

// IncrementalVariable: renders the DELTA from the last absolute value.
function IncrementalVariable(specifiers, format) {
  Variable.call(this, specifiers, format);
  this.__current = 0;
  if (specifiers && specifiers.first !== undefined) {
    this.__current = specifiers.first;
  }
}
IncrementalVariable.prototype = Object.create(Variable.prototype);
IncrementalVariable.prototype.constructor = IncrementalVariable;
IncrementalVariable.prototype.format = function (value) {
  if (!this.__enabled) {
    return "";
  }
  var delta = value - this.__current;
  if (!this.__force && !this.__fmt.isSignificant(delta)) {
    return "";
  }
  this.__current = value;
  if (typeof this.__onchange === "function") {
    this.__onchange();
  }
  return this.__prefix + this.__fmt.format(delta);
};

function createIncrementalVariable(specifiers, format) {
  return new IncrementalVariable(specifiers, format);
}

// OutputVariable: the modern unified factory (r45892+). control
// replaces the force flag; type selects absolute/incremental output.
function OutputVariable(specifiers, format) {
  Variable.call(this, specifiers, format);
  this.__control = CONTROL_CHANGED;
  this.__type = TYPE_ABSOLUTE;
  this.__suffix = "";
  if (specifiers) {
    if (specifiers.control !== undefined) {
      this.__control = specifiers.control;
    }
    if (specifiers.force) {
      this.__control = CONTROL_FORCE;
    }
    if (specifiers.type !== undefined) {
      this.__type = specifiers.type;
    }
    if (specifiers.suffix !== undefined) {
      this.__suffix = specifiers.suffix;
    }
  }
}
OutputVariable.prototype = Object.create(Variable.prototype);
OutputVariable.prototype.constructor = OutputVariable;
OutputVariable.prototype.format = function (value) {
  if (!this.__enabled) {
    return "";
  }
  var out = value;
  if (this.__type === TYPE_INCREMENTAL) {
    out = value - (this.__current === undefined ? 0 : this.__current);
  }
  var rv = this.__fmt.getResultingValue(value);
  var emit;
  if (this.__control === CONTROL_FORCE) {
    emit = true;
  } else if (this.__control === CONTROL_NONZERO) {
    emit = this.__fmt.isSignificant(out);
  } else {
    emit =
      this.__current === undefined ||
      rv !== this.__current ||
      (this.__type === TYPE_INCREMENTAL && this.__fmt.isSignificant(out));
  }
  if (!emit) {
    return "";
  }
  this.__current = rv;
  if (typeof this.__onchange === "function") {
    this.__onchange();
  }
  return this.__prefix + this.__fmt.format(out) + this.__suffix;
};
OutputVariable.prototype.getControl = function () {
  return this.__control;
};
OutputVariable.prototype.setControl = function (control) {
  this.__control = control;
};
OutputVariable.prototype.getType = function () {
  return this.__type;
};
OutputVariable.prototype.setType = function (type) {
  this.__type = type;
};
OutputVariable.prototype.getSuffix = function () {
  return this.__suffix;
};
OutputVariable.prototype.setSuffix = function (suffix) {
  this.__suffix = suffix;
};

function createOutputVariable(specifiers, format) {
  return new OutputVariable(specifiers, format);
}

// ModalGroup: mutual exclusivity across groups of codes (G-code modal
// groups). Rendered through an attached FormatNumber.
function ModalGroup() {
  this.__groups = []; // group id (1-based) → array of codes
  this.__active = {}; // group id → active code
  this.__fmt = undefined;
  this.__prefix = "";
  this.__suffix = "";
  this.__enabled = true;
  this.__force = false;
  this.__strict = false;
  this.__autoReset = false;
}
ModalGroup.prototype.createGroup = function () {
  this.__groups.push([]);
  return this.__groups.length;
};
ModalGroup.prototype.addCode = function (group, code) {
  if (this.isGroup(group)) {
    this.__groups[group - 1].push(code);
  }
};
ModalGroup.prototype.removeCode = function (code) {
  for (var g = 0; g < this.__groups.length; ++g) {
    var idx = this.__groups[g].indexOf(code);
    if (idx >= 0) {
      this.__groups[g].splice(idx, 1);
    }
  }
};
ModalGroup.prototype.getGroup = function (code) {
  for (var g = 0; g < this.__groups.length; ++g) {
    if (this.__groups[g].indexOf(code) >= 0) {
      return g + 1;
    }
  }
  return 0;
};
ModalGroup.prototype.isGroup = function (group) {
  return group >= 1 && group <= this.__groups.length;
};
ModalGroup.prototype.isCodeDefined = function (code) {
  return this.getGroup(code) !== 0;
};
ModalGroup.prototype.inSameGroup = function (a, b) {
  var ga = this.getGroup(a);
  return ga !== 0 && ga === this.getGroup(b);
};
ModalGroup.prototype.getNumberOfGroups = function () {
  return this.__groups.length;
};
ModalGroup.prototype.getNumberOfCodes = function () {
  var n = 0;
  for (var g = 0; g < this.__groups.length; ++g) {
    n += this.__groups[g].length;
  }
  return n;
};
ModalGroup.prototype.getNumberOfCodesInGroup = function (group) {
  return this.isGroup(group) ? this.__groups[group - 1].length : 0;
};
ModalGroup.prototype.hasActiveCode = function (group) {
  return this.__active[group] !== undefined;
};
ModalGroup.prototype.getActiveCode = function (group) {
  var code = this.__active[group];
  return code === undefined ? -1 : code;
};
ModalGroup.prototype.isActiveCode = function (code) {
  var group = this.getGroup(code);
  return group !== 0 && this.__active[group] === code;
};
ModalGroup.prototype.makeActiveCode = function (code) {
  var group = this.getGroup(code);
  if (group !== 0) {
    this.__active[group] = code;
  }
};
ModalGroup.prototype.resetGroup = function (group) {
  var code = this.getActiveCode(group);
  delete this.__active[group];
  return code;
};
ModalGroup.prototype.reset = function () {
  this.__active = {};
};
ModalGroup.prototype.format = function (value) {
  if (!this.__enabled) {
    return "";
  }
  var code = this.__fmt ? this.__fmt.getResultingValue(value) : value;
  var group = this.getGroup(code);
  if (group === 0) {
    if (this.__strict) {
      return "";
    }
    if (this.__autoReset) {
      this.reset();
    }
  } else {
    if (!this.__force && this.__active[group] === code) {
      return "";
    }
    this.__active[group] = code;
  }
  var body = this.__fmt ? this.__fmt.format(value) : String(value);
  return this.__prefix + body + this.__suffix;
};
ModalGroup.prototype.enable = function () {
  this.__enabled = true;
};
ModalGroup.prototype.disable = function () {
  this.__enabled = false;
};
ModalGroup.prototype.isEnabled = function () {
  return this.__enabled;
};
ModalGroup.prototype.setForce = function (force) {
  this.__force = !!force;
};
ModalGroup.prototype.setStrict = function (strict) {
  this.__strict = !!strict;
};
ModalGroup.prototype.setAutoReset = function (autoreset) {
  this.__autoReset = !!autoreset;
};
ModalGroup.prototype.setFormatNumber = function (formatNumber) {
  this.__fmt = formatNumber;
};
ModalGroup.prototype.setPrefix = function (prefix) {
  this.__prefix = prefix;
};
ModalGroup.prototype.setSuffix = function (suffix) {
  this.__suffix = suffix;
};
ModalGroup.prototype.setLogUndefined = function (_logundefined) {
  // Diagnostic knob only; no output effect in this runtime.
};

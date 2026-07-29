// 04_format.js — createFormat() / FormatNumber.
//
// THE highest-precision module: every character of NC output funnels
// through here. Value pipeline order is part of the contract:
//
//     cyclic map → scale → offset → round(decimals) → clamp → render
//
// Rounding is half-AWAY-from-zero with an epsilon nudge against binary
// representation error — never toFixed (its ties diverge), and the
// renderer works on an exact integer so no engine float→string
// formatting ever reaches the NC text.

function FormatNumber(spec) {
  this.prefix = "";
  this.suffix = "";
  this.decimals = 6;
  this.forceDecimal = false;
  this.forceSign = false;
  this.scale = 1;
  this.offset = 0;
  this.width = 0;
  this.zeropad = false;
  this.trim = true;
  this.trimLeadZero = false;
  this.trimZeroDecimals = false;
  this.cyclicLimit = 0; // 0 = no cyclic mapping
  this.cyclicSign = 0;
  this.minimum = undefined;
  this.maximum = undefined;
  this.minDigitsLeft = 1;
  this.minDigitsRight = 0;
  this.base = 10;
  this.decimalSymbol = ".";
  this.type = FORMAT_REAL;
  if (spec) {
    for (var key in spec) {
      if (Object.prototype.hasOwnProperty.call(spec, key)) {
        this[key] = spec[key];
      }
    }
    // Type shorthands map onto the flag set (r45877+ style specs).
    if (this.type === FORMAT_INTEGER) {
      this.decimals = 0;
    } else if (this.type === FORMAT_LZS) {
      this.trimLeadZero = true;
    } else if (this.type === FORMAT_TZS) {
      this.trim = true;
    }
  }
}

// Round half away from zero at `decimals`, defeating representation
// error: 2.6745 scaled by 10^3 is 2674.4999999999995 in binary, but the
// intended decimal is exactly half — the relative nudge lifts it over.
FormatNumber.prototype.__round = function (value) {
  var p = Math.pow(10, this.decimals);
  var a = Math.abs(value) * p;
  var nudge = Math.max(1e-9, a * 1e-12);
  var n = Math.round(a + nudge);
  return (value < 0 ? -n : n) / p;
};

/** Value after cyclic mapping, scaling, and offset (pre-round). */
FormatNumber.prototype.remap = function (value) {
  var v = Number(value);
  if (this.cyclicLimit > 0) {
    var limit = this.cyclicLimit;
    if (this.cyclicSign > 0) {
      v = v % limit;
      if (v < 0) {
        v += limit;
      }
    } else if (this.cyclicSign < 0) {
      v = v % limit;
      if (v > 0) {
        v -= limit;
      }
    } else {
      v = v % (2 * limit);
      if (v > limit) {
        v -= 2 * limit;
      } else if (v < -limit) {
        v += 2 * limit;
      }
    }
  }
  return v * this.scale + this.offset;
};

/** The post-map rounded (and clamped) number — what variables compare. */
FormatNumber.prototype.getResultingValue = function (value) {
  var v = this.__round(this.remap(value));
  if (this.minimum !== undefined && v < this.minimum) {
    v = this.minimum;
  }
  if (this.maximum !== undefined && v > this.maximum) {
    v = this.maximum;
  }
  // Normalize -0 so it renders (and compares) as 0.
  return v === 0 ? 0 : v;
};

FormatNumber.prototype.format = function (value) {
  var v = this.getResultingValue(value);
  var negative = v < 0;
  var p = Math.pow(10, this.decimals);
  // v is exactly on the decimals grid (post-round), so this recovers
  // the exact scaled integer.
  var n = Math.round(Math.abs(v) * p);

  var body;
  if (this.base !== 10 && this.decimals === 0) {
    body = n.toString(this.base).toUpperCase();
    if (this.zeropad && this.width > 0) {
      while (body.length < this.width) {
        body = "0" + body;
      }
    }
  } else {
    var digits = String(n);
    while (digits.length < this.decimals + 1) {
      digits = "0" + digits;
    }
    var intPart =
      this.decimals > 0 ? digits.slice(0, digits.length - this.decimals) : digits;
    var fracPart = this.decimals > 0 ? digits.slice(digits.length - this.decimals) : "";

    if (this.trim) {
      fracPart = fracPart.replace(/0+$/, "");
    }
    while (fracPart.length < this.minDigitsRight) {
      fracPart += "0";
    }

    var minLeft = this.minDigitsLeft;
    if (this.zeropad && this.width > minLeft) {
      minLeft = this.width;
    }
    while (intPart.length < minLeft) {
      intPart = "0" + intPart;
    }
    if (this.trimLeadZero && fracPart !== "" && /^0+$/.test(intPart)) {
      intPart = "";
    }

    if (fracPart !== "") {
      body = intPart + this.decimalSymbol + fracPart;
    } else if (this.forceDecimal && !this.trimZeroDecimals) {
      body = intPart + this.decimalSymbol;
    } else {
      body = intPart;
    }
  }

  var sign = "";
  if (negative && n !== 0) {
    sign = "-";
  } else if (this.forceSign && n !== 0) {
    sign = "+";
  }
  return this.prefix + sign + body + this.suffix;
};

/** True if the two values render differently. */
FormatNumber.prototype.areDifferent = function (a, b) {
  return this.getResultingValue(a) !== this.getResultingValue(b);
};

/** True if the value would be non-zero when formatted. */
FormatNumber.prototype.isSignificant = function (value) {
  return this.getResultingValue(value) !== 0;
};

/** Rounding error introduced for the value. */
FormatNumber.prototype.getError = function (value) {
  return this.remap(value) - this.getResultingValue(value);
};

/** Epsilon of the format: 1 for 0 decimals, 0.1 for 1, ... */
FormatNumber.prototype.getMinimumValue = function () {
  return Math.pow(10, -this.decimals);
};

FormatNumber.prototype.getMinimumDecimals = function () {
  return this.minDigitsRight;
};
FormatNumber.prototype.getNumberOfDecimals = function () {
  return this.decimals;
};
FormatNumber.prototype.setNumberOfDecimals = function (decimals) {
  this.decimals = decimals;
};
FormatNumber.prototype.isSignedFormat = function () {
  return !!this.forceSign;
};
FormatNumber.prototype.getBase = function () {
  return this.base;
};
FormatNumber.prototype.setBase = function (base) {
  this.base = base;
};
FormatNumber.prototype.getCyclicLimit = function () {
  return this.cyclicLimit;
};
FormatNumber.prototype.getCyclicSign = function () {
  return this.cyclicSign;
};
FormatNumber.prototype.setCyclicMapping = function (limit, sign) {
  this.cyclicLimit = limit;
  this.cyclicSign = sign;
};
FormatNumber.prototype.getDecimalSymbol = function () {
  return this.decimalSymbol;
};
FormatNumber.prototype.setDecimalSymbol = function (decimalSymbol) {
  this.decimalSymbol = decimalSymbol;
};
FormatNumber.prototype.getForceDecimal = function () {
  return this.forceDecimal;
};
FormatNumber.prototype.setForceDecimal = function (forceDecimal) {
  this.forceDecimal = forceDecimal;
};
FormatNumber.prototype.getForceSign = function () {
  return this.forceSign;
};
FormatNumber.prototype.setForceSign = function (forceSign) {
  this.forceSign = forceSign;
};
FormatNumber.prototype.getMaximum = function () {
  return this.maximum;
};
FormatNumber.prototype.setMaximum = function (value) {
  this.maximum = value;
};
FormatNumber.prototype.getMinimum = function () {
  return this.minimum;
};
FormatNumber.prototype.setMinimum = function (value) {
  this.minimum = value;
};
FormatNumber.prototype.getMinDigitsLeft = function () {
  return this.minDigitsLeft;
};
FormatNumber.prototype.setMinDigitsLeft = function (value) {
  this.minDigitsLeft = value;
};
FormatNumber.prototype.getMinDigitsRight = function () {
  return this.minDigitsRight;
};
FormatNumber.prototype.setMinDigitsRight = function (value) {
  this.minDigitsRight = value;
};
FormatNumber.prototype.getOffset = function () {
  return this.offset;
};
FormatNumber.prototype.setOffset = function (offset) {
  this.offset = offset;
};
FormatNumber.prototype.getPrefix = function () {
  return this.prefix;
};
FormatNumber.prototype.setPrefix = function (prefix) {
  this.prefix = prefix;
};
FormatNumber.prototype.getScale = function () {
  return this.scale;
};
FormatNumber.prototype.setScale = function (scale) {
  this.scale = scale;
};
FormatNumber.prototype.getSuffix = function () {
  return this.suffix;
};
FormatNumber.prototype.setSuffix = function (suffix) {
  this.suffix = suffix;
};
FormatNumber.prototype.getTrimLeadZero = function () {
  return this.trimLeadZero;
};
FormatNumber.prototype.setTrimLeadZero = function (trimLeadZero) {
  this.trimLeadZero = trimLeadZero;
};
FormatNumber.prototype.getTrimZeroDecimals = function () {
  return this.trimZeroDecimals;
};
FormatNumber.prototype.setTrimZeroDecimals = function (trimZeroDecimals) {
  this.trimZeroDecimals = trimZeroDecimals;
};
FormatNumber.prototype.getType = function () {
  return this.type;
};
FormatNumber.prototype.setType = function (type) {
  this.type = type;
  if (type === FORMAT_INTEGER) {
    this.decimals = 0;
  } else if (type === FORMAT_LZS) {
    this.trimLeadZero = true;
  } else if (type === FORMAT_TZS) {
    this.trim = true;
  }
};
FormatNumber.prototype.getWidth = function () {
  return this.width;
};
FormatNumber.prototype.setWidth = function (width) {
  this.width = width;
};
FormatNumber.prototype.getZeroPad = function () {
  return this.zeropad;
};
FormatNumber.prototype.setZeroPad = function (zeropad) {
  this.zeropad = zeropad;
};

function createFormat(specifiers) {
  return new FormatNumber(specifiers);
}

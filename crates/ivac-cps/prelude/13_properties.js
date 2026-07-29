// 13_properties.js — post properties: the `properties` object the post
// declares at top level, user overrides, and the getProperty /
// setProperty pair.
//
// Contract quirks this must honor (FANUC oracle):
// - getProperty reads the LIVE object — posts may replace setProperty
//   wholesale (the FANUC post writes `properties[p].current = v`), so
//   `current` shadows `value` on read.
// - Entries are either rich objects ({title, type, value, ...}) or, in
//   old-style posts, bare values.

var properties = {};

function getProperty(name, defaultValue) {
  var entry = properties[name];
  if (entry === undefined) {
    return defaultValue;
  }
  if (entry !== null && typeof entry === "object") {
    if (entry.current !== undefined) {
      return entry.current;
    }
    if (entry.value !== undefined) {
      return entry.value;
    }
    return defaultValue;
  }
  return entry;
}

function setProperty(name, value) {
  var entry = properties[name];
  if (entry !== null && typeof entry === "object") {
    entry.current = value;
  } else {
    properties[name] = value;
  }
}

/** Driver-side: apply user overrides after the post's top level
 * defined its property sheet. Unknown names are recorded as
 * diagnostics, not errors — a stale saved project must not brick a
 * generate. */
function __ivacApplyPropertyOverrides(overrides) {
  for (var name in overrides) {
    if (!Object.prototype.hasOwnProperty.call(overrides, name)) {
      continue;
    }
    var entry = properties[name];
    if (entry === undefined) {
      __ivac.diag("warning", "unknown post property override: " + name);
      continue;
    }
    if (entry !== null && typeof entry === "object") {
      entry.value = overrides[name];
    } else {
      properties[name] = overrides[name];
    }
  }
}

/** Property sheet extraction for inspect_post → PostMeta. */
function __ivacDescribeProperties() {
  var out = [];
  for (var name in properties) {
    if (!Object.prototype.hasOwnProperty.call(properties, name)) {
      continue;
    }
    var entry = properties[name];
    var meta = {
      name: name,
      title: name,
      description: "",
      kind: { type: "string" },
      default: "",
    };
    var value = entry;
    if (entry !== null && typeof entry === "object") {
      if (entry.title !== undefined) {
        meta.title = String(entry.title);
      }
      if (entry.description !== undefined) {
        meta.description = String(entry.description);
      }
      value = entry.value;
      if (entry.type === "enum" && entry.values instanceof Array) {
        var values = [];
        for (var i = 0; i < entry.values.length; ++i) {
          values.push({
            id: String(entry.values[i].id),
            title: String(entry.values[i].title !== undefined ? entry.values[i].title : entry.values[i].id),
          });
        }
        meta.kind = { type: "enum", values: values };
        meta.default = String(value);
        out.push(meta);
        continue;
      }
    }
    if (typeof value === "boolean") {
      meta.kind = { type: "bool" };
      meta.default = value;
    } else if (typeof value === "number") {
      meta.kind =
        entry !== null && typeof entry === "object" && entry.type === "integer"
          ? { type: "integer" }
          : { type: "number" };
      meta.default = value;
    } else {
      meta.kind = { type: "string" };
      meta.default = value === undefined ? "" : String(value);
    }
    out.push(meta);
  }
  return out;
}

// Old-style compat: some posts read/write propertyDefinitions directly.
var propertyDefinitions = {};

// WCS definitions (Fusion "Multiple WCS" post capability) — stored so
// posts that declare them don't fail; ivacam drives WCS from the IR.
var wcsDefinitions = undefined;

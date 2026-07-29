// 07_text.js — text helpers: localize/subst/conditional/filterText and
// the strict numeric parsers posts use for validation.

function localize(message) {
  // v1: identity (the host hook is the future translation seam).
  if (typeof __ivac.localize === "function") {
    return __ivac.localize(String(message));
  }
  return String(message);
}

/** Replaces %1..%16 in `message` with the remaining arguments. */
function subst(message) {
  var text = String(message);
  var args = arguments;
  return text.replace(/%(\d+)/g, function (match, index) {
    var i = parseInt(index, 10);
    return i >= 1 && i < args.length ? String(args[i]) : match;
  });
}

/** `value` when the condition holds, else "" (word-suppression idiom). */
function conditional(condition, value) {
  return condition ? value : "";
}

/** Keeps only the characters present in `keep`. */
function filterText(text, keep) {
  var s = String(text);
  var out = "";
  for (var i = 0; i < s.length; ++i) {
    if (keep.indexOf(s.charAt(i)) >= 0) {
      out += s.charAt(i);
    }
  }
  return out;
}

/** Strict integer parse — throws when the text is not fully numeric
 * (FANUC catches this to validate program names). */
function getAsInt(text) {
  var s = String(text).replace(/^\s+|\s+$/g, "");
  if (!/^[-+]?\d+$/.test(s)) {
    throw new Error(subst(localize("Invalid integer: %1"), String(text)));
  }
  return parseInt(s, 10);
}

/** Strict float parse — throws when the text is not fully numeric. */
function getAsFloat(text) {
  var s = String(text).replace(/^\s+|\s+$/g, "");
  if (!/^[-+]?(\d+\.?\d*|\.\d+)([eE][-+]?\d+)?$/.test(s)) {
    throw new Error(subst(localize("Invalid number: %1"), String(text)));
  }
  return parseFloat(s);
}

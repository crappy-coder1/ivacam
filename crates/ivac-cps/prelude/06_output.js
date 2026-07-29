// 06_output.js — write/writeln/writeWords/writeWords2/formatWords and
// the redirection sink stack.
//
// Emission model: text flows through `write` in raw chunks. When no
// redirection is active, chunks go to the host (`__ivac.emit`); an
// active redirection captures them into an in-memory buffer instead
// (subprogram assembly). `writeWords*` flatten their arguments
// RECURSIVELY — posts pass strings, arrays, and whole `arguments`
// objects interchangeably — dropping undefined/null/"" words.

var __output = {
  wordSeparator: " ",
  eol: "\n",
  sinks: [], // active redirection buffers, innermost last
  files: {}, // redirectToFile targets, name → text (in-memory only)
  warnedFile: false,
};

function write(text) {
  var s = String(text);
  if (__output.sinks.length > 0) {
    var sink = __output.sinks[__output.sinks.length - 1];
    sink.text += s;
  } else {
    __ivac.emit(s);
  }
}

function writeln(text) {
  write((text === undefined ? "" : String(text)) + __output.eol);
}

function setEOL(eol) {
  __output.eol = eol;
}

function getWordSeparator() {
  return __output.wordSeparator;
}

function setWordSeparator(separator) {
  __output.wordSeparator = separator;
}

// Array-like: has a numeric length but is not a string/function —
// covers Array AND `arguments` objects (writeBlock passes those).
function __isWordList(item) {
  return (
    item !== null &&
    typeof item === "object" &&
    typeof item.length === "number"
  );
}

function __flattenWords(out, item) {
  if (item === undefined || item === null) {
    return;
  }
  if (__isWordList(item)) {
    for (var i = 0; i < item.length; ++i) {
      __flattenWords(out, item[i]);
    }
    return;
  }
  var s = String(item);
  if (s !== "") {
    out.push(s);
  }
}

function formatWords() {
  var words = [];
  __flattenWords(words, arguments);
  return words.join(__output.wordSeparator);
}

function writeWords() {
  var text = formatWords(arguments);
  if (text) {
    writeln(text);
  }
}

// Like writeWords, but the line is only output when argument 2 and
// above produce text — the block-number idiom: the leading "N10" alone
// must not create a line.
function writeWords2() {
  var first = [];
  __flattenWords(first, arguments.length > 0 ? arguments[0] : undefined);
  var rest = [];
  for (var i = 1; i < arguments.length; ++i) {
    __flattenWords(rest, arguments[i]);
  }
  if (rest.length === 0) {
    return;
  }
  writeln(first.concat(rest).join(__output.wordSeparator));
}

// --- redirection ---

function redirectToBuffer() {
  __output.sinks.push({ text: "" });
}

function redirectToFile(path) {
  if (!__output.warnedFile) {
    __output.warnedFile = true;
    __ivac.log(
      "redirectToFile: file output is not supported by ivacam CPS v1 — captured in memory only (" +
        String(path) +
        ")"
    );
  }
  __output.sinks.push({ text: "", file: String(path) });
}

function isRedirecting() {
  return __output.sinks.length > 0;
}

function getRedirectionBuffer() {
  if (__output.sinks.length === 0) {
    return "";
  }
  return __output.sinks[__output.sinks.length - 1].text;
}

function closeRedirection() {
  var sink = __output.sinks.pop();
  if (sink && sink.file !== undefined) {
    __output.files[sink.file] = sink.text;
  }
}

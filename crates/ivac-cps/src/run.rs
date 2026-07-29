//! The crate's public execution API: [`run_post`] (recorded program →
//! NC text) and [`inspect_post`] (top-level eval → property sheet).

use boa_engine::JsValue;

use crate::diag::{Diagnostic, PostError};
use crate::engine::{Engine, PRELUDE};
use crate::ir;
use crate::meta::PostMeta;

/// boa 0.21.1 mis-stores GLOBAL `var` bindings: a `var x = <init>`
/// following expression statements can persist the preceding
/// statement's completion value instead of the initializer (pinned in
/// the `boa_new_after_prototype_assignment_bug` spike test; the same
/// class corrupted `var gFormat = createFormat(...)` into aliasing the
/// post's `properties` object). FUNCTION-scoped locals use a different,
/// correct code path — so the runtime wraps the prelude + post in one
/// IIFE (every declaration becomes a local) and communicates through
/// `__ivacExports`, an explicit global-object property (plain property
/// writes are unaffected). [`SourceMap`] keeps error attribution per
/// original file.
const BUNDLE_NAME: &str = "__ivacbundle__";
const BUNDLE_HEADER: &str = "(function () {\n";
const BUNDLE_FOOTER: &str =
    "\n__ivacExports = { execute: __ivacExecute, inspect: __ivacInspect };\n}).call(this);\n";

/// Line spans of the bundle's constituent files.
struct SourceMap {
    /// `(file name, first line in bundle [1-based], line count)`.
    spans: Vec<(String, usize, usize)>,
}

impl SourceMap {
    fn resolve(&self, bundle_line: usize) -> Option<(&str, usize)> {
        for (name, start, len) in &self.spans {
            if bundle_line >= *start && bundle_line < start + len {
                return Some((name, bundle_line - start + 1));
            }
        }
        None
    }

    /// Rewrite `__ivacbundle__:<line>` stack references and
    /// `line <n>, col` positions into file-relative ones.
    fn remap_message(&self, message: &str) -> String {
        let mut out = String::with_capacity(message.len());
        let mut rest = message;
        loop {
            if let Some(idx) = rest.find(BUNDLE_NAME) {
                out.push_str(&rest[..idx]);
                rest = &rest[idx + BUNDLE_NAME.len()..];
                if let Some(tail) = rest.strip_prefix(':') {
                    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
                    if let Ok(line) = digits.parse::<usize>() {
                        if let Some((file, rel)) = self.resolve(line) {
                            out.push_str(file);
                            out.push(':');
                            out.push_str(&rel.to_string());
                            rest = &tail[digits.len()..];
                            continue;
                        }
                    }
                }
                out.push_str(BUNDLE_NAME);
            } else {
                out.push_str(rest);
                break;
            }
        }
        // Parse errors carry "at line N, col M" without a file.
        if let Some(idx) = out.find("line ") {
            let digits: String = out[idx + 5..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if let Ok(line) = digits.parse::<usize>() {
                if let Some((file, rel)) = self.resolve(line) {
                    out = format!(
                        "{}{file} line {rel}{}",
                        &out[..idx],
                        &out[idx + 5 + digits.len()..]
                    );
                }
            }
        }
        out
    }

    /// The file a parse-error line falls into (parse errors report
    /// bundle-absolute lines and no path).
    fn file_of_message(&self, message: &str) -> Option<&str> {
        let idx = message.find("line ")?;
        let digits: String = message[idx + 5..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let line = digits.parse::<usize>().ok()?;
        self.resolve(line).map(|(file, _)| file)
    }
}

fn build_bundle(script: &str, script_name: &str) -> (String, SourceMap) {
    let mut text = String::from(BUNDLE_HEADER);
    let mut spans = Vec::with_capacity(PRELUDE.len() + 1);
    let mut line = 1 + BUNDLE_HEADER.bytes().filter(|b| *b == b'\n').count();
    for (name, source) in PRELUDE {
        let mut chunk = (*source).to_string();
        if !chunk.ends_with('\n') {
            chunk.push('\n');
        }
        let len = chunk.bytes().filter(|b| *b == b'\n').count();
        spans.push(((*name).to_string(), line, len));
        line += len;
        text.push_str(&chunk);
    }
    let mut chunk = script.to_string();
    if !chunk.ends_with('\n') {
        chunk.push('\n');
    }
    let len = chunk.bytes().filter(|b| *b == b'\n').count();
    spans.push((script_name.to_string(), line, len));
    text.push_str(&chunk);
    text.push_str(BUNDLE_FOOTER);
    (text, SourceMap { spans })
}

/// Call one of the bundle's exported entry points
/// (`__ivacExports.execute` / `.inspect`).
fn call_export(
    engine: &mut Engine,
    name: &str,
    args: &[JsValue],
) -> Result<JsValue, boa_engine::JsError> {
    use boa_engine::{JsNativeError, JsString};
    let ctx = engine.context_mut();
    let exports = ctx
        .global_object()
        .get(JsString::from("__ivacExports"), ctx)?;
    let Some(exports) = exports.as_object() else {
        return Err(JsNativeError::typ()
            .with_message("bundle did not install __ivacExports")
            .into());
    };
    let function = exports.get(JsString::from(name), ctx)?;
    let Some(function) = function.as_callable() else {
        return Err(JsNativeError::typ()
            .with_message(format!("__ivacExports.{name} is not callable"))
            .into());
    };
    function.call(&JsValue::undefined(), args, ctx)
}

/// Evaluate prelude + post as one bundle, mapping failures back to
/// their file: prelude files are ivac bugs, the post's are its own.
fn eval_bundled(
    engine: &mut Engine,
    script: &str,
    script_name: &str,
) -> Result<SourceMap, PostError> {
    let (text, map) = build_bundle(script, script_name);
    match engine.eval_named(BUNDLE_NAME, &text) {
        Ok(_) => Ok(map),
        Err(e) => {
            let raw = e.to_string();
            let message = map.remap_message(&raw);
            let file = map.file_of_message(&raw).unwrap_or(script_name);
            if raw.starts_with("SyntaxError") {
                if file == script_name {
                    Err(PostError::Parse {
                        source_name: script_name.to_string(),
                        message,
                    })
                } else {
                    Err(PostError::PreludeBug {
                        source_name: file.to_string(),
                        message,
                    })
                }
            } else {
                Err(PostError::PostRuntime { message })
            }
        }
    }
}

/// A successful post run.
#[derive(Debug, Clone, PartialEq)]
pub struct PostOutput {
    /// The NC program text, exactly as the post wrote it.
    pub text: String,
    /// File extension the post declares (e.g. `"nc"`).
    pub extension: String,
    /// `"mm"` or `"in"` — the post's output unit.
    pub unit: String,
    pub program_name: String,
    /// Post display name (its `description` global).
    pub description: String,
    /// Warnings collected during the run.
    pub diagnostics: Vec<Diagnostic>,
}

fn collect_diagnostics(engine: &Engine) -> Vec<Diagnostic> {
    engine
        .diagnostics()
        .into_iter()
        .map(Diagnostic::from_pair)
        .collect()
}

/// Run a `.cps` post over a recorded program.
///
/// `properties` is a JSON object of user overrides (name → bare
/// bool/number/string), matching the wire `CpsParamValue` shape.
///
/// # Errors
///
/// See [`PostError`] — parse/runtime failures of the script, deliberate
/// `error()` halts (with partial output), prelude bugs, cancellation.
pub fn run_post(
    script: &str,
    script_name: &str,
    program: &ir::Program,
    properties: &serde_json::Value,
) -> Result<PostOutput, PostError> {
    if program.version != ir::IR_VERSION {
        return Err(PostError::IrVersionMismatch(format!(
            "program is v{}, runtime supports v{}",
            program.version,
            ir::IR_VERSION
        )));
    }
    let mut engine = Engine::new();
    let map = eval_bundled(&mut engine, script, script_name)?;

    let program_json =
        serde_json::to_value(program).map_err(|e| PostError::Contract(e.to_string()))?;
    let program_js = JsValue::from_json(&program_json, engine.context_mut())
        .map_err(|e| PostError::Contract(e.to_string()))?;
    let overrides_js = JsValue::from_json(properties, engine.context_mut())
        .map_err(|e| PostError::Contract(e.to_string()))?;

    let envelope =
        call_export(&mut engine, "execute", &[program_js, overrides_js]).map_err(|e| {
            let message = e.to_string();
            if message.contains("__IVAC_CANCELLED__") {
                PostError::Cancelled
            } else {
                PostError::PostRuntime {
                    message: map.remap_message(&message),
                }
            }
        })?;

    let envelope = envelope
        .to_json(engine.context_mut())
        .map_err(|e| PostError::Contract(e.to_string()))?
        .ok_or_else(|| PostError::Contract("driver returned undefined".into()))?;

    let diagnostics = collect_diagnostics(&engine);
    let ok = envelope["ok"].as_bool().unwrap_or(false);
    if !ok {
        let messages = envelope["errors"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        return Err(PostError::PostErrorCall {
            messages,
            diagnostics,
            partial_output: engine.output_text(),
        });
    }

    let field = |key: &str| envelope[key].as_str().unwrap_or_default().to_string();
    Ok(PostOutput {
        text: engine.output_text(),
        extension: field("extension"),
        unit: field("unit"),
        program_name: field("programName"),
        description: field("description"),
        diagnostics,
    })
}

/// Evaluate only a post's top level and extract its metadata (identity
/// + property sheet) for UI property forms.
///
/// # Errors
///
/// [`PostError::Parse`] / [`PostError::PostRuntime`] when the script's
/// top level fails, [`PostError::PreludeBug`] for runtime-side faults.
pub fn inspect_post(script: &str, script_name: &str) -> Result<PostMeta, PostError> {
    let mut engine = Engine::new();
    let map = eval_bundled(&mut engine, script, script_name)?;

    let meta = call_export(&mut engine, "inspect", &[]).map_err(|e| PostError::PostRuntime {
        message: map.remap_message(&e.to_string()),
    })?;
    let meta = meta
        .to_json(engine.context_mut())
        .map_err(|e| PostError::Contract(e.to_string()))?
        .ok_or_else(|| PostError::Contract("inspect returned undefined".into()))?;
    serde_json::from_value(meta).map_err(|e| PostError::Contract(e.to_string()))
}

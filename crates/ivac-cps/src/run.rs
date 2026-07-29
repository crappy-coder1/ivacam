//! The crate's public execution API: [`run_post`] (recorded program →
//! NC text) and [`inspect_post`] (top-level eval → property sheet).

use boa_engine::{JsError, JsValue};

use crate::diag::{Diagnostic, PostError};
use crate::engine::{Engine, PRELUDE};
use crate::ir;
use crate::meta::PostMeta;

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

fn eval_prelude(engine: &mut Engine) -> Result<(), PostError> {
    for (name, source) in PRELUDE {
        engine
            .eval_named(name, source)
            .map_err(|e| PostError::PreludeBug {
                source_name: (*name).to_string(),
                message: e.to_string(),
            })?;
    }
    Ok(())
}

fn classify_script_error(source_name: &str, error: &JsError) -> PostError {
    let message = error.to_string();
    if message.starts_with("SyntaxError") {
        PostError::Parse {
            source_name: source_name.to_string(),
            message,
        }
    } else {
        PostError::PostRuntime { message }
    }
}

fn eval_script(engine: &mut Engine, source_name: &str, script: &str) -> Result<(), PostError> {
    engine
        .eval_named(source_name, script)
        .map(|_| ())
        .map_err(|e| classify_script_error(source_name, &e))
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
    eval_prelude(&mut engine)?;
    eval_script(&mut engine, script_name, script)?;

    let program_json =
        serde_json::to_value(program).map_err(|e| PostError::Contract(e.to_string()))?;
    let program_js = JsValue::from_json(&program_json, engine.context_mut())
        .map_err(|e| PostError::Contract(e.to_string()))?;
    let overrides_js = JsValue::from_json(properties, engine.context_mut())
        .map_err(|e| PostError::Contract(e.to_string()))?;

    let envelope = engine
        .call_global("__ivacExecute", &[program_js, overrides_js])
        .map_err(|e| {
            let message = e.to_string();
            if message.contains("__IVAC_CANCELLED__") {
                PostError::Cancelled
            } else {
                PostError::PostRuntime { message }
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
    eval_prelude(&mut engine)?;
    eval_script(&mut engine, script_name, script)?;

    let meta = engine
        .call_global("__ivacInspect", &[])
        .map_err(|e| PostError::PostRuntime {
            message: e.to_string(),
        })?;
    let meta = meta
        .to_json(engine.context_mut())
        .map_err(|e| PostError::Contract(e.to_string()))?
        .ok_or_else(|| PostError::Contract("inspect returned undefined".into()))?;
    serde_json::from_value(meta).map_err(|e| PostError::Contract(e.to_string()))
}

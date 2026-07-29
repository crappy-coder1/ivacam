//! boa [`Context`] lifecycle: host bindings, script evaluation, entry-point
//! calls.
//!
//! The host surface is deliberately tiny: `__ivac` carries `emit`
//! (NC-text sink), `diag` (diagnostics), `log`, `localize`,
//! `abortCheck` and a deterministic `now`. Everything else the `.cps`
//! runtime needs lives in the JS prelude ([`PRELUDE`]); the public
//! execution API is [`crate::run_post`] / [`crate::inspect_post`].

use std::path::Path;

use boa_engine::gc::{Gc, GcRefCell};
use boa_engine::object::ObjectInitializer;
use boa_engine::property::Attribute;
use boa_engine::{Context, JsError, JsNativeError, JsString, JsValue, NativeFunction, Source};

/// Shared NC-output sink `__ivac.emit` appends to. Entries are raw text
/// CHUNKS (the prelude's `write` may send partial lines); join them
/// verbatim for the program text.
///
/// `Gc<GcRefCell<..>>` rather than `Rc<RefCell<..>>`: closure captures
/// handed to boa must implement `Trace`, and this keeps the host
/// function on the SAFE `from_copy_closure_with_captures` constructor
/// (the workspace lints on `unsafe_code`).
pub type EmitSink = Gc<GcRefCell<Vec<String>>>;

/// The runtime prelude, in eval order. Each entry is
/// `(source name, source)` — the name shows up in boa diagnostics, so a
/// failure inside the prelude is attributable to its module (an
/// ivac bug) rather than to the user's post.
pub const PRELUDE: &[(&str, &str)] = &[
    ("00_polyfill.js", include_str!("../prelude/00_polyfill.js")),
    (
        "01_constants.js",
        include_str!("../prelude/01_constants.js"),
    ),
    (
        "02_vector_matrix.js",
        include_str!("../prelude/02_vector_matrix.js"),
    ),
    ("03_euler.js", include_str!("../prelude/03_euler.js")),
    ("04_format.js", include_str!("../prelude/04_format.js")),
    (
        "05_variables.js",
        include_str!("../prelude/05_variables.js"),
    ),
    ("06_output.js", include_str!("../prelude/06_output.js")),
    ("07_text.js", include_str!("../prelude/07_text.js")),
    (
        "08_tool_section.js",
        include_str!("../prelude/08_tool_section.js"),
    ),
    ("09_machine.js", include_str!("../prelude/09_machine.js")),
    ("10_state.js", include_str!("../prelude/10_state.js")),
    ("11_circular.js", include_str!("../prelude/11_circular.js")),
    ("12_cycles.js", include_str!("../prelude/12_cycles.js")),
    (
        "13_properties.js",
        include_str!("../prelude/13_properties.js"),
    ),
    ("14_stubs.js", include_str!("../prelude/14_stubs.js")),
    ("15_driver.js", include_str!("../prelude/15_driver.js")),
];

/// Diagnostics sink: `(severity, message)` pairs pushed by the
/// prelude's `__ivac.diag` (severity is `"error"` or `"warning"`).
pub type DiagSink = Gc<GcRefCell<Vec<(String, String)>>>;

/// A boa context with the `__ivac` host object installed.
pub struct Engine {
    context: Context,
    sink: EmitSink,
    diags: DiagSink,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("emitted_lines", &self.sink.borrow().len())
            .field("diagnostics", &self.diags.borrow().len())
            .finish_non_exhaustive()
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// Build a fresh context and install `__ivac` with the host API.
    ///
    /// # Panics
    ///
    /// Never in practice: the only fallible step is registering
    /// `__ivac` on a context this function just created, which cannot
    /// already carry that binding.
    #[must_use]
    pub fn new() -> Self {
        let mut context = Context::default();
        let sink: EmitSink = Gc::new(GcRefCell::new(Vec::new()));

        let emit = NativeFunction::from_copy_closure_with_captures(
            |_this, args, sink, ctx| {
                let text = args
                    .first()
                    .cloned()
                    .unwrap_or_default()
                    .to_string(ctx)?
                    .to_std_string_escaped();
                sink.borrow_mut().push(text);
                Ok(JsValue::undefined())
            },
            sink.clone(),
        );

        let log = NativeFunction::from_copy_closure(|_this, args, ctx| {
            let text = args
                .first()
                .cloned()
                .unwrap_or_default()
                .to_string(ctx)?
                .to_std_string_escaped();
            tracing::debug!(target: "ivac_cps::post", "{text}");
            Ok(JsValue::undefined())
        });
        // v1 localization is identity — the hook exists so the prelude
        // has one stable seam when real translation arrives.
        let localize = NativeFunction::from_copy_closure(|_this, args, _ctx| {
            Ok(args.first().cloned().unwrap_or_default())
        });

        let diags: DiagSink = Gc::new(GcRefCell::new(Vec::new()));
        let diag = NativeFunction::from_copy_closure_with_captures(
            |_this, args, diags, ctx| {
                let severity = args
                    .first()
                    .cloned()
                    .unwrap_or_default()
                    .to_string(ctx)?
                    .to_std_string_escaped();
                let message = args
                    .get(1)
                    .cloned()
                    .unwrap_or_default()
                    .to_string(ctx)?
                    .to_std_string_escaped();
                diags.borrow_mut().push((severity, message));
                Ok(JsValue::undefined())
            },
            diags.clone(),
        );
        // Cancellation probe — always "keep going" until the pipeline
        // wires a real token through (cps.7/cps.8 budgets).
        let abort_check =
            NativeFunction::from_copy_closure(|_this, _args, _ctx| Ok(JsValue::from(false)));
        // Deterministic clock: posts occasionally timestamp headers;
        // a fixed epoch keeps output byte-stable across runs.
        let now = NativeFunction::from_copy_closure(|_this, _args, _ctx| Ok(JsValue::from(0)));

        let ivac = ObjectInitializer::new(&mut context)
            .function(emit, JsString::from("emit"), 1)
            .function(log, JsString::from("log"), 1)
            .function(localize, JsString::from("localize"), 1)
            .function(diag, JsString::from("diag"), 2)
            .function(abort_check, JsString::from("abortCheck"), 0)
            .function(now, JsString::from("now"), 0)
            .build();
        context
            .register_global_property(JsString::from("__ivac"), ivac, Attribute::all())
            .expect("fresh context: __ivac cannot already exist");

        Self {
            context,
            sink,
            diags,
        }
    }

    /// Diagnostics pushed via `__ivac.diag` so far, as
    /// `(severity, message)` pairs.
    #[must_use]
    pub fn diagnostics(&self) -> Vec<(String, String)> {
        self.diags.borrow().clone()
    }

    /// Evaluate the runtime prelude ([`PRELUDE`]) in order. An error
    /// here is an ivac bug, not a post bug — the failing module's name
    /// is in the source position of the returned error.
    ///
    /// # Errors
    ///
    /// Returns the engine error of the first prelude module that fails
    /// to parse or evaluate.
    pub fn eval_prelude(&mut self) -> Result<(), JsError> {
        for (name, source) in PRELUDE {
            self.eval_named(name, source)?;
        }
        Ok(())
    }

    /// Evaluate `src` under `name` — the name shows up in boa
    /// diagnostics so a broken prelude file or user post is
    /// attributable.
    ///
    /// # Errors
    ///
    /// Returns the engine error when `src` fails to parse or throws
    /// during evaluation.
    pub fn eval_named(&mut self, name: &str, src: &str) -> Result<JsValue, JsError> {
        self.context
            .eval(Source::from_bytes(src.as_bytes()).with_path(Path::new(name)))
    }

    /// Call a global function by name (the `.cps` entry-point pattern:
    /// resolve the CURRENT binding at call time, so posts may redefine
    /// entry points at will).
    ///
    /// # Errors
    ///
    /// Returns a `TypeError` when the global is missing or not
    /// callable, or the thrown value when the call itself throws.
    pub fn call_global(&mut self, name: &str, args: &[JsValue]) -> Result<JsValue, JsError> {
        let value = self
            .context
            .global_object()
            .get(JsString::from(name), &mut self.context)?;
        let Some(function) = value.as_callable() else {
            return Err(JsNativeError::typ()
                .with_message(format!("global '{name}' is not callable"))
                .into());
        };
        function.call(&JsValue::undefined(), args, &mut self.context)
    }

    /// Raw chunks collected by `__ivac.emit` so far.
    #[must_use]
    pub fn output(&self) -> Vec<String> {
        self.sink.borrow().clone()
    }

    /// The emitted program text — all chunks joined verbatim.
    #[must_use]
    pub fn output_text(&self) -> String {
        self.sink.borrow().concat()
    }

    /// Direct context access for tests and future engine layers.
    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.context
    }

    /// A second handle to the emit sink (tests register extra host
    /// functions over it; `Gc` handles are cheap clones).
    #[must_use]
    pub fn sink(&self) -> EmitSink {
        self.sink.clone()
    }
}

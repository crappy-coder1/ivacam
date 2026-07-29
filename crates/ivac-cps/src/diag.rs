//! Diagnostics + the crate's error surface.
//!
//! Attribution matters here: a failure in a prelude module is an ivac
//! bug ([`PostError::PreludeBug`]), a failure in the user's `.cps` is
//! theirs ([`PostError::Parse`] / [`PostError::PostRuntime`]), and a
//! post calling `error()` is a deliberate halt with partial output
//! preserved ([`PostError::PostErrorCall`]).

use serde::{Deserialize, Serialize};

/// One non-fatal message a post run produced (`warning()` calls,
/// kernel warnings like unsupported records).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

impl Diagnostic {
    pub(crate) fn from_pair(pair: (String, String)) -> Self {
        Self {
            severity: if pair.0 == "error" {
                Severity::Error
            } else {
                Severity::Warning
            },
            message: pair.1,
        }
    }
}

/// Failure surface of [`crate::run_post`] / [`crate::inspect_post`].
#[derive(Debug, thiserror::Error)]
pub enum PostError {
    /// The `.cps` source failed to parse. boa's message carries
    /// `line N, col M`; `source_name` is attached here because parse
    /// errors don't carry the path themselves.
    #[error("post script {source_name} failed to parse: {message}")]
    Parse {
        source_name: String,
        message: String,
    },
    /// A JS exception escaped the post (a `validate()` throw, a bug in
    /// the post, …). The message includes boa's stack frame with
    /// source name + line for runtime errors.
    #[error("post runtime error: {message}")]
    PostRuntime { message: String },
    /// The post called `error()` — a deliberate halt. Output produced
    /// before the halt is preserved for display next to the messages.
    #[error("post reported: {}", messages.join("; "))]
    PostErrorCall {
        messages: Vec<String>,
        diagnostics: Vec<Diagnostic>,
        partial_output: String,
    },
    /// A prelude module failed to evaluate — an ivac bug, never the
    /// post's fault. Please report.
    #[error("prelude bug in {source_name}: {message}")]
    PreludeBug {
        source_name: String,
        message: String,
    },
    /// The recorded program's IR version doesn't match this runtime.
    #[error("program IR version mismatch: {0}")]
    IrVersionMismatch(String),
    /// The run envelope came back in a shape the runtime didn't
    /// expect — an ivac bug in the driver contract.
    #[error("driver contract violation: {0}")]
    Contract(String),
    /// The host cancelled the run.
    #[error("post execution cancelled")]
    Cancelled,
}

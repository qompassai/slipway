//! Shared outcome types for tool dispatch.
//!
//! Failure taxonomy (fleet-wide): argument-shape failures are
//! protocol errors (`-32602`, `is_params`); domain failures —
//! gate refusals, missing state, failed checks — return as
//! `isError` content carrying a stable machine-readable `code`.

/// One tool failure: a stable code, a human message, and whether
/// it is an argument-shape (protocol) failure.
#[derive(Debug)]
pub struct Failure {
    pub code: String,
    pub message: String,
    pub is_params: bool,
}

impl Failure {
    pub fn domain(code: &str, message: impl Into<String>) -> Self {
        Failure {
            code: code.to_string(),
            message: message.into(),
            is_params: false,
        }
    }

    pub fn params(message: impl Into<String>) -> Self {
        Failure {
            code: "invalid_params".to_string(),
            message: message.into(),
            is_params: true,
        }
    }
}

pub type ToolResult = Result<serde_json::Value, Failure>;

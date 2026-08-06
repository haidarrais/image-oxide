//! Error registry — exactly the `IPC-20` table. Client maps unknown codes to
//! non-retryable `INTERNAL` (`IPC-21`), so the enum is `#[non_exhaustive]`.

use serde::Serialize;

/// Protocol version pinned by the hello/ack handshake (`IPC-06`–`IPC-11`).
/// The protocol is semver-frozen at 1.0.0 and additive-only after (`IPC-11`).
pub const PROTOCOL_VERSION: &str = "1.0.0";

pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Server-side error code — mirrors `IPC-20` exactly, minus `INVALID_REQUEST`
/// (a framing concern, handled before op dispatch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[non_exhaustive]
pub enum ErrorCode {
    InvalidRequest,
    FrameTooLarge,
    ProtocolVersionMismatch,
    AccessDenied,
    InputNotFound,
    InputUnreadable,
    DecodeFailed,
    UnsupportedOperation,
    OpFailed,
    OutputWriteFailed,
    DaemonOverloaded,
    Internal,
}

/// JSON-serializable error body for the failure response shape (`IPC-18`).
#[derive(Debug, Serialize)]
pub struct OxideError {
    pub code: ErrorCode,
    pub message: String,
    pub op_index: Option<usize>,
}

impl OxideError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            op_index: None,
        }
    }

    pub fn with_op_index(mut self, op_index: usize) -> Self {
        self.op_index = Some(op_index);
        self
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    pub fn op_failed(op_index: usize, message: impl Into<String>) -> Self {
        Self::new(ErrorCode::OpFailed, message).with_op_index(op_index)
    }
}

impl From<anyhow::Error> for OxideError {
    fn from(err: anyhow::Error) -> Self {
        Self::internal(err.to_string())
    }
}

/// `IPC-20` retryable set: `INPUT_UNREADABLE`, `OUTPUT_WRITE_FAILED`,
/// `DAEMON_OVERLOADED`. Client applies capped exponential backoff (`IPC-22`).
pub fn is_retryable(code: ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::InputUnreadable | ErrorCode::OutputWriteFailed | ErrorCode::DaemonOverloaded
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_21_unknown_codes_are_internal() {
        // The client's contract is: unknown code → non-retryable `INTERNAL`.
        // Here we pin that `INTERNAL` itself is never marked retryable.
        assert!(!is_retryable(ErrorCode::Internal));
        assert!(!is_retryable(ErrorCode::OpFailed));
    }

    #[test]
    fn ipc_22_retryable_codes_marked() {
        for code in [
            ErrorCode::InputUnreadable,
            ErrorCode::OutputWriteFailed,
            ErrorCode::DaemonOverloaded,
        ] {
            assert!(is_retryable(code), "{code:?} must be retryable");
        }
    }
}

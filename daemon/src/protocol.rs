//! Message shapes for the 001 wire protocol. Serde's default behavior skips
//! unknown fields — that is the forward-compatible additive evolution of
//! `IPC-10`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{ErrorCode, OxideError, PROTOCOL_VERSION, SERVER_VERSION};

// ---- handshake (`IPC-06`–`IPC-08`) ----

#[derive(Debug, Deserialize)]
pub struct Hello {
    #[serde(rename = "protocol_version")]
    pub protocol_version: String,
    #[serde(rename = "client_name")]
    pub client_name: String,
}

#[derive(Debug, Serialize)]
pub struct Ack {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(rename = "protocol_version")]
    pub protocol_version: &'static str,
    #[serde(rename = "server_version")]
    pub server_version: &'static str,
}

impl Default for Ack {
    fn default() -> Self {
        Self::new()
    }
}

impl Ack {
    pub fn new() -> Self {
        Self {
            kind: "ack",
            protocol_version: PROTOCOL_VERSION,
            server_version: SERVER_VERSION,
        }
    }
}

// ---- request / response (`IPC-12`, `IPC-17`, `IPC-18`) ----

#[derive(Debug, Deserialize)]
pub struct InputRef {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct OutputRef {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct Request {
    pub id: String,
    pub ops: Vec<Value>,
    pub input: InputRef,
    pub output: OutputRef,
    /// OPS-05: lossy encode quality (1–100). Optional, defaults server-side.
    #[serde(default)]
    pub quality: Option<u8>,
}

#[derive(Debug, Serialize)]
pub struct Success {
    pub id: String,
    pub status: &'static str,
    #[serde(rename = "output_path")]
    pub output_path: String,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    #[serde(rename = "duration_ms")]
    pub duration_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct Failure {
    pub id: String,
    pub status: &'static str,
    pub error: OxideError,
}

// ---- parsing ----

/// `IPC-04`: a frame's JSON must be a single object; malformed → `INVALID_REQUEST`,
/// connection stays open. `IPC-12`: `ops` is a non-empty array.
pub fn parse_request(body: &[u8]) -> Result<Request, OxideError> {
    let request: Request = serde_json::from_slice(body).map_err(|e| {
        OxideError::new(ErrorCode::InvalidRequest, format!("invalid request: {e}"))
    })?;
    if request.ops.is_empty() {
        return Err(OxideError::new(
            ErrorCode::InvalidRequest,
            "ops[] must be a non-empty array (`IPC-12`)",
        ));
    }
    Ok(request)
}

/// Decodes the mandatory first message (`IPC-06`). Anything that is not a
/// `hello` object is `INVALID_REQUEST` — a pre-ack request is rejected
/// (`IPC-09`).
pub fn parse_hello(body: &[u8]) -> Result<Hello, OxideError> {
    serde_json::from_slice(body).map_err(|e| {
        OxideError::new(
            ErrorCode::InvalidRequest,
            format!("expected hello before ack (`IPC-06`, `IPC-09`): {e}"),
        )
    })
}

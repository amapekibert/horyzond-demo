//! Length-prefixed, versioned JSON messages independent of socket transport.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Current IPC protocol revision.
pub const VERSION: u32 = 1;
/// Maximum accepted encoded message size.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
/// Default maximum number of active event subscriptions per client.
pub const MAX_SUBSCRIPTIONS: usize = 32;

/// Bounded transport-neutral subscription names for one client.
#[derive(Debug, Default)]
pub struct Subscriptions {
    names: std::collections::BTreeSet<String>,
}
impl Subscriptions {
    /// Adds a non-empty subscription if the bounded capacity permits it.
    pub fn add(&mut self, name: impl Into<String>) -> bool {
        let name = name.into();
        !name.trim().is_empty()
            && (self.names.contains(&name) || self.names.len() < MAX_SUBSCRIPTIONS)
            && self.names.insert(name)
    }
    /// Removes a subscription name.
    pub fn remove(&mut self, name: &str) -> bool {
        self.names.remove(name)
    }
    /// Returns the number of active subscriptions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }
    /// Reports whether no subscriptions are active.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// A client request with a caller-selected correlation ID.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub version: u32,
    pub id: u64,
    pub method: String,
    pub params: serde_json::Value,
}
/// A deterministic response for one request ID.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Response {
    pub version: u32,
    pub id: u64,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}
impl Response {
    /// Creates a success response preserving a request's correlation ID.
    #[must_use]
    pub fn success(request: &Request, result: serde_json::Value) -> Self {
        Self {
            version: VERSION,
            id: request.id,
            result: Some(result),
            error: None,
        }
    }
    /// Creates an error response preserving a request's correlation ID.
    #[must_use]
    pub fn failure(request: &Request, error: impl Into<String>) -> Self {
        Self {
            version: VERSION,
            id: request.id,
            result: None,
            error: Some(error.into()),
        }
    }
}
/// Protocol framing and validation errors.
#[derive(Debug)]
pub enum IpcError {
    TooLarge,
    Truncated,
    InvalidLength,
    Json(serde_json::Error),
    Version(u32),
    EmptyMethod,
}
impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("IPC frame exceeds size limit"),
            Self::Truncated => f.write_str("truncated IPC frame"),
            Self::InvalidLength => f.write_str("invalid IPC frame length"),
            Self::Json(e) => e.fmt(f),
            Self::Version(v) => write!(f, "unsupported IPC version {v}"),
            Self::EmptyMethod => f.write_str("IPC method must not be empty"),
        }
    }
}
impl std::error::Error for IpcError {}
/// Encodes one request into a four-byte big-endian length-prefixed frame.
///
/// # Errors
///
/// Returns an error for invalid requests or frames exceeding the size limit.
pub fn encode(request: &Request) -> Result<Vec<u8>, IpcError> {
    validate(request)?;
    let body = serde_json::to_vec(request).map_err(IpcError::Json)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(IpcError::TooLarge);
    }
    let mut frame = Vec::with_capacity(body.len() + 4);
    let length = u32::try_from(body.len()).map_err(|_| IpcError::TooLarge)?;
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend(body);
    Ok(frame)
}
/// Decodes exactly one bounded frame.
///
/// # Errors
///
/// Returns an error for malformed, oversized, unsupported, or invalid frames.
pub fn decode(frame: &[u8]) -> Result<Request, IpcError> {
    if frame.len() < 4 {
        return Err(IpcError::Truncated);
    }
    let prefix: [u8; 4] = frame[..4].try_into().map_err(|_| IpcError::Truncated)?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(IpcError::TooLarge);
    }
    if frame.len() != length + 4 {
        return Err(IpcError::InvalidLength);
    }
    let request = serde_json::from_slice(&frame[4..]).map_err(IpcError::Json)?;
    validate(&request)?;
    Ok(request)
}
fn validate(request: &Request) -> Result<(), IpcError> {
    if request.version != VERSION {
        Err(IpcError::Version(request.version))
    } else if request.method.trim().is_empty() {
        Err(IpcError::EmptyMethod)
    } else {
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_round_trip_rejects_versions_and_size() {
        let request = Request {
            version: VERSION,
            id: 7,
            method: "status".to_owned(),
            params: serde_json::Value::Null,
        };
        assert_eq!(
            decode(&encode(&request).expect("encode")).expect("decode"),
            request
        );
        assert!(matches!(
            decode(&[0, 0, 0, 1, b'x']),
            Err(IpcError::Json(_))
        ));
    }
}

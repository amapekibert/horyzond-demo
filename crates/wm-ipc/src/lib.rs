//! Length-prefixed, versioned JSON messages independent of socket transport.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

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
    Io(io::Error),
    UnsupportedTransport,
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
            Self::Io(error) => error.fmt(f),
            Self::UnsupportedTransport => f.write_str("IPC requires a Unix-domain socket platform"),
        }
    }
}
impl std::error::Error for IpcError {}
impl From<io::Error> for IpcError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
/// Encodes one request into a four-byte big-endian length-prefixed frame.
///
/// # Errors
///
/// Returns an error for invalid requests or frames exceeding the size limit.
pub fn encode(request: &Request) -> Result<Vec<u8>, IpcError> {
    validate(request)?;
    encode_value(request)
}

/// Encodes a response with the same bounded framing as requests.
///
/// # Errors
///
/// Returns an error when the response exceeds the frame size limit.
pub fn encode_response(response: &Response) -> Result<Vec<u8>, IpcError> {
    if response.version != VERSION {
        return Err(IpcError::Version(response.version));
    }
    encode_value(response)
}

fn encode_value(value: &impl Serialize) -> Result<Vec<u8>, IpcError> {
    let body = serde_json::to_vec(value).map_err(IpcError::Json)?;
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
    let request = decode_value(frame)?;
    validate(&request)?;
    Ok(request)
}

/// Decodes one framed response.
///
/// # Errors
///
/// Returns an error for malformed, oversized, or unsupported frames.
pub fn decode_response(frame: &[u8]) -> Result<Response, IpcError> {
    let response: Response = decode_value(frame)?;
    if response.version != VERSION {
        return Err(IpcError::Version(response.version));
    }
    Ok(response)
}

fn decode_value<T: for<'de> Deserialize<'de>>(frame: &[u8]) -> Result<T, IpcError> {
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
    serde_json::from_slice(&frame[4..]).map_err(IpcError::Json)
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

/// A same-user Unix-domain IPC listener. The listener is non-blocking; each
/// accepted stream receives short read and write deadlines so an incomplete
/// client cannot hold a coordinator loop indefinitely.
#[derive(Debug)]
pub struct IpcServer {
    #[cfg(unix)]
    listener: UnixListener,
    path: PathBuf,
    timeout: Duration,
}
impl IpcServer {
    /// Binds an owner-only IPC socket at `path`.
    ///
    /// The parent directory must already be private to the current user. A
    /// stale socket at the exact requested path is removed before binding.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket cannot be created or secured.
    pub fn bind(path: impl Into<PathBuf>) -> Result<Self, IpcError> {
        let path = path.into();
        #[cfg(unix)]
        {
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
            let listener = UnixListener::bind(&path)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            listener.set_nonblocking(true)?;
            Ok(Self {
                listener,
                path,
                timeout: Duration::from_millis(100),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(IpcError::UnsupportedTransport)
        }
    }
    /// Returns the owned socket path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Accepts and handles at most one complete request without blocking.
    ///
    /// A would-block accept returns `Ok(false)`. Malformed clients receive a
    /// protocol error where framing permits it, then are disconnected.
    ///
    /// # Errors
    ///
    /// Returns an error only for listener-level failures.
    pub fn poll(&self, handler: impl FnOnce(Request) -> Response) -> Result<bool, IpcError> {
        #[cfg(unix)]
        match self.listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(self.timeout))?;
                stream.set_write_timeout(Some(self.timeout))?;
                let response = match read_request(&mut stream) {
                    Ok(request) => handler(request),
                    Err(error) => Response {
                        version: VERSION,
                        id: 0,
                        result: None,
                        error: Some(error.to_string()),
                    },
                };
                let _ = write_frame(&mut stream, &encode_response(&response)?);
                Ok(true)
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(false),
            Err(error) => Err(error.into()),
        }
        #[cfg(not(unix))]
        {
            let _ = handler;
            Err(IpcError::UnsupportedTransport)
        }
    }
}
impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Sends one bounded request to a same-user Unix-domain server.
///
/// # Errors
///
/// Returns an error for connection, deadline, framing, or protocol failures.
pub fn request(path: &Path, request: &Request) -> Result<Response, IpcError> {
    #[cfg(unix)]
    {
        let mut stream = UnixStream::connect(path)?;
        let timeout = Duration::from_millis(250);
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        write_frame(&mut stream, &encode(request)?)?;
        read_response(&mut stream)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, request);
        Err(IpcError::UnsupportedTransport)
    }
}

fn write_frame(stream: &mut impl Write, frame: &[u8]) -> Result<(), IpcError> {
    stream.write_all(frame)?;
    stream.flush()?;
    Ok(())
}

fn read_request(stream: &mut impl Read) -> Result<Request, IpcError> {
    let frame = read_frame(stream)?;
    decode(&frame)
}

fn read_response(stream: &mut impl Read) -> Result<Response, IpcError> {
    let frame = read_frame(stream)?;
    decode_response(&frame)
}

fn read_frame(stream: &mut impl Read) -> Result<Vec<u8>, IpcError> {
    let mut prefix = [0_u8; 4];
    stream.read_exact(&mut prefix)?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(IpcError::TooLarge);
    }
    let mut frame = Vec::with_capacity(length + 4);
    frame.extend(prefix);
    frame.resize(length + 4, 0);
    stream.read_exact(&mut frame[4..])?;
    Ok(frame)
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
    #[test]
    fn subscriptions_are_idempotent_and_bounded() {
        let mut subscriptions = Subscriptions::default();
        assert!(subscriptions.add("workspace"));
        assert!(!subscriptions.add("workspace"));
        for index in 1..MAX_SUBSCRIPTIONS {
            assert!(subscriptions.add(format!("event-{index}")));
        }
        assert_eq!(subscriptions.len(), MAX_SUBSCRIPTIONS);
        assert!(!subscriptions.add("overflow"));
        assert!(subscriptions.remove("workspace"));
        assert!(subscriptions.add("replacement"));
    }
    #[cfg(unix)]
    #[test]
    fn owner_only_transport_preserves_request_ids() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{SystemTime, UNIX_EPOCH};

        let path = std::env::temp_dir().join(format!(
            "horyzond-ipc-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let server = IpcServer::bind(&path).expect("server");
        assert_eq!(
            std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let worker = std::thread::spawn(move || {
            loop {
                if server
                    .poll(|request| Response::success(&request, serde_json::json!({ "ok": true })))
                    .expect("poll")
                {
                    break;
                }
                std::thread::yield_now();
            }
        });
        let request = Request {
            version: VERSION,
            id: 42,
            method: "status".to_owned(),
            params: serde_json::Value::Null,
        };
        let response = super::request(&path, &request).expect("response");
        assert_eq!(response.id, request.id);
        assert_eq!(response.result, Some(serde_json::json!({ "ok": true })));
        worker.join().expect("worker");
    }
}

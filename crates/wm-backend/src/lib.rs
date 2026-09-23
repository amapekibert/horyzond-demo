//! Contracts for window-system adapters.
//!
//! A backend owns protocol state and translates it to normalized events. It
//! does not depend on a renderer or expose protocol-native objects to core.

use std::fmt;

use wm_types::{Capabilities, OutputId, OutputInfo, Rect, WindowId, WindowMetadata};

/// A geometry request awaiting the client's protocol acknowledgement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConfigureTransaction {
    /// Window whose geometry is being negotiated.
    pub window: WindowId,
    /// Adapter-assigned serial, opaque outside the configure lifecycle.
    pub serial: u64,
    /// Requested logical geometry.
    pub geometry: Rect,
}

/// Events emitted by a window-system adapter in coordinator order.
#[derive(Clone, Debug, PartialEq)]
pub enum BackendEvent {
    /// A new output became available.
    OutputAdded(OutputInfo),
    /// An output disappeared.
    OutputRemoved(OutputId),
    /// A client window became manageable.
    WindowMapped(WindowId),
    /// Metadata changed for an already managed window.
    WindowMetadataChanged(WindowId, WindowMetadata),
    /// A client window disappeared.
    WindowUnmapped(WindowId),
}

/// A platform-neutral window-system adapter.
pub trait WindowSystem {
    /// Initializes protocol resources and makes initial output state available.
    ///
    /// # Errors
    ///
    /// Returns an adapter-specific initialization failure.
    fn initialize(&mut self) -> Result<(), BackendError>;
    /// Returns events accumulated since the preceding poll in deterministic order.
    ///
    /// # Errors
    ///
    /// Returns an adapter-specific polling failure.
    fn poll_events(&mut self) -> Result<Vec<BackendEvent>, BackendError>;
    /// Returns current outputs as normalized descriptors.
    fn outputs(&self) -> &[OutputInfo];
    /// Returns available features.
    fn capabilities(&self) -> Capabilities;
    /// Requests a client geometry and returns its pending configure transaction.
    ///
    /// # Errors
    ///
    /// Returns an adapter-specific request failure, including an unknown window.
    fn request_window_geometry(
        &mut self,
        window: WindowId,
        geometry: Rect,
    ) -> Result<ConfigureTransaction, BackendError>;
    /// Records that the client acknowledged a pending geometry transaction.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown, stale, or mismatched transactions.
    fn acknowledge_configure(
        &mut self,
        transaction: ConfigureTransaction,
    ) -> Result<(), BackendError>;
}

/// An adapter failure that core can report without inspecting native state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendError {
    message: String,
}

impl BackendError {
    /// Creates an adapter error with safe, user-facing context.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BackendError {}

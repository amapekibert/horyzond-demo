//! Contracts for window-system adapters.
//!
//! A backend owns protocol state and translates it to normalized events. It
//! does not depend on a renderer or expose protocol-native objects to core.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

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

/// One pending configure transaction together with its adapter monotonic time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PendingConfigure {
    /// The serial-bearing request sent to the client.
    pub transaction: ConfigureTransaction,
    /// Adapter-supplied monotonic timestamp when the request was issued.
    pub requested_at: Duration,
}

/// Deterministic latest-request-wins ledger for asynchronous client configures.
#[derive(Debug, Default)]
pub struct ConfigureLedger {
    next_serial: u64,
    pending: BTreeMap<WindowId, PendingConfigure>,
}
impl ConfigureLedger {
    /// Issues a request, superseding the older pending request for that window.
    #[must_use]
    pub fn issue(
        &mut self,
        window: WindowId,
        geometry: Rect,
        requested_at: Duration,
    ) -> ConfigureTransaction {
        let serial = self.next_serial.saturating_add(1);
        self.next_serial = serial;
        let transaction = ConfigureTransaction {
            window,
            serial,
            geometry,
        };
        self.pending.insert(
            window,
            PendingConfigure {
                transaction,
                requested_at,
            },
        );
        transaction
    }
    /// Records a request whose protocol assigned the serial externally.
    ///
    /// The supplied serial is opaque to this neutral ledger but must be unique
    /// among simultaneously pending requests for correct acknowledgement.
    #[must_use]
    pub fn issue_with_serial(
        &mut self,
        window: WindowId,
        geometry: Rect,
        serial: u64,
        requested_at: Duration,
    ) -> ConfigureTransaction {
        self.next_serial = self.next_serial.max(serial);
        let transaction = ConfigureTransaction {
            window,
            serial,
            geometry,
        };
        self.pending.insert(
            window,
            PendingConfigure {
                transaction,
                requested_at,
            },
        );
        transaction
    }
    /// Acknowledges exactly the most recent request for its window.
    pub fn acknowledge(&mut self, transaction: ConfigureTransaction) -> bool {
        if self
            .pending
            .get(&transaction.window)
            .is_some_and(|pending| pending.transaction == transaction)
        {
            self.pending.remove(&transaction.window);
            true
        } else {
            false
        }
    }
    /// Acknowledges the latest request for a window by its protocol serial.
    ///
    /// Native protocols such as xdg-shell return only the serial in their
    /// acknowledgement callback. The adapter still owns the requested
    /// geometry in this ledger, so it must not reconstruct that geometry from
    /// protocol data merely to acknowledge the request.
    pub fn acknowledge_serial(&mut self, window: WindowId, serial: u64) -> bool {
        if self
            .pending
            .get(&window)
            .is_some_and(|pending| pending.transaction.serial == serial)
        {
            self.pending.remove(&window);
            true
        } else {
            false
        }
    }
    /// Removes and returns transactions older than the supplied timeout.
    pub fn expire(&mut self, now: Duration, timeout: Duration) -> Vec<ConfigureTransaction> {
        let expired = self
            .pending
            .iter()
            .filter_map(|(window, pending)| {
                (now.saturating_sub(pending.requested_at) >= timeout).then_some(*window)
            })
            .collect::<Vec<_>>();
        expired
            .into_iter()
            .filter_map(|window| {
                self.pending
                    .remove(&window)
                    .map(|pending| pending.transaction)
            })
            .collect()
    }
    /// Drops a pending request when its window is destroyed.
    pub fn forget(&mut self, window: WindowId) {
        self.pending.remove(&window);
    }
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
    /// A mapped client window received active seat focus.
    WindowFocused(WindowId),
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

#[cfg(test)]
mod tests {
    use super::ConfigureLedger;
    use std::time::Duration;
    use wm_types::{Rect, WindowId};

    #[test]
    fn latest_request_wins_and_timeout_releases_the_ledger() {
        let mut ledger = ConfigureLedger::default();
        let first = ledger.issue(
            WindowId::new(1),
            Rect::new(0.0, 0.0, 10.0, 10.0).expect("geometry"),
            Duration::from_millis(1),
        );
        let latest = ledger.issue(
            WindowId::new(1),
            Rect::new(0.0, 0.0, 20.0, 10.0).expect("geometry"),
            Duration::from_millis(2),
        );
        assert!(!ledger.acknowledge(first));
        assert_eq!(
            ledger.expire(Duration::from_millis(4), Duration::from_millis(2)),
            [latest]
        );
    }

    #[test]
    fn protocol_serial_acknowledgements_reject_stale_requests() {
        let mut ledger = ConfigureLedger::default();
        let window = WindowId::new(2);
        let geometry = Rect::new(0.0, 0.0, 10.0, 10.0).expect("geometry");
        let first = ledger.issue_with_serial(window, geometry, 41, Duration::ZERO);
        let latest = ledger.issue_with_serial(window, geometry, 42, Duration::from_millis(1));
        assert!(!ledger.acknowledge_serial(window, first.serial));
        assert!(ledger.acknowledge_serial(window, latest.serial));
    }
}

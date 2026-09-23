//! Deterministic virtual outputs and events for tests without graphics or a seat.

use std::collections::{BTreeSet, VecDeque};
use std::time::Duration;

use wm_backend::{BackendError, BackendEvent, ConfigureLedger, ConfigureTransaction, WindowSystem};
use wm_types::{Capabilities, OutputInfo, Rect, WindowId, WindowMetadata};

/// A simple, deterministic backend for core and contract tests.
#[derive(Debug)]
pub struct HeadlessBackend {
    outputs: Vec<OutputInfo>,
    events: VecDeque<BackendEvent>,
    mapped_windows: BTreeSet<WindowId>,
    configures: ConfigureLedger,
    initialized: bool,
}

impl HeadlessBackend {
    /// Creates a backend with virtual outputs. Initial output events are emitted
    /// when the backend is initialized.
    #[must_use]
    pub fn new(outputs: impl IntoIterator<Item = OutputInfo>) -> Self {
        Self {
            outputs: outputs.into_iter().collect(),
            events: VecDeque::new(),
            mapped_windows: BTreeSet::new(),
            configures: ConfigureLedger::default(),
            initialized: false,
        }
    }

    /// Injects a map event for a previously unknown window.
    ///
    /// # Errors
    ///
    /// Returns an error before initialization or when the window is mapped already.
    pub fn map_window(&mut self, window: WindowId) -> Result<(), BackendError> {
        self.require_initialized()?;
        if !self.mapped_windows.insert(window) {
            return Err(BackendError::new(format!("{window} is already mapped")));
        }
        self.events.push_back(BackendEvent::WindowMapped(window));
        Ok(())
    }

    /// Injects an unmap event for an existing window.
    ///
    /// # Errors
    ///
    /// Returns an error before initialization or when the window is not mapped.
    pub fn unmap_window(&mut self, window: WindowId) -> Result<(), BackendError> {
        self.require_initialized()?;
        if !self.mapped_windows.remove(&window) {
            return Err(BackendError::new(format!("{window} is not mapped")));
        }
        self.configures.forget(window);
        self.events.push_back(BackendEvent::WindowUnmapped(window));
        Ok(())
    }

    /// Injects a metadata change for an existing window.
    ///
    /// # Errors
    ///
    /// Returns an error before initialization or for an unmapped window.
    pub fn update_window_metadata(
        &mut self,
        window: WindowId,
        metadata: WindowMetadata,
    ) -> Result<(), BackendError> {
        self.require_initialized()?;
        if !self.mapped_windows.contains(&window) {
            return Err(BackendError::new(format!("{window} is not mapped")));
        }
        self.events
            .push_back(BackendEvent::WindowMetadataChanged(window, metadata));
        Ok(())
    }

    fn require_initialized(&self) -> Result<(), BackendError> {
        if self.initialized {
            Ok(())
        } else {
            Err(BackendError::new("headless backend is not initialized"))
        }
    }
}

impl WindowSystem for HeadlessBackend {
    fn initialize(&mut self) -> Result<(), BackendError> {
        if self.initialized {
            return Err(BackendError::new("headless backend is already initialized"));
        }
        self.initialized = true;
        self.events
            .extend(self.outputs.iter().cloned().map(BackendEvent::OutputAdded));
        Ok(())
    }

    fn poll_events(&mut self) -> Result<Vec<BackendEvent>, BackendError> {
        self.require_initialized()?;
        Ok(self.events.drain(..).collect())
    }

    fn outputs(&self) -> &[OutputInfo] {
        &self.outputs
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn request_window_geometry(
        &mut self,
        window: WindowId,
        geometry: Rect,
    ) -> Result<ConfigureTransaction, BackendError> {
        self.require_initialized()?;
        if self.mapped_windows.contains(&window) {
            Ok(self.configures.issue(window, geometry, Duration::ZERO))
        } else {
            Err(BackendError::new(format!(
                "cannot configure unmapped {window}"
            )))
        }
    }

    fn acknowledge_configure(
        &mut self,
        transaction: ConfigureTransaction,
    ) -> Result<(), BackendError> {
        self.require_initialized()?;
        if self.configures.acknowledge(transaction) {
            Ok(())
        } else {
            Err(BackendError::new(format!(
                "unknown or stale configure transaction {} for {}",
                transaction.serial, transaction.window
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use wm_backend::{BackendEvent, WindowSystem};
    use wm_types::{OutputInfo, Rect, WindowId, WindowMetadata};

    use super::HeadlessBackend;

    #[test]
    fn events_are_deterministic_and_geometry_requires_a_mapped_window() {
        let mut backend = HeadlessBackend::new([OutputInfo::new("headless-0", 800, 600, 1.0)]);
        backend.initialize().expect("initialization succeeds");
        assert!(matches!(
            backend.poll_events().expect("poll succeeds").as_slice(),
            [BackendEvent::OutputAdded(_)]
        ));
        let window = WindowId::new(1);
        backend.map_window(window).expect("mapping succeeds");
        backend
            .update_window_metadata(
                window,
                WindowMetadata {
                    app_id: "terminal".to_owned(),
                    title: "shell".to_owned(),
                },
            )
            .expect("metadata update succeeds");
        assert!(matches!(
            backend.poll_events().expect("poll succeeds").as_slice(),
            [
                BackendEvent::WindowMapped(_),
                BackendEvent::WindowMetadataChanged(_, _)
            ]
        ));
        let transaction = backend
            .request_window_geometry(
                window,
                Rect::new(0.0, 0.0, 200.0, 100.0).expect("valid geometry"),
            )
            .expect("mapped window accepts geometry");
        backend
            .acknowledge_configure(transaction)
            .expect("matching transaction acknowledges");
        let first = backend
            .request_window_geometry(
                window,
                Rect::new(0.0, 0.0, 201.0, 100.0).expect("valid geometry"),
            )
            .expect("first replacement request succeeds");
        let latest = backend
            .request_window_geometry(
                window,
                Rect::new(0.0, 0.0, 202.0, 100.0).expect("valid geometry"),
            )
            .expect("latest replacement request succeeds");
        assert!(latest.serial > first.serial);
        assert!(backend.acknowledge_configure(first).is_err());
        backend
            .acknowledge_configure(latest)
            .expect("latest transaction acknowledges");
        backend.unmap_window(window).expect("unmapping succeeds");
        assert!(
            backend
                .request_window_geometry(
                    window,
                    Rect::new(0.0, 0.0, 1.0, 1.0).expect("valid geometry")
                )
                .is_err()
        );
    }
}

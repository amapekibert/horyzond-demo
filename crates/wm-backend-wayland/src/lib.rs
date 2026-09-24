//! Concrete nested Wayland adapter boundary.
//!
//! The default build deliberately contains no Smithay dependency activation.
//! Enable the `nested` feature only from a visible P6 runtime composition.

use wm_backend::BackendError;
use wm_backend::{ConfigureLedger, ConfigureTransaction};
use wm_types::{Rect, WindowId};

/// Protocol-neutral lifecycle state for a newly created xdg toplevel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum XdgSurfaceState {
    /// The role exists but no configure was sent.
    New,
    /// A configure serial must be acknowledged before the first commit.
    AwaitingConfigure,
    /// The latest configure serial was acknowledged and may be committed.
    ReadyToCommit,
    /// The surface completed its first valid commit and is manageable.
    Mapped,
}

/// Feature-independent ordering guard for xdg toplevel callbacks.
#[derive(Debug, Default)]
pub struct XdgLifecycle {
    states: std::collections::BTreeMap<WindowId, XdgSurfaceState>,
    configures: ConfigureLedger,
}
impl XdgLifecycle {
    /// Registers an unmapped xdg toplevel.
    ///
    /// # Errors
    ///
    /// Returns an error when the window already has an xdg lifecycle.
    pub fn register(&mut self, window: WindowId) -> Result<(), BackendError> {
        if self.states.insert(window, XdgSurfaceState::New).is_some() {
            return Err(BackendError::new(format!(
                "{window} already has an xdg lifecycle"
            )));
        }
        Ok(())
    }
    /// Issues or supersedes a pending configure for one registered toplevel.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown or already mapped toplevel.
    pub fn configure(
        &mut self,
        window: WindowId,
        geometry: Rect,
        now: std::time::Duration,
    ) -> Result<ConfigureTransaction, BackendError> {
        match self.states.get(&window) {
            Some(XdgSurfaceState::New | XdgSurfaceState::AwaitingConfigure) => {
                self.states
                    .insert(window, XdgSurfaceState::AwaitingConfigure);
                Ok(self.configures.issue(window, geometry, now))
            }
            Some(XdgSurfaceState::ReadyToCommit | XdgSurfaceState::Mapped) => {
                Err(BackendError::new(format!(
                    "{window} cannot receive a new configure in its current lifecycle state"
                )))
            }
            None => Err(BackendError::new(format!("unknown xdg toplevel {window}"))),
        }
    }
    /// Accepts only the current configure serial.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown, stale, or out-of-order acknowledgement.
    pub fn acknowledge(&mut self, transaction: ConfigureTransaction) -> Result<(), BackendError> {
        if self.states.get(&transaction.window) == Some(&XdgSurfaceState::AwaitingConfigure)
            && self.configures.acknowledge(transaction)
        {
            self.states
                .insert(transaction.window, XdgSurfaceState::ReadyToCommit);
            Ok(())
        } else {
            Err(BackendError::new(format!(
                "invalid configure acknowledgement {} for {}",
                transaction.serial, transaction.window
            )))
        }
    }
    /// Expires unanswered configure requests and returns each timed-out request.
    ///
    /// An expired toplevel returns to `New`, allowing the adapter to issue a
    /// fresh configure without treating a slow client as mapped.
    #[must_use]
    pub fn expire_configures(
        &mut self,
        now: std::time::Duration,
        timeout: std::time::Duration,
    ) -> Vec<ConfigureTransaction> {
        let expired = self.configures.expire(now, timeout);
        for transaction in &expired {
            if self.states.get(&transaction.window) == Some(&XdgSurfaceState::AwaitingConfigure) {
                self.states.insert(transaction.window, XdgSurfaceState::New);
            }
        }
        expired
    }
    /// Maps a toplevel only after its initial configure acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns an error when the surface has not acknowledged a configure.
    pub fn commit(&mut self, window: WindowId) -> Result<(), BackendError> {
        if self.states.get(&window) == Some(&XdgSurfaceState::ReadyToCommit) {
            self.states.insert(window, XdgSurfaceState::Mapped);
            Ok(())
        } else {
            Err(BackendError::new(format!(
                "{window} committed before acknowledging its initial configure"
            )))
        }
    }
    /// Forgets all protocol state for a destroyed toplevel.
    pub fn destroy(&mut self, window: WindowId) {
        self.states.remove(&window);
        self.configures.forget(window);
    }
    /// Returns the current lifecycle state, if registered.
    #[must_use]
    pub fn state(&self, window: WindowId) -> Option<XdgSurfaceState> {
        self.states.get(&window).copied()
    }
}

/// Reports whether the nested Wayland runtime was compiled into this binary.
#[must_use]
pub const fn nested_runtime_available() -> bool {
    cfg!(feature = "nested")
}

/// Explains why the nested runtime cannot be started in a headless-only build.
///
/// Native Wayland, EGL, and GL handles remain confined to the feature-gated
/// implementation that follows in P6.
///
/// # Errors
///
/// Returns an error when this binary was built without the `nested` feature.
pub fn require_nested_runtime() -> Result<(), BackendError> {
    if nested_runtime_available() {
        #[cfg(feature = "nested")]
        let _ = smithay_boundary::smithay_version_is_linked();
        Ok(())
    } else {
        Err(BackendError::new(
            "nested Wayland runtime is not enabled; rebuild with the P6 nested adapter feature",
        ))
    }
}

#[cfg(feature = "nested")]
mod smithay_boundary {
    /// Proves the optional feature resolves Smithay without exporting its types.
    pub(super) fn smithay_version_is_linked() -> bool {
        let _ = std::any::TypeId::of::<
            smithay::backend::winit::WinitGraphicsBackend<
                smithay::backend::renderer::gles::GlesRenderer,
            >,
        >();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{XdgLifecycle, XdgSurfaceState, nested_runtime_available, require_nested_runtime};
    use std::time::Duration;
    use wm_types::{Rect, WindowId};

    #[test]
    fn default_build_keeps_the_nested_runtime_disabled() {
        assert!(!nested_runtime_available());
        assert!(require_nested_runtime().is_err());
    }

    #[test]
    fn xdg_lifecycle_requires_acknowledgement_before_mapping() {
        let mut lifecycle = XdgLifecycle::default();
        let window = WindowId::new(4);
        lifecycle.register(window).expect("register");
        assert!(lifecycle.commit(window).is_err());
        let configure = lifecycle
            .configure(
                window,
                Rect::new(0.0, 0.0, 100.0, 50.0).expect("geometry"),
                Duration::ZERO,
            )
            .expect("configure");
        assert_eq!(
            lifecycle.state(window),
            Some(XdgSurfaceState::AwaitingConfigure)
        );
        lifecycle.acknowledge(configure).expect("acknowledge");
        lifecycle.commit(window).expect("commit");
        assert_eq!(lifecycle.state(window), Some(XdgSurfaceState::Mapped));
    }

    #[test]
    fn xdg_lifecycle_timeout_allows_a_fresh_configure() {
        let mut lifecycle = XdgLifecycle::default();
        let window = WindowId::new(5);
        let geometry = Rect::new(0.0, 0.0, 100.0, 50.0).expect("geometry");
        lifecycle.register(window).expect("register");
        let expired = lifecycle
            .configure(window, geometry, Duration::from_millis(10))
            .expect("configure");
        assert_eq!(
            lifecycle.expire_configures(Duration::from_millis(20), Duration::from_millis(5)),
            [expired]
        );
        assert_eq!(lifecycle.state(window), Some(XdgSurfaceState::New));
        assert!(
            lifecycle
                .configure(window, geometry, Duration::from_millis(21))
                .is_ok()
        );
    }
}

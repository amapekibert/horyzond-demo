//! Concrete nested Wayland adapter boundary.
//!
//! The default build deliberately contains no Smithay dependency activation.
//! Enable the `nested` feature only from a visible P6 runtime composition.

use wm_backend::BackendError;

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
    use super::{nested_runtime_available, require_nested_runtime};

    #[test]
    fn default_build_keeps_the_nested_runtime_disabled() {
        assert!(!nested_runtime_available());
        assert!(require_nested_runtime().is_err());
    }
}

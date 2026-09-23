//! Coordinates configuration generations with opaque layout providers.
//!
//! The coordinator is the only layer that knows both configuration and core
//! state. `wm-core` remains independent of Lua files and provider registries.

use std::fmt;
use wm_config::ConfigManager;
use wm_core::CoreState;
use wm_layout::{LayoutId, LayoutProviders, ProviderReload};
use wm_types::Rect;

/// The result of synchronizing one accepted configuration generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayoutRuntimeUpdate {
    Unchanged,
    Applied {
        generation: u64,
        providers: ProviderReload,
    },
}

/// Configuration-to-layout bridge applied only at a caller-selected safe boundary.
#[derive(Debug)]
pub struct LayoutRuntime {
    generation: u64,
    default_layout: LayoutId,
    providers: LayoutProviders,
}
impl LayoutRuntime {
    /// Creates a bridge from an already accepted configuration candidate.
    ///
    /// # Errors
    ///
    /// Returns an error before the configuration manager has an active candidate
    /// or when its default layout ID is empty.
    pub fn from_config(configuration: &ConfigManager) -> Result<Self, LayoutRuntimeError> {
        let default_layout = active_layout_id(configuration)?;
        let paths = configuration
            .layout_paths()
            .ok_or(LayoutRuntimeError::NoActiveConfiguration)?;
        Ok(Self {
            generation: configuration.generation(),
            default_layout,
            providers: LayoutProviders::from_layout_paths(paths),
        })
    }

    /// Incorporates an accepted configuration generation without mutating core.
    ///
    /// # Errors
    ///
    /// Returns an error when the manager has no active candidate or its default
    /// layout ID is invalid.
    pub fn synchronize(
        &mut self,
        configuration: &ConfigManager,
    ) -> Result<LayoutRuntimeUpdate, LayoutRuntimeError> {
        if self.generation == configuration.generation() {
            return Ok(LayoutRuntimeUpdate::Unchanged);
        }
        let default_layout = active_layout_id(configuration)?;
        let paths = configuration
            .layout_paths()
            .ok_or(LayoutRuntimeError::NoActiveConfiguration)?;
        let providers = self.providers.reload(paths);
        self.default_layout = default_layout;
        self.generation = configuration.generation();
        Ok(LayoutRuntimeUpdate::Applied {
            generation: self.generation,
            providers,
        })
    }

    /// Applies the configured default layout to the active workspace.
    pub fn apply_default(&self, core: &mut CoreState, bounds: Rect) {
        let provider = self.providers.provider(&self.default_layout);
        core.apply_layout(provider.as_ref(), bounds);
    }

    /// Reapplies the active workspace layout after a provider reload.
    pub fn reapply_active(&self, core: &mut CoreState, bounds: Rect) {
        let provider = self.providers.provider(core.active_workspace().layout());
        core.apply_layout(provider.as_ref(), bounds);
    }

    /// Returns the current default opaque layout ID.
    #[must_use]
    pub fn default_layout(&self) -> &LayoutId {
        &self.default_layout
    }
}

fn active_layout_id(configuration: &ConfigManager) -> Result<LayoutId, LayoutRuntimeError> {
    let name = configuration
        .default_layout()
        .ok_or(LayoutRuntimeError::NoActiveConfiguration)?;
    LayoutId::new(name.to_owned()).map_err(LayoutRuntimeError::InvalidLayoutId)
}

/// A runtime-to-configuration bridge failure.
#[derive(Debug)]
pub enum LayoutRuntimeError {
    NoActiveConfiguration,
    InvalidLayoutId(wm_layout::LayoutIdError),
}
impl fmt::Display for LayoutRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoActiveConfiguration => formatter.write_str("no active configuration candidate"),
            Self::InvalidLayoutId(error) => error.fmt(formatter),
        }
    }
}
impl std::error::Error for LayoutRuntimeError {}

#[cfg(test)]
mod tests {
    use super::{LayoutRuntime, LayoutRuntimeUpdate};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    use wm_config::{ConfigManager, ConfigPath, install_defaults};
    use wm_core::CoreState;
    use wm_script::ScriptLimits;
    use wm_types::{Point, Rect, WindowId};

    fn root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "horyzond-runtime-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[test]
    fn accepted_configuration_generation_replaces_an_arbitrary_default_layout() {
        let root = root();
        let path = ConfigPath::from_override(&root);
        install_defaults(&path).expect("defaults");
        let mut configuration =
            ConfigManager::new(&path, ScriptLimits::default()).expect("manager");
        let _ = configuration.load_initial();
        let mut runtime = LayoutRuntime::from_config(&configuration).expect("runtime");
        assert_eq!(runtime.default_layout().as_str(), "spatial");

        fs::write(
            root.join("layouts/custom.lua"),
            "layout = { id = 'custom', api_version = 1 }\nfunction calculate() return { [1] = { x = 5, y = 6, width = 7, height = 8 } } end\n",
        )
        .expect("provider");
        fs::write(
            root.join("config.lua"),
            "settings = { default_layout = 'custom' }\nmodes = {}\nlayouts = { custom = 'layouts/custom.lua' }\n",
        )
        .expect("config");
        let _ = configuration.reload();
        assert!(matches!(
            runtime.synchronize(&configuration),
            Ok(LayoutRuntimeUpdate::Applied { generation: 2, .. })
        ));
        let mut core = CoreState::default();
        core.apply_event(wm_backend::BackendEvent::WindowMapped(WindowId::new(1)))
            .expect("map");
        runtime.apply_default(
            &mut core,
            Rect::new(0.0, 0.0, 100.0, 100.0).expect("bounds"),
        );
        assert_eq!(runtime.default_layout().as_str(), "custom");
        assert_eq!(
            core.active_workspace()
                .scene
                .pick(Point::new(6.0, 7.0).expect("point")),
            Some(WindowId::new(1))
        );
        fs::remove_dir_all(root).expect("cleanup");
    }
}

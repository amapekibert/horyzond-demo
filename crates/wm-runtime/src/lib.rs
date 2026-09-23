//! Coordinates configuration generations with opaque layout providers.
//!
//! The coordinator is the only layer that knows both configuration and core
//! state. `wm-core` remains independent of Lua files and provider registries.

use std::fmt;
use wm_config::ConfigManager;
use wm_core::CoreState;
use wm_input::{InputCommand, InputOutcome, InputState};
use wm_layout::{LayoutId, LayoutInteraction, LayoutProviders, ProviderReload};
use wm_rules::{Match as RuleMatch, Rule, RuleAction, RuleEngine, WindowMetadata};
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

/// A launch request represented without shell interpretation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessLaunch {
    pub executable: String,
    pub arguments: Vec<String>,
}

/// A command emitted after a configured input press.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeCommand {
    Launch(ProcessLaunch),
    Opaque(InputCommand),
}

/// Configuration-to-layout bridge applied only at a caller-selected safe boundary.
#[derive(Debug)]
pub struct LayoutRuntime {
    generation: u64,
    default_layout: LayoutId,
    providers: LayoutProviders,
    input: InputState,
    rules: RuleEngine,
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
        let input = InputState::from_script(
            configuration
                .input()
                .ok_or(LayoutRuntimeError::NoActiveConfiguration)?,
        )
        .map_err(LayoutRuntimeError::Input)?;
        Ok(Self {
            generation: configuration.generation(),
            default_layout,
            providers: LayoutProviders::from_layout_paths(paths),
            input,
            rules: active_rules(configuration)?,
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
        self.input
            .replace_from_script(
                configuration
                    .input()
                    .ok_or(LayoutRuntimeError::NoActiveConfiguration)?,
            )
            .map_err(LayoutRuntimeError::Input)?;
        self.rules = active_rules(configuration)?;
        self.default_layout = default_layout;
        self.generation = configuration.generation();
        Ok(LayoutRuntimeUpdate::Applied {
            generation: self.generation,
            providers,
        })
    }

    /// Applies the configured default layout to the active workspace.
    pub fn apply_default(&self, core: &mut CoreState, bounds: Rect) {
        self.apply(core, &self.default_layout, bounds);
    }

    /// Applies an opaque configured layout selection to the active workspace.
    ///
    /// Selection policy belongs to callers such as modes or IPC. An unknown or
    /// unavailable ID receives the generic recovery provider.
    pub fn apply(&self, core: &mut CoreState, layout: &LayoutId, bounds: Rect) {
        let provider = self.providers.provider(layout);
        core.apply_layout(provider.as_ref(), bounds);
    }

    /// Routes an opaque interaction to the active workspace provider.
    pub fn interact(&self, core: &mut CoreState, interaction: &LayoutInteraction, bounds: Rect) {
        let layout = core.active_workspace().layout().clone();
        let provider = self.providers.provider(&layout);
        core.interact_layout(provider.as_ref(), interaction, bounds);
    }
    /// Handles one normalized configured input press.
    pub fn press(&mut self, chord: &str) -> InputOutcome {
        self.input.press(chord)
    }
    /// Dispatches a configured press into a typed runtime request.
    ///
    /// `spawn` requires a non-empty executable as its first configured
    /// argument. Other action names remain opaque for later P5 subsystems.
    pub fn dispatch_press(&mut self, chord: &str) -> Option<RuntimeCommand> {
        let command = self.press(chord).command?;
        if command.name == "switch_mode" {
            return None;
        }
        if command.name != "spawn" {
            return Some(RuntimeCommand::Opaque(command));
        }
        let (executable, arguments) = command.arguments.split_first()?;
        if executable.trim().is_empty() {
            return None;
        }
        Some(RuntimeCommand::Launch(ProcessLaunch {
            executable: executable.clone(),
            arguments: arguments.to_vec(),
        }))
    }
    /// Clears and reports a release corresponding to a consumed press.
    pub fn release(&mut self, chord: &str) -> bool {
        self.input.release(chord)
    }
    /// Returns the current configured mode name.
    #[must_use]
    pub fn mode(&self) -> &str {
        self.input.current_mode().as_str()
    }
    /// Evaluates the active generation's deterministic data-only rules once.
    #[must_use]
    pub fn evaluate_rules(&self, metadata: &WindowMetadata) -> Vec<RuleAction> {
        self.rules.evaluate(metadata).actions
    }

    /// Reapplies the active workspace layout after a provider reload.
    pub fn reapply_active(&self, core: &mut CoreState, bounds: Rect) {
        let layout = core.active_workspace().layout().clone();
        self.apply(core, &layout, bounds);
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

fn active_rules(configuration: &ConfigManager) -> Result<RuleEngine, LayoutRuntimeError> {
    let rules = configuration
        .rules()
        .ok_or(LayoutRuntimeError::NoActiveConfiguration)?;
    Ok(RuleEngine::new(
        rules
            .iter()
            .map(|rule| Rule {
                priority: rule.priority,
                matcher: RuleMatch {
                    app_id_contains: rule.app_id_contains.clone(),
                    title_contains: rule.title_contains.clone(),
                },
                actions: rule
                    .actions
                    .iter()
                    .map(|action| RuleAction {
                        name: action.name.clone(),
                        arguments: action.arguments.clone(),
                    })
                    .collect(),
                stop: rule.stop,
            })
            .collect(),
    ))
}

/// A runtime-to-configuration bridge failure.
#[derive(Debug)]
pub enum LayoutRuntimeError {
    NoActiveConfiguration,
    InvalidLayoutId(wm_layout::LayoutIdError),
    Input(wm_input::InputError),
}
impl fmt::Display for LayoutRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoActiveConfiguration => formatter.write_str("no active configuration candidate"),
            Self::InvalidLayoutId(error) => error.fmt(formatter),
            Self::Input(error) => error.fmt(formatter),
        }
    }
}
impl std::error::Error for LayoutRuntimeError {}

#[cfg(test)]
mod tests {
    use super::{LayoutRuntime, LayoutRuntimeUpdate, ProcessLaunch, RuntimeCommand};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    use wm_config::{ConfigManager, ConfigPath, install_defaults};
    use wm_core::CoreState;
    use wm_layout::LayoutInteraction;
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
        assert_eq!(runtime.mode(), "normal");
        assert_eq!(
            runtime.dispatch_press("Super+Enter"),
            Some(RuntimeCommand::Launch(ProcessLaunch {
                executable: "ghostty".to_owned(),
                arguments: vec![],
            }))
        );

        fs::write(
            root.join("layouts/custom.lua"),
            "layout = { id = 'custom', api_version = 1 }\nfunction calculate() return { [1] = { x = 5, y = 6, width = 7, height = 8 } } end\nfunction interact(windows, bounds, camera, state, event) return { [1] = { x = 5, y = 6, width = 7, height = 8 }, state = { action = event.action } } end\n",
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
        assert!(!runtime.release("Super+Enter"));
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
        runtime.interact(
            &mut core,
            &LayoutInteraction::new("advance", serde_json::Value::Null).expect("interaction"),
            Rect::new(0.0, 0.0, 100.0, 100.0).expect("bounds"),
        );
        assert_eq!(
            core.active_layout_state()
                .provider_state(core.active_workspace().layout()),
            Some(&serde_json::json!({ "action": "advance" }))
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn caller_can_select_an_arbitrary_configured_provider() {
        let root = root();
        let path = ConfigPath::from_override(&root);
        install_defaults(&path).expect("defaults");
        fs::write(
            root.join("layouts/custom.lua"),
            "layout = { id = 'custom', api_version = 1 }\nfunction calculate() return { [1] = { x = 5, y = 6, width = 7, height = 8 } } end\n",
        )
        .expect("provider");
        fs::write(
            root.join("config.lua"),
            "settings = { default_layout = 'spatial' }\nmodes = {}\nlayouts = { spatial = 'layouts/spatial.lua', custom = 'layouts/custom.lua' }\n",
        )
        .expect("config");
        let mut configuration =
            ConfigManager::new(&path, ScriptLimits::default()).expect("manager");
        let _ = configuration.load_initial();
        let runtime = LayoutRuntime::from_config(&configuration).expect("runtime");
        let mut core = CoreState::default();
        core.apply_event(wm_backend::BackendEvent::WindowMapped(WindowId::new(1)))
            .expect("map");
        runtime.apply(
            &mut core,
            &wm_layout::LayoutId::new("custom").expect("ID"),
            Rect::new(0.0, 0.0, 100.0, 100.0).expect("bounds"),
        );
        assert_eq!(core.active_workspace().layout().as_str(), "custom");
        assert_eq!(
            core.active_workspace()
                .scene
                .pick(Point::new(6.0, 7.0).expect("point")),
            Some(WindowId::new(1))
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn provider_reload_migrates_data_only_state_for_the_same_opaque_id() {
        let root = root();
        let path = ConfigPath::from_override(&root);
        install_defaults(&path).expect("defaults");
        let provider = root.join("layouts/custom.lua");
        fs::write(
            &provider,
            "layout = { id = 'custom', api_version = 1 }\nfunction calculate() return { [1] = { x = 0, y = 0, width = 10, height = 10 }, state = { count = 1 } } end\n",
        )
        .expect("provider");
        fs::write(
            root.join("config.lua"),
            "settings = { default_layout = 'custom' }\nmodes = {}\nlayouts = { custom = 'layouts/custom.lua' }\n",
        )
        .expect("config");
        let mut configuration =
            ConfigManager::new(&path, ScriptLimits::default()).expect("manager");
        let _ = configuration.load_initial();
        let mut runtime = LayoutRuntime::from_config(&configuration).expect("runtime");
        let mut core = CoreState::default();
        core.apply_event(wm_backend::BackendEvent::WindowMapped(WindowId::new(1)))
            .expect("map");
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0).expect("bounds");
        runtime.apply_default(&mut core, bounds);
        fs::write(
            &provider,
            "layout = { id = 'custom', api_version = 1 }\nfunction calculate(windows, bounds, camera, state) return { [1] = { x = 0, y = 0, width = 10, height = 10 }, state = { count = state.count + 10 } } end\n",
        )
        .expect("updated provider");
        let _ = configuration.reload();
        assert!(matches!(
            runtime.synchronize(&configuration),
            Ok(LayoutRuntimeUpdate::Applied { .. })
        ));
        runtime.reapply_active(&mut core, bounds);
        assert_eq!(
            core.active_layout_state()
                .provider_state(core.active_workspace().layout()),
            Some(&serde_json::json!({ "count": 11 }))
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn runtime_evaluates_rules_from_the_active_configuration_generation() {
        let root = root();
        let path = ConfigPath::from_override(&root);
        install_defaults(&path).expect("defaults");
        fs::write(
            root.join("config.lua"),
            "settings = { default_layout = 'spatial' }\nmodes = {}\nlayouts = { spatial = 'layouts/spatial.lua' }\nrules = { { priority = 1, match = { app_id_contains = 'term' }, actions = { { action = 'select_layout', arguments = { 'spatial' } } }, stop = true } }\n",
        )
        .expect("config");
        let mut configuration =
            ConfigManager::new(&path, ScriptLimits::default()).expect("configuration");
        let _ = configuration.load_initial();
        let runtime = LayoutRuntime::from_config(&configuration).expect("runtime");
        assert_eq!(
            runtime
                .evaluate_rules(&wm_rules::WindowMetadata {
                    app_id: "terminal".to_owned(),
                    title: String::new(),
                })
                .first()
                .expect("matched action")
                .name,
            "select_layout"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }
}

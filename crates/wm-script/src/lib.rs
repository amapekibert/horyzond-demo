//! Sandboxed, bounded Lua loading with Horyzond's `source()` import function.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mlua::{
    Error as LuaError, Function, HookTriggers, Lua, LuaOptions, LuaSerdeExt, StdLib, Value, VmState,
};

/// Limits applied to every candidate configuration evaluation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScriptLimits {
    pub memory_bytes: usize,
    pub instruction_limit: u64,
}

impl Default for ScriptLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 8 * 1024 * 1024,
            instruction_limit: 1_000_000,
        }
    }
}

/// A successfully loaded Lua configuration candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedScript {
    pub dependencies: BTreeSet<PathBuf>,
    /// Canonical profile-provider files selected by the `profiles` table.
    ///
    /// The paths are confined to the configuration root and are also included
    /// in `dependencies`, so editing a provider triggers a configuration reload.
    pub layout_paths: BTreeMap<String, PathBuf>,
    /// The requested initial profile, defaulting to the canonical SPATIAL name.
    pub default_layout: String,
    /// Data-only modal bindings extracted from `modes` and `keybinds`.
    pub input: InputConfig,
    /// Data-only deterministic window rules from the active candidate.
    pub rules: Vec<RuleConfig>,
    /// Canonical hook paths keyed by user-selected lifecycle event name.
    pub hook_paths: BTreeMap<String, PathBuf>,
}
/// Data-only modal configuration owned by the Lua configuration candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputConfig {
    pub initial_mode: String,
    pub modes: BTreeMap<String, Vec<InputBinding>>,
}
/// One configured modal binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputBinding {
    pub key: String,
    pub action: String,
    pub arguments: Vec<String>,
    pub next_mode: Option<String>,
}
/// One configured window rule with literal metadata predicates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleConfig {
    pub priority: u32,
    pub app_id_contains: Option<String>,
    pub title_contains: Option<String>,
    pub actions: Vec<RuleActionConfig>,
    pub stop: bool,
}
/// One opaque action selected by a configured rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleActionConfig {
    pub name: String,
    pub arguments: Vec<String>,
}

/// Loads one configuration root and all files imported with `source()`.
#[derive(Clone, Debug)]
pub struct ScriptLoader {
    root: PathBuf,
    limits: ScriptLimits,
}

impl ScriptLoader {
    /// Creates a loader rooted at an existing configuration directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the root cannot be canonicalized.
    pub fn new(root: impl AsRef<Path>, limits: ScriptLimits) -> Result<Self, ScriptError> {
        Ok(Self {
            root: fs::canonicalize(root).map_err(ScriptError::Io)?,
            limits,
        })
    }

    /// Evaluates `config.lua` in a fresh restricted Lua state.
    ///
    /// # Errors
    ///
    /// Returns an error for I/O, Lua, import, quota, or root-boundary failures.
    pub fn load(&self) -> Result<LoadedScript, ScriptError> {
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8,
            LuaOptions::default(),
        )
        .map_err(ScriptError::Lua)?;
        lua.set_memory_limit(self.limits.memory_bytes)
            .map_err(ScriptError::Lua)?;
        let remaining = Rc::new(RefCell::new(self.limits.instruction_limit));
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(1_000),
            move |_, _| {
                let mut remaining = remaining.borrow_mut();
                if *remaining < 1_000 {
                    return Err(LuaError::RuntimeError(
                        "Horyzond Lua instruction limit exceeded".to_owned(),
                    ));
                }
                *remaining -= 1_000;
                Ok(VmState::Continue)
            },
        )
        .map_err(ScriptError::Lua)?;
        let state = Rc::new(RefCell::new(ImportState::default()));
        let source_state = Rc::clone(&state);
        let root = self.root.clone();
        let source = lua
            .create_function(move |lua, relative: String| {
                execute_file(lua, &root, &source_state, &relative)
                    .map_err(|error| LuaError::RuntimeError(error.to_string()))
            })
            .map_err(ScriptError::Lua)?;
        lua.globals()
            .set("source", source)
            .map_err(ScriptError::Lua)?;
        execute_file(&lua, &self.root, &state, "config.lua")?;
        validate_configuration(&lua)?;
        let layout_paths = extract_layout_paths(&lua, &self.root)?;
        let default_layout = extract_default_layout(&lua)?;
        let input = extract_input(&lua)?;
        let rules = extract_rules(&lua)?;
        let hook_paths = extract_hook_paths(&lua, &self.root)?;
        state
            .borrow_mut()
            .dependencies
            .extend(layout_paths.values().cloned());
        state
            .borrow_mut()
            .dependencies
            .extend(hook_paths.values().cloned());
        Ok(LoadedScript {
            dependencies: state.borrow().dependencies.clone(),
            layout_paths,
            default_layout,
            input,
            rules,
            hook_paths,
        })
    }

    /// Executes one configured hook in a fresh restricted Lua state.
    ///
    /// # Errors
    ///
    /// Returns an error when the hook is outside this configuration root,
    /// invalid, or exceeds Lua resource limits.
    pub fn execute_hook(&self, path: &Path, event: &serde_json::Value) -> Result<(), ScriptError> {
        let path = fs::canonicalize(path).map_err(ScriptError::Io)?;
        if !path.starts_with(&self.root) {
            return Err(ScriptError::OutsideRoot(path));
        }
        let lua = restricted_lua(self.limits)?;
        let source = fs::read_to_string(&path).map_err(ScriptError::Io)?;
        let hook: Function = lua
            .load(&source)
            .set_name(path.to_string_lossy())
            .eval()
            .map_err(ScriptError::Lua)?;
        hook.call::<()>(lua.to_value(event).map_err(ScriptError::Lua)?)
            .map_err(ScriptError::Lua)
    }
}

fn restricted_lua(limits: ScriptLimits) -> Result<Lua, ScriptError> {
    let lua = Lua::new_with(
        StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8,
        LuaOptions::default(),
    )
    .map_err(ScriptError::Lua)?;
    lua.set_memory_limit(limits.memory_bytes)
        .map_err(ScriptError::Lua)?;
    let remaining = Rc::new(RefCell::new(limits.instruction_limit));
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(1_000),
        move |_, _| {
            let mut remaining = remaining.borrow_mut();
            if *remaining < 1_000 {
                return Err(LuaError::RuntimeError(
                    "Horyzond Lua instruction limit exceeded".to_owned(),
                ));
            }
            *remaining -= 1_000;
            Ok(VmState::Continue)
        },
    )
    .map_err(ScriptError::Lua)?;
    Ok(lua)
}

fn extract_rules(lua: &Lua) -> Result<Vec<RuleConfig>, ScriptError> {
    const MAX_RULES: usize = 256;
    const MAX_ACTIONS_PER_RULE: usize = 64;
    let Some(rules) = lua
        .globals()
        .get::<Option<mlua::Table>>("rules")
        .map_err(ScriptError::Lua)?
    else {
        return Ok(Vec::new());
    };
    let mut extracted = Vec::new();
    for entry in rules.sequence_values::<mlua::Table>() {
        if extracted.len() == MAX_RULES {
            return Err(ScriptError::Schema(
                "rules exceeds its 256-entry limit".to_owned(),
            ));
        }
        let entry = entry.map_err(ScriptError::Lua)?;
        let matcher = entry
            .get::<Option<mlua::Table>>("match")
            .map_err(ScriptError::Lua)?;
        let app_id_contains = matcher
            .as_ref()
            .map(|table| table.get::<Option<String>>("app_id_contains"))
            .transpose()
            .map_err(ScriptError::Lua)?
            .flatten();
        let title_contains = matcher
            .as_ref()
            .map(|table| table.get::<Option<String>>("title_contains"))
            .transpose()
            .map_err(ScriptError::Lua)?
            .flatten();
        let actions: mlua::Table = entry.get("actions").map_err(ScriptError::Lua)?;
        let mut configured_actions = Vec::new();
        for action in actions.sequence_values::<mlua::Table>() {
            if configured_actions.len() == MAX_ACTIONS_PER_RULE {
                return Err(ScriptError::Schema(
                    "a rule exceeds its 64-action limit".to_owned(),
                ));
            }
            let action = action.map_err(ScriptError::Lua)?;
            configured_actions.push(RuleActionConfig {
                name: action.get("action").map_err(ScriptError::Lua)?,
                arguments: action
                    .get::<Option<mlua::Table>>("arguments")
                    .map_err(ScriptError::Lua)?
                    .map(|arguments| {
                        arguments
                            .sequence_values::<String>()
                            .collect::<Result<Vec<_>, _>>()
                            .map_err(ScriptError::Lua)
                    })
                    .transpose()?
                    .unwrap_or_default(),
            });
        }
        extracted.push(RuleConfig {
            priority: entry.get("priority").map_err(ScriptError::Lua)?,
            app_id_contains,
            title_contains,
            actions: configured_actions,
            stop: entry
                .get::<Option<bool>>("stop")
                .map_err(ScriptError::Lua)?
                .unwrap_or(false),
        });
    }
    Ok(extracted)
}

fn extract_input(lua: &Lua) -> Result<InputConfig, ScriptError> {
    let modes: mlua::Table = lua.globals().get("modes").map_err(ScriptError::Lua)?;
    let mut extracted = BTreeMap::new();
    for pair in modes.pairs::<String, mlua::Table>() {
        let (name, mode) = pair.map_err(ScriptError::Lua)?;
        extracted.insert(
            name,
            extract_bindings(mode.get("binds").map_err(ScriptError::Lua)?)?,
        );
    }
    let keybinds: Option<mlua::Table> = lua.globals().get("keybinds").map_err(ScriptError::Lua)?;
    let global = keybinds
        .map(|table| extract_bindings(Some(table)))
        .transpose()?
        .unwrap_or_default();
    extracted
        .entry("normal".to_owned())
        .or_default()
        .extend(global);
    Ok(InputConfig {
        initial_mode: "normal".to_owned(),
        modes: extracted,
    })
}

fn extract_bindings(table: Option<mlua::Table>) -> Result<Vec<InputBinding>, ScriptError> {
    let Some(table) = table else {
        return Ok(Vec::new());
    };
    table
        .sequence_values::<mlua::Table>()
        .map(|entry| {
            let entry = entry.map_err(ScriptError::Lua)?;
            let key = entry.get("key").map_err(ScriptError::Lua)?;
            let action = entry.get("action").map_err(ScriptError::Lua)?;
            let arguments = entry
                .get::<Option<mlua::Table>>("command")
                .map_err(ScriptError::Lua)?
                .map(|command| {
                    command
                        .sequence_values::<String>()
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(ScriptError::Lua)
                })
                .transpose()?
                .unwrap_or_default();
            Ok(InputBinding {
                key,
                action,
                arguments,
                next_mode: entry.get("mode").map_err(ScriptError::Lua)?,
            })
        })
        .collect()
}

fn validate_configuration(lua: &Lua) -> Result<(), ScriptError> {
    for name in ["settings", "modes", "layouts"] {
        if !matches!(
            lua.globals().get::<Value>(name).map_err(ScriptError::Lua)?,
            Value::Table(_)
        ) {
            return Err(ScriptError::Schema(format!(
                "global '{name}' must be a table"
            )));
        }
    }
    Ok(())
}

fn extract_layout_paths(lua: &Lua, root: &Path) -> Result<BTreeMap<String, PathBuf>, ScriptError> {
    let profiles = lua
        .globals()
        .get::<mlua::Table>("layouts")
        .map_err(ScriptError::Lua)?;
    let mut paths = BTreeMap::new();
    for pair in profiles.pairs::<String, Value>() {
        let (name, value) = pair.map_err(ScriptError::Lua)?;
        let Value::String(relative) = value else {
            return Err(ScriptError::Schema(format!(
                "layouts.{name} must be a string path"
            )));
        };
        let relative = relative.to_str().map_err(ScriptError::Lua)?;
        let path = fs::canonicalize(root.join(relative.as_ref())).map_err(ScriptError::Io)?;
        if !path.starts_with(root) {
            return Err(ScriptError::OutsideRoot(path));
        }
        paths.insert(name, path);
    }
    Ok(paths)
}

fn extract_hook_paths(lua: &Lua, root: &Path) -> Result<BTreeMap<String, PathBuf>, ScriptError> {
    let Some(hooks) = lua
        .globals()
        .get::<Option<mlua::Table>>("hooks")
        .map_err(ScriptError::Lua)?
    else {
        return Ok(BTreeMap::new());
    };
    let mut paths = BTreeMap::new();
    for pair in hooks.pairs::<String, Value>() {
        let (name, value) = pair.map_err(ScriptError::Lua)?;
        let Value::String(relative) = value else {
            return Err(ScriptError::Schema(format!(
                "hooks.{name} must be a string path"
            )));
        };
        let relative = relative.to_str().map_err(ScriptError::Lua)?;
        let path = fs::canonicalize(root.join(relative.as_ref())).map_err(ScriptError::Io)?;
        if !path.starts_with(root) {
            return Err(ScriptError::OutsideRoot(path));
        }
        paths.insert(name, path);
    }
    Ok(paths)
}

fn extract_default_layout(lua: &Lua) -> Result<String, ScriptError> {
    let settings = lua
        .globals()
        .get::<mlua::Table>("settings")
        .map_err(ScriptError::Lua)?;
    settings
        .get::<Option<String>>("default_layout")
        .map_err(ScriptError::Lua)
        .map(|profile| profile.unwrap_or_else(|| "spatial".to_owned()))
}

#[derive(Default)]
struct ImportState {
    stack: Vec<PathBuf>,
    dependencies: BTreeSet<PathBuf>,
    cache: HashMap<PathBuf, mlua::RegistryKey>,
}

fn execute_file(
    lua: &Lua,
    root: &Path,
    state: &Rc<RefCell<ImportState>>,
    requested: &str,
) -> Result<Value, ScriptError> {
    let parent = state
        .borrow()
        .stack
        .last()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| root.to_path_buf());
    let path = fs::canonicalize(parent.join(requested)).map_err(ScriptError::Io)?;
    if !path.starts_with(root) {
        return Err(ScriptError::OutsideRoot(path));
    }
    if state.borrow().stack.contains(&path) {
        return Err(ScriptError::ImportCycle(state.borrow().stack.clone(), path));
    }
    if let Some(key) = state.borrow().cache.get(&path) {
        return lua.registry_value(key).map_err(ScriptError::Lua);
    }
    let source = fs::read_to_string(&path).map_err(ScriptError::Io)?;
    state.borrow_mut().stack.push(path.clone());
    state.borrow_mut().dependencies.insert(path.clone());
    let result = lua
        .load(&source)
        .set_name(path.to_string_lossy())
        .eval::<Value>();
    state.borrow_mut().stack.pop();
    let value = result.map_err(ScriptError::Lua)?;
    let key = lua
        .create_registry_value(value.clone())
        .map_err(ScriptError::Lua)?;
    state.borrow_mut().cache.insert(path, key);
    Ok(value)
}

/// A configuration scripting failure with import context.
#[derive(Debug)]
pub enum ScriptError {
    Io(std::io::Error),
    Lua(LuaError),
    OutsideRoot(PathBuf),
    ImportCycle(Vec<PathBuf>, PathBuf),
    Schema(String),
}
impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Lua(error) => error.fmt(f),
            Self::OutsideRoot(path) => {
                write!(f, "import escapes configuration root: {}", path.display())
            }
            Self::ImportCycle(stack, path) => write!(
                f,
                "Lua import cycle through {} file(s), ending at {}",
                stack.len(),
                path.display()
            ),
            Self::Schema(message) => write!(f, "configuration schema error: {message}"),
        }
    }
}
impl std::error::Error for ScriptError {}

#[cfg(test)]
mod tests {
    use super::{ScriptLimits, ScriptLoader};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    fn root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "horyzond-script-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }
    #[test]
    fn loads_relative_imports_once() {
        let root = root();
        fs::create_dir_all(&root).expect("root");
        fs::write(
            root.join("config.lua"),
            "settings = {}\nmodes = {}\nlayouts = {}\nrules = {}\nvalue = source('sub.lua')\nagain = source('sub.lua')\n",
        )
        .expect("config");
        fs::write(root.join("sub.lua"), "return { enabled = true }\n").expect("sub");
        let loaded = ScriptLoader::new(&root, ScriptLimits::default())
            .expect("loader")
            .load()
            .expect("load");
        assert_eq!(loaded.dependencies.len(), 2);
        fs::remove_dir_all(root).expect("cleanup");
    }
    #[test]
    fn rejects_import_cycles() {
        let root = root();
        fs::create_dir_all(&root).expect("root");
        fs::write(
            root.join("config.lua"),
            "settings = {}\nmodes = {}\nlayouts = {}\nrules = {}\nsource('a.lua')\n",
        )
        .expect("config");
        fs::write(root.join("a.lua"), "source('config.lua')\n").expect("cycle");
        assert!(
            ScriptLoader::new(&root, ScriptLimits::default())
                .expect("loader")
                .load()
                .is_err()
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn resolves_profile_files_and_watches_them() {
        let root = root();
        fs::create_dir_all(root.join("layouts")).expect("layouts");
        fs::write(
            root.join("config.lua"),
            "settings = {}\nmodes = {}\nlayouts = { spatial = 'layouts/spatial.lua' }\n",
        )
        .expect("config");
        let profile = root.join("layouts/spatial.lua");
        fs::write(&profile, "function calculate() return {} end\n").expect("profile");
        let loaded = ScriptLoader::new(&root, ScriptLimits::default())
            .expect("loader")
            .load()
            .expect("load");
        let profile = fs::canonicalize(profile).expect("canonical profile");
        assert_eq!(loaded.layout_paths["spatial"], profile);
        assert_eq!(loaded.default_layout, "spatial");
        assert!(loaded.dependencies.contains(&profile));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn rejects_profile_path_outside_the_configuration_root() {
        let root = root();
        fs::create_dir_all(&root).expect("root");
        let outside = root.with_extension("outside.lua");
        fs::write(&outside, "function calculate() return {} end\n").expect("outside");
        fs::write(
            root.join("config.lua"),
            format!(
                "settings = {{}}\nmodes = {{}}\nlayouts = {{ spatial = '{}' }}\n",
                outside.display()
            ),
        )
        .expect("config");
        assert!(
            ScriptLoader::new(&root, ScriptLimits::default())
                .expect("loader")
                .load()
                .is_err()
        );
        fs::remove_dir_all(root).expect("cleanup root");
        fs::remove_file(outside).expect("cleanup outside");
    }

    #[test]
    fn extracts_bounded_data_only_rules() {
        let root = root();
        fs::create_dir_all(&root).expect("root");
        fs::write(
            root.join("config.lua"),
            "settings = {}\nmodes = {}\nlayouts = {}\nrules = { { priority = 2, match = { app_id_contains = 'term' }, actions = { { action = 'select_layout', arguments = { 'custom' } } }, stop = true } }\n",
        )
        .expect("config");
        let loaded = ScriptLoader::new(&root, ScriptLimits::default())
            .expect("loader")
            .load()
            .expect("load");
        assert_eq!(loaded.rules.len(), 1);
        assert_eq!(loaded.rules[0].app_id_contains.as_deref(), Some("term"));
        assert_eq!(loaded.rules[0].actions[0].name, "select_layout");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn executes_a_configured_hook_with_data_only_event_data() {
        let root = root();
        fs::create_dir_all(root.join("hooks")).expect("hooks");
        fs::write(
            root.join("config.lua"),
            "settings = {}\nmodes = {}\nlayouts = {}\nhooks = { opened = 'hooks/opened.lua' }\n",
        )
        .expect("config");
        fs::write(
            root.join("hooks/opened.lua"),
            "return function(event) if event.window_id ~= 7 then error('wrong event') end end\n",
        )
        .expect("hook");
        let loader = ScriptLoader::new(&root, ScriptLimits::default()).expect("loader");
        let candidate = loader.load().expect("load");
        loader
            .execute_hook(
                &candidate.hook_paths["opened"],
                &serde_json::json!({ "window_id": 7 }),
            )
            .expect("hook execution");
        fs::remove_dir_all(root).expect("cleanup");
    }
}

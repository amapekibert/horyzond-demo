//! Sandboxed, bounded Lua loading with Horyzond's `source()` import function.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mlua::{Error as LuaError, HookTriggers, Lua, LuaOptions, StdLib, Value, VmState};

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
        state
            .borrow_mut()
            .dependencies
            .extend(layout_paths.values().cloned());
        Ok(LoadedScript {
            dependencies: state.borrow().dependencies.clone(),
            layout_paths,
            default_layout,
        })
    }
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
            "settings = {}\nmodes = {}\nlayouts = {}\nvalue = source('sub.lua')\nagain = source('sub.lua')\n",
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
            "settings = {}\nmodes = {}\nlayouts = {}\nsource('a.lua')\n",
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
}

//! Non-destructive installation of Horyzond's shipped configuration files.
//!
//! Lua parsing and file watching deliberately begin in P2. This P1 crate only
//! creates absent files and never replaces user configuration.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use wm_script::{LoadedScript, ScriptLimits, ScriptLoader};

/// Resolves the Horyzond configuration directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigPath {
    root: PathBuf,
}

impl ConfigPath {
    /// Uses an explicit path, intended for tests or an additional instance.
    #[must_use]
    pub fn from_override(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Resolves the requested default path as `~/.config/horyzond`.
    ///
    /// # Errors
    ///
    /// Returns an error when the process home directory is unavailable.
    pub fn default_for_current_user() -> Result<Self, ConfigError> {
        let home = std::env::var_os("HOME").ok_or(ConfigError::MissingHomeDirectory)?;
        Ok(Self::from_override(
            PathBuf::from(home).join(".config").join("horyzond"),
        ))
    }

    /// Returns the resolved configuration root.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.root
    }

    /// Acquires an exclusive lock outside the watched configuration tree.
    ///
    /// # Errors
    ///
    /// Returns an error when the runtime directory is unavailable, cannot be
    /// created, or another Horyzond instance already uses this config root.
    pub fn acquire_instance_lock(&self) -> Result<InstanceLock, ConfigError> {
        let runtime =
            std::env::var_os("XDG_RUNTIME_DIR").ok_or(ConfigError::MissingRuntimeDirectory)?;
        self.acquire_instance_lock_at(Path::new(&runtime))
    }

    /// Acquires an exclusive lock below an explicit runtime directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the lock cannot be acquired.
    pub fn acquire_instance_lock_at(&self, runtime: &Path) -> Result<InstanceLock, ConfigError> {
        let directory = runtime.join("horyzond");
        fs::create_dir_all(&directory)?;
        let lock_path = directory.join(format!("{}.lock", stable_path_hash(&self.root)));
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&lock_path)
        {
            Ok(mut file) => {
                writeln!(file, "pid={}", std::process::id())?;
                file.sync_all()?;
                Ok(InstanceLock { path: lock_path })
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Err(ConfigError::AlreadyRunning(lock_path))
            }
            Err(error) => Err(error.into()),
        }
    }
}

/// An exclusive runtime lock released when dropped.
#[derive(Debug)]
pub struct InstanceLock {
    path: PathBuf,
}

impl InstanceLock {
    /// Returns the runtime lock path for diagnostics.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Details of a successful non-destructive bootstrap.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BootstrapReport {
    pub created: Vec<PathBuf>,
    pub preserved: Vec<PathBuf>,
}

/// Creates all required configuration directories and only absent default files.
///
/// # Errors
///
/// Returns an I/O error when the root, a required directory, or an absent file
/// cannot be created. Existing files are never opened for writing.
pub fn install_defaults(config: &ConfigPath) -> Result<BootstrapReport, ConfigError> {
    let root = config.as_path();
    fs::create_dir_all(root)?;
    let mut report = BootstrapReport::default();
    for asset in DEFAULT_ASSETS {
        let target = root.join(asset.relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        match write_if_absent(&target, asset.contents) {
            Ok(true) => report.created.push(target),
            Ok(false) => report.preserved.push(target),
            Err(error) => return Err(error.into()),
        }
    }
    for directory in ["state", "logs", "crashes"] {
        fs::create_dir_all(root.join(directory))?;
    }
    Ok(report)
}

/// A transactional configuration candidate manager using filesystem polling.
///
/// The manager is deliberately independent of compositor state: callers apply
/// a returned generation only at their own safe event-loop boundary.
#[derive(Debug)]
pub struct ConfigManager {
    loader: ScriptLoader,
    active: Option<LoadedScript>,
    fingerprints: std::collections::BTreeMap<PathBuf, FileFingerprint>,
    generation: u64,
}

/// The outcome of an initial load or a reload attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReloadOutcome {
    /// The active candidate remains current.
    Unchanged,
    /// A fully evaluated candidate replaced the previous generation.
    Applied { generation: u64 },
    /// The previous valid candidate remains active; the diagnostic is safe to log.
    Rejected { diagnostic: String },
}

impl ConfigManager {
    /// Creates a manager with bounded Lua evaluation.
    ///
    /// # Errors
    ///
    /// Returns an error when the configuration root cannot be canonicalized.
    pub fn new(config: &ConfigPath, limits: ScriptLimits) -> Result<Self, ConfigError> {
        Ok(Self {
            loader: ScriptLoader::new(config.as_path(), limits).map_err(ConfigError::Script)?,
            active: None,
            fingerprints: std::collections::BTreeMap::new(),
            generation: 0,
        })
    }

    /// Loads the initial candidate without changing an existing valid candidate on failure.
    pub fn load_initial(&mut self) -> ReloadOutcome {
        self.reload()
    }

    /// Reloads only when a loaded dependency changed, was removed, or was replaced.
    pub fn reload_if_changed(&mut self) -> ReloadOutcome {
        if self.active.is_none()
            || self
                .fingerprints
                .iter()
                .any(|(path, saved)| match FileFingerprint::read(path) {
                    Ok(current) => current != *saved,
                    Err(_) => true,
                })
        {
            self.reload()
        } else {
            ReloadOutcome::Unchanged
        }
    }

    /// Forces an evaluation and commits it only after it succeeds.
    pub fn reload(&mut self) -> ReloadOutcome {
        match self.loader.load() {
            Ok(candidate) => {
                self.fingerprints = candidate
                    .dependencies
                    .iter()
                    .filter_map(|path| {
                        FileFingerprint::read(path)
                            .ok()
                            .map(|fingerprint| (path.clone(), fingerprint))
                    })
                    .collect();
                self.active = Some(candidate);
                self.generation += 1;
                ReloadOutcome::Applied {
                    generation: self.generation,
                }
            }
            Err(error) => ReloadOutcome::Rejected {
                diagnostic: error.to_string(),
            },
        }
    }

    /// Returns the active generation, or zero before the first valid load.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the canonical Lua provider path selected for a layout by the
    /// active configuration candidate.
    #[must_use]
    pub fn layout_path(&self, layout: &str) -> Option<&Path> {
        self.active
            .as_ref()?
            .layout_paths
            .get(layout)
            .map(PathBuf::as_path)
    }

    /// Returns every canonical provider path from the active configuration candidate.
    #[must_use]
    pub fn layout_paths(&self) -> Option<&std::collections::BTreeMap<String, PathBuf>> {
        self.active
            .as_ref()
            .map(|candidate| &candidate.layout_paths)
    }

    /// Returns the default layout selected by the active configuration candidate.
    #[must_use]
    pub fn default_layout(&self) -> Option<&str> {
        self.active
            .as_ref()
            .map(|candidate| candidate.default_layout.as_str())
    }
    /// Returns modal bindings from the active configuration candidate.
    #[must_use]
    pub fn input(&self) -> Option<&wm_script::InputConfig> {
        self.active.as_ref().map(|candidate| &candidate.input)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileFingerprint {
    modified: Option<SystemTime>,
    length: u64,
}
impl FileFingerprint {
    fn read(path: &Path) -> io::Result<Self> {
        let metadata = fs::metadata(path)?;
        Ok(Self {
            modified: metadata.modified().ok(),
            length: metadata.len(),
        })
    }
}

fn write_if_absent(path: &Path, contents: &str) -> io::Result<bool> {
    let mut file = match OpenOptions::new().create_new(true).write(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(error) => return Err(error),
    };
    if let Err(error) = file
        .write_all(contents.as_bytes())
        .and_then(|()| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(true)
}

struct DefaultAsset {
    relative_path: &'static str,
    contents: &'static str,
}

const DEFAULT_ASSETS: &[DefaultAsset] = &[
    DefaultAsset {
        relative_path: "config.lua",
        contents: include_str!("../../../config/config.lua"),
    },
    DefaultAsset {
        relative_path: "keybinds.lua",
        contents: include_str!("../../../config/keybinds.lua"),
    },
    DefaultAsset {
        relative_path: "modes.lua",
        contents: include_str!("../../../config/modes.lua"),
    },
    DefaultAsset {
        relative_path: "rules.lua",
        contents: include_str!("../../../config/rules.lua"),
    },
    DefaultAsset {
        relative_path: "theme.lua",
        contents: include_str!("../../../config/theme.lua"),
    },
    DefaultAsset {
        relative_path: "layouts/spatial.lua",
        contents: include_str!("../../../config/layouts/spatial.lua"),
    },
    DefaultAsset {
        relative_path: "layouts/scrolling.lua",
        contents: include_str!("../../../config/layouts/scrolling.lua"),
    },
    DefaultAsset {
        relative_path: "layouts/tiling.lua",
        contents: include_str!("../../../config/layouts/tiling.lua"),
    },
    DefaultAsset {
        relative_path: "layouts/stacking.lua",
        contents: include_str!("../../../config/layouts/stacking.lua"),
    },
    DefaultAsset {
        relative_path: "rules.d/media.lua",
        contents: include_str!("../../../config/rules.d/media.lua"),
    },
    DefaultAsset {
        relative_path: "rules.d/dev.lua",
        contents: include_str!("../../../config/rules.d/dev.lua"),
    },
    DefaultAsset {
        relative_path: "hooks/on_window_open.lua",
        contents: include_str!("../../../config/hooks/on_window_open.lua"),
    },
    DefaultAsset {
        relative_path: "hooks/on_workspace_change.lua",
        contents: include_str!("../../../config/hooks/on_workspace_change.lua"),
    },
    DefaultAsset {
        relative_path: "shaders/gl/window.vert",
        contents: include_str!("../../../config/shaders/gl/window.vert"),
    },
    DefaultAsset {
        relative_path: "shaders/gl/window.frag",
        contents: include_str!("../../../config/shaders/gl/window.frag"),
    },
    DefaultAsset {
        relative_path: "shaders/gl/rounded_border.frag",
        contents: include_str!("../../../config/shaders/gl/rounded_border.frag"),
    },
];

/// A path resolution or bootstrap failure.
#[derive(Debug)]
pub enum ConfigError {
    MissingHomeDirectory,
    MissingRuntimeDirectory,
    AlreadyRunning(PathBuf),
    Script(wm_script::ScriptError),
    Io(io::Error),
}
impl From<io::Error> for ConfigError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHomeDirectory => formatter.write_str("HOME is not set"),
            Self::MissingRuntimeDirectory => formatter.write_str("XDG_RUNTIME_DIR is not set"),
            Self::AlreadyRunning(path) => {
                write!(
                    formatter,
                    "another instance already holds {}",
                    path.display()
                )
            }
            Self::Script(error) => error.fmt(formatter),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}
impl std::error::Error for ConfigError {}

fn stable_path_hash(path: &Path) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in path.as_os_str().as_encoded_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::{ConfigPath, install_defaults};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "horyzond-config-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[test]
    fn install_creates_defaults_without_overwriting_edits() {
        let root = temporary_root();
        let config = ConfigPath::from_override(&root);
        let first = install_defaults(&config).expect("first install");
        assert!(
            first
                .created
                .iter()
                .any(|path| path.ends_with("config.lua"))
        );
        let config_file = root.join("config.lua");
        fs::write(&config_file, "-- user edit\n").expect("edit config");
        let second = install_defaults(&config).expect("second install");
        assert!(second.preserved.contains(&config_file));
        assert_eq!(
            fs::read_to_string(config_file).expect("read config"),
            "-- user edit\n"
        );
        assert!(root.join("shaders/gl/window.frag").exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn an_instance_lock_is_exclusive_and_lives_outside_the_config_root() {
        let root = temporary_root();
        let runtime = temporary_root();
        let config = ConfigPath::from_override(&root);
        let lock = config
            .acquire_instance_lock_at(&runtime)
            .expect("first lock");
        assert!(!lock.path().starts_with(&root));
        assert!(config.acquire_instance_lock_at(&runtime).is_err());
        drop(lock);
        assert!(config.acquire_instance_lock_at(&runtime).is_ok());
        if root.exists() {
            fs::remove_dir_all(root).expect("cleanup root");
        }
        fs::remove_dir_all(runtime).expect("cleanup runtime");
    }

    #[test]
    fn invalid_reload_preserves_the_last_valid_generation() {
        let root = temporary_root();
        fs::create_dir_all(&root).expect("root");
        fs::write(
            root.join("config.lua"),
            "settings = {}\nmodes = {}\nlayouts = {}\n",
        )
        .expect("valid config");
        let config = ConfigPath::from_override(&root);
        let mut manager = super::ConfigManager::new(&config, wm_script::ScriptLimits::default())
            .expect("manager");
        assert!(matches!(
            manager.load_initial(),
            super::ReloadOutcome::Applied { generation: 1 }
        ));
        fs::write(root.join("config.lua"), "this is not lua\n").expect("invalid config");
        assert!(matches!(
            manager.reload_if_changed(),
            super::ReloadOutcome::Rejected { .. }
        ));
        assert_eq!(manager.generation(), 1);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn shipped_configuration_loads_as_one_candidate() {
        let root = temporary_root();
        let config = ConfigPath::from_override(&root);
        install_defaults(&config).expect("install defaults");
        let mut manager = super::ConfigManager::new(&config, wm_script::ScriptLimits::default())
            .expect("manager");
        assert!(matches!(
            manager.load_initial(),
            super::ReloadOutcome::Applied { generation: 1 }
        ));
        assert!(manager.layout_path("spatial").is_some());
        assert_eq!(manager.default_layout(), Some("spatial"));
        fs::remove_dir_all(root).expect("cleanup");
    }
}

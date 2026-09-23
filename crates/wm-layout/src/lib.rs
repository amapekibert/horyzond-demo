//! Layout-provider protocol and bounded Lua runtime.
//!
//! This crate intentionally has no built-in knowledge of shipped layouts. A
//! layout is an opaque ID plus a provider file selected by configuration.

#![allow(clippy::cast_precision_loss)]

use mlua::{Error as LuaError, HookTriggers, Lua, LuaOptions, StdLib, Table, Value, VmState};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use wm_script::ScriptLimits;
use wm_types::{Rect, WindowId};

/// An opaque stable name chosen entirely by configuration.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct LayoutId(String);
impl LayoutId {
    /// Creates a non-empty layout identifier.
    ///
    /// # Errors
    ///
    /// Returns an error when the name has no non-whitespace text.
    pub fn new(value: impl Into<String>) -> Result<Self, LayoutIdError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(LayoutIdError::Empty);
        }
        Ok(Self(value))
    }
    /// Returns the configured name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for LayoutId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
/// Invalid layout identifier input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutIdError {
    Empty,
}
impl fmt::Display for LayoutIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("layout ID must not be empty")
    }
}
impl std::error::Error for LayoutIdError {}

/// Versioned, data-only state for one workspace's selected layouts.
///
/// Lua closures and provider source are intentionally excluded. The state can
/// survive provider replacement and be validated before use.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LayoutState {
    version: u32,
    active_layout: LayoutId,
    geometry: BTreeMap<LayoutId, BTreeMap<WindowId, Rect>>,
}
impl LayoutState {
    /// Current format revision for serialized layout state.
    pub const VERSION: u32 = 1;
    /// Creates state from a workspace's active layout and saved geometry.
    #[must_use]
    pub fn new(
        active_layout: LayoutId,
        geometry: BTreeMap<LayoutId, BTreeMap<WindowId, Rect>>,
    ) -> Self {
        Self {
            version: Self::VERSION,
            active_layout,
            geometry,
        }
    }
    /// Returns the active layout recorded in this state.
    #[must_use]
    pub fn active_layout(&self) -> &LayoutId {
        &self.active_layout
    }
    /// Returns saved geometry for one opaque layout ID.
    #[must_use]
    pub fn geometry(&self, layout: &LayoutId) -> Option<&BTreeMap<WindowId, Rect>> {
        self.geometry.get(layout)
    }
    /// Consumes this state into its data-only geometry map.
    #[must_use]
    pub fn into_geometry(self) -> BTreeMap<LayoutId, BTreeMap<WindowId, Rect>> {
        self.geometry
    }
    /// Encodes a deterministic JSON document for persistence or diagnosis.
    ///
    /// # Errors
    ///
    /// Returns an error only when the in-memory data cannot be represented as JSON.
    pub fn to_json(&self) -> Result<String, LayoutStateError> {
        serde_json::to_string(self).map_err(LayoutStateError::Json)
    }
    /// Decodes and validates a persisted JSON document.
    ///
    /// # Errors
    ///
    /// Returns an error when JSON is malformed or has an unsupported format version.
    pub fn from_json(json: &str) -> Result<Self, LayoutStateError> {
        let state: Self = serde_json::from_str(json).map_err(LayoutStateError::Json)?;
        if state.version != Self::VERSION {
            return Err(LayoutStateError::UnsupportedVersion(state.version));
        }
        Ok(state)
    }
}
/// A failure while encoding or decoding data-only layout state.
#[derive(Debug)]
pub enum LayoutStateError {
    Json(serde_json::Error),
    UnsupportedVersion(u32),
}
impl fmt::Display for LayoutStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => error.fmt(formatter),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported layout state version {version}")
            }
        }
    }
}
impl std::error::Error for LayoutStateError {}

/// Immutable data passed into one layout calculation.
#[derive(Clone, Debug)]
pub struct LayoutInput<'a> {
    pub windows: &'a [WindowId],
    pub bounds: Rect,
    pub existing: &'a BTreeMap<WindowId, Rect>,
    pub focused: Option<WindowId>,
}
/// An external layout provider that only returns world-space geometry.
pub trait LayoutEngine {
    /// Returns the opaque ID selected by configuration.
    fn id(&self) -> &LayoutId;
    /// Calculates one rectangle for every supplied window.
    fn calculate(&self, input: &LayoutInput<'_>) -> BTreeMap<WindowId, Rect>;
}

/// Generic last-resort placement for any unavailable or failing provider.
///
/// It preserves valid existing geometry and cascades newly seen windows. It
/// deliberately contains no policy tied to a shipped layout name.
#[derive(Clone, Debug)]
pub struct RecoveryLayout {
    id: LayoutId,
}
impl RecoveryLayout {
    /// Creates recovery placement for an arbitrary configured layout ID.
    #[must_use]
    pub fn new(id: LayoutId) -> Self {
        Self { id }
    }
}
impl LayoutEngine for RecoveryLayout {
    fn id(&self) -> &LayoutId {
        &self.id
    }
    fn calculate(&self, input: &LayoutInput<'_>) -> BTreeMap<WindowId, Rect> {
        input
            .windows
            .iter()
            .enumerate()
            .map(|(index, window)| {
                let offset = index as f64 * 24.0;
                let geometry = input.existing.get(window).copied().unwrap_or_else(|| {
                    Rect::new(
                        input.bounds.x + offset,
                        input.bounds.y + offset,
                        (input.bounds.width * 0.7).max(f64::MIN_POSITIVE),
                        (input.bounds.height * 0.7).max(f64::MIN_POSITIVE),
                    )
                    .expect("derived recovery rectangle")
                });
                (*window, geometry)
            })
            .collect()
    }
}

/// A restricted external Lua layout provider with generic recovery placement.
#[derive(Clone, Debug)]
pub struct LuaLayout {
    id: LayoutId,
    source: String,
    limits: ScriptLimits,
}
impl LuaLayout {
    /// Loads a provider source file.
    ///
    /// # Errors
    ///
    /// Returns an I/O failure when the provider file cannot be read.
    pub fn from_file(id: LayoutId, path: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        Self::from_file_with_limits(id, path, ScriptLimits::default())
    }
    /// Loads a provider source file with explicit callback resource limits.
    ///
    /// # Errors
    ///
    /// Returns an I/O failure when the provider file cannot be read.
    pub fn from_file_with_limits(
        id: LayoutId,
        path: impl AsRef<Path>,
        limits: ScriptLimits,
    ) -> Result<Self, std::io::Error> {
        Ok(Self {
            id,
            source: fs::read_to_string(path)?,
            limits,
        })
    }

    /// Validates the provider declaration before it can enter an active registry.
    ///
    /// This evaluates only the top-level declaration. `calculate()` remains
    /// isolated and bounded for each layout request.
    fn validate(&self) -> Result<(), String> {
        let lua = restricted_lua(self.limits)?;
        lua.load(&self.source)
            .exec()
            .map_err(|error| error.to_string())?;
        validate_layout_contract(&lua, &self.id)
    }

    fn calculate_lua(&self, input: &LayoutInput<'_>) -> Result<BTreeMap<WindowId, Rect>, String> {
        let lua = restricted_lua(self.limits)?;
        lua.load(&self.source)
            .exec()
            .map_err(|error| error.to_string())?;
        validate_layout_contract(&lua, &self.id)?;
        let calculate: mlua::Function = lua
            .globals()
            .get("calculate")
            .map_err(|error| error.to_string())?;
        let windows = lua.create_table().map_err(|error| error.to_string())?;
        for (index, window_id) in input.windows.iter().enumerate() {
            let window = lua.create_table().map_err(|error| error.to_string())?;
            window
                .set("id", window_id.get())
                .map_err(|error| error.to_string())?;
            if let Some(rect) = input.existing.get(window_id) {
                window.set("x", rect.x).map_err(|error| error.to_string())?;
                window.set("y", rect.y).map_err(|error| error.to_string())?;
                window
                    .set("width", rect.width)
                    .map_err(|error| error.to_string())?;
                window
                    .set("height", rect.height)
                    .map_err(|error| error.to_string())?;
            }
            windows
                .set(index + 1, window)
                .map_err(|error| error.to_string())?;
        }
        let result: Table = calculate
            .call((
                windows,
                rectangle_table(&lua, input.bounds)?,
                lua.create_table().map_err(|error| error.to_string())?,
            ))
            .map_err(|error| error.to_string())?;
        let mut output = BTreeMap::new();
        for window_id in input.windows {
            let value: Value = result
                .get(window_id.get())
                .map_err(|error| error.to_string())?;
            let Value::Table(table) = value else {
                return Err(format!("missing rectangle for {window_id}"));
            };
            output.insert(*window_id, read_rect(&table)?);
        }
        Ok(output)
    }
}
impl LayoutEngine for LuaLayout {
    fn id(&self) -> &LayoutId {
        &self.id
    }
    fn calculate(&self, input: &LayoutInput<'_>) -> BTreeMap<WindowId, Rect> {
        self.calculate_lua(input)
            .unwrap_or_else(|_| RecoveryLayout::new(self.id.clone()).calculate(input))
    }
}

/// Provider files selected from a validated configuration candidate.
#[derive(Clone, Debug, Default)]
pub struct LayoutProviders {
    providers: BTreeMap<LayoutId, LuaLayout>,
}

/// The independently committed result of replacing provider files.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProviderReload {
    /// IDs whose new providers passed validation and replaced the old entry.
    pub applied: Vec<LayoutId>,
    /// IDs retaining their previous validated provider after a failed update.
    pub retained: Vec<LayoutId>,
}

impl LayoutProviders {
    /// Loads every named provider from canonical configuration paths.
    #[must_use]
    pub fn from_layout_paths(layout_paths: &BTreeMap<String, PathBuf>) -> Self {
        let mut providers = Self::default();
        let _ = providers.reload(layout_paths);
        providers
    }

    /// Replaces valid providers independently and retains a previous valid
    /// provider when its new file is invalid. Removing an ID from the active
    /// configuration intentionally removes it from this registry.
    pub fn reload(&mut self, layout_paths: &BTreeMap<String, PathBuf>) -> ProviderReload {
        let previous = std::mem::take(&mut self.providers);
        let mut next = BTreeMap::new();
        let mut result = ProviderReload::default();
        for (name, path) in layout_paths {
            let Ok(id) = LayoutId::new(name.clone()) else {
                continue;
            };
            let candidate = LuaLayout::from_file(id.clone(), path)
                .ok()
                .filter(|provider| provider.validate().is_ok());
            if let Some(provider) = candidate {
                result.applied.push(id.clone());
                next.insert(id, provider);
            } else if let Some(previous) = previous.get(&id) {
                result.retained.push(id.clone());
                next.insert(id, previous.clone());
            }
        }
        self.providers = next;
        result
    }
    /// Returns the selected provider or generic recovery placement.
    #[must_use]
    pub fn provider(&self, id: &LayoutId) -> Box<dyn LayoutEngine> {
        self.providers.get(id).cloned().map_or_else(
            || Box::new(RecoveryLayout::new(id.clone())) as Box<dyn LayoutEngine>,
            |provider| Box::new(provider),
        )
    }
    /// Reports whether a readable provider exists for an opaque layout ID.
    #[must_use]
    pub fn is_configured(&self, id: &LayoutId) -> bool {
        self.providers.contains_key(id)
    }
}

fn restricted_lua(limits: ScriptLimits) -> Result<Lua, String> {
    let lua = Lua::new_with(
        StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8,
        LuaOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    lua.set_memory_limit(limits.memory_bytes)
        .map_err(|error| error.to_string())?;
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
    .map_err(|error| error.to_string())?;
    Ok(lua)
}
fn validate_layout_contract(lua: &Lua, id: &LayoutId) -> Result<(), String> {
    let contract: Table = lua
        .globals()
        .get("layout")
        .map_err(|error| error.to_string())?;
    let api_version: u64 = contract
        .get("api_version")
        .map_err(|error| error.to_string())?;
    if api_version != 1 {
        return Err(format!("unsupported layout API version {api_version}"));
    }
    let declared_id: String = contract.get("id").map_err(|error| error.to_string())?;
    if declared_id != id.as_str() {
        return Err(format!(
            "layout ID '{declared_id}' does not match configured ID '{id}'"
        ));
    }
    Ok(())
}
fn rectangle_table(lua: &Lua, rect: Rect) -> Result<Table, String> {
    let table = lua.create_table().map_err(|error| error.to_string())?;
    table.set("x", rect.x).map_err(|error| error.to_string())?;
    table.set("y", rect.y).map_err(|error| error.to_string())?;
    table
        .set("width", rect.width)
        .map_err(|error| error.to_string())?;
    table
        .set("height", rect.height)
        .map_err(|error| error.to_string())?;
    Ok(table)
}
fn read_rect(table: &Table) -> Result<Rect, String> {
    Rect::new(
        table.get("x").map_err(|error| error.to_string())?,
        table.get("y").map_err(|error| error.to_string())?,
        table.get("width").map_err(|error| error.to_string())?,
        table.get("height").map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(value: &str) -> LayoutId {
        LayoutId::new(value).expect("ID")
    }
    fn input<'a>(ids: &'a [WindowId], existing: &'a BTreeMap<WindowId, Rect>) -> LayoutInput<'a> {
        LayoutInput {
            windows: ids,
            bounds: Rect::new(0., 0., 100., 100.).expect("bounds"),
            existing,
            focused: None,
        }
    }
    fn source(name: &str, body: &str) -> String {
        format!("layout = {{ api_version = 1, id = '{name}' }} {body}")
    }
    #[test]
    fn arbitrary_ids_are_valid_without_builtin_registration() {
        let custom = id("my-own-layout");
        let ids = [WindowId::new(1)];
        let existing = BTreeMap::new();
        assert_eq!(RecoveryLayout::new(custom.clone()).id(), &custom);
        assert_eq!(
            RecoveryLayout::new(custom)
                .calculate(&input(&ids, &existing))
                .len(),
            1
        );
    }
    #[test]
    fn lua_provider_uses_opaque_configured_id() {
        let ids = [WindowId::new(1)];
        let existing = BTreeMap::new();
        let layout = LuaLayout {
            id: id("third-party"),
            source: source(
                "third-party",
                "function calculate() return { [1] = { x = 7, y = 8, width = 9, height = 10 } } end",
            ),
            limits: ScriptLimits::default(),
        };
        assert!((layout.calculate(&input(&ids, &existing))[&ids[0]].x - 7.).abs() < f64::EPSILON);
    }
    #[test]
    fn invalid_lua_geometry_uses_generic_recovery() {
        let ids = [WindowId::new(1)];
        let existing = BTreeMap::new();
        let layout = LuaLayout {
            id: id("broken"),
            source: source(
                "broken",
                "function calculate() return { [1] = { x = 0, y = 0, width = 0, height = 1 } } end",
            ),
            limits: ScriptLimits::default(),
        };
        assert!(
            (layout.calculate(&input(&ids, &existing))[&ids[0]].width - 70.).abs() < f64::EPSILON
        );
    }
    #[test]
    fn provider_map_discovers_an_unlisted_custom_layout() {
        let root = std::env::temp_dir().join(format!(
            "horyzond-layout-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("root");
        let path = root.join("custom.lua");
        fs::write(
            &path,
            source(
                "custom-grid",
                "function calculate() return { [1] = { x = 1, y = 2, width = 3, height = 4 } } end",
            ),
        )
        .expect("provider");
        let providers =
            LayoutProviders::from_layout_paths(&BTreeMap::from([("custom-grid".to_owned(), path)]));
        let layout_id = id("custom-grid");
        let ids = [WindowId::new(1)];
        let existing = BTreeMap::new();
        assert!(providers.is_configured(&layout_id));
        assert!(
            (providers
                .provider(&layout_id)
                .calculate(&input(&ids, &existing))[&ids[0]]
                .x
                - 1.)
                .abs()
                < f64::EPSILON
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn invalid_provider_is_excluded_without_affecting_other_providers() {
        let root = std::env::temp_dir().join(format!(
            "horyzond-layout-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("root");
        let valid = root.join("valid.lua");
        let invalid = root.join("invalid.lua");
        fs::write(
            &valid,
            source(
                "valid",
                "function calculate() return { [1] = { x = 1, y = 2, width = 3, height = 4 } } end",
            ),
        )
        .expect("valid provider");
        fs::write(
            &invalid,
            source(
                "different-id",
                "function calculate() return { [1] = { x = 1, y = 2, width = 3, height = 4 } } end",
            ),
        )
        .expect("invalid provider");
        let providers = LayoutProviders::from_layout_paths(&BTreeMap::from([
            ("valid".to_owned(), valid),
            ("invalid".to_owned(), invalid),
        ]));
        assert!(providers.is_configured(&id("valid")));
        assert!(!providers.is_configured(&id("invalid")));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn invalid_reload_keeps_only_the_previous_provider_for_that_id() {
        let root = std::env::temp_dir().join(format!(
            "horyzond-layout-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("root");
        let retained_path = root.join("retained.lua");
        let replaced_path = root.join("replaced.lua");
        fs::write(
            &retained_path,
            source(
                "retained",
                "function calculate() return { [1] = { x = 1, y = 2, width = 3, height = 4 } } end",
            ),
        )
        .expect("retained provider");
        fs::write(
            &replaced_path,
            source(
                "replaced",
                "function calculate() return { [1] = { x = 5, y = 6, width = 7, height = 8 } } end",
            ),
        )
        .expect("replaced provider");
        let paths = BTreeMap::from([
            ("retained".to_owned(), retained_path.clone()),
            ("replaced".to_owned(), replaced_path.clone()),
        ]);
        let mut providers = LayoutProviders::from_layout_paths(&paths);
        fs::write(&retained_path, "layout = { api_version = 1, id = 'wrong' }")
            .expect("broken provider");
        fs::write(
            &replaced_path,
            source(
                "replaced",
                "function calculate() return { [1] = { x = 9, y = 10, width = 11, height = 12 } } end",
            ),
        )
        .expect("updated provider");
        let result = providers.reload(&paths);
        assert_eq!(result.retained, vec![id("retained")]);
        assert_eq!(result.applied, vec![id("replaced")]);
        let ids = [WindowId::new(1)];
        let existing = BTreeMap::new();
        assert!(
            (providers
                .provider(&id("retained"))
                .calculate(&input(&ids, &existing))[&ids[0]]
                .x
                - 1.0)
                .abs()
                < f64::EPSILON
        );
        assert!(
            (providers
                .provider(&id("replaced"))
                .calculate(&input(&ids, &existing))[&ids[0]]
                .x
                - 9.0)
                .abs()
                < f64::EPSILON
        );
        fs::remove_dir_all(root).expect("cleanup");
    }
    #[test]
    fn timed_out_provider_uses_recovery() {
        let ids = [WindowId::new(1)];
        let existing = BTreeMap::new();
        let layout = LuaLayout {
            id: id("loop"),
            source: source("loop", "function calculate() while true do end end"),
            limits: ScriptLimits {
                memory_bytes: 1024 * 1024,
                instruction_limit: 1_000,
            },
        };
        assert_eq!(layout.calculate(&input(&ids, &existing)).len(), 1);
    }
    #[test]
    fn layout_state_round_trips_without_provider_code() {
        let custom = id("custom");
        let state = LayoutState::new(
            custom.clone(),
            BTreeMap::from([(
                custom,
                BTreeMap::from([(WindowId::new(7), Rect::new(1., 2., 3., 4.).expect("rect"))]),
            )]),
        );
        let json = state.to_json().expect("encode");
        assert!(!json.contains("function"));
        assert_eq!(LayoutState::from_json(&json).expect("decode"), state);
    }
    #[test]
    fn layout_state_rejects_unknown_format_versions() {
        assert!(matches!(
            LayoutState::from_json(r#"{"version":99,"active_layout":"custom","geometry":{}}"#),
            Err(LayoutStateError::UnsupportedVersion(99))
        ));
    }
}

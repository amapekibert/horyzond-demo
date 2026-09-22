//! Deterministic world-space layout algorithms without backend or renderer state.

#![allow(clippy::cast_precision_loss)]

use mlua::{Error as LuaError, HookTriggers, Lua, LuaOptions, StdLib, Table, Value, VmState};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use wm_script::ScriptLimits;
use wm_types::{Rect, WindowId};

/// The four canonical Horyzond workspace profiles.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum LayoutProfile {
    Spatial,
    Scrolling,
    Tiling,
    Stacking,
}

/// Versioned, data-only state for one workspace's layout profiles.
///
/// Lua closures and provider source are intentionally excluded. The state can
/// therefore survive provider replacement and be validated before use.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LayoutState {
    version: u32,
    active_profile: LayoutProfile,
    profile_geometry: BTreeMap<LayoutProfile, BTreeMap<WindowId, Rect>>,
}
impl LayoutState {
    /// Current format revision for serialized layout state.
    pub const VERSION: u32 = 1;

    /// Creates state from a workspace's active profile and saved geometry.
    #[must_use]
    pub fn new(
        active_profile: LayoutProfile,
        profile_geometry: BTreeMap<LayoutProfile, BTreeMap<WindowId, Rect>>,
    ) -> Self {
        Self {
            version: Self::VERSION,
            active_profile,
            profile_geometry,
        }
    }

    /// Returns the active profile recorded in this state.
    #[must_use]
    pub const fn active_profile(&self) -> LayoutProfile {
        self.active_profile
    }

    /// Returns saved geometry for one profile.
    #[must_use]
    pub fn geometry(&self, profile: LayoutProfile) -> Option<&BTreeMap<WindowId, Rect>> {
        self.profile_geometry.get(&profile)
    }

    /// Consumes this state into its data-only profile geometry.
    #[must_use]
    pub fn into_geometry(self) -> BTreeMap<LayoutProfile, BTreeMap<WindowId, Rect>> {
        self.profile_geometry
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

/// A failure while encoding or decoding data-only profile state.
#[derive(Debug)]
pub enum LayoutStateError {
    Json(serde_json::Error),
    UnsupportedVersion(u32),
}
impl std::fmt::Display for LayoutStateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(error) => error.fmt(formatter),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported layout state version {version}")
            }
        }
    }
}
impl std::error::Error for LayoutStateError {}
impl LayoutProfile {
    /// Resolves one stable configuration-table name.
    #[must_use]
    pub fn from_config_name(name: &str) -> Option<Self> {
        match name {
            "spatial" => Some(Self::Spatial),
            "scrolling" => Some(Self::Scrolling),
            "tiling" => Some(Self::Tiling),
            "stacking" => Some(Self::Stacking),
            _ => None,
        }
    }
    /// Returns the stable configuration-table name for this profile.
    #[must_use]
    pub const fn config_name(self) -> &'static str {
        match self {
            Self::Spatial => "spatial",
            Self::Scrolling => "scrolling",
            Self::Tiling => "tiling",
            Self::Stacking => "stacking",
        }
    }
}

/// Immutable data passed into a profile calculation.
#[derive(Clone, Debug)]
pub struct LayoutInput<'a> {
    pub windows: &'a [WindowId],
    pub bounds: Rect,
    pub existing: &'a BTreeMap<WindowId, Rect>,
    pub focused: Option<WindowId>,
}

/// A profile computes world rectangles without observing protocol or renderer state.
pub trait LayoutEngine {
    fn profile(&self) -> LayoutProfile;
    fn calculate(&self, input: &LayoutInput<'_>) -> BTreeMap<WindowId, Rect>;
}

/// Preserves explicit world rectangles and gives new windows a centered default.
#[derive(Debug, Default)]
pub struct SpatialLayout;
impl LayoutEngine for SpatialLayout {
    fn profile(&self) -> LayoutProfile {
        LayoutProfile::Spatial
    }
    fn calculate(&self, input: &LayoutInput<'_>) -> BTreeMap<WindowId, Rect> {
        input
            .windows
            .iter()
            .enumerate()
            .map(|(index, id)| {
                (
                    *id,
                    input.existing.get(id).copied().unwrap_or_else(|| {
                        Rect::new(
                            input.bounds.x - 320.0 + index as f64 * 32.0,
                            input.bounds.y - 240.0 + index as f64 * 32.0,
                            640.0,
                            480.0,
                        )
                        .expect("constant rectangle")
                    }),
                )
            })
            .collect()
    }
}
/// Arranges windows in an infinite horizontal ribbon of columns.
#[derive(Debug, Default)]
pub struct ScrollingLayout;
impl LayoutEngine for ScrollingLayout {
    fn profile(&self) -> LayoutProfile {
        LayoutProfile::Scrolling
    }
    fn calculate(&self, input: &LayoutInput<'_>) -> BTreeMap<WindowId, Rect> {
        input
            .windows
            .iter()
            .enumerate()
            .map(|(index, id)| {
                (
                    *id,
                    Rect::new(
                        input.bounds.x + index as f64 * input.bounds.width,
                        input.bounds.y,
                        input.bounds.width,
                        input.bounds.height,
                    )
                    .expect("derived rectangle"),
                )
            })
            .collect()
    }
}
/// Arranges a master pane plus an equal vertical stack.
#[derive(Clone, Copy, Debug)]
pub struct TilingLayout {
    master_ratio: f64,
    gap: f64,
}
impl Default for TilingLayout {
    fn default() -> Self {
        Self {
            master_ratio: 0.5,
            gap: 0.0,
        }
    }
}
impl TilingLayout {
    /// Creates a master-stack layout with a positive inner gap.
    ///
    /// # Errors
    ///
    /// Returns an error when the ratio is outside `(0, 1)` or either setting
    /// is non-finite, or when the gap is negative.
    pub fn new(master_ratio: f64, gap: f64) -> Result<Self, TilingLayoutError> {
        if !master_ratio.is_finite() || master_ratio <= 0.0 || master_ratio >= 1.0 {
            return Err(TilingLayoutError::InvalidMasterRatio);
        }
        if !gap.is_finite() || gap < 0.0 {
            return Err(TilingLayoutError::InvalidGap);
        }
        Ok(Self { master_ratio, gap })
    }

    /// Returns the fraction of available width allocated to the master pane.
    #[must_use]
    pub const fn master_ratio(self) -> f64 {
        self.master_ratio
    }

    /// Returns the requested gap between adjacent panes.
    #[must_use]
    pub const fn gap(self) -> f64 {
        self.gap
    }
}
/// Invalid native master-stack layout settings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TilingLayoutError {
    InvalidMasterRatio,
    InvalidGap,
}
impl std::fmt::Display for TilingLayoutError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMasterRatio => {
                formatter.write_str("master ratio must be finite and in (0, 1)")
            }
            Self::InvalidGap => formatter.write_str("gap must be finite and non-negative"),
        }
    }
}
impl std::error::Error for TilingLayoutError {}
impl LayoutEngine for TilingLayout {
    fn profile(&self) -> LayoutProfile {
        LayoutProfile::Tiling
    }
    fn calculate(&self, input: &LayoutInput<'_>) -> BTreeMap<WindowId, Rect> {
        let n = input.windows.len();
        if n <= 1 {
            return input
                .windows
                .iter()
                .copied()
                .map(|window| (window, input.bounds))
                .collect();
        }
        let horizontal_gap = self.gap.min(input.bounds.width / 3.0);
        let available_width = input.bounds.width - horizontal_gap;
        let master_width = available_width * self.master_ratio;
        let stack_width = available_width - master_width;
        let stack_count = n - 1;
        let vertical_gap = if stack_count <= 1 {
            0.0
        } else {
            self.gap
                .min(input.bounds.height / (2.0 * stack_count as f64))
        };
        let stack_height =
            (input.bounds.height - vertical_gap * (stack_count - 1) as f64) / stack_count as f64;
        input
            .windows
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let r = if i == 0 {
                    Rect::new(
                        input.bounds.x,
                        input.bounds.y,
                        master_width,
                        input.bounds.height,
                    )
                    .expect("master")
                } else {
                    Rect::new(
                        input.bounds.x + master_width + horizontal_gap,
                        input.bounds.y + (i - 1) as f64 * (stack_height + vertical_gap),
                        stack_width,
                        stack_height,
                    )
                    .expect("stack")
                };
                (*id, r)
            })
            .collect()
    }
}
/// Preserves manually controlled rectangles and cascades new windows.
#[derive(Debug, Default)]
pub struct StackingLayout;
impl LayoutEngine for StackingLayout {
    fn profile(&self) -> LayoutProfile {
        LayoutProfile::Stacking
    }
    fn calculate(&self, input: &LayoutInput<'_>) -> BTreeMap<WindowId, Rect> {
        input
            .windows
            .iter()
            .enumerate()
            .map(|(i, id)| {
                (
                    *id,
                    input.existing.get(id).copied().unwrap_or_else(|| {
                        Rect::new(
                            input.bounds.x + 40.0 + i as f64 * 24.0,
                            input.bounds.y + 40.0 + i as f64 * 24.0,
                            input.bounds.width * 0.7,
                            input.bounds.height * 0.7,
                        )
                        .expect("cascade")
                    }),
                )
            })
            .collect()
    }
}

/// Returns the built-in safe provider for a profile when a future Lua provider fails.
#[must_use]
pub fn builtin(profile: LayoutProfile) -> Box<dyn LayoutEngine> {
    match profile {
        LayoutProfile::Spatial => Box::new(SpatialLayout),
        LayoutProfile::Scrolling => Box::new(ScrollingLayout),
        LayoutProfile::Tiling => Box::new(TilingLayout::default()),
        LayoutProfile::Stacking => Box::new(StackingLayout),
    }
}

/// A restricted external Lua layout provider with a native fallback.
#[derive(Clone, Debug)]
pub struct LuaLayout {
    profile: LayoutProfile,
    source: String,
    limits: ScriptLimits,
}

/// Profile providers selected from a validated configuration candidate.
///
/// Files that cannot be read are omitted. Callers receive the independent
/// native provider for an omitted profile, so a broken provider never blocks
/// the other profiles.
#[derive(Clone, Debug, Default)]
pub struct LayoutProviders {
    providers: BTreeMap<LayoutProfile, LuaLayout>,
}
impl LayoutProviders {
    /// Loads recognized profile files from canonical configuration paths.
    #[must_use]
    pub fn from_profile_paths(profile_paths: &BTreeMap<String, std::path::PathBuf>) -> Self {
        let mut providers = BTreeMap::new();
        for profile in [
            LayoutProfile::Spatial,
            LayoutProfile::Scrolling,
            LayoutProfile::Tiling,
            LayoutProfile::Stacking,
        ] {
            if let Some(path) = profile_paths.get(profile.config_name())
                && let Ok(provider) = LuaLayout::from_file(profile, path)
            {
                providers.insert(profile, provider);
            }
        }
        Self { providers }
    }

    /// Returns the configured provider or the safe native provider for one profile.
    #[must_use]
    pub fn provider(&self, profile: LayoutProfile) -> Box<dyn LayoutEngine> {
        self.providers
            .get(&profile)
            .cloned()
            .map_or_else(|| builtin(profile), |provider| Box::new(provider))
    }

    /// Reports whether a profile has a readable configured provider.
    #[must_use]
    pub fn is_configured(&self, profile: LayoutProfile) -> bool {
        self.providers.contains_key(&profile)
    }
}
impl LuaLayout {
    /// Loads a profile source file.
    ///
    /// # Errors
    ///
    /// Returns an I/O failure when the profile file cannot be read.
    pub fn from_file(
        profile: LayoutProfile,
        path: impl AsRef<Path>,
    ) -> Result<Self, std::io::Error> {
        Ok(Self {
            profile,
            source: fs::read_to_string(path)?,
            limits: ScriptLimits::default(),
        })
    }
    /// Loads a profile source file with explicit callback resource limits.
    ///
    /// # Errors
    ///
    /// Returns an I/O failure when the profile file cannot be read.
    pub fn from_file_with_limits(
        profile: LayoutProfile,
        path: impl AsRef<Path>,
        limits: ScriptLimits,
    ) -> Result<Self, std::io::Error> {
        Ok(Self {
            profile,
            source: fs::read_to_string(path)?,
            limits,
        })
    }
    fn calculate_lua(&self, input: &LayoutInput<'_>) -> Result<BTreeMap<WindowId, Rect>, String> {
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8,
            LuaOptions::default(),
        )
        .map_err(|e| e.to_string())?;
        lua.set_memory_limit(self.limits.memory_bytes)
            .map_err(|e| e.to_string())?;
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
        .map_err(|e| e.to_string())?;
        lua.load(&self.source).exec().map_err(|e| e.to_string())?;
        validate_layout_contract(&lua, self.profile)?;
        let calculate: mlua::Function =
            lua.globals().get("calculate").map_err(|e| e.to_string())?;
        let windows = lua.create_table().map_err(|e| e.to_string())?;
        for (index, id) in input.windows.iter().enumerate() {
            let window = lua.create_table().map_err(|e| e.to_string())?;
            window.set("id", id.get()).map_err(|e| e.to_string())?;
            if let Some(rect) = input.existing.get(id) {
                window.set("x", rect.x).map_err(|e| e.to_string())?;
                window.set("y", rect.y).map_err(|e| e.to_string())?;
                window.set("width", rect.width).map_err(|e| e.to_string())?;
                window
                    .set("height", rect.height)
                    .map_err(|e| e.to_string())?;
            }
            windows.set(index + 1, window).map_err(|e| e.to_string())?;
        }
        let bounds = rectangle_table(&lua, input.bounds)?;
        let camera = lua.create_table().map_err(|e| e.to_string())?;
        let result: Table = calculate
            .call((windows, bounds, camera))
            .map_err(|e| e.to_string())?;
        let mut output = BTreeMap::new();
        for id in input.windows {
            let value: Value = result.get(id.get()).map_err(|e| e.to_string())?;
            let Value::Table(table) = value else {
                return Err(format!("missing rectangle for {id}"));
            };
            output.insert(*id, read_rect(&table)?);
        }
        Ok(output)
    }
}

fn validate_layout_contract(lua: &Lua, profile: LayoutProfile) -> Result<(), String> {
    let contract: Table = lua.globals().get("layout").map_err(|e| e.to_string())?;
    let api_version: u64 = contract.get("api_version").map_err(|e| e.to_string())?;
    if api_version != 1 {
        return Err(format!("unsupported layout API version {api_version}"));
    }
    let declared_profile: String = contract.get("profile").map_err(|e| e.to_string())?;
    if declared_profile != profile.config_name() {
        return Err(format!(
            "layout profile '{declared_profile}' does not match '{}'",
            profile.config_name()
        ));
    }
    Ok(())
}
impl LayoutEngine for LuaLayout {
    fn profile(&self) -> LayoutProfile {
        self.profile
    }
    fn calculate(&self, input: &LayoutInput<'_>) -> BTreeMap<WindowId, Rect> {
        self.calculate_lua(input)
            .unwrap_or_else(|_| builtin(self.profile).calculate(input))
    }
}
fn rectangle_table(lua: &Lua, rect: Rect) -> Result<Table, String> {
    let table = lua.create_table().map_err(|e| e.to_string())?;
    table.set("x", rect.x).map_err(|e| e.to_string())?;
    table.set("y", rect.y).map_err(|e| e.to_string())?;
    table.set("width", rect.width).map_err(|e| e.to_string())?;
    table
        .set("height", rect.height)
        .map_err(|e| e.to_string())?;
    Ok(table)
}
fn read_rect(table: &Table) -> Result<Rect, String> {
    Rect::new(
        table.get("x").map_err(|e| e.to_string())?,
        table.get("y").map_err(|e| e.to_string())?,
        table.get("width").map_err(|e| e.to_string())?,
        table.get("height").map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input<'a>(ids: &'a [WindowId], existing: &'a BTreeMap<WindowId, Rect>) -> LayoutInput<'a> {
        LayoutInput {
            windows: ids,
            bounds: Rect::new(0., 0., 100., 100.).expect("bounds"),
            existing,
            focused: None,
        }
    }
    #[test]
    fn profiles_produce_valid_rectangles() {
        let ids = [WindowId::new(1), WindowId::new(2), WindowId::new(3)];
        let existing = BTreeMap::new();
        for profile in [
            LayoutProfile::Spatial,
            LayoutProfile::Scrolling,
            LayoutProfile::Tiling,
            LayoutProfile::Stacking,
        ] {
            assert_eq!(builtin(profile).calculate(&input(&ids, &existing)).len(), 3);
        }
    }
    #[test]
    fn tiling_has_no_overlap_in_stack() {
        let ids = [WindowId::new(1), WindowId::new(2), WindowId::new(3)];
        let existing = BTreeMap::new();
        let result = TilingLayout::default().calculate(&input(&ids, &existing));
        assert!((result[&ids[0]].width - 50.).abs() < f64::EPSILON);
        assert!((result[&ids[1]].height - 50.).abs() < f64::EPSILON);
    }

    #[test]
    fn tiling_options_apply_master_ratio_and_gaps() {
        let ids = [WindowId::new(1), WindowId::new(2), WindowId::new(3)];
        let existing = BTreeMap::new();
        let layout = TilingLayout::new(0.6, 10.0).expect("options");
        let result = layout.calculate(&input(&ids, &existing));
        assert!((layout.master_ratio() - 0.6).abs() < f64::EPSILON);
        assert!((layout.gap() - 10.0).abs() < f64::EPSILON);
        assert!((result[&ids[0]].width - 54.0).abs() < f64::EPSILON);
        assert!((result[&ids[1]].x - 64.0).abs() < f64::EPSILON);
        assert!((result[&ids[1]].height - 45.0).abs() < f64::EPSILON);
        assert!((result[&ids[2]].y - 55.0).abs() < f64::EPSILON);
    }

    #[test]
    fn tiling_options_reject_invalid_values() {
        assert!(TilingLayout::new(0.0, 0.0).is_err());
        assert!(TilingLayout::new(1.0, 0.0).is_err());
        assert!(TilingLayout::new(0.5, -1.0).is_err());
    }
    #[test]
    fn lua_provider_returns_validated_geometry() {
        let ids = [WindowId::new(1)];
        let existing = BTreeMap::new();
        let layout = LuaLayout { profile: LayoutProfile::Spatial, source: "layout = { api_version = 1, profile = 'spatial' } function calculate(windows, bounds, camera) return { [1] = { x = 7, y = 8, width = 9, height = 10 } } end".to_owned(), limits: ScriptLimits::default() };
        let result = layout.calculate(&input(&ids, &existing));
        assert!((result[&ids[0]].x - 7.0).abs() < f64::EPSILON);
    }
    #[test]
    fn invalid_lua_geometry_uses_builtin_fallback() {
        let ids = [WindowId::new(1)];
        let existing = BTreeMap::new();
        let layout = LuaLayout {
            profile: LayoutProfile::Tiling,
            source:
                "layout = { api_version = 1, profile = 'tiling' } function calculate() return { [1] = { x = 0, y = 0, width = 0, height = 1 } } end"
                    .to_owned(),
            limits: ScriptLimits::default(),
        };
        let result = layout.calculate(&input(&ids, &existing));
        assert!((result[&ids[0]].width - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn configured_provider_is_selected_by_profile_name() {
        let root = std::env::temp_dir().join(format!(
            "horyzond-layout-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("root");
        let provider = root.join("spatial.lua");
        fs::write(
            &provider,
            "layout = { api_version = 1, profile = 'spatial' } function calculate() return { [1] = { x = 11, y = 12, width = 13, height = 14 } } end",
        )
        .expect("provider");
        let providers = LayoutProviders::from_profile_paths(&BTreeMap::from([(
            "spatial".to_owned(),
            provider,
        )]));
        let ids = [WindowId::new(1)];
        let existing = BTreeMap::new();
        assert!(providers.is_configured(LayoutProfile::Spatial));
        assert!(!providers.is_configured(LayoutProfile::Tiling));
        assert!(
            (providers
                .provider(LayoutProfile::Spatial)
                .calculate(&input(&ids, &existing))[&ids[0]]
                .x
                - 11.0)
                .abs()
                < f64::EPSILON
        );
        assert_eq!(
            providers
                .provider(LayoutProfile::Tiling)
                .calculate(&input(&ids, &existing))[&ids[0]],
            Rect::new(0., 0., 100., 100.).expect("bounds")
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn timed_out_provider_uses_native_fallback() {
        let ids = [WindowId::new(1)];
        let existing = BTreeMap::new();
        let layout = LuaLayout {
            profile: LayoutProfile::Tiling,
            source: "layout = { api_version = 1, profile = 'tiling' } function calculate() while true do end end".to_owned(),
            limits: ScriptLimits {
                memory_bytes: 1024 * 1024,
                instruction_limit: 1_000,
            },
        };
        let result = layout.calculate(&input(&ids, &existing));
        assert_eq!(
            result[&ids[0]],
            Rect::new(0., 0., 100., 100.).expect("bounds")
        );
    }

    #[test]
    fn layout_state_round_trips_without_provider_code() {
        let state = LayoutState::new(
            LayoutProfile::Spatial,
            BTreeMap::from([(
                LayoutProfile::Spatial,
                BTreeMap::from([(
                    WindowId::new(7),
                    Rect::new(1.0, 2.0, 3.0, 4.0).expect("rect"),
                )]),
            )]),
        );
        let json = state.to_json().expect("encode");
        assert!(!json.contains("function"));
        assert_eq!(LayoutState::from_json(&json).expect("decode"), state);
    }

    #[test]
    fn layout_state_rejects_unknown_format_versions() {
        let error = LayoutState::from_json(
            r#"{"version":99,"active_profile":"Spatial","profile_geometry":{}}"#,
        )
        .expect_err("unsupported version");
        assert!(matches!(error, LayoutStateError::UnsupportedVersion(99)));
    }

    #[test]
    fn shipped_lua_contracts_match_canonical_profile_names() {
        let empty = [];
        let existing = BTreeMap::new();
        for (profile, source) in [
            (
                LayoutProfile::Spatial,
                include_str!("../../../config/layouts/spatial.lua"),
            ),
            (
                LayoutProfile::Scrolling,
                include_str!("../../../config/layouts/scrolling.lua"),
            ),
            (
                LayoutProfile::Tiling,
                include_str!("../../../config/layouts/tiling.lua"),
            ),
            (
                LayoutProfile::Stacking,
                include_str!("../../../config/layouts/stacking.lua"),
            ),
        ] {
            let layout = LuaLayout {
                profile,
                source: source.to_owned(),
                limits: ScriptLimits::default(),
            };
            assert!(layout.calculate_lua(&input(&empty, &existing)).is_ok());
        }
    }
}

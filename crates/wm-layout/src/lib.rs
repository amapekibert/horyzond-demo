//! Deterministic world-space layout algorithms without backend or renderer state.

#![allow(clippy::cast_precision_loss)]

use mlua::{Lua, LuaOptions, StdLib, Table, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use wm_types::{Rect, WindowId};

/// The four canonical Horyzond workspace profiles.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LayoutProfile {
    Spatial,
    Scrolling,
    Tiling,
    Stacking,
}
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
#[derive(Debug, Default)]
pub struct TilingLayout;
impl LayoutEngine for TilingLayout {
    fn profile(&self) -> LayoutProfile {
        LayoutProfile::Tiling
    }
    fn calculate(&self, input: &LayoutInput<'_>) -> BTreeMap<WindowId, Rect> {
        let n = input.windows.len();
        input
            .windows
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let r = if n <= 1 {
                    input.bounds
                } else if i == 0 {
                    Rect::new(
                        input.bounds.x,
                        input.bounds.y,
                        input.bounds.width / 2.0,
                        input.bounds.height,
                    )
                    .expect("master")
                } else {
                    let h = input.bounds.height / (n - 1) as f64;
                    Rect::new(
                        input.bounds.x + input.bounds.width / 2.0,
                        input.bounds.y + (i - 1) as f64 * h,
                        input.bounds.width / 2.0,
                        h,
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
        LayoutProfile::Tiling => Box::new(TilingLayout),
        LayoutProfile::Stacking => Box::new(StackingLayout),
    }
}

/// A restricted external Lua layout provider with a native fallback.
#[derive(Clone, Debug)]
pub struct LuaLayout {
    profile: LayoutProfile,
    source: String,
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
        })
    }
    fn calculate_lua(&self, input: &LayoutInput<'_>) -> Result<BTreeMap<WindowId, Rect>, String> {
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8,
            LuaOptions::default(),
        )
        .map_err(|e| e.to_string())?;
        lua.load(&self.source).exec().map_err(|e| e.to_string())?;
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
        let result = TilingLayout.calculate(&input(&ids, &existing));
        assert!((result[&ids[0]].width - 50.).abs() < f64::EPSILON);
        assert!((result[&ids[1]].height - 50.).abs() < f64::EPSILON);
    }
    #[test]
    fn lua_provider_returns_validated_geometry() {
        let ids = [WindowId::new(1)];
        let existing = BTreeMap::new();
        let layout = LuaLayout { profile: LayoutProfile::Spatial, source: "function calculate(windows, bounds, camera) return { [1] = { x = 7, y = 8, width = 9, height = 10 } } end".to_owned() };
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
                "function calculate() return { [1] = { x = 0, y = 0, width = 0, height = 1 } } end"
                    .to_owned(),
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
            "function calculate() return { [1] = { x = 11, y = 12, width = 13, height = 14 } } end",
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
}

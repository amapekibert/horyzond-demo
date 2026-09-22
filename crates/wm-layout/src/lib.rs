//! Deterministic world-space layout algorithms without backend or renderer state.

#![allow(clippy::cast_precision_loss)]

use std::collections::BTreeMap;
use wm_types::{Rect, WindowId};

/// The four canonical Horyzond workspace profiles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutProfile {
    Spatial,
    Scrolling,
    Tiling,
    Stacking,
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
}

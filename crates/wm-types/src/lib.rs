//! Platform-neutral data used at Horyzond module boundaries.
//!
//! This crate deliberately has no dependency on Lua, Wayland, X11, OpenGL, or
//! Vulkan. Native resources remain inside the adapters that own them.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A stable identifier for a managed window during one compositor session.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
pub struct WindowId(u64);

impl WindowId {
    /// Creates an identifier from a coordinator-assigned value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the coordinator-assigned value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for WindowId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "window-{}", self.0)
    }
}

/// Normalized metadata supplied by a window-system adapter for rule matching.
/// It deliberately excludes protocol-native handles and unbounded payloads.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WindowMetadata {
    pub app_id: String,
    pub title: String,
}

/// A stable identifier for an output during one compositor session.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OutputId(String);

impl OutputId {
    /// Creates an output identifier when it is non-empty.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError::EmptyOutputId`] when the value has no text.
    pub fn new(value: impl Into<String>) -> Result<Self, GeometryError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(GeometryError::EmptyOutputId);
        }
        Ok(Self(value))
    }

    /// Returns the identifier as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OutputId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A point in a named two-dimensional coordinate space.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    /// Creates a finite point.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError::NonFiniteValue`] when either coordinate is not finite.
    pub fn new(x: f64, y: f64) -> Result<Self, GeometryError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(GeometryError::NonFiniteValue);
        }
        Ok(Self { x, y })
    }
}

/// A finite, positive rectangle. Layouts return world-space rectangles; the
/// scene module is solely responsible for projection to output-local space.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    /// Creates a finite rectangle with positive extents.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError::NonFiniteValue`] for a non-finite value and
    /// [`GeometryError::NonPositiveExtent`] for a zero or negative extent.
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Result<Self, GeometryError> {
        if !x.is_finite() || !y.is_finite() || !width.is_finite() || !height.is_finite() {
            return Err(GeometryError::NonFiniteValue);
        }
        if width <= 0.0 || height <= 0.0 {
            return Err(GeometryError::NonPositiveExtent);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }
}

/// A physical output description normalized for core and renderer contracts.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputInfo {
    /// Stable identifier supplied by the adapter.
    pub id: OutputId,
    /// Physical pixel width.
    pub physical_width: u32,
    /// Physical pixel height.
    pub physical_height: u32,
    /// Logical-to-physical scale factor.
    pub scale: f64,
}

impl OutputInfo {
    /// Creates a validated output description.
    ///
    /// # Panics
    ///
    /// Panics when an extent is zero, the scale is non-finite or non-positive,
    /// or the output ID is empty.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        physical_width: u32,
        physical_height: u32,
        scale: f64,
    ) -> Self {
        assert!(physical_width > 0, "output width must be positive");
        assert!(physical_height > 0, "output height must be positive");
        assert!(
            scale.is_finite() && scale > 0.0,
            "output scale must be positive and finite"
        );
        Self {
            id: OutputId::new(id).expect("output ID must be non-empty"),
            physical_width,
            physical_height,
            scale,
        }
    }
}

/// Features an adapter exposes to the coordinator and configuration validator.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Capabilities {
    /// Client surfaces can be composited.
    pub compositing: bool,
    /// Camera scaling can be rendered accurately.
    pub camera_scaling: bool,
    /// The adapter can import client buffers directly.
    pub buffer_import: bool,
    /// The renderer can apply visual effects.
    pub effects: bool,
    /// Presentation timing is available.
    pub presentation_timing: bool,
}

impl Capabilities {
    /// Returns whether all requested features are available.
    #[must_use]
    pub const fn supports(self, requested: Self) -> bool {
        (!requested.compositing || self.compositing)
            && (!requested.camera_scaling || self.camera_scaling)
            && (!requested.buffer_import || self.buffer_import)
            && (!requested.effects || self.effects)
            && (!requested.presentation_timing || self.presentation_timing)
    }
}

/// Validation failures for neutral geometry data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryError {
    NonFiniteValue,
    NonPositiveExtent,
    EmptyOutputId,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteValue => formatter.write_str("geometry values must be finite"),
            Self::NonPositiveExtent => formatter.write_str("rectangle extents must be positive"),
            Self::EmptyOutputId => formatter.write_str("output ID must not be empty"),
        }
    }
}

impl std::error::Error for GeometryError {}

#[cfg(test)]
mod tests {
    use super::{Capabilities, GeometryError, OutputId, Point, Rect, WindowId};

    #[test]
    fn geometry_rejects_invalid_values() {
        assert_eq!(
            Point::new(f64::NAN, 0.0),
            Err(GeometryError::NonFiniteValue)
        );
        assert_eq!(
            Rect::new(0.0, 0.0, 0.0, 1.0),
            Err(GeometryError::NonPositiveExtent)
        );
        assert_eq!(OutputId::new("  "), Err(GeometryError::EmptyOutputId));
    }

    #[test]
    fn identifiers_are_stable_and_readable() {
        assert_eq!(WindowId::new(42).to_string(), "window-42");
    }

    #[test]
    fn capability_check_requires_every_requested_feature() {
        let available = Capabilities {
            compositing: true,
            ..Capabilities::default()
        };
        assert!(available.supports(Capabilities {
            compositing: true,
            ..Capabilities::default()
        }));
        assert!(!available.supports(Capabilities {
            effects: true,
            ..Capabilities::default()
        }));
    }
}

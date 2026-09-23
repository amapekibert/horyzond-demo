//! World-space scene state projected once through a two-dimensional camera.

use std::collections::BTreeMap;
use std::fmt;

use wm_render::{SceneSnapshot, SurfaceQuad};
use wm_types::{Point, Rect, WindowId};

/// A camera mapping the unbounded world plane into logical output coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera2D {
    position: Point,
    zoom: f64,
}

impl Default for Camera2D {
    fn default() -> Self {
        Self {
            position: Point::new(0.0, 0.0).expect("origin is finite"),
            zoom: 1.0,
        }
    }
}

impl Camera2D {
    /// Creates a finite camera with a positive zoom factor.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-finite position or non-positive/non-finite zoom.
    pub fn new(position: Point, zoom: f64) -> Result<Self, SceneError> {
        if !zoom.is_finite() || zoom <= 0.0 {
            return Err(SceneError::InvalidZoom);
        }
        Ok(Self { position, zoom })
    }
    /// Projects a world point into output-local logical coordinates.
    ///
    /// # Panics
    ///
    /// Panics only if validated finite inputs overflow floating-point range.
    #[must_use]
    pub fn project_point(self, world: Point, viewport: Rect) -> Point {
        Point::new(
            viewport.x + viewport.width / 2.0 + (world.x - self.position.x) * self.zoom,
            viewport.y + viewport.height / 2.0 + (world.y - self.position.y) * self.zoom,
        )
        .expect("finite camera projection")
    }
    /// Maps output-local logical coordinates back into world coordinates.
    ///
    /// # Panics
    ///
    /// Panics only if validated finite inputs overflow floating-point range.
    #[must_use]
    pub fn unproject_point(self, local: Point, viewport: Rect) -> Point {
        Point::new(
            self.position.x + (local.x - viewport.x - viewport.width / 2.0) / self.zoom,
            self.position.y + (local.y - viewport.y - viewport.height / 2.0) / self.zoom,
        )
        .expect("finite camera inverse projection")
    }
    /// Projects a world rectangle once into output-local logical coordinates.
    ///
    /// # Panics
    ///
    /// Panics only if validated finite inputs overflow floating-point range.
    #[must_use]
    pub fn project_rect(self, world: Rect, viewport: Rect) -> Rect {
        let origin = self.project_point(
            Point::new(world.x, world.y).expect("validated rectangle"),
            viewport,
        );
        Rect::new(
            origin.x,
            origin.y,
            world.width * self.zoom,
            world.height * self.zoom,
        )
        .expect("positive projection")
    }
}

/// World rectangles and paint order for one workspace scene.
#[derive(Debug, Default)]
pub struct Scene {
    windows: BTreeMap<WindowId, Rect>,
    order: Vec<WindowId>,
    generation: u64,
    damage: DamageTracker,
}

impl Scene {
    /// Removes every window while retaining the damage required to repaint them.
    pub fn clear(&mut self) {
        for bounds in self.windows.values().copied() {
            self.damage.add(bounds);
        }
        self.windows.clear();
        self.order.clear();
        self.generation += 1;
    }
    /// Inserts or updates a world rectangle and marks both old and new bounds damaged.
    pub fn set_window(&mut self, window: WindowId, bounds: Rect) {
        if let Some(old) = self.windows.insert(window, bounds) {
            self.damage.add(old);
        } else {
            self.order.push(window);
        }
        self.damage.add(bounds);
        self.generation += 1;
    }
    /// Removes a window and returns its previous world rectangle.
    pub fn remove_window(&mut self, window: WindowId) -> Option<Rect> {
        let old = self.windows.remove(&window)?;
        self.order.retain(|id| *id != window);
        self.damage.add(old);
        self.generation += 1;
        Some(old)
    }
    /// Moves a mapped window to the front of paint order.
    ///
    /// Returns `false` when the window is absent from this scene.
    pub fn raise_window(&mut self, window: WindowId) -> bool {
        let Some(bounds) = self.windows.get(&window).copied() else {
            return false;
        };
        let Some(position) = self.order.iter().position(|id| *id == window) else {
            return false;
        };
        if position + 1 == self.order.len() {
            return true;
        }
        self.order.remove(position);
        self.order.push(window);
        self.damage.add(bounds);
        self.generation += 1;
        true
    }
    /// Replaces paint order when it lists every mapped scene window exactly once.
    ///
    /// Returns `false` without mutation for incomplete, duplicate, or unknown IDs.
    pub fn set_order(&mut self, order: &[WindowId]) -> bool {
        if order.len() != self.order.len()
            || order.iter().any(|id| !self.windows.contains_key(id))
            || order
                .iter()
                .enumerate()
                .any(|(index, id)| order[..index].contains(id))
        {
            return false;
        }
        if self.order == order {
            return true;
        }
        self.order = order.to_vec();
        for bounds in self.windows.values().copied() {
            self.damage.add(bounds);
        }
        self.generation += 1;
        true
    }
    /// Returns the topmost world-space window containing a point.
    #[must_use]
    pub fn pick(&self, world: Point) -> Option<WindowId> {
        self.order.iter().rev().copied().find(|id| {
            self.windows.get(id).is_some_and(|r| {
                world.x >= r.x
                    && world.x < r.x + r.width
                    && world.y >= r.y
                    && world.y < r.y + r.height
            })
        })
    }
    /// Prepares an immutable renderer snapshot using camera projection exactly once.
    #[must_use]
    pub fn snapshot(&self, camera: Camera2D, viewport: Rect) -> SceneSnapshot {
        SceneSnapshot {
            generation: self.generation,
            surfaces: self
                .order
                .iter()
                .filter_map(|window| {
                    self.windows.get(window).map(|bounds| SurfaceQuad {
                        window: *window,
                        bounds: camera.project_rect(*bounds, viewport),
                    })
                })
                .collect(),
        }
    }
    /// Drains coalesced world-space damage accumulated since the preceding call.
    pub fn take_damage(&mut self) -> Vec<Rect> {
        self.damage.take()
    }
}

/// Minimal correct damage coalescing with a full-redraw-safe fallback.
#[derive(Debug, Default)]
pub struct DamageTracker {
    regions: Vec<Rect>,
}
impl DamageTracker {
    /// Adds a changed world rectangle.
    pub fn add(&mut self, rect: Rect) {
        self.regions.push(rect);
    }
    /// Returns one coalesced bound for all accumulated changes.
    ///
    /// # Panics
    ///
    /// Panics only if validated finite inputs overflow floating-point range.
    pub fn take(&mut self) -> Vec<Rect> {
        let Some(first) = self.regions.first().copied() else {
            return Vec::new();
        };
        let union = self.regions.iter().skip(1).fold(first, |a, b| {
            Rect::new(
                a.x.min(b.x),
                a.y.min(b.y),
                (a.x + a.width).max(b.x + b.width) - a.x.min(b.x),
                (a.y + a.height).max(b.y + b.height) - a.y.min(b.y),
            )
            .expect("union is positive")
        });
        self.regions.clear();
        vec![union]
    }
}

/// Scene validation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SceneError {
    InvalidZoom,
}
impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("camera zoom must be positive and finite")
    }
}
impl std::error::Error for SceneError {}

#[cfg(test)]
mod tests {
    use super::{Camera2D, Scene};
    use wm_types::{Point, Rect, WindowId};
    #[test]
    fn camera_round_trip_and_scene_picking_are_stable() {
        let camera = Camera2D::new(Point::new(10.0, -5.0).expect("point"), 2.0).expect("camera");
        let viewport = Rect::new(0.0, 0.0, 100.0, 100.0).expect("viewport");
        let point = Point::new(20.0, 5.0).expect("point");
        assert_eq!(
            camera.unproject_point(camera.project_point(point, viewport), viewport),
            point
        );
        let mut scene = Scene::default();
        scene.set_window(
            WindowId::new(1),
            Rect::new(0.0, 0.0, 20.0, 20.0).expect("rect"),
        );
        scene.set_window(
            WindowId::new(2),
            Rect::new(5.0, 5.0, 10.0, 10.0).expect("rect"),
        );
        assert_eq!(
            scene.pick(Point::new(6.0, 6.0).expect("point")),
            Some(WindowId::new(2))
        );
        assert_eq!(
            scene.snapshot(Camera2D::default(), viewport).surfaces.len(),
            2
        );
        assert_eq!(scene.take_damage().len(), 1);
    }

    #[test]
    fn raise_changes_paint_order_without_changing_geometry() {
        let mut scene = Scene::default();
        scene.set_window(
            WindowId::new(1),
            Rect::new(0.0, 0.0, 20.0, 20.0).expect("rect"),
        );
        scene.set_window(
            WindowId::new(2),
            Rect::new(5.0, 5.0, 10.0, 10.0).expect("rect"),
        );
        assert_eq!(
            scene.pick(Point::new(6.0, 6.0).expect("point")),
            Some(WindowId::new(2))
        );
        assert!(scene.raise_window(WindowId::new(1)));
        assert_eq!(
            scene.pick(Point::new(6.0, 6.0).expect("point")),
            Some(WindowId::new(1))
        );
    }

    #[test]
    fn set_order_replaces_only_a_complete_unique_order() {
        let mut scene = Scene::default();
        scene.set_window(
            WindowId::new(1),
            Rect::new(0.0, 0.0, 20.0, 20.0).expect("rect"),
        );
        scene.set_window(
            WindowId::new(2),
            Rect::new(5.0, 5.0, 10.0, 10.0).expect("rect"),
        );
        assert!(scene.set_order(&[WindowId::new(2), WindowId::new(1)]));
        assert_eq!(
            scene.pick(Point::new(6.0, 6.0).expect("point")),
            Some(WindowId::new(1))
        );
        assert!(!scene.set_order(&[WindowId::new(1), WindowId::new(1)]));
        assert!(!scene.set_order(&[WindowId::new(1), WindowId::new(3)]));
        assert_eq!(
            scene.pick(Point::new(6.0, 6.0).expect("point")),
            Some(WindowId::new(1))
        );
    }
}

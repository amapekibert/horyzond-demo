//! Deterministic core state independent of window-system and renderer details.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use wm_backend::BackendEvent;
use wm_scene::{Camera2D, Scene};
use wm_types::{OutputId, OutputInfo, Rect, WindowId};

/// A workspace identifier stable during one Horyzond session.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorkspaceId(u64);
impl WorkspaceId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

/// Mutable state owned by one workspace.
#[derive(Debug)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub camera: Camera2D,
    pub scene: Scene,
    focused: Option<WindowId>,
}
impl Workspace {
    fn new(id: WorkspaceId) -> Self {
        Self {
            id,
            camera: Camera2D::default(),
            scene: Scene::default(),
            focused: None,
        }
    }
    /// Returns the focused mapped window, if any.
    #[must_use]
    pub const fn focused(&self) -> Option<WindowId> {
        self.focused
    }
}

/// Core state transitions and invariant checks.
#[derive(Debug)]
pub struct CoreState {
    outputs: BTreeMap<OutputId, OutputInfo>,
    workspaces: BTreeMap<WorkspaceId, Workspace>,
    windows: BTreeSet<WindowId>,
    active_workspace: WorkspaceId,
}
impl Default for CoreState {
    fn default() -> Self {
        let id = WorkspaceId::new(1);
        let mut workspaces = BTreeMap::new();
        workspaces.insert(id, Workspace::new(id));
        Self {
            outputs: BTreeMap::new(),
            workspaces,
            windows: BTreeSet::new(),
            active_workspace: id,
        }
    }
}

impl CoreState {
    /// Applies one normalized backend event in order.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid window lifecycle transitions.
    pub fn apply_event(&mut self, event: BackendEvent) -> Result<(), CoreError> {
        match event {
            BackendEvent::OutputAdded(output) => {
                self.outputs.insert(output.id.clone(), output);
            }
            BackendEvent::OutputRemoved(id) => {
                self.outputs.remove(&id);
            }
            BackendEvent::WindowMapped(window) => {
                if !self.windows.insert(window) {
                    return Err(CoreError::AlreadyMapped(window));
                }
            }
            BackendEvent::WindowUnmapped(window) => {
                if !self.windows.remove(&window) {
                    return Err(CoreError::NotMapped(window));
                }
                for workspace in self.workspaces.values_mut() {
                    workspace.scene.remove_window(window);
                    if workspace.focused == Some(window) {
                        workspace.focused = None;
                    }
                }
            }
        }
        Ok(())
    }
    /// Sets desired world geometry in the active workspace.
    ///
    /// # Errors
    ///
    /// Returns an error if the window is not mapped.
    pub fn set_geometry(&mut self, window: WindowId, geometry: Rect) -> Result<(), CoreError> {
        if !self.windows.contains(&window) {
            return Err(CoreError::NotMapped(window));
        }
        self.active_workspace_mut()
            .scene
            .set_window(window, geometry);
        Ok(())
    }
    /// Changes focus to a mapped window in the active workspace.
    ///
    /// # Errors
    ///
    /// Returns an error if the window is not mapped.
    pub fn focus(&mut self, window: WindowId) -> Result<(), CoreError> {
        if !self.windows.contains(&window) {
            return Err(CoreError::NotMapped(window));
        }
        self.active_workspace_mut().focused = Some(window);
        Ok(())
    }
    /// Returns the active workspace.
    ///
    /// # Panics
    ///
    /// Panics only if internal core state was corrupted and lost its default workspace.
    #[must_use]
    pub fn active_workspace(&self) -> &Workspace {
        self.workspaces
            .get(&self.active_workspace)
            .expect("default workspace exists")
    }
    fn active_workspace_mut(&mut self) -> &mut Workspace {
        self.workspaces
            .get_mut(&self.active_workspace)
            .expect("default workspace exists")
    }
}

/// A lifecycle invariant violation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreError {
    AlreadyMapped(WindowId),
    NotMapped(WindowId),
}
impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyMapped(id) => write!(f, "{id} is already mapped"),
            Self::NotMapped(id) => write!(f, "{id} is not mapped"),
        }
    }
}
impl std::error::Error for CoreError {}

#[cfg(test)]
mod tests {
    use super::CoreState;
    use wm_backend::BackendEvent;
    use wm_types::{Rect, WindowId};
    #[test]
    fn lifecycle_clears_focus_and_scene() {
        let mut core = CoreState::default();
        let id = WindowId::new(1);
        core.apply_event(BackendEvent::WindowMapped(id))
            .expect("map");
        core.set_geometry(id, Rect::new(0.0, 0.0, 10.0, 10.0).expect("rect"))
            .expect("geometry");
        core.focus(id).expect("focus");
        core.apply_event(BackendEvent::WindowUnmapped(id))
            .expect("unmap");
        assert_eq!(core.active_workspace().focused(), None);
        assert_eq!(
            core.active_workspace()
                .scene
                .pick(wm_types::Point::new(1.0, 1.0).expect("point")),
            None
        );
    }
}

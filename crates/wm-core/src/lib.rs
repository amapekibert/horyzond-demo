//! Deterministic core state independent of window-system and renderer details.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use wm_backend::BackendEvent;
use wm_layout::{LayoutInput, LayoutProfile, builtin};
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
    profile: LayoutProfile,
    profile_geometry: BTreeMap<LayoutProfile, BTreeMap<WindowId, Rect>>,
}
impl Workspace {
    fn new(id: WorkspaceId) -> Self {
        Self {
            id,
            camera: Camera2D::default(),
            scene: Scene::default(),
            focused: None,
            profile: LayoutProfile::Spatial,
            profile_geometry: BTreeMap::new(),
        }
    }
    /// Returns the focused mapped window, if any.
    #[must_use]
    pub const fn focused(&self) -> Option<WindowId> {
        self.focused
    }
    /// Returns the active layout profile.
    #[must_use]
    pub const fn profile(&self) -> LayoutProfile {
        self.profile
    }
}

/// Core state transitions and invariant checks.
#[derive(Debug)]
pub struct CoreState {
    outputs: BTreeMap<OutputId, OutputInfo>,
    workspaces: BTreeMap<WorkspaceId, Workspace>,
    windows: BTreeSet<WindowId>,
    active_workspace: WorkspaceId,
    next_workspace: u64,
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
            next_workspace: 2,
        }
    }
}

impl CoreState {
    /// Creates an empty workspace and returns its stable session ID.
    pub fn create_workspace(&mut self) -> WorkspaceId {
        let id = WorkspaceId::new(self.next_workspace);
        self.next_workspace += 1;
        self.workspaces.insert(id, Workspace::new(id));
        id
    }
    /// Makes an existing workspace active.
    ///
    /// # Errors
    ///
    /// Returns an error when the requested workspace does not exist.
    pub fn switch_workspace(&mut self, workspace: WorkspaceId) -> Result<(), CoreError> {
        if !self.workspaces.contains_key(&workspace) {
            return Err(CoreError::UnknownWorkspace(workspace));
        }
        self.active_workspace = workspace;
        Ok(())
    }
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
        let profile = self.active_workspace().profile;
        let workspace = self.active_workspace_mut();
        workspace.scene.set_window(window, geometry);
        workspace
            .profile_geometry
            .entry(profile)
            .or_default()
            .insert(window, geometry);
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
    /// Switches profile and applies the corresponding layout to all mapped windows.
    pub fn set_layout_profile(&mut self, profile: LayoutProfile, bounds: Rect) {
        let windows = self.windows.iter().copied().collect::<Vec<_>>();
        let workspace = self.active_workspace_mut();
        workspace.profile = profile;
        let existing = workspace
            .profile_geometry
            .entry(profile)
            .or_default()
            .clone();
        let result = builtin(profile).calculate(&LayoutInput {
            windows: &windows,
            bounds,
            existing: &existing,
            focused: workspace.focused,
        });
        for (window, geometry) in &result {
            workspace.scene.set_window(*window, *geometry);
        }
        workspace.profile_geometry.insert(profile, result);
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
    UnknownWorkspace(WorkspaceId),
}
impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyMapped(id) => write!(f, "{id} is already mapped"),
            Self::NotMapped(id) => write!(f, "{id} is not mapped"),
            Self::UnknownWorkspace(id) => write!(f, "workspace {} does not exist", id.0),
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

    #[test]
    fn profile_switch_keeps_mapped_windows() {
        let mut core = CoreState::default();
        let first = WindowId::new(1);
        let second = WindowId::new(2);
        core.apply_event(BackendEvent::WindowMapped(first))
            .expect("map first");
        core.apply_event(BackendEvent::WindowMapped(second))
            .expect("map second");
        core.set_layout_profile(
            wm_layout::LayoutProfile::Tiling,
            Rect::new(0.0, 0.0, 100.0, 100.0).expect("bounds"),
        );
        assert_eq!(
            core.active_workspace().profile(),
            wm_layout::LayoutProfile::Tiling
        );
        assert!(
            core.active_workspace()
                .scene
                .pick(wm_types::Point::new(25.0, 25.0).expect("point"))
                .is_some()
        );
    }
    #[test]
    fn workspaces_keep_independent_scene_state() {
        let mut core = CoreState::default();
        let first = WindowId::new(1);
        core.apply_event(BackendEvent::WindowMapped(first))
            .expect("map");
        core.set_geometry(first, Rect::new(0.0, 0.0, 10.0, 10.0).expect("rect"))
            .expect("geometry");
        let second_workspace = core.create_workspace();
        core.switch_workspace(second_workspace).expect("switch");
        assert_eq!(
            core.active_workspace()
                .scene
                .pick(wm_types::Point::new(1.0, 1.0).expect("point")),
            None
        );
        core.switch_workspace(super::WorkspaceId::new(1))
            .expect("switch back");
        assert_eq!(
            core.active_workspace()
                .scene
                .pick(wm_types::Point::new(1.0, 1.0).expect("point")),
            Some(first)
        );
    }
}

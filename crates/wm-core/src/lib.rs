//! Deterministic coordinator state with opaque external layout providers.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use wm_backend::BackendEvent;
use wm_layout::{LayoutEngine, LayoutId, LayoutInput, LayoutInteraction, LayoutState};
use wm_scene::{Camera2D, Scene};
use wm_types::{OutputId, OutputInfo, Point, Rect, WindowId};

/// A workspace identifier stable during one Horyzond session.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorkspaceId(u64);
impl WorkspaceId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    /// Returns the coordinator-assigned numeric value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Mutable state owned by one workspace.
#[derive(Debug)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub camera: Camera2D,
    pub scene: Scene,
    focused: Option<WindowId>,
    layout: LayoutId,
    layout_geometry: BTreeMap<LayoutId, BTreeMap<WindowId, Rect>>,
    provider_state: BTreeMap<LayoutId, serde_json::Value>,
}
impl Workspace {
    fn new(id: WorkspaceId) -> Self {
        Self {
            id,
            camera: Camera2D::default(),
            scene: Scene::default(),
            focused: None,
            layout: LayoutId::new("default").expect("constant ID"),
            layout_geometry: BTreeMap::new(),
            provider_state: BTreeMap::new(),
        }
    }
    #[must_use]
    pub const fn focused(&self) -> Option<WindowId> {
        self.focused
    }
    #[must_use]
    pub fn layout(&self) -> &LayoutId {
        &self.layout
    }
    fn state(&self) -> LayoutState {
        LayoutState::with_provider_state(
            self.layout.clone(),
            self.layout_geometry.clone(),
            self.provider_state.clone(),
        )
    }
    fn restore(&mut self, state: LayoutState, windows: &BTreeSet<WindowId>) {
        self.layout = state.active_layout().clone();
        let (layout_geometry, provider_state) = state.into_parts();
        self.layout_geometry = layout_geometry;
        self.provider_state = provider_state;
        for geometry in self.layout_geometry.values_mut() {
            geometry.retain(|window, _| windows.contains(window));
        }
        self.scene.clear();
        if let Some(geometry) = self.layout_geometry.get(&self.layout) {
            for (window, bounds) in geometry {
                self.scene.set_window(*window, *bounds);
            }
        }
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
    pub fn create_workspace(&mut self) -> WorkspaceId {
        let id = WorkspaceId::new(self.next_workspace);
        self.next_workspace += 1;
        self.workspaces.insert(id, Workspace::new(id));
        id
    }
    pub fn switch_workspace(&mut self, workspace: WorkspaceId) -> Result<(), CoreError> {
        if !self.workspaces.contains_key(&workspace) {
            return Err(CoreError::UnknownWorkspace(workspace));
        }
        self.active_workspace = workspace;
        Ok(())
    }
    /// Replaces the active workspace camera after validating its world origin
    /// and positive finite scale.
    pub fn set_active_camera(&mut self, position: Point, zoom: f64) -> Result<(), CoreError> {
        self.active_workspace_mut().camera =
            Camera2D::new(position, zoom).map_err(CoreError::Camera)?;
        Ok(())
    }
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
            BackendEvent::WindowMetadataChanged(window, _) if !self.windows.contains(&window) => {
                return Err(CoreError::NotMapped(window));
            }
            BackendEvent::WindowMetadataChanged(_, _) => {}
            BackendEvent::WindowUnmapped(window) => {
                if !self.windows.remove(&window) {
                    return Err(CoreError::NotMapped(window));
                }
                for workspace in self.workspaces.values_mut() {
                    workspace.scene.remove_window(window);
                    if workspace.focused == Some(window) {
                        workspace.focused = None;
                    }
                    for geometry in workspace.layout_geometry.values_mut() {
                        geometry.remove(&window);
                    }
                }
            }
        }
        Ok(())
    }
    pub fn set_geometry(&mut self, window: WindowId, geometry: Rect) -> Result<(), CoreError> {
        if !self.windows.contains(&window) {
            return Err(CoreError::NotMapped(window));
        }
        let workspace = self.active_workspace_mut();
        let layout = workspace.layout.clone();
        workspace.scene.set_window(window, geometry);
        workspace
            .layout_geometry
            .entry(layout)
            .or_default()
            .insert(window, geometry);
        Ok(())
    }
    pub fn focus(&mut self, window: WindowId) -> Result<(), CoreError> {
        if !self.windows.contains(&window) {
            return Err(CoreError::NotMapped(window));
        }
        self.active_workspace_mut().focused = Some(window);
        Ok(())
    }
    pub fn raise(&mut self, window: WindowId) -> Result<(), CoreError> {
        if !self.windows.contains(&window) {
            return Err(CoreError::NotMapped(window));
        }
        if !self.active_workspace_mut().scene.raise_window(window) {
            return Err(CoreError::NotInActiveWorkspace(window));
        }
        Ok(())
    }
    /// Applies an externally selected provider. Core never interprets its ID.
    pub fn apply_layout(&mut self, engine: &dyn LayoutEngine, bounds: Rect) {
        let windows = self.windows.iter().copied().collect::<Vec<_>>();
        let workspace = self.active_workspace_mut();
        let layout = engine.id().clone();
        workspace.layout = layout.clone();
        let existing = workspace
            .layout_geometry
            .entry(layout.clone())
            .or_default()
            .clone();
        let provider_state = workspace.provider_state.get(&layout).cloned();
        let result = engine.calculate(&LayoutInput {
            windows: &windows,
            bounds,
            existing: &existing,
            focused: workspace.focused,
            provider_state: provider_state.as_ref(),
        });
        for (window, geometry) in &result.geometry {
            workspace.scene.set_window(*window, *geometry);
        }
        let _ = workspace.scene.set_order(&result.order);
        workspace
            .provider_state
            .insert(layout.clone(), result.provider_state);
        workspace.layout_geometry.insert(layout, result.geometry);
    }
    /// Routes an opaque interaction to the selected provider and commits its output.
    ///
    /// Core neither interprets the interaction action nor its JSON payload.
    pub fn interact_layout(
        &mut self,
        engine: &dyn LayoutEngine,
        interaction: &LayoutInteraction,
        bounds: Rect,
    ) {
        let windows = self.windows.iter().copied().collect::<Vec<_>>();
        let workspace = self.active_workspace_mut();
        let layout = engine.id().clone();
        workspace.layout = layout.clone();
        let existing = workspace
            .layout_geometry
            .entry(layout.clone())
            .or_default()
            .clone();
        let provider_state = workspace.provider_state.get(&layout).cloned();
        let result = engine.interact(
            &LayoutInput {
                windows: &windows,
                bounds,
                existing: &existing,
                focused: workspace.focused,
                provider_state: provider_state.as_ref(),
            },
            interaction,
        );
        for (window, geometry) in &result.geometry {
            workspace.scene.set_window(*window, *geometry);
        }
        let _ = workspace.scene.set_order(&result.order);
        workspace
            .provider_state
            .insert(layout.clone(), result.provider_state);
        workspace.layout_geometry.insert(layout, result.geometry);
    }
    #[must_use]
    pub fn active_layout_state(&self) -> LayoutState {
        self.active_workspace().state()
    }
    pub fn restore_active_layout_state(&mut self, state: LayoutState) {
        let windows = self.windows.clone();
        self.active_workspace_mut().restore(state, &windows);
    }
    /// Returns every mapped window ID in deterministic order.
    #[must_use]
    pub fn windows(&self) -> impl ExactSizeIterator<Item = WindowId> + '_ {
        self.windows.iter().copied()
    }
    /// Returns every workspace in deterministic ID order.
    #[must_use]
    pub fn workspaces(&self) -> impl ExactSizeIterator<Item = &Workspace> + '_ {
        self.workspaces.values()
    }
    /// Returns the selected workspace ID.
    #[must_use]
    pub const fn active_workspace_id(&self) -> WorkspaceId {
        self.active_workspace
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreError {
    AlreadyMapped(WindowId),
    NotMapped(WindowId),
    NotInActiveWorkspace(WindowId),
    UnknownWorkspace(WorkspaceId),
    Camera(wm_scene::SceneError),
}
impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyMapped(id) => write!(f, "{id} is already mapped"),
            Self::NotMapped(id) => write!(f, "{id} is not mapped"),
            Self::NotInActiveWorkspace(id) => {
                write!(f, "{id} is not present in the active workspace")
            }
            Self::UnknownWorkspace(id) => write!(f, "workspace {} does not exist", id.0),
            Self::Camera(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for CoreError {}

#[cfg(test)]
mod tests {
    use super::*;
    use wm_layout::{LayoutOutput, RecoveryLayout};
    use wm_types::Point;

    struct OrderedLayout {
        id: LayoutId,
    }
    impl LayoutEngine for OrderedLayout {
        fn id(&self) -> &LayoutId {
            &self.id
        }

        fn calculate(&self, input: &LayoutInput<'_>) -> LayoutOutput {
            let bounds = Rect::new(0., 0., 20., 20.).expect("rect");
            LayoutOutput {
                geometry: input
                    .windows
                    .iter()
                    .map(|window| (*window, bounds))
                    .collect(),
                order: input.windows.iter().rev().copied().collect(),
                provider_state: serde_json::Value::Null,
            }
        }
    }

    struct StatefulLayout {
        id: LayoutId,
    }
    impl LayoutEngine for StatefulLayout {
        fn id(&self) -> &LayoutId {
            &self.id
        }

        fn calculate(&self, input: &LayoutInput<'_>) -> LayoutOutput {
            let count = input
                .provider_state
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
                + 1;
            LayoutOutput {
                geometry: input
                    .windows
                    .iter()
                    .map(|window| {
                        (
                            *window,
                            Rect::new(0., 0., 20., 20.).expect("constant rectangle"),
                        )
                    })
                    .collect(),
                order: input.windows.to_vec(),
                provider_state: serde_json::json!(count),
            }
        }
    }
    #[test]
    fn arbitrary_external_layout_id_drives_core() {
        let mut core = CoreState::default();
        let window = WindowId::new(1);
        core.apply_event(BackendEvent::WindowMapped(window))
            .expect("map");
        let layout = RecoveryLayout::new(LayoutId::new("plugin-layout").expect("ID"));
        core.apply_layout(&layout, Rect::new(0., 0., 100., 100.).expect("bounds"));
        assert_eq!(core.active_workspace().layout().as_str(), "plugin-layout");
        assert!(
            core.active_workspace()
                .scene
                .pick(Point::new(1., 1.).expect("point"))
                .is_some()
        );
    }
    #[test]
    fn external_layout_order_drives_scene_painting() {
        let mut core = CoreState::default();
        core.apply_event(BackendEvent::WindowMapped(WindowId::new(1)))
            .expect("map");
        core.apply_event(BackendEvent::WindowMapped(WindowId::new(2)))
            .expect("map");
        let layout = OrderedLayout {
            id: LayoutId::new("ordered-plugin").expect("ID"),
        };
        core.apply_layout(&layout, Rect::new(0., 0., 100., 100.).expect("bounds"));
        assert_eq!(
            core.active_workspace()
                .scene
                .pick(Point::new(1., 1.).expect("point")),
            Some(WindowId::new(1))
        );
    }
    #[test]
    fn provider_state_is_preserved_per_opaque_layout_id() {
        let mut core = CoreState::default();
        core.apply_event(BackendEvent::WindowMapped(WindowId::new(1)))
            .expect("map");
        let layout = StatefulLayout {
            id: LayoutId::new("stateful-plugin").expect("ID"),
        };
        let bounds = Rect::new(0., 0., 100., 100.).expect("bounds");
        core.apply_layout(&layout, bounds);
        core.apply_layout(&layout, bounds);
        assert_eq!(
            core.active_layout_state()
                .provider_state(layout.id())
                .and_then(serde_json::Value::as_u64),
            Some(2)
        );
    }
    #[test]
    fn switching_providers_preserves_each_provider_state_history() {
        let mut core = CoreState::default();
        let window = WindowId::new(1);
        core.apply_event(BackendEvent::WindowMapped(window))
            .expect("map");
        let first = StatefulLayout {
            id: LayoutId::new("first-provider").expect("ID"),
        };
        let second = StatefulLayout {
            id: LayoutId::new("second-provider").expect("ID"),
        };
        let bounds = Rect::new(0., 0., 100., 100.).expect("bounds");
        core.apply_layout(&first, bounds);
        core.apply_layout(&second, bounds);
        core.apply_layout(&first, bounds);
        let state = core.active_layout_state();
        assert_eq!(state.active_layout(), first.id());
        assert_eq!(
            state
                .provider_state(first.id())
                .and_then(serde_json::Value::as_u64),
            Some(2)
        );
        assert_eq!(
            state
                .provider_state(second.id())
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert_eq!(
            core.active_workspace()
                .scene
                .pick(Point::new(1., 1.).expect("point")),
            Some(window)
        );
    }
    #[test]
    fn workspace_selection_keeps_layout_state_independent() {
        let mut core = CoreState::default();
        core.apply_event(BackendEvent::WindowMapped(WindowId::new(1)))
            .expect("map");
        let first = StatefulLayout {
            id: LayoutId::new("first-workspace-provider").expect("ID"),
        };
        let second = StatefulLayout {
            id: LayoutId::new("second-workspace-provider").expect("ID"),
        };
        let bounds = Rect::new(0., 0., 100., 100.).expect("bounds");
        core.apply_layout(&first, bounds);
        let other = core.create_workspace();
        core.switch_workspace(other).expect("workspace");
        core.apply_layout(&second, bounds);
        assert_eq!(core.active_workspace().layout(), second.id());
        assert_eq!(
            core.active_layout_state()
                .provider_state(second.id())
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        core.switch_workspace(WorkspaceId::new(1))
            .expect("workspace");
        assert_eq!(core.active_workspace().layout(), first.id());
        assert_eq!(
            core.active_layout_state()
                .provider_state(first.id())
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
    }
    #[test]
    fn state_restores_only_mapped_windows() {
        let mut source = CoreState::default();
        let window = WindowId::new(1);
        source
            .apply_event(BackendEvent::WindowMapped(window))
            .expect("map");
        source
            .set_geometry(window, Rect::new(1., 2., 3., 4.).expect("rect"))
            .expect("geometry");
        let state = source.active_layout_state();
        let mut restored = CoreState::default();
        restored
            .apply_event(BackendEvent::WindowMapped(window))
            .expect("map");
        restored.restore_active_layout_state(state);
        assert_eq!(
            restored
                .active_workspace()
                .scene
                .pick(Point::new(2., 3.).expect("point")),
            Some(window)
        );
    }
}

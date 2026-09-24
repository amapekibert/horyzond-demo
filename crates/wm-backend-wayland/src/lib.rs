//! Concrete nested Wayland adapter boundary.
//!
//! The default build deliberately contains no Smithay dependency activation.
//! Enable the `nested` feature only from a visible P6 runtime composition.

use wm_backend::BackendError;
use wm_backend::{ConfigureLedger, ConfigureTransaction};
use wm_types::{Rect, WindowId};

/// Protocol-neutral lifecycle state for a newly created xdg toplevel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum XdgSurfaceState {
    /// The role exists but no configure was sent.
    New,
    /// A configure serial must be acknowledged before the first commit.
    AwaitingConfigure,
    /// The latest configure serial was acknowledged and may be committed.
    ReadyToCommit,
    /// The surface completed its first valid commit and is manageable.
    Mapped,
}

/// Feature-independent ordering guard for xdg toplevel callbacks.
#[derive(Debug, Default)]
pub struct XdgLifecycle {
    states: std::collections::BTreeMap<WindowId, XdgSurfaceState>,
    configures: ConfigureLedger,
}
impl XdgLifecycle {
    /// Registers an unmapped xdg toplevel.
    ///
    /// # Errors
    ///
    /// Returns an error when the window already has an xdg lifecycle.
    pub fn register(&mut self, window: WindowId) -> Result<(), BackendError> {
        if self.states.insert(window, XdgSurfaceState::New).is_some() {
            return Err(BackendError::new(format!(
                "{window} already has an xdg lifecycle"
            )));
        }
        Ok(())
    }
    /// Issues or supersedes a pending configure for one registered toplevel.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown or acknowledgement-ready toplevel.
    pub fn configure(
        &mut self,
        window: WindowId,
        geometry: Rect,
        now: std::time::Duration,
    ) -> Result<ConfigureTransaction, BackendError> {
        match self.states.get(&window) {
            Some(
                XdgSurfaceState::New | XdgSurfaceState::AwaitingConfigure | XdgSurfaceState::Mapped,
            ) => {
                self.states
                    .insert(window, XdgSurfaceState::AwaitingConfigure);
                Ok(self.configures.issue(window, geometry, now))
            }
            Some(XdgSurfaceState::ReadyToCommit) => Err(BackendError::new(format!(
                "{window} cannot receive a new configure in its current lifecycle state"
            ))),
            None => Err(BackendError::new(format!("unknown xdg toplevel {window}"))),
        }
    }
    /// Tracks a configure issued by a protocol that supplies its own serial.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown or acknowledgement-ready toplevel, or for a
    /// zero serial that cannot identify a protocol transaction.
    pub fn configure_with_serial(
        &mut self,
        window: WindowId,
        geometry: Rect,
        serial: u64,
        now: std::time::Duration,
    ) -> Result<ConfigureTransaction, BackendError> {
        if serial == 0 {
            return Err(BackendError::new("xdg configure serial must not be zero"));
        }
        match self.states.get(&window) {
            Some(
                XdgSurfaceState::New | XdgSurfaceState::AwaitingConfigure | XdgSurfaceState::Mapped,
            ) => {
                self.states
                    .insert(window, XdgSurfaceState::AwaitingConfigure);
                Ok(self
                    .configures
                    .issue_with_serial(window, geometry, serial, now))
            }
            Some(XdgSurfaceState::ReadyToCommit) => Err(BackendError::new(format!(
                "{window} cannot receive a new configure in its current lifecycle state"
            ))),
            None => Err(BackendError::new(format!("unknown xdg toplevel {window}"))),
        }
    }
    /// Accepts only the current configure serial.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown, stale, or out-of-order acknowledgement.
    pub fn acknowledge(&mut self, transaction: ConfigureTransaction) -> Result<(), BackendError> {
        if self.states.get(&transaction.window) == Some(&XdgSurfaceState::AwaitingConfigure)
            && self.configures.acknowledge(transaction)
        {
            self.states
                .insert(transaction.window, XdgSurfaceState::ReadyToCommit);
            Ok(())
        } else {
            Err(BackendError::new(format!(
                "invalid configure acknowledgement {} for {}",
                transaction.serial, transaction.window
            )))
        }
    }
    /// Accepts the current configure serial supplied by xdg-shell.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown, stale, or out-of-order acknowledgement.
    pub fn acknowledge_serial(
        &mut self,
        window: WindowId,
        serial: u64,
    ) -> Result<(), BackendError> {
        if self.states.get(&window) == Some(&XdgSurfaceState::AwaitingConfigure)
            && self.configures.acknowledge_serial(window, serial)
        {
            self.states.insert(window, XdgSurfaceState::ReadyToCommit);
            Ok(())
        } else {
            Err(BackendError::new(format!(
                "invalid configure acknowledgement {serial} for {window}"
            )))
        }
    }
    /// Expires unanswered configure requests and returns each timed-out request.
    ///
    /// An expired toplevel returns to `New`, allowing the adapter to issue a
    /// fresh configure without treating a slow client as mapped.
    #[must_use]
    pub fn expire_configures(
        &mut self,
        now: std::time::Duration,
        timeout: std::time::Duration,
    ) -> Vec<ConfigureTransaction> {
        let expired = self.configures.expire(now, timeout);
        for transaction in &expired {
            if self.states.get(&transaction.window) == Some(&XdgSurfaceState::AwaitingConfigure) {
                self.states.insert(transaction.window, XdgSurfaceState::New);
            }
        }
        expired
    }
    /// Maps a toplevel only after its initial configure acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns an error when the surface has not acknowledged a configure.
    pub fn commit(&mut self, window: WindowId) -> Result<(), BackendError> {
        if self.states.get(&window) == Some(&XdgSurfaceState::ReadyToCommit) {
            self.states.insert(window, XdgSurfaceState::Mapped);
            Ok(())
        } else {
            Err(BackendError::new(format!(
                "{window} committed before acknowledging its initial configure"
            )))
        }
    }
    /// Forgets all protocol state for a destroyed toplevel.
    pub fn destroy(&mut self, window: WindowId) {
        self.states.remove(&window);
        self.configures.forget(window);
    }
    /// Returns the current lifecycle state, if registered.
    #[must_use]
    pub fn state(&self, window: WindowId) -> Option<XdgSurfaceState> {
        self.states.get(&window).copied()
    }
}

/// Reports whether the nested Wayland runtime was compiled into this binary.
#[must_use]
pub const fn nested_runtime_available() -> bool {
    cfg!(feature = "nested")
}

/// Explains why the nested runtime cannot be started in a headless-only build.
///
/// Native Wayland, EGL, and GL handles remain confined to the feature-gated
/// implementation that follows in P6.
///
/// # Errors
///
/// Returns an error when this binary was built without the `nested` feature.
pub fn require_nested_runtime() -> Result<(), BackendError> {
    if nested_runtime_available() {
        #[cfg(feature = "nested")]
        let _ = smithay_boundary::smithay_version_is_linked();
        Ok(())
    } else {
        Err(BackendError::new(
            "nested Wayland runtime is not enabled; rebuild with the P6 nested adapter feature",
        ))
    }
}

#[cfg(feature = "nested")]
mod smithay_boundary {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use smithay::backend::input::{
        AbsolutePositionEvent, ButtonState, InputEvent, KeyboardKeyEvent, PointerButtonEvent,
    };
    use smithay::backend::renderer::element::Kind;
    use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
    use smithay::backend::renderer::element::surface::{
        WaylandSurfaceRenderElement, render_elements_from_surface_tree,
    };
    use smithay::backend::renderer::gles::GlesRenderer;
    use smithay::backend::renderer::utils::{draw_render_elements, on_commit_buffer_handler};
    use smithay::backend::renderer::{Color32F, Frame, Renderer};
    use smithay::backend::winit::{self, WinitEvent};
    use smithay::delegate_compositor;
    use smithay::delegate_output;
    use smithay::delegate_seat;
    use smithay::delegate_shm;
    use smithay::delegate_xdg_shell;
    use smithay::desktop::{
        PopupKeyboardGrab, PopupKind, PopupManager, PopupPointerGrab, find_popup_root_surface,
    };
    use smithay::input::keyboard::{FilterResult, KeyboardHandle, XkbConfig};
    use smithay::input::pointer::{
        ButtonEvent, CursorImageStatus, CursorImageSurfaceData, Focus, MotionEvent, PointerHandle,
    };
    use smithay::input::{Seat, SeatHandler, SeatState};
    use smithay::output::{Mode, Output, PhysicalProperties, Scale, Subpixel};
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
    use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
    use smithay::reexports::wayland_server::protocol::wl_buffer;
    use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
    use smithay::reexports::wayland_server::{Client, Display, ListeningSocket, Resource};
    use smithay::utils::{Logical, Physical, Point, Rectangle, Serial, Size, Transform};
    use smithay::wayland::buffer::BufferHandler;
    use smithay::wayland::compositor::{
        CompositorClientState, CompositorHandler, CompositorState, with_states,
    };
    use smithay::wayland::output::OutputHandler;
    use smithay::wayland::shell::xdg::{
        Configure, PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        XdgToplevelSurfaceData,
    };
    use smithay::wayland::shm::{ShmHandler, ShmState};
    use wm_backend::BackendEvent;
    use wm_render::{FrameCompletion, FrameLedger, FrameToken};

    use super::{BackendError, Rect, WindowId, XdgLifecycle, XdgSurfaceState};
    use wm_types::{OutputInfo, WindowMetadata};

    const INITIAL_WIDTH: i32 = 800;
    const INITIAL_HEIGHT: i32 = 600;
    const NESTED_OUTPUT_ID: &str = "nested-0";
    const NESTED_OUTPUT_WIDTH: u32 = 1280;
    const NESTED_OUTPUT_HEIGHT: u32 = 720;
    const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(2);
    const FRAME_INTERVAL: Duration = Duration::from_millis(16);
    const BORDER_WIDTH: i32 = 2;

    /// Proves the optional feature resolves Smithay without exporting its types.
    pub(super) fn smithay_version_is_linked() -> bool {
        let _ = std::any::TypeId::of::<
            smithay::backend::winit::WinitGraphicsBackend<
                smithay::backend::renderer::gles::GlesRenderer,
            >,
        >();
        true
    }

    /// A feature-gated, nested Wayland protocol server.
    ///
    /// This owns only Smithay-native resources. It deliberately exposes
    /// normalized lifecycle events instead of protocol objects to its caller.
    #[derive(Debug)]
    pub struct NestedWaylandServer {
        display: Display<NestedState>,
        listener: ListeningSocket,
        clients: Vec<Client>,
        state: NestedState,
    }

    impl NestedWaylandServer {
        /// Binds a Wayland socket and publishes the compositor, shm, and
        /// xdg-shell globals. The caller owns integration with the nested host
        /// event loop and presentation backend.
        ///
        /// # Errors
        ///
        /// Returns an error when the Wayland display or socket cannot be made.
        pub fn bind(socket_name: &str) -> Result<Self, BackendError> {
            let display = Display::new().map_err(|error| {
                BackendError::new(format!("cannot create nested Wayland display: {error}"))
            })?;
            let handle = display.handle();
            let output = OutputInfo::new(
                NESTED_OUTPUT_ID,
                NESTED_OUTPUT_WIDTH,
                NESTED_OUTPUT_HEIGHT,
                1.0,
            );
            let state = NestedState::new(&handle, output.clone())?;
            let listener = ListeningSocket::bind(socket_name).map_err(|error| {
                BackendError::new(format!(
                    "cannot bind nested Wayland socket {socket_name:?}: {error}"
                ))
            })?;
            Ok(Self {
                display,
                listener,
                clients: Vec::new(),
                state,
            })
        }

        /// Returns the bound Wayland socket name.
        #[must_use]
        pub fn socket_name(&self) -> Option<&std::ffi::OsStr> {
            self.listener.socket_name()
        }

        /// Returns the initial nested-host output exposed to the coordinator.
        #[must_use]
        pub fn output(&self) -> &OutputInfo {
            &self.state.output_info
        }

        /// Returns the current nested output bounds in logical coordinates.
        ///
        /// # Panics
        ///
        /// Panics only if the internally validated output state is corrupted.
        #[must_use]
        pub fn logical_bounds(&self) -> Rect {
            Rect::new(
                0.0,
                0.0,
                f64::from(self.state.output_info.physical_width) / self.state.output_info.scale,
                f64::from(self.state.output_info.physical_height) / self.state.output_info.scale,
            )
            .expect("validated nested output has positive logical bounds")
        }

        /// Replaces the normalized scene placement used to draw and hit-test
        /// mapped xdg toplevels. Entries are ordered back-to-front.
        pub fn synchronize_window_geometry(
            &mut self,
            windows: &[(WindowId, Rect)],
            focused_window: Option<WindowId>,
        ) {
            self.state
                .synchronize_window_geometry(windows, focused_window);
        }

        /// Accepts all currently pending client connections.
        ///
        /// # Errors
        ///
        /// Returns an error when a pending client cannot be registered.
        pub fn accept_clients(&mut self) -> Result<(), BackendError> {
            while let Some(stream) = self.listener.accept().map_err(|error| {
                BackendError::new(format!("cannot accept nested Wayland client: {error}"))
            })? {
                let client = self
                    .display
                    .handle()
                    .insert_client(stream, Arc::new(ClientState::default()))
                    .map_err(|error| {
                        BackendError::new(format!("cannot register nested Wayland client: {error}"))
                    })?;
                self.clients.push(client);
            }
            Ok(())
        }

        /// Dispatches pending Wayland requests and flushes protocol events.
        ///
        /// # Errors
        ///
        /// Returns an error when protocol dispatch or flushing fails.
        pub fn dispatch_clients(&mut self) -> Result<(), BackendError> {
            self.display
                .dispatch_clients(&mut self.state)
                .map_err(|error| {
                    BackendError::new(format!("nested Wayland dispatch failed: {error}"))
                })?;
            self.display
                .flush_clients()
                .map_err(|error| BackendError::new(format!("nested Wayland flush failed: {error}")))
        }

        /// Expires unacknowledged configure requests using the server clock.
        #[must_use]
        pub fn expire_configures(
            &mut self,
            timeout: Duration,
        ) -> Vec<wm_backend::ConfigureTransaction> {
            self.state
                .lifecycle
                .expire_configures(self.state.now(), timeout)
        }

        fn retry_expired_configures(&mut self) {
            for transaction in self.expire_configures(CONFIGURE_TIMEOUT) {
                let Some(surface) = self
                    .state
                    .xdg_shell_state
                    .toplevel_surfaces()
                    .iter()
                    .find(|surface| {
                        self.state.window_for(surface.wl_surface()) == Some(transaction.window)
                    })
                    .cloned()
                else {
                    continue;
                };
                let (width, height) = configure_extent(transaction.geometry);
                surface.with_pending_state(|state| {
                    state.size = Some((width, height).into());
                });
                let serial = surface.send_configure();
                let _ = self.state.lifecycle.configure_with_serial(
                    transaction.window,
                    transaction.geometry,
                    u64::from(u32::from(serial)),
                    self.state.now(),
                );
            }
        }

        /// Drains normalized lifecycle events in callback order.
        #[must_use]
        pub fn take_events(&mut self) -> Vec<BackendEvent> {
            std::mem::take(&mut self.state.events)
        }

        /// Flushes pending protocol events and releases nested client handles.
        ///
        /// # Errors
        ///
        /// Returns an error when the final protocol flush fails.
        pub fn shutdown(&mut self) -> Result<(), BackendError> {
            self.display.flush_clients().map_err(|error| {
                BackendError::new(format!("cannot flush nested shutdown events: {error}"))
            })?;
            self.clients.clear();
            Ok(())
        }

        /// Runs the nested Winit host and presents complete Wayland surface
        /// trees through the GLES renderer until the host window closes.
        ///
        /// # Errors
        ///
        /// Returns an error when the nested host, renderer, protocol dispatch,
        /// or presentation backend fails.
        pub fn run(self) -> Result<(), BackendError> {
            self.run_with_events(|_| {})
        }

        /// Runs the nested host and delivers normalized lifecycle events in
        /// protocol callback order to the composition root.
        ///
        /// # Errors
        ///
        /// Returns an error when the nested host, renderer, protocol dispatch,
        /// or presentation backend fails.
        pub fn run_with_events(
            self,
            mut on_event: impl FnMut(BackendEvent),
        ) -> Result<(), BackendError> {
            self.run_with_callbacks(|_, event| on_event(event), |_| {})
        }

        /// Runs the nested host and invokes callbacks after protocol events
        /// and once per completed event-loop iteration.
        ///
        /// # Errors
        ///
        /// Returns an error when the nested host, renderer, protocol dispatch,
        /// or presentation backend fails.
        #[allow(clippy::too_many_lines)]
        pub fn run_with_callbacks(
            mut self,
            mut on_event: impl FnMut(&mut Self, BackendEvent),
            mut on_tick: impl FnMut(&mut Self),
        ) -> Result<(), BackendError> {
            use smithay::reexports::winit::platform::pump_events::PumpStatus;

            let (mut backend, mut event_loop) = winit::init::<GlesRenderer>().map_err(|error| {
                BackendError::new(format!("cannot initialize nested Winit host: {error}"))
            })?;
            loop {
                let frame_started = Instant::now();
                let output_size = self.state.logical_output_size();
                let status = event_loop.dispatch_new_events(|event| {
                    forward_host_input(&mut self.state, event, output_size);
                });
                if matches!(status, PumpStatus::Exit(_)) {
                    self.shutdown()?;
                    return Ok(());
                }

                self.accept_clients()?;
                self.dispatch_clients()?;
                self.retry_expired_configures();
                for event in self.take_events() {
                    on_event(&mut self, event);
                }
                on_tick(&mut self);
                if let Some(cursor) = self.state.take_cursor_status() {
                    match cursor {
                        CursorImageStatus::Hidden | CursorImageStatus::Surface(_) => {
                            // The renderer draws client cursor surfaces, so
                            // hide the host cursor to avoid showing both.
                            backend.window().set_cursor_visible(false);
                        }
                        CursorImageStatus::Named(icon) => {
                            backend.window().set_cursor_visible(true);
                            backend.window().set_cursor(icon);
                        }
                    }
                }

                let size = backend.window_size();
                let damage = Rectangle::from_size(size);
                let frame_token = self.state.acquire_frame();
                {
                    let (renderer, mut framebuffer) = backend.bind().map_err(|error| {
                        BackendError::new(format!("cannot bind nested GLES frame: {error}"))
                    })?;
                    let mut borders = Vec::new();
                    let mut elements = Vec::<WaylandSurfaceRenderElement<GlesRenderer>>::new();
                    for window in &self.state.window_order {
                        let Some(geometry) = self.state.window_geometry.get(window) else {
                            continue;
                        };
                        let Some(surface) = self
                            .state
                            .xdg_shell_state
                            .toplevel_surfaces()
                            .iter()
                            .find(|surface| {
                                self.state.window_for(surface.wl_surface()) == Some(*window)
                            })
                        else {
                            continue;
                        };
                        borders.push(border_buffer(
                            *geometry,
                            self.state.output_info.scale,
                            self.state.focused_window == Some(*window),
                        ));
                        elements.extend(render_elements_from_surface_tree(
                            renderer,
                            surface.wl_surface(),
                            physical_surface_location(*geometry, self.state.output_info.scale),
                            1.0,
                            1.0,
                            Kind::Unspecified,
                        ));
                    }
                    for popup in self.state.xdg_shell_state.popup_surfaces() {
                        let location = self.state.popup_location(popup.wl_surface());
                        elements.extend(render_elements_from_surface_tree(
                            renderer,
                            popup.wl_surface(),
                            physical_popup_location(location, self.state.output_info.scale),
                            1.0,
                            1.0,
                            Kind::Unspecified,
                        ));
                    }
                    if let Some((surface, location)) = self.state.cursor_surface_with_location() {
                        elements.extend(render_elements_from_surface_tree(
                            renderer,
                            &surface,
                            physical_logical_point(location, self.state.output_info.scale),
                            1.0,
                            1.0,
                            Kind::Cursor,
                        ));
                    }
                    let mut frame = renderer
                        .render(&mut framebuffer, size, Transform::Flipped180)
                        .map_err(|error| {
                            BackendError::new(format!("cannot start nested GLES frame: {error}"))
                        })?;
                    frame
                        .clear(Color32F::new(0.05, 0.05, 0.08, 1.0), &[damage])
                        .map_err(|error| {
                            BackendError::new(format!("cannot clear nested GLES frame: {error}"))
                        })?;
                    let border_elements = borders
                        .iter()
                        .filter_map(|(buffer, location)| {
                            buffer.as_ref().map(|buffer| {
                                SolidColorRenderElement::from_buffer(
                                    buffer,
                                    *location,
                                    1.0,
                                    1.0,
                                    Kind::Unspecified,
                                )
                            })
                        })
                        .collect::<Vec<_>>();
                    draw_render_elements::<GlesRenderer, _, _>(
                        &mut frame,
                        1.0,
                        &border_elements,
                        &[damage],
                    )
                    .map_err(|error| {
                        BackendError::new(format!("cannot draw nested window borders: {error}"))
                    })?;
                    draw_render_elements(&mut frame, 1.0, &elements, &[damage]).map_err(
                        |error| {
                            BackendError::new(format!("cannot draw nested surface tree: {error}"))
                        },
                    )?;
                    // The nested Winit presenter performs the final display
                    // synchronization in `submit`; no fence crosses this
                    // adapter boundary.
                    let _ = frame.finish().map_err(|error| {
                        BackendError::new(format!("cannot finish nested GLES frame: {error}"))
                    })?;
                }
                self.state
                    .complete_frame(&frame_token, FrameCompletion::Rendered);

                let frame_time = self.state.now().as_millis().try_into().unwrap_or(u32::MAX);
                for surface in self.state.xdg_shell_state.toplevel_surfaces() {
                    send_frame_callbacks(surface.wl_surface(), frame_time);
                }
                for popup in self.state.xdg_shell_state.popup_surfaces() {
                    send_frame_callbacks(popup.wl_surface(), frame_time);
                }
                self.display.flush_clients().map_err(|error| {
                    BackendError::new(format!("cannot flush nested frame callbacks: {error}"))
                })?;
                backend.submit(Some(&[damage])).map_err(|error| {
                    BackendError::new(format!("cannot present nested GLES frame: {error}"))
                })?;
                self.state
                    .complete_frame(&frame_token, FrameCompletion::Presented);
                if let Some(remaining) = FRAME_INTERVAL.checked_sub(frame_started.elapsed()) {
                    std::thread::sleep(remaining);
                }
            }
        }
    }

    fn forward_host_input(
        state: &mut NestedState,
        event: WinitEvent,
        output_size: Size<i32, Logical>,
    ) {
        let event = match event {
            WinitEvent::Resized { size, scale_factor } => {
                state.update_output(size, scale_factor);
                return;
            }
            WinitEvent::Input(event) => event,
            _ => return,
        };
        let time = state.now().as_millis().try_into().unwrap_or(u32::MAX);
        match event {
            InputEvent::Keyboard { event } => {
                let keyboard = state.keyboard.clone();
                let _ = keyboard.input::<(), _>(
                    state,
                    event.key_code(),
                    event.state(),
                    0.into(),
                    time,
                    |_, _, _| FilterResult::Forward,
                );
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let location = event.position_transformed(output_size);
                state.pointer_location = logical_pointer_location(location, output_size);
                let focus = state
                    .popup_focus_at(location)
                    .or_else(|| state.pointer_focus_at(location));
                let pointer = state.pointer.clone();
                pointer.motion(
                    state,
                    focus,
                    &MotionEvent {
                        location,
                        serial: 0.into(),
                        time,
                    },
                );
                pointer.frame(state);
            }
            InputEvent::PointerButton { event } => {
                let pointer_location = (
                    f64::from(state.pointer_location.x),
                    f64::from(state.pointer_location.y),
                )
                    .into();
                if event.state() == ButtonState::Pressed
                    && let Some((surface, _)) = state
                        .popup_focus_at(pointer_location)
                        .or_else(|| state.pointer_focus_at(pointer_location))
                {
                    let keyboard = state.keyboard.clone();
                    keyboard.set_focus(state, Some(surface.clone()), 0.into());
                    if let Some(window) = state.focus_window_for(&surface) {
                        state.events.push(BackendEvent::WindowFocused(window));
                    }
                }
                let pointer = state.pointer.clone();
                pointer.button(
                    state,
                    &ButtonEvent {
                        button: event.button_code(),
                        state: event.state(),
                        serial: 0.into(),
                        time,
                    },
                );
                pointer.frame(state);
            }
            _ => {}
        }
    }

    fn logical_pointer_location(
        location: Point<f64, Logical>,
        bounds: Size<i32, Logical>,
    ) -> Point<i32, Logical> {
        let maximum_x = f64::from(bounds.w.saturating_sub(1));
        let maximum_y = f64::from(bounds.h.saturating_sub(1));
        let x = location.x.clamp(0.0, maximum_x).floor();
        let y = location.y.clamp(0.0, maximum_y).floor();
        // The values were clamped to the representable `i32` output bounds.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            (x as i32, y as i32).into()
        }
    }

    fn physical_surface_location(geometry: Rect, scale: f64) -> Point<i32, Physical> {
        physical_point((geometry.x, geometry.y).into(), scale)
    }

    fn physical_logical_point(location: Point<i32, Logical>, scale: f64) -> Point<i32, Physical> {
        physical_point((f64::from(location.x), f64::from(location.y)).into(), scale)
    }

    fn physical_point(location: Point<f64, Logical>, scale: f64) -> Point<i32, Physical> {
        let x = (location.x * scale)
            .round()
            .clamp(f64::from(i32::MIN), f64::from(i32::MAX));
        let y = (location.y * scale)
            .round()
            .clamp(f64::from(i32::MIN), f64::from(i32::MAX));
        // The values were clamped to the representable physical coordinate range.
        #[allow(clippy::cast_possible_truncation)]
        {
            (x as i32, y as i32).into()
        }
    }

    fn border_buffer(
        geometry: Rect,
        scale: f64,
        focused: bool,
    ) -> (Option<SolidColorBuffer>, Point<i32, Physical>) {
        let base = physical_surface_location(geometry, scale);
        let location = (base.x - BORDER_WIDTH, base.y - BORDER_WIDTH).into();
        let width = (geometry.width * scale)
            .round()
            .clamp(1.0, f64::from(i32::MAX - BORDER_WIDTH * 2));
        let height = (geometry.height * scale)
            .round()
            .clamp(1.0, f64::from(i32::MAX - BORDER_WIDTH * 2));
        #[allow(clippy::cast_possible_truncation)]
        let size = (
            width as i32 + BORDER_WIDTH * 2,
            height as i32 + BORDER_WIDTH * 2,
        );
        (
            Some(SolidColorBuffer::new(
                size,
                if focused {
                    [0.22, 0.48, 0.82, 1.0]
                } else {
                    [0.16, 0.18, 0.23, 1.0]
                },
            )),
            location,
        )
    }

    fn configure_extent(geometry: Rect) -> (i32, i32) {
        let width = geometry.width.round().clamp(1.0, f64::from(i32::MAX));
        let height = geometry.height.round().clamp(1.0, f64::from(i32::MAX));
        #[allow(clippy::cast_possible_truncation)]
        {
            (width as i32, height as i32)
        }
    }

    fn physical_popup_location(location: Point<i32, Logical>, scale: f64) -> Point<i32, Physical> {
        let x = (f64::from(location.x) * scale)
            .round()
            .clamp(f64::from(i32::MIN), f64::from(i32::MAX));
        let y = (f64::from(location.y) * scale)
            .round()
            .clamp(f64::from(i32::MIN), f64::from(i32::MAX));
        #[allow(clippy::cast_possible_truncation)]
        {
            (x as i32, y as i32).into()
        }
    }

    fn send_frame_callbacks(surface: &WlSurface, time: u32) {
        use smithay::wayland::compositor::{
            SurfaceAttributes, TraversalAction, with_surface_tree_downward,
        };

        with_surface_tree_downward(
            surface,
            (),
            |_, _, &()| TraversalAction::DoChildren(()),
            |_surface, states, &()| {
                for callback in states
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .current()
                    .frame_callbacks
                    .drain(..)
                {
                    callback.done(time);
                }
            },
            |_, _, &()| true,
        );
    }

    #[derive(Debug)]
    struct NestedState {
        compositor_state: CompositorState,
        shm_state: ShmState,
        xdg_shell_state: XdgShellState,
        output_info: OutputInfo,
        output: Output,
        seat_state: SeatState<Self>,
        seat: Seat<Self>,
        keyboard: KeyboardHandle<Self>,
        pointer: PointerHandle<Self>,
        cursor_status: CursorImageStatus,
        cursor_surface: Option<WlSurface>,
        pointer_location: Point<i32, Logical>,
        cursor_changed: bool,
        lifecycle: XdgLifecycle,
        windows: BTreeMap<u32, WindowId>,
        window_geometry: BTreeMap<WindowId, Rect>,
        window_order: Vec<WindowId>,
        focused_window: Option<WindowId>,
        popups: BTreeMap<u32, PopupPlacement>,
        popup_manager: PopupManager,
        frame_ledger: FrameLedger,
        next_scene_generation: u64,
        next_window: u64,
        events: Vec<BackendEvent>,
        started_at: Instant,
    }

    #[derive(Clone, Copy, Debug)]
    struct PopupPlacement {
        location: Point<i32, Logical>,
        size: Size<i32, Logical>,
        positioner: PositionerState,
    }

    impl NestedState {
        fn new(
            handle: &smithay::reexports::wayland_server::DisplayHandle,
            output: OutputInfo,
        ) -> Result<Self, BackendError> {
            let mut seat_state = SeatState::new();
            let mut seat = seat_state.new_wl_seat(handle, "horyzond");
            let keyboard = seat
                .add_keyboard(XkbConfig::default(), 200, 25)
                .map_err(|error| {
                    BackendError::new(format!("cannot create nested Wayland keyboard: {error}"))
                })?;
            let pointer = seat.add_pointer();
            let smithay_output = Output::new(
                NESTED_OUTPUT_ID.into(),
                PhysicalProperties {
                    size: (510, 287).into(),
                    subpixel: Subpixel::Unknown,
                    make: "Horyzond".into(),
                    model: "Nested Winit".into(),
                },
            );
            smithay_output.create_global::<Self>(handle);
            let mode = Mode {
                size: (1280, 720).into(),
                refresh: 60_000,
            };
            smithay_output.change_current_state(
                Some(mode),
                Some(Transform::Normal),
                Some(Scale::Integer(1)),
                Some((0, 0).into()),
            );
            smithay_output.set_preferred(mode);
            Ok(Self {
                compositor_state: CompositorState::new::<Self>(handle),
                shm_state: ShmState::new::<Self>(handle, Vec::new()),
                xdg_shell_state: XdgShellState::new::<Self>(handle),
                output_info: output.clone(),
                output: smithay_output,
                seat_state,
                seat,
                keyboard,
                pointer,
                cursor_status: CursorImageStatus::default_named(),
                cursor_surface: None,
                pointer_location: (0, 0).into(),
                cursor_changed: false,
                lifecycle: XdgLifecycle::default(),
                windows: BTreeMap::new(),
                window_geometry: BTreeMap::new(),
                window_order: Vec::new(),
                focused_window: None,
                popups: BTreeMap::new(),
                popup_manager: PopupManager::default(),
                frame_ledger: FrameLedger::default(),
                next_scene_generation: 0,
                next_window: 1,
                events: vec![BackendEvent::OutputAdded(output)],
                started_at: Instant::now(),
            })
        }

        fn now(&self) -> Duration {
            self.started_at.elapsed()
        }

        fn surface_key(surface: &WlSurface) -> u32 {
            surface.id().protocol_id()
        }

        fn window_for(&self, surface: &WlSurface) -> Option<WindowId> {
            self.windows.get(&Self::surface_key(surface)).copied()
        }

        fn focus_window_for(&self, surface: &WlSurface) -> Option<WindowId> {
            self.window_for(surface).or_else(|| {
                self.xdg_shell_state
                    .popup_surfaces()
                    .iter()
                    .find(|popup| popup.wl_surface() == surface)
                    .and_then(PopupSurface::get_parent_surface)
                    .and_then(|parent| self.focus_window_for(&parent))
            })
        }

        fn push_metadata_for(&mut self, surface: &ToplevelSurface) {
            let Some(window) = self.window_for(surface.wl_surface()) else {
                return;
            };
            let metadata = with_states(surface.wl_surface(), |states| {
                let attributes = states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .expect("xdg toplevel always has role attributes")
                    .lock()
                    .expect("xdg toplevel attributes lock");
                WindowMetadata {
                    app_id: attributes.app_id.clone().unwrap_or_default(),
                    title: attributes.title.clone().unwrap_or_default(),
                }
            });
            self.events
                .push(BackendEvent::WindowMetadataChanged(window, metadata));
        }

        fn synchronize_window_geometry(
            &mut self,
            windows: &[(WindowId, Rect)],
            focused_window: Option<WindowId>,
        ) {
            let changed = windows
                .iter()
                .filter_map(|(window, geometry)| {
                    (self.window_geometry.get(window) != Some(geometry))
                        .then_some((*window, *geometry))
                })
                .collect::<Vec<_>>();
            self.window_geometry = windows.iter().copied().collect();
            self.window_order = windows.iter().map(|(window, _)| *window).collect();
            self.focused_window = focused_window;
            for (window, geometry) in changed {
                self.reconfigure_mapped_toplevel(window, geometry);
            }
            self.refresh_popup_placements();
        }

        fn reconfigure_mapped_toplevel(&mut self, window: WindowId, geometry: Rect) {
            if self.lifecycle.state(window) != Some(XdgSurfaceState::Mapped) {
                return;
            }
            let Some(surface) = self
                .xdg_shell_state
                .toplevel_surfaces()
                .iter()
                .find(|surface| self.window_for(surface.wl_surface()) == Some(window))
                .cloned()
            else {
                return;
            };
            let (width, height) = configure_extent(geometry);
            surface.with_pending_state(|state| {
                state.size = Some((width, height).into());
            });
            let serial = surface.send_configure();
            let _ = self.lifecycle.configure_with_serial(
                window,
                geometry,
                u64::from(u32::from(serial)),
                self.now(),
            );
        }

        fn pointer_focus_at(
            &self,
            location: Point<f64, Logical>,
        ) -> Option<(WlSurface, Point<f64, Logical>)> {
            self.window_order.iter().rev().find_map(|window| {
                let geometry = self.window_geometry.get(window)?;
                let contains = location.x >= geometry.x
                    && location.x < geometry.x + geometry.width
                    && location.y >= geometry.y
                    && location.y < geometry.y + geometry.height;
                contains.then(|| {
                    self.xdg_shell_state
                        .toplevel_surfaces()
                        .iter()
                        .find(|surface| self.window_for(surface.wl_surface()) == Some(*window))
                        .map(|surface| {
                            (
                                surface.wl_surface().clone(),
                                (geometry.x, geometry.y).into(),
                            )
                        })
                })?
            })
        }

        fn popup_location(&self, surface: &WlSurface) -> Point<i32, Logical> {
            self.popups
                .get(&Self::surface_key(surface))
                .map_or_else(|| (0, 0).into(), |placement| placement.location)
        }

        fn popup_focus_at(
            &self,
            location: Point<f64, Logical>,
        ) -> Option<(WlSurface, Point<f64, Logical>)> {
            self.xdg_shell_state
                .popup_surfaces()
                .iter()
                .rev()
                .find_map(|popup| {
                    let placement = self.popups.get(&Self::surface_key(popup.wl_surface()))?;
                    let x = f64::from(placement.location.x);
                    let y = f64::from(placement.location.y);
                    let contains = location.x >= x
                        && location.x < x + f64::from(placement.size.w)
                        && location.y >= y
                        && location.y < y + f64::from(placement.size.h);
                    contains.then(|| (popup.wl_surface().clone(), (x, y).into()))
                })
        }

        fn place_popup(&mut self, surface: &PopupSurface, positioner: &PositionerState) {
            let parent: Point<i32, Logical> = surface
                .get_parent_surface()
                .and_then(|parent| {
                    self.window_for(&parent)
                        .and_then(|window| self.window_geometry.get(&window))
                        .map(|geometry| {
                            // Layout geometry is logical and finite; positioner values are protocol i32s.
                            #[allow(clippy::cast_possible_truncation)]
                            {
                                (geometry.x.round() as i32, geometry.y.round() as i32).into()
                            }
                        })
                        .or_else(|| {
                            self.popups
                                .get(&Self::surface_key(&parent))
                                .map(|popup| popup.location)
                        })
                })
                .unwrap_or_else(|| (0, 0).into());
            let output_size = self.logical_output_size();
            let target = Rectangle::new((-parent.x, -parent.y).into(), output_size);
            let geometry = (*positioner).get_unconstrained_geometry(target);
            self.popups.insert(
                Self::surface_key(surface.wl_surface()),
                PopupPlacement {
                    location: parent + geometry.loc,
                    size: geometry.size,
                    positioner: *positioner,
                },
            );
        }

        fn refresh_popup_placements(&mut self) {
            let popups = self
                .xdg_shell_state
                .popup_surfaces()
                .iter()
                .filter_map(|surface| {
                    self.popups
                        .get(&Self::surface_key(surface.wl_surface()))
                        .map(|placement| (surface.clone(), placement.positioner))
                })
                .collect::<Vec<_>>();
            for (surface, positioner) in popups {
                self.place_popup(&surface, &positioner);
            }
        }

        fn logical_output_size(&self) -> Size<i32, Logical> {
            let width = (f64::from(self.output_info.physical_width) / self.output_info.scale)
                .round()
                .clamp(1.0, f64::from(i32::MAX));
            let height = (f64::from(self.output_info.physical_height) / self.output_info.scale)
                .round()
                .clamp(1.0, f64::from(i32::MAX));
            #[allow(clippy::cast_possible_truncation)]
            {
                (width as i32, height as i32).into()
            }
        }

        fn acquire_frame(&mut self) -> FrameToken {
            self.next_scene_generation = self.next_scene_generation.saturating_add(1);
            let token = self.frame_ledger.acquire(&self.output_info.id);
            debug_assert!(
                self.frame_ledger
                    .submit(token.clone(), self.next_scene_generation)
            );
            token
        }

        fn complete_frame(&mut self, token: &FrameToken, completion: FrameCompletion) {
            let _ = self.frame_ledger.complete(token, completion);
        }

        fn update_output(&mut self, size: Size<i32, Physical>, scale: f64) {
            let (Ok(width), Ok(height)) = (u32::try_from(size.w), u32::try_from(size.h)) else {
                return;
            };
            if width == 0 || height == 0 {
                return;
            }
            let scale = if scale.is_finite() && scale > 0.0 {
                scale
            } else {
                1.0
            };
            if self.output_info.physical_width == width
                && self.output_info.physical_height == height
                && (self.output_info.scale - scale).abs() <= f64::EPSILON
            {
                return;
            }
            let previous = self.output_info.clone();
            let updated = OutputInfo::new(previous.id.as_str(), width, height, scale);
            let mode = Mode {
                size,
                refresh: 60_000,
            };
            self.output.change_current_state(
                Some(mode),
                Some(Transform::Normal),
                Some(Scale::Fractional(scale)),
                Some((0, 0).into()),
            );
            self.output.set_preferred(mode);
            self.output_info = updated.clone();
            self.events.push(BackendEvent::OutputRemoved(previous.id));
            self.events.push(BackendEvent::OutputAdded(updated));
        }

        fn take_cursor_status(&mut self) -> Option<CursorImageStatus> {
            self.cursor_changed.then(|| {
                self.cursor_changed = false;
                self.cursor_status.clone()
            })
        }

        fn cursor_surface_with_location(&self) -> Option<(WlSurface, Point<i32, Logical>)> {
            use smithay::wayland::compositor::with_states;

            let surface = self.cursor_surface.clone()?;
            let hotspot = with_states(&surface, |states| {
                states
                    .data_map
                    .get::<CursorImageSurfaceData>()
                    .map(|attributes| attributes.lock().expect("cursor attributes lock").hotspot)
            })
            .unwrap_or_else(|| (0, 0).into());
            Some((surface, self.pointer_location - hotspot))
        }
    }

    impl BufferHandler for NestedState {
        fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
    }

    impl CompositorHandler for NestedState {
        fn compositor_state(&mut self) -> &mut CompositorState {
            &mut self.compositor_state
        }

        fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
            &client
                .get_data::<ClientState>()
                .expect("nested Wayland clients always carry ClientState")
                .compositor_state
        }

        fn commit(&mut self, surface: &WlSurface) {
            on_commit_buffer_handler::<Self>(surface);
            self.popup_manager.commit(surface);
            if let Some(window) = self.window_for(surface)
                && self.lifecycle.commit(window).is_ok()
            {
                let keyboard = self.keyboard.clone();
                keyboard.set_focus(self, Some(surface.clone()), 0.into());
                self.events.push(BackendEvent::WindowMapped(window));
                self.events.push(BackendEvent::WindowFocused(window));
                if let Some(toplevel) = self
                    .xdg_shell_state
                    .toplevel_surfaces()
                    .iter()
                    .find(|toplevel| toplevel.wl_surface() == surface)
                    .cloned()
                {
                    self.push_metadata_for(&toplevel);
                }
            }
        }
    }

    impl ShmHandler for NestedState {
        fn shm_state(&self) -> &ShmState {
            &self.shm_state
        }
    }

    impl OutputHandler for NestedState {}

    impl SeatHandler for NestedState {
        type KeyboardFocus = WlSurface;
        type PointerFocus = WlSurface;
        type TouchFocus = WlSurface;

        fn seat_state(&mut self) -> &mut SeatState<Self> {
            &mut self.seat_state
        }

        fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}

        fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
            self.cursor_surface = match &image {
                CursorImageStatus::Surface(surface) => Some(surface.clone()),
                CursorImageStatus::Hidden | CursorImageStatus::Named(_) => None,
            };
            self.cursor_status = image;
            self.cursor_changed = true;
        }
    }

    impl XdgShellHandler for NestedState {
        fn xdg_shell_state(&mut self) -> &mut XdgShellState {
            &mut self.xdg_shell_state
        }

        fn new_toplevel(&mut self, surface: ToplevelSurface) {
            let window = WindowId::new(self.next_window);
            self.next_window = self.next_window.saturating_add(1);
            let wl_surface = surface.wl_surface().clone();
            let geometry = Rect::new(
                0.0,
                0.0,
                f64::from(INITIAL_WIDTH),
                f64::from(INITIAL_HEIGHT),
            )
            .expect("constant initial xdg geometry is valid");
            if self.lifecycle.register(window).is_err() {
                return;
            }
            self.windows.insert(Self::surface_key(&wl_surface), window);
            self.window_geometry.insert(window, geometry);
            self.window_order.push(window);
            surface.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Activated);
                state.size = Some((INITIAL_WIDTH, INITIAL_HEIGHT).into());
            });
            let serial = surface.send_configure();
            let _ = self.lifecycle.configure_with_serial(
                window,
                geometry,
                u64::from(u32::from(serial)),
                self.now(),
            );
        }

        fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
            self.place_popup(&surface, &positioner);
            let _ = self.popup_manager.track_popup(surface.clone().into());
            let _ = surface.send_configure();
        }

        fn grab(
            &mut self,
            surface: PopupSurface,
            _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
            serial: Serial,
        ) {
            let popup: PopupKind = surface.into();
            let Ok(root) = find_popup_root_surface(&popup) else {
                return;
            };
            let Ok(grab) = self
                .popup_manager
                .grab_popup::<Self>(root, popup, &self.seat, serial)
            else {
                return;
            };
            let keyboard = self.keyboard.clone();
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
            let pointer = self.pointer.clone();
            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }

        fn reposition_request(
            &mut self,
            surface: PopupSurface,
            positioner: PositionerState,
            token: u32,
        ) {
            self.place_popup(&surface, &positioner);
            let _ = surface.send_repositioned(token);
        }

        fn popup_destroyed(&mut self, surface: PopupSurface) {
            self.popups.remove(&Self::surface_key(surface.wl_surface()));
        }

        fn app_id_changed(&mut self, surface: ToplevelSurface) {
            self.push_metadata_for(&surface);
        }

        fn title_changed(&mut self, surface: ToplevelSurface) {
            self.push_metadata_for(&surface);
        }

        fn ack_configure(&mut self, surface: WlSurface, configure: Configure) {
            let Configure::Toplevel(configure) = configure else {
                return;
            };
            let Some(window) = self.window_for(&surface) else {
                return;
            };
            let _ = self
                .lifecycle
                .acknowledge_serial(window, u64::from(u32::from(configure.serial)));
        }

        fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
            let key = Self::surface_key(surface.wl_surface());
            if let Some(window) = self.windows.remove(&key) {
                self.lifecycle.destroy(window);
                self.window_geometry.remove(&window);
                self.window_order.retain(|candidate| *candidate != window);
                self.events.push(BackendEvent::WindowUnmapped(window));
            }
        }
    }

    #[derive(Debug, Default)]
    struct ClientState {
        compositor_state: CompositorClientState,
    }

    impl ClientData for ClientState {
        fn initialized(&self, _client_id: ClientId) {}

        fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
    }

    delegate_compositor!(NestedState);
    delegate_output!(NestedState);
    delegate_seat!(NestedState);
    delegate_shm!(NestedState);
    delegate_xdg_shell!(NestedState);
}

#[cfg(feature = "nested")]
pub use smithay_boundary::NestedWaylandServer;

#[cfg(test)]
mod tests {
    use super::{XdgLifecycle, XdgSurfaceState, nested_runtime_available, require_nested_runtime};
    use std::time::Duration;
    use wm_types::{Rect, WindowId};

    #[test]
    fn default_build_keeps_the_nested_runtime_disabled() {
        assert!(!nested_runtime_available());
        assert!(require_nested_runtime().is_err());
    }

    #[test]
    fn xdg_lifecycle_requires_acknowledgement_before_mapping() {
        let mut lifecycle = XdgLifecycle::default();
        let window = WindowId::new(4);
        lifecycle.register(window).expect("register");
        assert!(lifecycle.commit(window).is_err());
        let configure = lifecycle
            .configure(
                window,
                Rect::new(0.0, 0.0, 100.0, 50.0).expect("geometry"),
                Duration::ZERO,
            )
            .expect("configure");
        assert_eq!(
            lifecycle.state(window),
            Some(XdgSurfaceState::AwaitingConfigure)
        );
        lifecycle.acknowledge(configure).expect("acknowledge");
        lifecycle.commit(window).expect("commit");
        assert_eq!(lifecycle.state(window), Some(XdgSurfaceState::Mapped));
    }

    #[test]
    fn xdg_lifecycle_timeout_allows_a_fresh_configure() {
        let mut lifecycle = XdgLifecycle::default();
        let window = WindowId::new(5);
        let geometry = Rect::new(0.0, 0.0, 100.0, 50.0).expect("geometry");
        lifecycle.register(window).expect("register");
        let expired = lifecycle
            .configure(window, geometry, Duration::from_millis(10))
            .expect("configure");
        assert_eq!(
            lifecycle.expire_configures(Duration::from_millis(20), Duration::from_millis(5)),
            [expired]
        );
        assert_eq!(lifecycle.state(window), Some(XdgSurfaceState::New));
        assert!(
            lifecycle
                .configure(window, geometry, Duration::from_millis(21))
                .is_ok()
        );
    }

    #[test]
    fn xdg_lifecycle_allows_resizing_a_mapped_toplevel() {
        let mut lifecycle = XdgLifecycle::default();
        let window = WindowId::new(7);
        lifecycle.register(window).expect("register");
        let initial = lifecycle
            .configure(
                window,
                Rect::new(0.0, 0.0, 100.0, 50.0).expect("initial geometry"),
                Duration::ZERO,
            )
            .expect("initial configure");
        lifecycle
            .acknowledge(initial)
            .expect("initial acknowledgement");
        lifecycle.commit(window).expect("initial commit");
        let resized = lifecycle
            .configure(
                window,
                Rect::new(0.0, 0.0, 200.0, 100.0).expect("resized geometry"),
                Duration::from_millis(1),
            )
            .expect("resize configure");
        lifecycle
            .acknowledge(resized)
            .expect("resize acknowledgement");
        lifecycle.commit(window).expect("resize commit");
        assert_eq!(lifecycle.state(window), Some(XdgSurfaceState::Mapped));
    }

    #[test]
    fn xdg_lifecycle_accepts_the_protocols_configure_serial() {
        let mut lifecycle = XdgLifecycle::default();
        let window = WindowId::new(6);
        let geometry = Rect::new(0.0, 0.0, 100.0, 50.0).expect("geometry");
        lifecycle.register(window).expect("register");
        let configure = lifecycle
            .configure_with_serial(window, geometry, 42, Duration::ZERO)
            .expect("configure");
        assert_eq!(configure.serial, 42);
        lifecycle.acknowledge(configure).expect("acknowledge");
        lifecycle.commit(window).expect("commit");
        assert_eq!(lifecycle.state(window), Some(XdgSurfaceState::Mapped));
    }

    #[test]
    fn xdg_lifecycle_accepts_a_protocol_serial_without_reconstructing_geometry() {
        let mut lifecycle = XdgLifecycle::default();
        let window = WindowId::new(8);
        lifecycle.register(window).expect("register");
        lifecycle
            .configure_with_serial(
                window,
                Rect::new(15.0, 20.0, 320.0, 240.0).expect("resized geometry"),
                99,
                Duration::ZERO,
            )
            .expect("configure");
        lifecycle
            .acknowledge_serial(window, 99)
            .expect("serial acknowledgement");
        lifecycle.commit(window).expect("commit");
        assert_eq!(lifecycle.state(window), Some(XdgSurfaceState::Mapped));
    }
}

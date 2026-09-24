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
    /// Returns an error for an unknown or already mapped toplevel.
    pub fn configure(
        &mut self,
        window: WindowId,
        geometry: Rect,
        now: std::time::Duration,
    ) -> Result<ConfigureTransaction, BackendError> {
        match self.states.get(&window) {
            Some(XdgSurfaceState::New | XdgSurfaceState::AwaitingConfigure) => {
                self.states
                    .insert(window, XdgSurfaceState::AwaitingConfigure);
                Ok(self.configures.issue(window, geometry, now))
            }
            Some(XdgSurfaceState::ReadyToCommit | XdgSurfaceState::Mapped) => {
                Err(BackendError::new(format!(
                    "{window} cannot receive a new configure in its current lifecycle state"
                )))
            }
            None => Err(BackendError::new(format!("unknown xdg toplevel {window}"))),
        }
    }
    /// Tracks a configure issued by a protocol that supplies its own serial.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown or already mapped toplevel, or for a
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
            Some(XdgSurfaceState::New | XdgSurfaceState::AwaitingConfigure) => {
                self.states
                    .insert(window, XdgSurfaceState::AwaitingConfigure);
                Ok(self
                    .configures
                    .issue_with_serial(window, geometry, serial, now))
            }
            Some(XdgSurfaceState::ReadyToCommit | XdgSurfaceState::Mapped) => {
                Err(BackendError::new(format!(
                    "{window} cannot receive a new configure in its current lifecycle state"
                )))
            }
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

    use smithay::backend::renderer::element::Kind;
    use smithay::backend::renderer::element::surface::{
        WaylandSurfaceRenderElement, render_elements_from_surface_tree,
    };
    use smithay::backend::renderer::gles::GlesRenderer;
    use smithay::backend::renderer::utils::{draw_render_elements, on_commit_buffer_handler};
    use smithay::backend::renderer::{Color32F, Frame, Renderer};
    use smithay::backend::winit;
    use smithay::delegate_compositor;
    use smithay::delegate_seat;
    use smithay::delegate_shm;
    use smithay::delegate_xdg_shell;
    use smithay::input::pointer::CursorImageStatus;
    use smithay::input::{Seat, SeatHandler, SeatState};
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
    use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
    use smithay::reexports::wayland_server::protocol::wl_buffer;
    use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
    use smithay::reexports::wayland_server::{Client, Display, ListeningSocket, Resource};
    use smithay::utils::{Rectangle, Serial, Transform};
    use smithay::wayland::buffer::BufferHandler;
    use smithay::wayland::compositor::{CompositorClientState, CompositorHandler, CompositorState};
    use smithay::wayland::shell::xdg::{
        Configure, PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    };
    use smithay::wayland::shm::{ShmHandler, ShmState};
    use wm_backend::BackendEvent;

    use super::{BackendError, Rect, WindowId, XdgLifecycle};

    const INITIAL_WIDTH: i32 = 800;
    const INITIAL_HEIGHT: i32 = 600;

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
            let state = NestedState::new(&handle);
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

        /// Drains normalized lifecycle events in callback order.
        #[must_use]
        pub fn take_events(&mut self) -> Vec<BackendEvent> {
            std::mem::take(&mut self.state.events)
        }

        /// Runs the nested Winit host and presents complete Wayland surface
        /// trees through the GLES renderer until the host window closes.
        ///
        /// # Errors
        ///
        /// Returns an error when the nested host, renderer, protocol dispatch,
        /// or presentation backend fails.
        pub fn run(mut self) -> Result<(), BackendError> {
            use smithay::reexports::winit::platform::pump_events::PumpStatus;

            let (mut backend, mut event_loop) = winit::init::<GlesRenderer>().map_err(|error| {
                BackendError::new(format!("cannot initialize nested Winit host: {error}"))
            })?;
            loop {
                let status = event_loop.dispatch_new_events(|_| {});
                if matches!(status, PumpStatus::Exit(_)) {
                    return Ok(());
                }

                self.accept_clients()?;
                self.dispatch_clients()?;

                let size = backend.window_size();
                let damage = Rectangle::from_size(size);
                {
                    let (renderer, mut framebuffer) = backend.bind().map_err(|error| {
                        BackendError::new(format!("cannot bind nested GLES frame: {error}"))
                    })?;
                    let elements = self
                        .state
                        .xdg_shell_state
                        .toplevel_surfaces()
                        .iter()
                        .flat_map(|surface| {
                            render_elements_from_surface_tree(
                                renderer,
                                surface.wl_surface(),
                                (0, 0),
                                1.0,
                                1.0,
                                Kind::Unspecified,
                            )
                        })
                        .collect::<Vec<WaylandSurfaceRenderElement<GlesRenderer>>>();
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

                let frame_time = self.state.now().as_millis().try_into().unwrap_or(u32::MAX);
                for surface in self.state.xdg_shell_state.toplevel_surfaces() {
                    send_frame_callbacks(surface.wl_surface(), frame_time);
                }
                self.display.flush_clients().map_err(|error| {
                    BackendError::new(format!("cannot flush nested frame callbacks: {error}"))
                })?;
                backend.submit(Some(&[damage])).map_err(|error| {
                    BackendError::new(format!("cannot present nested GLES frame: {error}"))
                })?;
            }
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
        seat_state: SeatState<Self>,
        lifecycle: XdgLifecycle,
        windows: BTreeMap<u32, WindowId>,
        next_window: u64,
        events: Vec<BackendEvent>,
        started_at: Instant,
    }

    impl NestedState {
        fn new(handle: &smithay::reexports::wayland_server::DisplayHandle) -> Self {
            Self {
                compositor_state: CompositorState::new::<Self>(handle),
                shm_state: ShmState::new::<Self>(handle, Vec::new()),
                xdg_shell_state: XdgShellState::new::<Self>(handle),
                seat_state: SeatState::new(),
                lifecycle: XdgLifecycle::default(),
                windows: BTreeMap::new(),
                next_window: 1,
                events: Vec::new(),
                started_at: Instant::now(),
            }
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
            if let Some(window) = self.window_for(surface)
                && self.lifecycle.commit(window).is_ok()
            {
                self.events.push(BackendEvent::WindowMapped(window));
            }
        }
    }

    impl ShmHandler for NestedState {
        fn shm_state(&self) -> &ShmState {
            &self.shm_state
        }
    }

    impl SeatHandler for NestedState {
        type KeyboardFocus = WlSurface;
        type PointerFocus = WlSurface;
        type TouchFocus = WlSurface;

        fn seat_state(&mut self) -> &mut SeatState<Self> {
            &mut self.seat_state
        }

        fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}

        fn cursor_image(&mut self, _seat: &Seat<Self>, _image: CursorImageStatus) {}
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

        fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}

        fn grab(
            &mut self,
            _surface: PopupSurface,
            _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
            _serial: Serial,
        ) {
        }

        fn reposition_request(
            &mut self,
            _surface: PopupSurface,
            _positioner: PositionerState,
            _token: u32,
        ) {
        }

        fn ack_configure(&mut self, surface: WlSurface, configure: Configure) {
            let Configure::Toplevel(configure) = configure else {
                return;
            };
            let Some(window) = self.window_for(&surface) else {
                return;
            };
            let transaction = wm_backend::ConfigureTransaction {
                window,
                serial: u64::from(u32::from(configure.serial)),
                geometry: Rect::new(
                    0.0,
                    0.0,
                    f64::from(INITIAL_WIDTH),
                    f64::from(INITIAL_HEIGHT),
                )
                .expect("constant initial xdg geometry is valid"),
            };
            let _ = self.lifecycle.acknowledge(transaction);
        }

        fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
            let key = Self::surface_key(surface.wl_surface());
            if let Some(window) = self.windows.remove(&key) {
                self.lifecycle.destroy(window);
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
}

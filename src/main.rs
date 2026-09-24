//! Horyzond composition root.
//!
//! Platform adapters are selected here. The P0 executable intentionally wires
//! only deterministic headless components; Wayland and graphics adapters land
//! in later milestones.

use wm_backend::{BackendEvent, WindowSystem};
use wm_backend_headless::HeadlessBackend;
use wm_config::{ConfigManager, ConfigPath, ReloadOutcome, install_defaults};
use wm_core::CoreState;
use wm_diagnostics::{LogLevel, SessionId, SessionLogger, install_panic_hook};
use wm_hooks::{HookDispatcher, HookEvent};
use wm_ipc::{IpcServer, Request, Response};
use wm_render::{RecordingRenderer, Renderer};
use wm_runtime::{LayoutRuntime, LayoutRuntimeUpdate};
use wm_script::ScriptLimits;
use wm_types::{OutputInfo, Point, Rect};

#[cfg(feature = "nested-wayland")]
use wm_backend_wayland::NestedWaylandServer;

#[allow(clippy::too_many_lines)]
fn main() {
    let options = match parse_options() {
        Ok(options) => options,
        Err(error) => exit_with_error(&error),
    };
    let config = options.config;
    let _lock = match config.acquire_instance_lock() {
        Ok(lock) => lock,
        Err(error) => exit_with_error(&error.to_string()),
    };
    let bootstrap = match install_defaults(&config) {
        Ok(report) => report,
        Err(error) => exit_with_error(&error.to_string()),
    };
    let session_id = SessionId::generate();
    install_panic_hook(config.as_path(), session_id.clone());
    let mut logger = match SessionLogger::open(config.as_path(), session_id) {
        Ok(logger) => logger,
        Err(error) => exit_with_error(&error.to_string()),
    };
    if let Err(error) = logger.record(
        LogLevel::Info,
        "bootstrap",
        &format!(
            "created {} default file(s); preserved {}",
            bootstrap.created.len(),
            bootstrap.preserved.len()
        ),
    ) {
        exit_with_error(&error.to_string());
    }
    let mut backend = HeadlessBackend::new([OutputInfo::new("headless-0", 1280, 720, 1.0)]);
    let renderer = RecordingRenderer::default();

    let mut configuration = match ConfigManager::new(&config, ScriptLimits::default()) {
        Ok(manager) => manager,
        Err(error) => exit_with_error(&error.to_string()),
    };
    if let ReloadOutcome::Rejected { diagnostic } = configuration.load_initial() {
        exit_with_error(&format!("initial configuration rejected: {diagnostic}"));
    }
    let mut runtime = match LayoutRuntime::from_config(&configuration) {
        Ok(runtime) => runtime,
        Err(error) => exit_with_error(&error.to_string()),
    };
    let mut core = CoreState::default();
    runtime.apply_default(
        &mut core,
        Rect::new(0.0, 0.0, 1280.0, 720.0).expect("constant headless bounds"),
    );
    if let Err(error) = logger.record(
        LogLevel::Info,
        "layout",
        &format!(
            "selected {} layout with {} provider",
            runtime.default_layout(),
            "active Lua or recovery"
        ),
    ) {
        exit_with_error(&error.to_string());
    }
    let mut hooks = HookDispatcher::new(256);
    dispatch_hook(
        &mut hooks,
        &configuration,
        HookEvent::Startup,
        "on_startup",
        &serde_json::json!({ "generation": configuration.generation() }),
        &mut logger,
    );

    if let Some(socket_name) = options.nested_socket {
        #[cfg(feature = "nested-wayland")]
        {
            run_nested(
                &socket_name,
                &mut configuration,
                &mut runtime,
                &mut core,
                &mut logger,
                &mut hooks,
            );
            if let Err(error) = logger.close() {
                exit_with_error(&error.to_string());
            }
            return;
        }
        #[cfg(not(feature = "nested-wayland"))]
        exit_with_error(&format!(
            "nested Wayland runtime requested for socket {socket_name:?}, but this binary was built without --features nested-wayland"
        ));
    }

    if let Err(error) = backend.initialize() {
        eprintln!("failed to initialize the headless backend: {error}");
        std::process::exit(1);
    }
    let ipc = match config.ipc_socket_path().and_then(|path| {
        IpcServer::bind(path)
            .map_err(|error| wm_config::ConfigError::Io(std::io::Error::other(error)))
    }) {
        Ok(server) => server,
        Err(error) => exit_with_error(&error.to_string()),
    };
    println!(
        "Horyzond headless runtime is ready: {} output(s), {} layout, renderer capabilities: {:?}",
        backend.outputs().len(),
        runtime.default_layout(),
        renderer.capabilities()
    );
    if !options.once {
        run_headless_loop(
            &mut backend,
            &mut configuration,
            &mut runtime,
            &mut core,
            &mut logger,
            &ipc,
            &mut hooks,
        );
    }
    if let Err(error) = logger.close() {
        exit_with_error(&error.to_string());
    }
}

#[cfg(feature = "nested-wayland")]
fn run_nested(
    socket_name: &str,
    configuration: &mut ConfigManager,
    runtime: &mut LayoutRuntime,
    core: &mut CoreState,
    logger: &mut SessionLogger,
    hooks: &mut HookDispatcher,
) {
    let server = match NestedWaylandServer::bind(socket_name) {
        Ok(server) => server,
        Err(error) => exit_with_error(&error.to_string()),
    };
    println!(
        "Horyzond nested Wayland runtime is ready on {}; set WAYLAND_DISPLAY to connect clients.",
        server.socket_name().map_or_else(
            || socket_name.to_owned(),
            |name| name.to_string_lossy().into_owned(),
        )
    );
    if let Err(error) = server.run_with_events(|event| {
        if let Err(error) = core.apply_event(event.clone()) {
            let _ = logger.record(LogLevel::Warn, "backend", &error.to_string());
            return;
        }
        match event {
            BackendEvent::WindowMapped(window) => dispatch_hook(
                hooks,
                configuration,
                HookEvent::Committed,
                "on_window_open",
                &serde_json::json!({ "window_id": window.get() }),
                logger,
            ),
            BackendEvent::WindowUnmapped(window) => runtime.forget_window_metadata(window),
            BackendEvent::OutputAdded(_)
            | BackendEvent::OutputRemoved(_)
            | BackendEvent::WindowMetadataChanged(_, _) => {}
        }
    }) {
        exit_with_error(&error.to_string());
    }
}

fn run_headless_loop(
    backend: &mut HeadlessBackend,
    configuration: &mut ConfigManager,
    runtime: &mut LayoutRuntime,
    core: &mut CoreState,
    logger: &mut SessionLogger,
    ipc: &IpcServer,
    hooks: &mut HookDispatcher,
) {
    let bounds = Rect::new(0.0, 0.0, 1280.0, 720.0).expect("constant headless bounds");
    loop {
        match backend.poll_events() {
            Ok(events) => {
                for event in events {
                    if let Err(error) = core.apply_event(event.clone()) {
                        let _ = logger.record(LogLevel::Warn, "backend", &error.to_string());
                        continue;
                    }
                    match event {
                        BackendEvent::WindowMetadataChanged(window, metadata) => {
                            let actions = runtime.update_window_metadata(window, &metadata);
                            for action in &actions {
                                dispatch_rule_action(runtime, core, window, action, bounds, logger);
                            }
                        }
                        BackendEvent::WindowMapped(window) => {
                            dispatch_hook(
                                hooks,
                                configuration,
                                HookEvent::Committed,
                                "on_window_open",
                                &serde_json::json!({ "window_id": window.get() }),
                                logger,
                            );
                        }
                        BackendEvent::WindowUnmapped(window) => {
                            runtime.forget_window_metadata(window);
                        }
                        BackendEvent::OutputAdded(_) | BackendEvent::OutputRemoved(_) => {}
                    }
                }
            }
            Err(error) => {
                let _ = logger.record(LogLevel::Warn, "backend", &error.to_string());
            }
        }
        if let Err(error) =
            ipc.poll(|request| handle_ipc(&request, configuration, runtime, core, hooks, logger))
        {
            let _ = logger.record(LogLevel::Warn, "ipc", &error.to_string());
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
        match configuration.reload_if_changed() {
            ReloadOutcome::Unchanged => {}
            ReloadOutcome::Rejected { diagnostic } => {
                let _ = logger.record(LogLevel::Warn, "reload", &diagnostic);
            }
            ReloadOutcome::Applied { generation } => match runtime.synchronize(configuration) {
                Ok(LayoutRuntimeUpdate::Applied { providers, .. }) => {
                    runtime.reapply_active(core, bounds);
                    let _ = logger.record(
                        LogLevel::Info,
                        "reload",
                        &format!(
                            "applied generation {generation}: {} provider(s) updated, {} retained",
                            providers.applied.len(),
                            providers.retained.len()
                        ),
                    );
                    dispatch_hook(
                        hooks,
                        configuration,
                        HookEvent::Reload,
                        "on_reload",
                        &serde_json::json!({ "generation": generation }),
                        logger,
                    );
                }
                Ok(LayoutRuntimeUpdate::Unchanged) => {}
                Err(error) => {
                    let _ = logger.record(LogLevel::Error, "reload", &error.to_string());
                }
            },
        }
    }
}

fn dispatch_rule_action(
    runtime: &mut LayoutRuntime,
    core: &mut CoreState,
    window: wm_types::WindowId,
    action: &wm_runtime::RuleAction,
    bounds: Rect,
    logger: &mut SessionLogger,
) {
    match runtime.dispatch_action(core, &action.name, &action.arguments, bounds) {
        Ok(dispatched) => {
            let _ = logger.record(
                LogLevel::Info,
                "rules",
                &format!(
                    "dispatched {dispatched:?} action {} for {window}",
                    action.name
                ),
            );
        }
        Err(error) => {
            let _ = logger.record(
                LogLevel::Warn,
                "rules",
                &format!(
                    "could not dispatch action {} for {window}: {error}",
                    action.name
                ),
            );
        }
    }
}

fn dispatch_hook(
    hooks: &mut HookDispatcher,
    configuration: &ConfigManager,
    kind: HookEvent,
    event: &str,
    payload: &serde_json::Value,
    logger: &mut SessionLogger,
) {
    if configuration.hook_path(event).is_none() || !hooks.begin(kind) {
        return;
    }
    if let Err(error) = configuration.execute_hook(event, payload) {
        let _ = logger.record(LogLevel::Warn, "hooks", &error.to_string());
    }
    hooks.finish();
}

#[allow(clippy::too_many_lines)]
fn handle_ipc(
    request: &Request,
    configuration: &mut ConfigManager,
    runtime: &mut LayoutRuntime,
    core: &mut CoreState,
    hooks: &mut HookDispatcher,
    logger: &mut SessionLogger,
) -> Response {
    let result = match request.method.as_str() {
        "status" => Ok(serde_json::json!({
            "configuration_generation": configuration.generation(),
            "layout": runtime.default_layout().as_str(),
            "mode": runtime.mode(),
            "active_layout": core.active_workspace().layout().as_str(),
            "pending_spawn": runtime.pending_spawn().map(|spawn| serde_json::json!({
                "executable": spawn.executable,
                "arguments": spawn.arguments,
            })),
        })),
        "config.status" => Ok(serde_json::json!({
            "generation": configuration.generation(),
            "active": configuration.generation() > 0,
        })),
        "windows.list" => Ok(serde_json::json!({
            "windows": core.windows().map(wm_types::WindowId::get).collect::<Vec<_>>(),
        })),
        "workspaces.list" => Ok(serde_json::json!({
            "active": core.active_workspace_id().get(),
            "workspaces": core.workspaces().map(|workspace| serde_json::json!({
                "id": workspace.id.get(),
                "layout": workspace.layout().as_str(),
                "focused_window": workspace.focused().map(wm_types::WindowId::get),
                "camera": {
                    "x": workspace.camera.position().x,
                    "y": workspace.camera.position().y,
                    "zoom": workspace.camera.zoom(),
                },
            })).collect::<Vec<_>>(),
        })),
        "workspace.switch" => (|| {
            let workspace = request
                .params
                .get("workspace")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| {
                    "workspace.switch requires an unsigned workspace parameter".to_owned()
                })?;
            core.switch_workspace(wm_core::WorkspaceId::new(workspace))
                .map_err(|error| error.to_string())?;
            dispatch_hook(
                hooks,
                configuration,
                HookEvent::Committed,
                "on_workspace_change",
                &serde_json::json!({ "workspace_id": workspace }),
                logger,
            );
            Ok(serde_json::json!({ "workspace": workspace }))
        })(),
        "input.press" => (|| {
            let chord = request
                .params
                .get("chord")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "input.press requires a string chord parameter".to_owned())?;
            let outcome = runtime.press(chord);
            let dispatched = match outcome.command {
                Some(command) if command.name == "switch_mode" => None,
                Some(command) => Some(
                    runtime
                        .dispatch_action(
                            core,
                            &command.name,
                            &command.arguments,
                            Rect::new(0.0, 0.0, 1280.0, 720.0).expect("constant headless bounds"),
                        )
                        .map_err(|error| error.to_string())?,
                ),
                None => None,
            };
            Ok(serde_json::json!({
                "consumed": outcome.consumed,
                "mode": outcome.mode.as_str(),
                "dispatched": dispatched.map(|value| format!("{value:?}")),
            }))
        })(),
        "input.release" => (|| {
            let chord = request
                .params
                .get("chord")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "input.release requires a string chord parameter".to_owned())?;
            Ok(serde_json::json!({
                "consumed": runtime.release(chord),
                "mode": runtime.mode(),
            }))
        })(),
        "layout.select" => (|| {
            let layout = request
                .params
                .get("layout")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "layout.select requires a string layout parameter".to_owned())?;
            let layout = wm_layout::LayoutId::new(layout).map_err(|error| error.to_string())?;
            if configuration.layout_path(layout.as_str()).is_none() {
                return Err(format!("unknown configured layout: {layout}"));
            }
            runtime.apply(
                core,
                &layout,
                Rect::new(0.0, 0.0, 1280.0, 720.0).expect("constant headless bounds"),
            );
            Ok(serde_json::json!({ "layout": layout.as_str() }))
        })(),
        "camera.set" => (|| {
            let x = request
                .params
                .get("x")
                .and_then(serde_json::Value::as_f64)
                .ok_or_else(|| "camera.set requires a finite numeric x parameter".to_owned())?;
            let y = request
                .params
                .get("y")
                .and_then(serde_json::Value::as_f64)
                .ok_or_else(|| "camera.set requires a finite numeric y parameter".to_owned())?;
            let zoom = request
                .params
                .get("zoom")
                .and_then(serde_json::Value::as_f64)
                .ok_or_else(|| "camera.set requires a finite numeric zoom parameter".to_owned())?;
            let position = Point::new(x, y).map_err(|error| error.to_string())?;
            core.set_active_camera(position, zoom)
                .map_err(|error| error.to_string())?;
            Ok(serde_json::json!({ "x": x, "y": y, "zoom": zoom }))
        })(),
        "config.reload" => (|| match configuration.reload() {
            ReloadOutcome::Rejected { diagnostic } => Err(diagnostic),
            ReloadOutcome::Unchanged => Ok(serde_json::json!({
                "generation": configuration.generation(),
                "changed": false,
            })),
            ReloadOutcome::Applied { generation } => {
                let update = runtime
                    .synchronize(configuration)
                    .map_err(|error| error.to_string())?;
                runtime.reapply_active(
                    core,
                    Rect::new(0.0, 0.0, 1280.0, 720.0).expect("constant headless bounds"),
                );
                Ok(serde_json::json!({
                    "generation": generation,
                    "changed": !matches!(update, LayoutRuntimeUpdate::Unchanged),
                }))
            }
        })(),
        "spawn.pending" => (|| {
            let executable = request
                .params
                .get("executable")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "spawn.pending requires a string executable parameter".to_owned())?;
            let arguments = request
                .params
                .get("arguments")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| "spawn.pending requires an arguments array".to_owned())?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| "spawn.pending arguments must be strings".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?;
            runtime
                .begin_pending_spawn(wm_runtime::ProcessLaunch {
                    executable: executable.to_owned(),
                    arguments,
                })
                .map_err(|error| error.to_string())?;
            Ok(serde_json::json!({ "pending": true }))
        })(),
        "spawn.cancel" => Ok(serde_json::json!({
            "cancelled": runtime.cancel_pending_spawn().is_some(),
        })),
        _ => Err(format!("unknown IPC method: {}", request.method)),
    };
    match result {
        Ok(result) => Response::success(request, result),
        Err(error) => Response::failure(request, error),
    }
}

struct Options {
    config: ConfigPath,
    once: bool,
    nested_socket: Option<String>,
}

fn parse_options() -> Result<Options, String> {
    let mut config = None;
    let mut once = false;
    let mut nested_socket = None;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--config-dir" {
            config = Some(ConfigPath::from_override(
                arguments
                    .next()
                    .ok_or_else(|| "--config-dir requires a path".to_owned())?,
            ));
        } else if argument == "--once" {
            once = true;
        } else if argument == "--nested" {
            nested_socket.get_or_insert_with(|| "horyzond-0".to_owned());
        } else if argument == "--nested-socket" {
            let socket = arguments
                .next()
                .ok_or_else(|| "--nested-socket requires a socket name".to_owned())?;
            if nested_socket
                .replace(socket.to_string_lossy().into_owned())
                .is_some()
            {
                return Err("nested Wayland runtime was selected more than once".to_owned());
            }
        } else if argument == "--help" || argument == "-h" {
            println!(
                "Usage: horyzond [--config-dir PATH] [--once] [--nested | --nested-socket NAME]"
            );
            std::process::exit(0);
        } else {
            return Err(format!("unknown argument: {}", argument.to_string_lossy()));
        }
    }
    Ok(Options {
        config: config
            .map_or_else(ConfigPath::default_for_current_user, Ok)
            .map_err(|error| error.to_string())?,
        once,
        nested_socket,
    })
}

fn exit_with_error(message: &str) -> ! {
    eprintln!("horyzond: {message}");
    std::process::exit(1)
}

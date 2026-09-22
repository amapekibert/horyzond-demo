//! Horyzond composition root.
//!
//! Platform adapters are selected here. The P0 executable intentionally wires
//! only deterministic headless components; Wayland and graphics adapters land
//! in later milestones.

use wm_backend::WindowSystem;
use wm_backend_headless::HeadlessBackend;
use wm_config::{ConfigPath, install_defaults};
use wm_diagnostics::{LogLevel, SessionId, SessionLogger, install_panic_hook};
use wm_render::{RecordingRenderer, Renderer};
use wm_types::OutputInfo;

fn main() {
    let config = match parse_config_path() {
        Ok(config) => config,
        Err(error) => exit_with_error(&error),
    };
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

    if let Err(error) = backend.initialize() {
        eprintln!("failed to initialize the headless backend: {error}");
        std::process::exit(1);
    }

    println!(
        "Horyzond P1 foundation is ready: {} output(s), renderer capabilities: {:?}",
        backend.outputs().len(),
        renderer.capabilities()
    );
    if let Err(error) = logger.close() {
        exit_with_error(&error.to_string());
    }
}

fn parse_config_path() -> Result<ConfigPath, String> {
    let mut arguments = std::env::args_os().skip(1);
    match arguments.next() {
        None => ConfigPath::default_for_current_user().map_err(|error| error.to_string()),
        Some(argument) if argument == "--config-dir" => arguments
            .next()
            .map(ConfigPath::from_override)
            .ok_or_else(|| "--config-dir requires a path".to_owned()),
        Some(argument) if argument == "--help" || argument == "-h" => {
            println!("Usage: horyzond [--config-dir PATH]");
            std::process::exit(0);
        }
        Some(argument) => Err(format!("unknown argument: {}", argument.to_string_lossy())),
    }
}

fn exit_with_error(message: &str) -> ! {
    eprintln!("horyzond: {message}");
    std::process::exit(1)
}

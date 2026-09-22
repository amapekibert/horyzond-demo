//! Horyzond composition root.
//!
//! Platform adapters are selected here. The P0 executable intentionally wires
//! only deterministic headless components; Wayland and graphics adapters land
//! in later milestones.

use wm_backend::WindowSystem;
use wm_backend_headless::HeadlessBackend;
use wm_render::{RecordingRenderer, Renderer};
use wm_types::OutputInfo;

fn main() {
    let mut backend = HeadlessBackend::new([OutputInfo::new("headless-0", 1280, 720, 1.0)]);
    let renderer = RecordingRenderer::default();

    if let Err(error) = backend.initialize() {
        eprintln!("failed to initialize the headless backend: {error}");
        std::process::exit(1);
    }

    println!(
        "Horyzond P0 foundation is ready: {} output(s), renderer capabilities: {:?}",
        backend.outputs().len(),
        renderer.capabilities()
    );
}

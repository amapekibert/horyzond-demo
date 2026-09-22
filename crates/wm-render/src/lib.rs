//! Renderer contracts independent from presentation and window-system APIs.

use std::fmt;

use wm_types::{Capabilities, OutputId, Rect, WindowId};

/// An immutable scene description prepared by `wm-scene`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneSnapshot {
    /// Generation assigned by the coordinator.
    pub generation: u64,
    /// Already-projected output-local rectangles to render in order.
    pub surfaces: Vec<SurfaceQuad>,
}

/// A visible client surface in output-local logical coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceQuad {
    pub window: WindowId,
    pub bounds: Rect,
}

/// A completed rendering operation ready for a presenter to submit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameResult {
    pub output: OutputId,
    pub scene_generation: u64,
}

/// A renderer consumes prepared scene snapshots; presenters remain separate.
pub trait Renderer {
    /// Returns graphics capabilities supplied by this renderer.
    fn capabilities(&self) -> Capabilities;
    /// Renders one immutable scene snapshot for an output.
    ///
    /// # Errors
    ///
    /// Returns a renderer-specific failure without leaking native graphics resources.
    fn render(
        &mut self,
        output: &OutputId,
        scene: &SceneSnapshot,
    ) -> Result<FrameResult, RenderError>;
}

/// A renderer failure that does not leak graphics API objects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderError {
    message: String,
}

impl RenderError {
    /// Creates a renderer error with safe, user-facing context.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RenderError {}

/// A deterministic renderer for contract tests and headless development.
#[derive(Debug, Default)]
pub struct RecordingRenderer {
    frames: Vec<FrameResult>,
}

impl RecordingRenderer {
    /// Returns frames in render order.
    #[must_use]
    pub fn frames(&self) -> &[FrameResult] {
        &self.frames
    }
}

impl Renderer for RecordingRenderer {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn render(
        &mut self,
        output: &OutputId,
        scene: &SceneSnapshot,
    ) -> Result<FrameResult, RenderError> {
        let frame = FrameResult {
            output: output.clone(),
            scene_generation: scene.generation,
        };
        self.frames.push(frame.clone());
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::{RecordingRenderer, Renderer, SceneSnapshot};
    use wm_types::OutputId;

    #[test]
    fn recording_renderer_preserves_scene_generation() {
        let mut renderer = RecordingRenderer::default();
        let output = OutputId::new("test-output").expect("valid output ID");
        let frame = renderer
            .render(
                &output,
                &SceneSnapshot {
                    generation: 7,
                    surfaces: Vec::new(),
                },
            )
            .expect("recording cannot fail");
        assert_eq!(frame.scene_generation, 7);
        assert_eq!(renderer.frames(), [frame]);
    }
}

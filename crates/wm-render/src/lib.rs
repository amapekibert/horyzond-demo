//! Renderer contracts independent from presentation and window-system APIs.

use std::collections::BTreeMap;
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

/// Opaque identifier for one acquired presentation target.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FrameToken {
    /// Output whose target was acquired.
    pub output: OutputId,
    /// Presenter-assigned monotonic sequence number.
    pub sequence: u64,
}

/// Completion stage reported by a concrete presenter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameCompletion {
    /// GPU work no longer uses client content, but presentation may be pending.
    Rendered,
    /// The presentation system completed the frame.
    Presented,
}

/// Tracks target ownership until the presenter reports its completion stage.
#[derive(Debug, Default)]
pub struct FrameLedger {
    next_sequence: u64,
    pending: BTreeMap<FrameToken, u64>,
}
impl FrameLedger {
    /// Acquires a neutral target token for one output.
    #[must_use]
    pub fn acquire(&mut self, output: &OutputId) -> FrameToken {
        self.next_sequence = self.next_sequence.saturating_add(1);
        FrameToken {
            output: output.clone(),
            sequence: self.next_sequence,
        }
    }
    /// Marks a token submitted with its immutable scene generation.
    pub fn submit(&mut self, token: FrameToken, scene_generation: u64) -> bool {
        self.pending.insert(token, scene_generation).is_none()
    }
    /// Completes a submitted token. Rendering completion retains ownership until
    /// presentation completion; only the latter releases the token.
    pub fn complete(&mut self, token: &FrameToken, completion: FrameCompletion) -> Option<u64> {
        match completion {
            FrameCompletion::Rendered => self.pending.get(token).copied(),
            FrameCompletion::Presented => self.pending.remove(token),
        }
    }
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
    use super::{FrameCompletion, FrameLedger, RecordingRenderer, Renderer, SceneSnapshot};
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

    #[test]
    fn presentation_completion_releases_only_submitted_targets() {
        let output = OutputId::new("test-output").expect("valid output ID");
        let mut ledger = FrameLedger::default();
        let token = ledger.acquire(&output);
        assert!(ledger.submit(token.clone(), 9));
        assert_eq!(ledger.complete(&token, FrameCompletion::Rendered), Some(9));
        assert_eq!(ledger.complete(&token, FrameCompletion::Presented), Some(9));
        assert_eq!(ledger.complete(&token, FrameCompletion::Presented), None);
    }
}

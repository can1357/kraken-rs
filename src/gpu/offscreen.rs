use std::path::Path;

use anyhow::{Context, Result, bail};
use image::{ImageBuffer, Rgba};
use slab_kernel::{dispatch::Event, flatten::Frame};

use crate::{app::state::AppState, gpu::slab::SlabRenderer, ui::slab::SlabDispatch};

/// Renders deterministic application frames to PNG without creating a window.
pub(crate) struct OffscreenRenderer {
    slab: SlabRenderer,
}

impl OffscreenRenderer {
    /// Creates a headless renderer on the first compatible native adapter.
    pub(crate) async fn new() -> Result<Self> {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
                apply_limit_buckets: false,
            })
            .await
            .context("request a headless GPU adapter")?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("kraken offscreen device"),
                ..Default::default()
            })
            .await
            .context("request a headless GPU device")?;
        Ok(Self {
            slab: SlabRenderer::new(device, queue)?,
        })
    }

    /// Solves the Slab document and writes its tightly packed pixels to a PNG.
    pub(crate) fn render_png(&mut self, state: &AppState, output: &Path) -> Result<()> {
        let width = state.width.max(1);
        let height = state.height.max(1);
        let pixels = self.slab.render_pixels(state, width, height)?;
        let Some(image): Option<ImageBuffer<Rgba<u8>, Vec<u8>>> =
            ImageBuffer::from_raw(width, height, pixels)
        else {
            bail!("GPU returned an invalid screenshot byte count");
        };
        if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create screenshot directory {}", parent.display()))?;
        }
        image
            .save(output)
            .with_context(|| format!("write screenshot {}", output.display()))?;
        Ok(())
    }

    /// Routes one host event through the shared terminal-then-root routing
    /// used by the windowed path.
    pub(crate) fn dispatch(&mut self, state: &mut AppState, event: &Event) -> SlabDispatch {
        self.slab.route_event(state, event).0
    }

    /// Solves the document for `state` and returns the flattened semantic frame.
    pub(crate) fn semantic_frame(&mut self, state: &AppState) -> Frame {
        self.slab.semantic_frame(state)
    }

    /// Returns the scene-string pool backing role, label, and description
    /// references; index zero is the absent sentinel.
    pub(crate) fn scene_strings(&self) -> &[String] {
        self.slab.scene_strings()
    }

    /// Returns the authored key path for a scene node, empty when unkeyed.
    pub(crate) fn node_key(&self, node: u32) -> String {
        self.slab.node_key(node)
    }
}

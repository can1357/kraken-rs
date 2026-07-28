use std::path::Path;

use anyhow::{Context, Result, bail};
use image::{ImageBuffer, Rgba};
use slab_drive::PumpResponse;

use crate::{app::state::AppState, gpu::slab::SlabRenderer};

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

    /// Applies one Slab Drive Protocol request to the retained document.
    pub(crate) fn drive_request(&mut self, state: &mut AppState, line: &str) -> PumpResponse {
        self.slab.drive_request(state, line)
    }
}

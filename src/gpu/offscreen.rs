use std::{path::Path, sync::mpsc};

use anyhow::{Context, Result, bail};
use image::{ImageBuffer, Rgba};
use num_traits::ToPrimitive;

use crate::{
    gpu::renderer::Renderer,
    ui::{Color, Scene},
};

/// Renders deterministic application frames to PNG without creating a window.
pub(crate) struct OffscreenRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    format: wgpu::TextureFormat,
}

impl OffscreenRenderer {
    /// Creates a headless renderer on the first compatible native adapter.
    pub(crate) async fn new() -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
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
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let renderer = Renderer::new(&device, &queue, format);
        Ok(Self {
            device,
            queue,
            renderer,
            format,
        })
    }

    /// Draws one scene and writes its tightly packed pixels to a PNG.
    pub(crate) fn render_png(&mut self, scene: &Scene, clear: Color, output: &Path) -> Result<()> {
        let width = scene.width.round().to_u32().unwrap_or(1).max(1);
        let height = scene.height.round().to_u32().unwrap_or(1).max(1);
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen screenshot"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bytes_per_pixel = 4_u32;
        let unpadded_row = width.saturating_mul(bytes_per_pixel);
        let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_row = unpadded_row.div_ceil(alignment).saturating_mul(alignment);
        let output_size = u64::from(padded_row).saturating_mul(u64::from(height));
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot readback"),
            size: output_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("offscreen frame encoder"),
            });
        self.renderer
            .draw(&self.device, &self.queue, &mut encoder, &view, scene, clear)?;
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let submission = self.queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        let (sender, receiver) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .context("wait for screenshot readback")?;
        receiver
            .recv()
            .context("receive screenshot map result")?
            .context("map screenshot buffer")?;

        let mapped = slice
            .get_mapped_range()
            .context("read mapped screenshot buffer")?;
        let tight_row = unpadded_row.to_usize().unwrap_or(0);
        let source_row = padded_row.to_usize().unwrap_or(0);
        let capacity = tight_row.saturating_mul(height.to_usize().unwrap_or(0));
        let mut pixels = Vec::with_capacity(capacity);
        for row in mapped
            .chunks(source_row)
            .take(height.to_usize().unwrap_or(0))
        {
            pixels.extend_from_slice(&row[..tight_row]);
        }
        drop(mapped);
        readback.unmap();

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
}

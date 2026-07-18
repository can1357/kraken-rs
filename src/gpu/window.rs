use std::sync::Arc;

use anyhow::{Context, Result};
use winit::{event_loop::ActiveEventLoop, window::Window};

use crate::{
    gpu::renderer::Renderer,
    ui::{Color, Scene},
};

/// Owns a winit surface and submits custom UI frames to it.
pub(crate) struct WindowRenderer {
    instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    window: Arc<Window>,
}

impl WindowRenderer {
    /// Creates a Metal/Vulkan/DX12 surface for the native window.
    pub(crate) async fn new(window: Arc<Window>, event_loop: &ActiveEventLoop) -> Result<Self> {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(event_loop.owned_display_handle()),
        ));
        let surface = instance
            .create_surface(window.clone())
            .context("create native wgpu surface")?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                apply_limit_buckets: false,
            })
            .await
            .context("request surface-compatible GPU adapter")?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("kraken window device"),
                ..Default::default()
            })
            .await
            .context("request window GPU device")?;
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| capabilities.formats.first().copied())
            .context("surface exposes no texture formats")?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: capabilities
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Opaque),
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        let renderer = Renderer::new(&device, &queue, format);
        Ok(Self {
            instance,
            device,
            queue,
            surface,
            config,
            renderer,
            window,
        })
    }

    /// Returns the native window used for redraw and cursor updates.
    pub(crate) fn window(&self) -> &Window {
        &self.window
    }

    /// Reconfigures the swap chain after a native resize.
    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    /// Draws and presents one frame, repairing recoverable surface changes inline.
    pub(crate) fn render(&mut self, scene: &Scene, clear: Color) -> Result<()> {
        let mut repaired = false;
        let mut reconfigure_after_present = false;
        let frame = loop {
            match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame) => break frame,
                wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                    reconfigure_after_present = true;
                    break frame;
                }
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    return Ok(());
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    if repaired {
                        return Ok(());
                    }
                    self.surface.configure(&self.device, &self.config);
                    repaired = true;
                }
                wgpu::CurrentSurfaceTexture::Lost => {
                    if repaired {
                        return Ok(());
                    }
                    self.surface = self
                        .instance
                        .create_surface(self.window.clone())
                        .context("recreate lost surface")?;
                    self.surface.configure(&self.device, &self.config);
                    repaired = true;
                }
                wgpu::CurrentSurfaceTexture::Validation => {
                    anyhow::bail!("wgpu surface validation failed");
                }
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("kraken window frame"),
            });
        self.renderer
            .draw(&self.device, &self.queue, &mut encoder, &view, scene, clear)?;
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        if reconfigure_after_present {
            self.surface.configure(&self.device, &self.config);
        }
        Ok(())
    }
}

use std::sync::Arc;

use anyhow::{Context, Result};
use slab_kernel::dispatch::{self, Effects, Event};
use winit::{
    dpi::{LogicalPosition, LogicalSize},
    event_loop::ActiveEventLoop,
    window::{CursorIcon, Window},
};

use crate::{app::state::AppState, gpu::slab::SlabRenderer, ui::slab::SlabDispatch};

/// Owns a winit surface and submits custom UI frames to it.
pub(crate) struct WindowRenderer {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    slab: SlabRenderer,
    window: Arc<Window>,
}

impl WindowRenderer {
    /// Creates a Metal/Vulkan/DX12 surface for the native window.
    pub(crate) async fn new(window: Arc<Window>, _event_loop: &ActiveEventLoop) -> Result<Self> {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
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
            .find(|format| !format.is_srgb())
            .or_else(|| capabilities.formats.first().copied())
            .context("surface exposes no texture formats")?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
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
        };
        surface.configure(&device, &config);
        slab_native::surface::enable_transactional_presents(&surface);
        let slab = SlabRenderer::new(device, queue)?;
        Ok(Self {
            surface,
            config,
            slab,
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
        self.surface.configure(self.slab.device(), &self.config);
    }

    /// Routes one host event to the terminal hole first, then the root document.
    pub(crate) fn dispatch(&mut self, state: &mut AppState, event: &Event) -> SlabDispatch {
        let outcome = self.dispatch_without_redraw(state, event);
        if outcome.effects.repaint {
            self.window.request_redraw();
        }
        outcome
    }

    /// [`Self::dispatch`] without queueing a repaint redraw.
    ///
    /// The resize path presents synchronously inside the resize transaction
    /// right after dispatching; queueing a second render there starves the
    /// transactional drawable pool and stalls live resize in `nextDrawable`.
    pub(crate) fn dispatch_without_redraw(
        &mut self,
        state: &mut AppState,
        event: &Event,
    ) -> SlabDispatch {
        let (outcome, terminal_owned) = self.slab.route_event(state, event);
        self.apply_effects(&outcome.effects);
        if terminal_owned && state.terminal_accepts_input() {
            self.window.set_ime_allowed(true);
        }
        outcome
    }

    /// Returns clipboard text from the routed hole or focused root editor.
    pub(crate) fn take_copy_text(&mut self) -> Option<String> {
        self.slab
            .take_terminal_copy()
            .or_else(|| self.slab.selected_text())
    }

    fn apply_effects(&self, effects: &Effects) {
        let cursor = match effects.cursor {
            dispatch::CUR_POINTER => CursorIcon::Pointer,
            dispatch::CUR_TEXT => CursorIcon::Text,
            dispatch::CUR_COL_RESIZE => CursorIcon::ColResize,
            dispatch::CUR_ROW_RESIZE => CursorIcon::RowResize,
            _ => CursorIcon::Default,
        };
        self.window.set_cursor(cursor);
        let focused = effects.focus != slab_kernel::slir::NONE;
        self.window.set_ime_allowed(focused && effects.has_ime);
        if effects.has_ime {
            self.window.set_ime_cursor_area(
                LogicalPosition::new(effects.ime_x, effects.ime_y),
                LogicalSize::new(effects.ime_w.max(1.0), effects.ime_h.max(1.0)),
            );
        }
    }

    /// Solves the Slab UI and presents one frame, repairing surface changes
    /// inline; unpresentable frames schedule their own retry.
    pub(crate) fn render(&mut self, state: &AppState) {
        let mut repaired = false;
        let frame = loop {
            match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => break frame,
                // The drawable pool is exhausted; retry on the next display
                // cycle instead of dropping the frame.
                wgpu::CurrentSurfaceTexture::Timeout => {
                    self.window.request_redraw();
                    return;
                }
                wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                    if repaired {
                        // Still settling (e.g. the tail of a live resize):
                        // schedule a retry so the frame is never silently
                        // dropped, which would leave stale layout onscreen
                        // until the next input event.
                        self.window.request_redraw();
                        return;
                    }
                    self.surface.configure(self.slab.device(), &self.config);
                    repaired = true;
                }
                // Occluded windows repaint via `WindowEvent::Occluded(false)`;
                // retrying here would spin while hidden. Validation failures
                // are programming errors a retry cannot fix.
                _ => return,
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.slab.render_surface(
            state,
            &view,
            self.config.format,
            self.config.width,
            self.config.height,
            self.window.scale_factor(),
        );
        self.window.pre_present_notify();
        self.slab.queue().present(frame);
        if self.slab.needs_frame() {
            self.window.request_redraw();
        }
    }
}


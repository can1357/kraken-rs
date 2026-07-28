use anyhow::{Result, bail};
use slab_kernel::{
    dispatch::{
        E_BLUR, E_COMPOSITION_END, E_COMPOSITION_START, E_COMPOSITION_UPDATE, E_COPY, E_CUT,
        E_KEY_DOWN, E_PASTE, E_POINTER_DOWN, E_POINTER_MOVE, E_POINTER_UP, E_TEXT, E_WHEEL,
        Effects, Event,
    },
    flatten::Frame,
    frame::{self as kframe, HoleRect},
};
use slab_native::{RegisteredFont, holes::HoleContent, renderer::LayerInput};

use crate::{
    app::state::AppState,
    term::TerminalHole,
    ui::{
        action::UiAction,
        slab::{SlabDispatch, SlabDocument, generated},
    },
};

/// Renders the application-owned Slab document through the shared native kernel.
pub(crate) struct SlabRenderer {
    document: SlabDocument,
    renderer: slab_native::renderer::Renderer,
    frame: Frame,
    doc_id: usize,
    terminal_hole: TerminalHole,
    terminal_doc_id: usize,
    terminal_frame: Frame,
    terminal_rect: Option<HoleRect>,
    terminal_capture: bool,
}

/// UI font faces carried over from the pre-slab renderer: Instrument Sans for
/// chrome text and the Nerd Font mono for code and codicon glyphs. Without
/// these the kernel falls back to Slab's bundled Inter/JetBrains faces, which
/// lack the codicon PUA glyphs.
const APP_FONTS: [(&str, &[u8]); 2] = [
    (
        "Instrument Sans",
        include_bytes!("../../assets/fonts/InstrumentSans.ttf"),
    ),
    (
        "JetBrainsMono Nerd Font Mono",
        include_bytes!("../../assets/fonts/JetBrainsMonoNerdFontMono-Regular.ttf"),
    ),
];

/// Registers the application faces in the document font table and returns the
/// byte-backed faces the GPU rasterizer must load for them.
fn register_app_fonts(inst: &mut kframe::Instance) -> Vec<RegisteredFont> {
    APP_FONTS
        .iter()
        .map(|(family, bytes)| {
            let metrics = slab_fonts::parse_metrics(bytes).expect("bundled app font metrics");
            kframe::inst_font_register(
                inst,
                family,
                u32::from(metrics.weight),
                u32::from(metrics.upem),
                i32::from(metrics.ascent),
                i32::from(metrics.descent),
                i32::from(metrics.line_gap),
                u32::from(metrics.default_advance),
                &metrics.cps,
                &metrics.gids,
                &metrics.advances,
            );
            RegisteredFont {
                name: (*family).to_owned(),
                weight: u32::from(metrics.weight),
                bytes: bytes.to_vec(),
            }
        })
        .collect()
}

impl SlabRenderer {
    /// Decodes the macro-generated SLIR and registers the byte-backed app faces.
    pub(crate) fn new(device: wgpu::Device, queue: wgpu::Queue) -> Result<Self> {
        let mut document = SlabDocument::new(generated::Doc::new());
        let app_fonts = register_app_fonts(&mut document.doc.inst);
        let terminal_hole = TerminalHole::new();
        let mut renderer = slab_native::renderer::Renderer::new(device, queue);
        let doc_id = renderer.register_doc(&document.doc.inst.doc, &document.doc.imgs, &app_fonts);
        let terminal_doc_id = renderer.register_doc(
            &terminal_hole.instance().doc,
            &[],
            terminal_hole.registered_fonts(),
        );
        Ok(Self {
            document,
            renderer,
            doc_id,
            frame: slab_kernel::flatten::frame_new(),
            terminal_hole,
            terminal_doc_id,
            terminal_frame: slab_kernel::flatten::frame_new(),
            terminal_rect: None,
            terminal_capture: false,
        })
    }

    /// Returns the device that owns the surface and Slab render resources.
    pub(crate) fn device(&self) -> &wgpu::Device {
        &self.renderer.device
    }

    /// Returns the queue used to submit and present Slab frames.
    pub(crate) fn queue(&self) -> &wgpu::Queue {
        &self.renderer.queue
    }

    /// Routes a host event into the mounted terminal when its viewport or
    /// keyboard focus owns the event. Pointer coordinates become hole-local.
    pub(crate) fn dispatch_terminal_event(
        &mut self,
        state: &AppState,
        event: &Event,
    ) -> Option<Effects> {
        let pointer = matches!(
            event.etype,
            E_POINTER_MOVE | E_POINTER_DOWN | E_POINTER_UP | E_WHEEL
        );
        let terminal_input = matches!(
            event.etype,
            E_KEY_DOWN
                | E_TEXT
                | E_PASTE
                | E_COPY
                | E_CUT
                | E_COMPOSITION_START
                | E_COMPOSITION_UPDATE
                | E_COMPOSITION_END
                | E_BLUR
        );
        let rect = self.terminal_rect.clone()?;
        let inside = event.x >= rect.x
            && event.x < rect.x + rect.w
            && event.y >= rect.y
            && event.y < rect.y + rect.h;
        if event.etype == E_POINTER_DOWN && inside {
            self.terminal_capture = true;
        }
        let routed = if pointer {
            inside || self.terminal_capture
        } else {
            terminal_input && state.terminal_accepts_input()
        };
        if !routed {
            return None;
        }
        let mut local = event.clone();
        if pointer {
            local.x -= rect.x;
            local.y -= rect.y;
        }
        let effects = self.terminal_hole.dispatch(&local);
        if event.etype == E_POINTER_UP {
            self.terminal_capture = false;
        }
        Some(effects)
    }

    pub(crate) fn take_terminal_copy(&mut self) -> Option<String> {
        self.terminal_hole.take_copy_text()
    }

    pub(crate) fn needs_frame(&self) -> bool {
        let root = &self.document.doc.inst;
        !root.solved || root.dirty || root.ms.active || self.terminal_hole.needs_frame()
    }

    /// Dispatches an event through the generated root document.
    pub(crate) fn dispatch(&mut self, state: &mut AppState, event: &Event) -> SlabDispatch {
        self.document.dispatch(state, event)
    }

    /// Routes one host event to the terminal hole first, then the root
    /// document — the shared path behind both the windowed and offscreen
    /// front ends. Returns the dispatch outcome and whether the terminal
    /// hole consumed the event, so windowed callers can keep IME enabled
    /// while the terminal accepts input.
    pub(crate) fn route_event(
        &mut self,
        state: &mut AppState,
        event: &Event,
    ) -> (SlabDispatch, bool) {
        if let Some(effects) = self.dispatch_terminal_event(state, event) {
            if event.etype == E_POINTER_DOWN {
                state.dispatch(UiAction::FocusTerminal);
            }
            if event.etype == E_BLUR {
                // Blur reaches both surfaces; a repaint request from either
                // must survive the handoff to the root document.
                let mut outcome = self.dispatch(state, event);
                outcome.effects.repaint = outcome.effects.repaint || effects.repaint;
                return (outcome, false);
            }
            return (
                SlabDispatch {
                    effects,
                    host_commands: Vec::new(),
                },
                true,
            );
        }
        if event.etype == E_POINTER_DOWN {
            state.terminal_focused = false;
        }
        (self.dispatch(state, event), false)
    }

    /// Solves the document for `state` and returns the flattened frame whose
    /// semantic scene automation inspects.
    pub(crate) fn semantic_frame(&mut self, state: &AppState) -> Frame {
        self.document.frame(state)
    }

    /// Returns the per-instance scene-string pool backing role, label, and
    /// description references; index zero is the absent sentinel.
    pub(crate) fn scene_strings(&self) -> &[String] {
        &self.document.doc.inst.st.scene_strs
    }

    /// Returns the authored key path for a scene node, empty when unkeyed.
    pub(crate) fn node_key(&self, node: u32) -> String {
        let inst = &self.document.doc.inst;
        slab_kernel::scene::key_of(&inst.doc, &inst.st.lists, node)
    }

    /// Returns selected text from the focused root-document editor.
    pub(crate) fn selected_text(&self) -> Option<String> {
        self.document.selected_text()
    }

    fn prepare_frames(&mut self, state: &AppState) {
        self.terminal_hole.sync(
            state.terminal.as_ref(),
            f64::from(state.settings.terminal_font_size),
            state.terminal_accepts_input(),
        );
        let natural = self.terminal_hole.natural();
        kframe::inst_set_hole_size(
            &mut self.document.doc.inst,
            generated::HOLE_TERMINAL,
            natural.0,
            natural.1,
        );
        self.document.update_frame(state, &mut self.frame);
        self.terminal_rect = kframe::inst_holes_retained(&self.document.doc.inst)
            .into_iter()
            .find(|rect| rect.hole == generated::HOLE_TERMINAL);
        if let Some(rect) = self.terminal_rect.as_ref() {
            self.terminal_hole.resize(rect.w, rect.h, true, false);
            self.terminal_hole.update_frame(&mut self.terminal_frame);
        } else {
            self.terminal_capture = false;
            self.terminal_hole.unmount();
        }
    }

    fn build_frame(
        &mut self,
        scale: f64,
        width: u32,
        height: u32,
    ) -> slab_native::renderer::FrameBuild {
        let root = LayerInput {
            doc_id: self.doc_id,
            inst: &self.document.doc.inst,
            frame: &self.frame,
            ox: 0.0,
            oy: 0.0,
            clip: None,
        };
        if let Some(rect) = self.terminal_rect.as_ref() {
            let terminal = LayerInput {
                doc_id: self.terminal_doc_id,
                inst: self.terminal_hole.instance(),
                frame: &self.terminal_frame,
                ox: rect.x,
                oy: rect.y,
                clip: Some((rect.x, rect.y, rect.w, rect.h, 4.0)),
            };
            self.renderer.build(&[root, terminal], scale, width, height)
        } else {
            self.renderer.build(&[root], scale, width, height)
        }
    }

    /// Solves current application state and blits it into a surface view.
    pub(crate) fn render_surface(
        &mut self,
        state: &AppState,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        scale: f64,
    ) {
        self.prepare_frames(state);
        let build = self.build_frame(scale, width, height);
        self.renderer.render(
            build,
            Some((view, format)),
            wgpu::Color {
                r: 7.0 / 255.0,
                g: 7.0 / 255.0,
                b: 7.0 / 255.0,
                a: 1.0,
            },
        );
    }

    /// Solves current application state and reads back tightly packed RGBA pixels.
    pub(crate) fn render_pixels(
        &mut self,
        state: &AppState,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>> {
        self.prepare_frames(state);
        let build = self.build_frame(1.0, width, height);
        self.renderer.render(
            build,
            None,
            wgpu::Color {
                r: 7.0 / 255.0,
                g: 7.0 / 255.0,
                b: 7.0 / 255.0,
                a: 1.0,
            },
        );
        let Some((rendered_width, rendered_height, pixels)) = self.renderer.read_pixels() else {
            bail!("Slab renderer produced no readable frame");
        };
        if rendered_width != width || rendered_height != height {
            bail!("Slab renderer returned {rendered_width}x{rendered_height} for {width}x{height}");
        }
        Ok(pixels)
    }
}

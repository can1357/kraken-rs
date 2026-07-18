use crate::ui::{
    action::{CursorHint, HitRegion, UiAction},
    geometry::{Color, Rect, px},
    icons,
};

/// Number of painter layers supported by the batched renderer.
pub(crate) const LAYER_COUNT: usize = 5;

/// A rounded rectangle rendered by the SDF fragment pipeline.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RoundedRect {
    pub(crate) rect: Rect,
    pub(crate) clip: Rect,
    pub(crate) fill: Color,
    pub(crate) border: Color,
    pub(crate) radius: f32,
    pub(crate) border_width: f32,
    /// Half-width of the edge falloff in pixels; `0.0` renders a crisp edge.
    /// Soft edges are used for drop shadows under elevated surfaces.
    pub(crate) softness: f32,
    /// When set, the fill is composited from a blurred backdrop by the frost pipeline.
    pub(crate) frost: bool,
}

/// A colored triangle vertex used for lines and graph curves.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MeshVertex {
    pub(crate) position: [f32; 2],
    pub(crate) color: Color,
    pub(crate) clip: Rect,
}

/// A clipped image from the renderer-managed avatar atlas.
#[derive(Clone, Debug)]
pub(crate) struct ImageQuad {
    pub(crate) rect: Rect,
    pub(crate) clip: Rect,
    pub(crate) key: String,
}

/// Font family intent for one text run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum FontFace {
    /// Proportional application copy at regular weight.
    Sans,
    /// Proportional copy at medium weight — buttons, labels, active states.
    SansMedium,
    /// Proportional copy at semibold weight — titles and dialog headers.
    SansBold,
    /// Private-use icon glyphs from the embedded Nerd Font face.
    Icons,
    /// Monospaced code and metadata.
    Monospace,
    /// PTY output rendered with terminal-specific metrics.
    Terminal,
}

/// One clipped glyphon text area.
#[derive(Clone, Debug)]
pub(crate) struct TextSpec {
    pub(crate) text: String,
    pub(crate) origin: [f32; 2],
    pub(crate) bounds: Rect,
    pub(crate) color: Color,
    pub(crate) size: f32,
    pub(crate) line_height: f32,
    pub(crate) face: FontFace,
}

/// Painter data retained for one layer of an immediate frame.
#[derive(Debug, Default)]
pub(crate) struct SceneLayer {
    pub(crate) rectangles: Vec<RoundedRect>,
    pub(crate) mesh: Vec<MeshVertex>,
    pub(crate) images: Vec<ImageQuad>,
    pub(crate) text: Vec<TextSpec>,
}

/// A complete immediate-mode frame with geometry, text, and semantic hit targets.
#[derive(Debug)]
pub(crate) struct Scene {
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) layers: [SceneLayer; LAYER_COUNT],
    pub(crate) hits: Vec<HitRegion>,
}

impl Scene {
    /// Creates an empty scene for the requested physical extent.
    pub(crate) fn new(width: u32, height: u32) -> Self {
        Self {
            width: px(width),
            height: px(height),
            layers: std::array::from_fn(|_| SceneLayer::default()),
            hits: Vec::with_capacity(256),
        }
    }

    /// Returns the full-frame clipping rectangle.
    pub(crate) fn viewport(&self) -> Rect {
        Rect::new(0.0, 0.0, self.width, self.height)
    }

    /// Adds a rounded, optionally bordered rectangle.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rounded_rect(
        &mut self,
        layer: usize,
        rect: Rect,
        clip: Rect,
        fill: Color,
        border: Color,
        radius: f32,
        border_width: f32,
    ) {
        let Some(rect) = rect.clipped(clip) else {
            return;
        };
        self.layers[layer.min(LAYER_COUNT - 1)]
            .rectangles
            .push(RoundedRect {
                rect,
                clip,
                fill,
                border,
                radius,
                border_width,
                softness: 0.0,
                frost: false,
            });
    }

    /// Adds a rounded popup surface whose fill is composited from a blurred backdrop.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn frosted_rounded_rect(
        &mut self,
        layer: usize,
        rect: Rect,
        clip: Rect,
        fill: Color,
        border: Color,
        radius: f32,
        border_width: f32,
    ) {
        let Some(rect) = rect.clipped(clip) else {
            return;
        };
        self.layers[layer.min(LAYER_COUNT - 1)]
            .rectangles
            .push(RoundedRect {
                rect,
                clip,
                fill,
                border,
                radius,
                border_width,
                softness: 0.0,
                frost: true,
            });
    }

    /// Draws a soft drop shadow beneath an elevated surface.
    ///
    /// Emit this before the surface itself: an ambient plate plus a wider key
    /// plate approximate a blurred shadow without a separate blur pass.
    pub(crate) fn shadow(&mut self, layer: usize, rect: Rect, clip: Rect, radius: f32) {
        // (y offset, falloff half-width, alpha)
        const PLATES: [(f32, f32, f32); 2] = [(2.0, 6.0, 0.30), (10.0, 28.0, 0.38)];
        for (drop, softness, alpha) in PLATES {
            let plate = Rect::new(rect.x, rect.y + drop, rect.width, rect.height);
            let ink = Color::rgba(0, 0, 0, 255).with_alpha(alpha);
            self.layers[layer.min(LAYER_COUNT - 1)]
                .rectangles
                .push(RoundedRect {
                    rect: plate,
                    clip,
                    fill: ink,
                    border: ink,
                    radius: radius + softness * 0.5,
                    border_width: 0.0,
                    softness,
                    frost: false,
                });
        }
    }

    /// Adds a square-cornered fill.
    pub(crate) fn rect(&mut self, layer: usize, rect: Rect, clip: Rect, fill: Color) {
        self.rounded_rect(layer, rect, clip, fill, fill, 0.0, 0.0);
    }

    /// Adds a quad whose fill interpolates horizontally between two colors.
    ///
    /// The geometry is emitted unclipped with per-vertex clip rects so the GPU
    /// scissors it without skewing the gradient ramp.
    pub(crate) fn gradient_rect_h(
        &mut self,
        layer: usize,
        rect: Rect,
        clip: Rect,
        left: Color,
        right: Color,
    ) {
        if rect.intersection(clip).is_none() {
            return;
        }
        let top_left = [rect.x, rect.y];
        let bottom_left = [rect.x, rect.bottom()];
        let top_right = [rect.right(), rect.y];
        let bottom_right = [rect.right(), rect.bottom()];
        let vertex = |position: [f32; 2], color: Color| MeshVertex {
            position,
            color,
            clip,
        };
        self.layers[layer.min(LAYER_COUNT - 1)]
            .mesh
            .extend_from_slice(&[
                vertex(top_left, left),
                vertex(bottom_left, left),
                vertex(top_right, right),
                vertex(top_right, right),
                vertex(bottom_left, left),
                vertex(bottom_right, right),
            ]);
    }

    /// Adds a clipped line quad with stable thickness.
    pub(crate) fn line(
        &mut self,
        layer: usize,
        from: [f32; 2],
        to: [f32; 2],
        width: f32,
        color: Color,
        clip: Rect,
    ) {
        if width <= 0.0 {
            return;
        }
        let dx = to[0] - from[0];
        let dy = to[1] - from[1];
        let length = dx.hypot(dy);
        if length <= f32::EPSILON {
            return;
        }
        let half = width * 0.5;
        let normal = [-dy / length * half, dx / length * half];
        let a = [from[0] + normal[0], from[1] + normal[1]];
        let b = [from[0] - normal[0], from[1] - normal[1]];
        let c = [to[0] + normal[0], to[1] + normal[1]];
        let d = [to[0] - normal[0], to[1] - normal[1]];
        self.triangle(layer, a, b, c, color, clip);
        self.triangle(layer, c, b, d, color, clip);
    }

    /// Draws a right-angle graph connector with a small rounded inside corner.
    pub(crate) fn rounded_elbow(
        &mut self,
        layer: usize,
        from: [f32; 2],
        to: [f32; 2],
        radius: f32,
        width: f32,
        color: Color,
        clip: Rect,
    ) {
        let direction = (to[0] - from[0]).signum();
        if direction == 0.0 || to[1] <= from[1] {
            self.line(layer, from, to, width, color, clip);
            return;
        }
        let radius = radius
            .min((to[1] - from[1]) * 0.5)
            .min((to[0] - from[0]).abs());
        let center = [from[0] + direction * radius, to[1] - radius];
        self.line(layer, from, [from[0], center[1]], width, color, clip);
        let start_angle = if direction > 0.0 {
            std::f32::consts::PI
        } else {
            0.0
        };
        let mut previous = [from[0], center[1]];
        for step in 1..=4 {
            let t = step as f32 / 4.0;
            let angle = start_angle + (std::f32::consts::FRAC_PI_2 - start_angle) * t;
            let point = [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ];
            self.line(layer, previous, point, width, color, clip);
            previous = point;
        }
        self.line(layer, previous, to, width, color, clip);
    }

    /// Adds one triangle to a mesh layer.
    fn triangle(
        &mut self,
        layer: usize,
        a: [f32; 2],
        b: [f32; 2],
        c: [f32; 2],
        color: Color,
        clip: Rect,
    ) {
        self.layers[layer.min(LAYER_COUNT - 1)]
            .mesh
            .extend_from_slice(&[
                MeshVertex {
                    position: a,
                    color,
                    clip,
                },
                MeshVertex {
                    position: b,
                    color,
                    clip,
                },
                MeshVertex {
                    position: c,
                    color,
                    clip,
                },
            ]);
    }

    /// Adds an image-quad resolved from the renderer-managed atlas.
    pub(crate) fn image(&mut self, layer: usize, rect: Rect, clip: Rect, key: impl Into<String>) {
        let Some(rect) = rect.clipped(clip) else {
            return;
        };
        self.layers[layer.min(LAYER_COUNT - 1)]
            .images
            .push(ImageQuad {
                rect,
                clip,
                key: key.into(),
            });
    }

    /// Adds clipped text with explicit font metrics.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn text(
        &mut self,
        text: impl Into<String>,
        origin: [f32; 2],
        bounds: Rect,
        color: Color,
        size: f32,
        line_height: f32,
        face: FontFace,
    ) {
        if bounds.width <= 0.0 || bounds.height <= 0.0 {
            return;
        }
        let text = text.into();
        let face = if icons::is_icon_only(&text) {
            FontFace::Icons
        } else {
            face
        };
        // Text follows the highest existing containing paint layer.  This keeps
        // legacy view builders layer-correct while their geometry remains the
        // source of stacking intent.
        let layer = self
            .layers
            .iter()
            .rposition(|layer| {
                layer
                    .rectangles
                    .iter()
                    .any(|rect| rect.rect.contains(origin))
            })
            .unwrap_or(0);
        self.layers[layer].text.push(TextSpec {
            text,
            origin,
            bounds,
            color,
            size,
            line_height,
            face,
        });
    }

    /// Adds a semantic click target and optional tooltip.
    pub(crate) fn hit(
        &mut self,
        rect: Rect,
        action: UiAction,
        cursor: CursorHint,
        tooltip: Option<&str>,
    ) {
        self.hits.push(HitRegion {
            rect,
            action,
            cursor,
            tooltip: tooltip.map(str::to_owned),
        });
    }

    /// Adds a semantic click target clipped to a scroll viewport.
    ///
    /// Scrollable views use this instead of `hit` so invisible row fragments
    /// cannot receive pointer input outside their viewport.
    pub(crate) fn hit_clipped(
        &mut self,
        rect: Rect,
        clip: Rect,
        action: UiAction,
        cursor: CursorHint,
        tooltip: Option<&str>,
    ) {
        if let Some(rect) = rect.intersection(clip) {
            self.hit(rect, action, cursor, tooltip);
        }
    }

    /// Removes base hit regions covered by an opaque overlay.
    pub(crate) fn mask_hits(&mut self, overlay: Rect) {
        self.hits
            .retain(|hit| hit.rect.intersection(overlay).is_none());
    }

    /// Hides base text that would otherwise bleed through an opaque overlay.
    pub(crate) fn mask_text(&mut self, overlay: Rect) {
        for layer in &mut self.layers {
            layer.text.retain(|text| {
                let point = [text.origin[0], text.origin[1]];
                !overlay.contains(point)
            });
        }
    }
}

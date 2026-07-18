use crate::ui::{
    FontFace, Rect, Scene, TextField, Theme,
    action::{CursorHint, ScrollTarget, UiAction},
    icons,
};

/// Draws a directly manipulable vertical scrollbar for a clipped scroll surface.
///
/// The track is one semantic target; the dispatch derives the clicked content
/// fraction from the pointer. The thumb is emitted last and starts a drag.
pub(crate) fn scrollbar(
    scene: &mut Scene,
    viewport: Rect,
    content_height: f32,
    scroll: f32,
    target: ScrollTarget,
    theme: &Theme,
) {
    if content_height <= viewport.height || viewport.height <= 0.0 {
        return;
    }
    let width = 7.0;
    let track = Rect::new(
        viewport.right() - width - 2.0,
        viewport.y,
        width,
        viewport.height,
    );
    let ratio = viewport.height / content_height;
    let thumb_height = (viewport.height * ratio).max(24.0).min(viewport.height);
    let max_scroll = (content_height - viewport.height).max(1.0);
    let travel = (viewport.height - thumb_height).max(0.0);
    let thumb = Rect::new(
        track.x,
        viewport.y + travel * (scroll / max_scroll).clamp(0.0, 1.0),
        width,
        thumb_height,
    );
    scene.rect(3, track, viewport, theme.input.with_alpha(0.72));
    scene.rect(3, thumb, viewport, theme.border_hard);
    scene.hit_clipped(
        Rect::new(track.x - 2.0, track.y, width + 4.0, track.height),
        viewport,
        UiAction::ScrollbarJump(target),
        CursorHint::Pointer,
        None,
    );
    scene.hit_clipped(
        thumb.inset(-2.0),
        viewport,
        UiAction::BeginScrollbarDrag(target),
        CursorHint::Pointer,
        None,
    );
}

/// Draws a single-line text run and, when the content cannot fit its bounds,
/// registers a passive hover region that reveals the full text in a popup.
///
/// Width estimation: exact for the monospace/terminal faces, conservative for
/// the proportional Sans face (over-triggering only shows the popup early).
pub(crate) fn truncated_text(
    scene: &mut Scene,
    text: &str,
    origin: [f32; 2],
    bounds: Rect,
    clip: Rect,
    color: crate::ui::geometry::Color,
    size: f32,
    line_height: f32,
    face: FontFace,
) {
    let visible = bounds.intersection(clip).unwrap_or(bounds);
    scene.text(text, origin, visible, color, size, line_height, face);
    let per_char = match face {
        FontFace::Sans => size * 0.52,
        FontFace::Icons | FontFace::Monospace | FontFace::Terminal => size * 0.6,
    };
    let estimated = text.chars().count() as f32 * per_char;
    let available = (bounds.right().min(clip.right()) - origin[0]).max(0.0);
    if estimated > available {
        scene.hit_clipped(
            visible,
            clip,
            UiAction::RevealText,
            CursorHint::Default,
            Some(text),
        );
    }
}

/// Draws a standard action button and registers its semantic hit target.
#[allow(clippy::too_many_arguments)]
pub(crate) fn button(
    scene: &mut Scene,
    rect: Rect,
    label: impl Into<String>,
    action: UiAction,
    mouse: [f32; 2],
    theme: &Theme,
    accent: bool,
    enabled: bool,
    tooltip: Option<&str>,
) {
    button_on_layer(
        scene, rect, label, action, mouse, theme, accent, enabled, tooltip, 2,
    );
}

/// Draws an action button above the modal popup surface.
#[allow(clippy::too_many_arguments)]
pub(crate) fn modal_button(
    scene: &mut Scene,
    rect: Rect,
    label: &str,
    action: UiAction,
    mouse: [f32; 2],
    theme: &Theme,
    accent: bool,
    enabled: bool,
    tooltip: Option<&str>,
) {
    button_on_layer(
        scene, rect, label, action, mouse, theme, accent, enabled, tooltip, 4,
    );
}

#[allow(clippy::too_many_arguments)]
fn button_on_layer(
    scene: &mut Scene,
    rect: Rect,
    label: impl Into<String>,
    action: UiAction,
    mouse: [f32; 2],
    theme: &Theme,
    accent: bool,
    enabled: bool,
    tooltip: Option<&str>,
    layer: usize,
) {
    let hovered = rect.contains(mouse) && enabled;
    let viewport = scene.viewport();
    if !enabled {
        // Disabled: transparent fill, default outline, tertiary text.
        scene.rounded_rect(
            layer,
            rect,
            viewport,
            theme.window.with_alpha(0.0),
            theme.border_strong,
            0.0,
            1.0,
        );
    } else if accent {
        // Primary: cyan fill, black text; hover fades in the strong dither.
        scene.rounded_rect(layer, rect, viewport, theme.accent, theme.accent, 0.0, 0.0);
        if hovered {
            scene.dither_rect(
                layer,
                rect,
                viewport,
                theme.accent_hover.with_alpha(0.6),
                crate::ui::scene::Pattern::Checker,
            );
        }
    } else {
        // Secondary: surface-1 + default border; hover surface-2 + strong border.
        scene.rounded_rect(
            layer,
            rect,
            viewport,
            if hovered {
                theme.panel_alt
            } else {
                theme.panel
            },
            if hovered {
                theme.border_hard
            } else {
                theme.border_strong
            },
            0.0,
            1.0,
        );
    }
    let color = if !enabled {
        theme.text_dim
    } else if accent {
        theme.on_accent
    } else if hovered {
        theme.text
    } else {
        theme.text_muted
    };
    scene.text(
        label,
        [rect.x + 9.0, rect.y + (rect.height - 16.0) * 0.5],
        rect.inset(4.0),
        color,
        13.0,
        16.0,
        FontFace::Sans,
    );
    if enabled {
        scene.hit(rect, action, CursorHint::Pointer, tooltip);
    }
}

/// Draws one checkbox row with a real persisted toggle action.
#[allow(clippy::too_many_arguments)]
pub(crate) fn checkbox(
    scene: &mut Scene,
    rect: Rect,
    label: &str,
    checked: bool,
    action: UiAction,
    mouse: [f32; 2],
    theme: &Theme,
) {
    let hovered = rect.contains(mouse);
    if hovered {
        scene.rect(1, rect, scene.viewport(), theme.row_hover.with_alpha(0.55));
    }
    let box_rect = Rect::new(rect.x + 2.0, rect.y + 5.0, 16.0, 16.0);
    scene.rounded_rect(
        2,
        box_rect,
        scene.viewport(),
        if checked { theme.accent } else { theme.input },
        if checked {
            theme.accent
        } else {
            theme.border_strong
        },
        0.0,
        1.0,
    );
    if checked {
        scene.text(
            icons::CHECK,
            [box_rect.x + 2.5, box_rect.y - 0.5],
            box_rect,
            theme.on_accent,
            13.0,
            16.0,
            FontFace::Monospace,
        );
    }
    scene.text(
        label,
        [rect.x + 30.0, rect.y + 5.0],
        Rect::new(rect.x + 28.0, rect.y, rect.width - 28.0, rect.height),
        theme.text,
        14.0,
        19.0,
        FontFace::Sans,
    );
    scene.hit(rect, action, CursorHint::Pointer, None);
}

/// Draws a one-pixel divider line.
pub(crate) fn divider(scene: &mut Scene, rect: Rect, theme: &Theme) {
    scene.rect(1, rect, scene.viewport(), theme.border);
}

/// Draws a compact uppercase section label.
pub(crate) fn section_label(scene: &mut Scene, rect: Rect, label: &str, theme: &Theme) {
    scene.text(
        label,
        [rect.x, rect.y],
        rect,
        theme.text_dim,
        10.0,
        13.0,
        FontFace::Monospace,
    );
}

/// Draws an input surface, its value or placeholder, and when focused the
/// selection band and caret.
#[allow(clippy::too_many_arguments)]
pub(crate) fn text_input(
    scene: &mut Scene,
    rect: Rect,
    field: &TextField,
    placeholder: &str,
    focused: bool,
    action: UiAction,
    mouse: [f32; 2],
    theme: &Theme,
    multiline: bool,
) {
    input_on_layer(
        scene,
        rect,
        field,
        placeholder,
        focused,
        action,
        mouse,
        theme,
        multiline,
        1,
    );
}

/// Draws an input above the modal popup surface.
#[allow(clippy::too_many_arguments)]
pub(crate) fn modal_text_input(
    scene: &mut Scene,
    rect: Rect,
    field: &TextField,
    placeholder: &str,
    focused: bool,
    action: UiAction,
    mouse: [f32; 2],
    theme: &Theme,
    multiline: bool,
) {
    input_on_layer(
        scene,
        rect,
        field,
        placeholder,
        focused,
        action,
        mouse,
        theme,
        multiline,
        4,
    );
}

#[allow(clippy::too_many_arguments)]
fn input_on_layer(
    scene: &mut Scene,
    rect: Rect,
    field: &TextField,
    placeholder: &str,
    focused: bool,
    action: UiAction,
    mouse: [f32; 2],
    theme: &Theme,
    multiline: bool,
    layer: usize,
) {
    scene.rounded_rect(
        layer,
        rect,
        scene.viewport(),
        theme.input,
        if focused {
            theme.accent
        } else {
            theme.border_strong
        },
        0.0,
        1.0,
    );
    let shown = if field.is_empty() {
        placeholder
    } else {
        field.text()
    };
    let line_height = if multiline { 19.0 } else { 17.0 };
    scene.text(
        shown,
        [rect.x + 9.0, rect.y + 7.0],
        rect.inset(7.0),
        if field.is_empty() {
            theme.text_dim
        } else {
            theme.text
        },
        13.0,
        line_height,
        FontFace::Sans,
    );
    if focused {
        caret_overlay(
            scene,
            layer + 2,
            [rect.x + 9.0, rect.y + 7.0],
            rect,
            field,
            7.1,
            line_height,
            theme,
        );
    }
    scene.hit(
        rect,
        action,
        CursorHint::Text,
        Some(if multiline {
            "Edit description"
        } else {
            "Edit text"
        }),
    );
    if rect.contains(mouse) {
        scene.rounded_rect(
            layer + 1,
            rect,
            scene.viewport(),
            theme.window.with_alpha(0.0),
            theme.border_hard,
            0.0,
            1.0,
        );
    }
}

/// Draws the selection band and caret for a focused [`TextField`].
///
/// `origin` is the top-left of the first glyph, `advance` the estimated glyph
/// width for the surface's face, and `line_height` both the caret height and
/// per-line stride. Shared by the input widgets and every custom-drawn field.
#[allow(clippy::too_many_arguments)]
pub(crate) fn caret_overlay(
    scene: &mut Scene,
    layer: usize,
    origin: [f32; 2],
    clip: Rect,
    field: &TextField,
    advance: f32,
    line_height: f32,
    theme: &Theme,
) {
    let caret = field.caret();
    let selection = caret.selection();
    if !selection.is_empty() {
        let mut line_start = 0_usize;
        for (line, text) in field.split('\n').enumerate() {
            let line_chars = text.chars().count();
            let start = selection.start.max(line_start);
            let end = selection.end.min(line_start + line_chars);
            if start < end {
                let band = Rect::new(
                    origin[0] + super::geometry::px(start - line_start) * advance,
                    origin[1] + super::geometry::px(line) * line_height,
                    super::geometry::px(end - start) * advance,
                    line_height,
                );
                if let Some(visible) = band.intersection(clip) {
                    scene.rect(layer, visible, clip, theme.accent.with_alpha(0.3));
                }
            }
            line_start += line_chars + 1;
        }
    }
    let x = origin[0] + super::geometry::px(caret.column) * advance;
    let caret_rect = Rect::new(
        x.min(clip.right() - 3.0),
        origin[1] + super::geometry::px(caret.line) * line_height,
        1.0,
        line_height,
    );
    if let Some(visible) = caret_rect.intersection(clip) {
        scene.rect(layer, visible, clip, theme.accent);
    }
}

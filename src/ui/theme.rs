use crate::ui::geometry::Color;

/// Cyanotype design tokens shared by every view.
///
/// Translated from the Stencil "Cyanotype" system: pure black chassis, hairline
/// borders, cyan as the single interaction prime, purple strictly for
/// live/presence, square geometry everywhere, and dither as a data material.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Theme {
    /// Page chassis — pure black.
    pub(crate) window: Color,
    /// Sunken wells (inputs, code, top chrome) — `--bg-sunken`.
    pub(crate) top_bar: Color,
    /// First elevation — `--surface-1`.
    pub(crate) toolbar: Color,
    /// Primary panel surface — `--surface-1`.
    pub(crate) panel: Color,
    /// Second elevation — `--surface-2`.
    pub(crate) panel_alt: Color,
    /// Third elevation for popovers/menus — `--surface-3`.
    pub(crate) surface_3: Color,
    /// Table row at rest sits on the chassis.
    pub(crate) row: Color,
    /// Row hover — `--surface-1`.
    pub(crate) row_hover: Color,
    /// Selected-row underlay — `--accent-muted` (pair with a cyan checker fill).
    pub(crate) row_selected: Color,
    /// Hairline — `--border-subtle`.
    pub(crate) border: Color,
    /// Component outline — `--border-default`.
    pub(crate) border_strong: Color,
    /// Emphasized outline — `--border-strong`.
    pub(crate) border_hard: Color,
    pub(crate) text: Color,
    pub(crate) text_muted: Color,
    pub(crate) text_dim: Color,
    pub(crate) text_disabled: Color,
    /// Interaction prime — `--accent` cyan. Buttons, focus, selection, links.
    pub(crate) accent: Color,
    /// `--accent-hover`.
    pub(crate) accent_hover: Color,
    /// `--accent-active`.
    pub(crate) accent_active: Color,
    /// `--accent-muted` fill for selected/active wells.
    pub(crate) accent_soft: Color,
    /// Text/icon color on accent fills — pure black.
    pub(crate) on_accent: Color,
    pub(crate) green: Color,
    pub(crate) green_muted: Color,
    pub(crate) orange: Color,
    pub(crate) orange_muted: Color,
    pub(crate) red: Color,
    pub(crate) red_muted: Color,
    /// Drop-target highlight while dragging a ref — olive/yellow like GitKraken.
    pub(crate) yellow: Color,
    pub(crate) yellow_muted: Color,
    /// `--signal` purple: live/presence ONLY (WIP, worktrees, collaborators).
    pub(crate) purple: Color,
    pub(crate) purple_muted: Color,
    /// Input well — `--bg-sunken`.
    pub(crate) input: Color,
    /// Modal scrim only; surfaces never carry drop shadows.
    pub(crate) shadow: Color,
    pub(crate) graph_lanes: [Color; 10],
}

impl Theme {
    /// Returns the locked Cyanotype token set.
    pub(crate) fn high_contrast() -> Self {
        Self {
            window: Color::rgb(0, 0, 0),
            top_bar: Color::rgb(5, 5, 8),
            toolbar: Color::rgb(10, 10, 12),
            panel: Color::rgb(10, 10, 12),
            panel_alt: Color::rgb(18, 18, 22),
            surface_3: Color::rgb(26, 26, 32),
            row: Color::rgb(0, 0, 0),
            row_hover: Color::rgb(10, 10, 12),
            row_selected: Color::rgb(11, 40, 51),
            border: Color::rgb(21, 21, 26),
            border_strong: Color::rgb(42, 42, 53),
            border_hard: Color::rgb(68, 68, 85),
            text: Color::rgb(245, 245, 246),
            text_muted: Color::rgb(163, 163, 172),
            text_dim: Color::rgb(99, 99, 109),
            text_disabled: Color::rgb(61, 61, 69),
            accent: Color::rgb(68, 207, 255),
            accent_hover: Color::rgb(117, 222, 255),
            accent_active: Color::rgb(32, 166, 216),
            accent_soft: Color::rgb(11, 40, 51),
            on_accent: Color::rgb(0, 0, 0),
            green: Color::rgb(74, 222, 128),
            green_muted: Color::rgb(12, 36, 23),
            orange: Color::rgb(245, 176, 74),
            orange_muted: Color::rgb(42, 30, 10),
            red: Color::rgb(244, 100, 74),
            red_muted: Color::rgb(43, 15, 13),
            yellow: Color::rgb(222, 200, 88),
            yellow_muted: Color::rgb(48, 43, 16),
            purple: Color::rgb(168, 106, 244),
            purple_muted: Color::rgb(32, 13, 43),
            input: Color::rgb(5, 5, 8),
            shadow: Color::rgba(0, 0, 0, 180),
            graph_lanes: [
                Color::rgb(50, 158, 174),
                Color::rgb(69, 91, 188),
                Color::rgb(112, 61, 181),
                Color::rgb(157, 43, 165),
                Color::rgb(168, 29, 96),
                Color::rgb(153, 27, 22),
                Color::rgb(196, 81, 62),
                Color::rgb(222, 200, 88),
                Color::rgb(132, 219, 86),
                Color::rgb(78, 205, 152),
            ],
        }
    }
}

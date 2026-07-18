use crate::ui::geometry::Color;

/// Corner radius for chips, badges, checkboxes, and small wells.
pub(crate) const RADIUS_SM: f32 = 3.0;
/// Corner radius for buttons, inputs, tabs, and segmented controls.
pub(crate) const RADIUS_MD: f32 = 4.0;
/// Corner radius for cards, wells, and menus.
pub(crate) const RADIUS_LG: f32 = 5.0;
/// Corner radius for modals and popovers.
pub(crate) const RADIUS_XL: f32 = 6.0;

/// Nocturne design tokens shared by every view.
///
/// A monochrome dark system: true-neutral grays with quiet tonal steps, near-
/// square geometry, and a white interaction prime (dark text on white fills).
/// Color survives only as data: graph lanes, file-status badges, search hits,
/// and the purple live/presence signal.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Theme {
    /// Page chassis — deep slate behind every panel.
    pub(crate) window: Color,
    /// Sunken strip behind the repository tabs.
    pub(crate) top_bar: Color,
    /// Primary card surface.
    pub(crate) panel: Color,
    /// Secondary fill for subheaders, wells, and quiet chips.
    pub(crate) panel_alt: Color,
    /// Elevated surface for popovers and menus (pair with `Scene::shadow`).
    pub(crate) surface_3: Color,
    /// Row hover wash.
    pub(crate) row_hover: Color,
    /// Selected-row fill — soft cobalt.
    pub(crate) row_selected: Color,
    /// Hairline between surfaces.
    pub(crate) border: Color,
    /// Component outline (buttons, inputs).
    pub(crate) border_strong: Color,
    /// Emphasized outline (hover, active wells).
    pub(crate) border_hard: Color,
    pub(crate) text: Color,
    pub(crate) text_muted: Color,
    pub(crate) text_dim: Color,
    pub(crate) text_disabled: Color,
    /// Interaction prime — cobalt. Buttons, focus, selection, links.
    pub(crate) accent: Color,
    /// Lighter cobalt for hovered fills.
    pub(crate) accent_hover: Color,
    /// Deeper cobalt for pressed fills.
    pub(crate) accent_active: Color,
    /// Soft cobalt fill for selected/active wells.
    pub(crate) accent_soft: Color,
    /// Text/icon color on accent fills.
    pub(crate) on_accent: Color,
    pub(crate) green: Color,
    pub(crate) green_muted: Color,
    pub(crate) orange: Color,
    pub(crate) orange_muted: Color,
    pub(crate) red: Color,
    pub(crate) red_muted: Color,
    /// Drop-target highlight while dragging a ref.
    pub(crate) yellow: Color,
    pub(crate) yellow_muted: Color,
    /// Live/presence signal ONLY (WIP, worktrees, collaborators).
    pub(crate) purple: Color,
    pub(crate) purple_muted: Color,
    /// Input well fill — sunken.
    pub(crate) input: Color,
    /// Modal scrim and shadow ink.
    pub(crate) shadow: Color,
    pub(crate) graph_lanes: [Color; 10],
}

impl Theme {
    /// Returns the Nocturne token set.
    pub(crate) fn dark() -> Self {
        Self {
            window: Color::rgb(7, 7, 7),
            top_bar: Color::rgb(4, 4, 4),
            panel: Color::rgb(12, 12, 12),
            panel_alt: Color::rgb(17, 17, 17),
            surface_3: Color::rgb(24, 24, 24),
            row_hover: Color::rgb(17, 17, 17),
            row_selected: Color::rgb(34, 34, 34),
            border: Color::rgb(34, 34, 34),
            border_strong: Color::rgb(48, 48, 48),
            border_hard: Color::rgb(70, 70, 70),
            text: Color::rgb(229, 229, 229),
            text_muted: Color::rgb(163, 163, 163),
            text_dim: Color::rgb(115, 115, 115),
            text_disabled: Color::rgb(82, 82, 82),
            accent: Color::rgb(237, 237, 237),
            accent_hover: Color::rgb(255, 255, 255),
            accent_active: Color::rgb(212, 212, 212),
            accent_soft: Color::rgb(34, 34, 34),
            on_accent: Color::rgb(10, 10, 10),
            green: Color::rgb(76, 183, 130),
            green_muted: Color::rgb(14, 42, 30),
            orange: Color::rgb(247, 165, 80),
            orange_muted: Color::rgb(52, 38, 15),
            red: Color::rgb(235, 87, 87),
            red_muted: Color::rgb(58, 22, 24),
            yellow: Color::rgb(222, 196, 80),
            yellow_muted: Color::rgb(56, 49, 18),
            purple: Color::rgb(168, 120, 245),
            purple_muted: Color::rgb(42, 33, 70),
            input: Color::rgb(5, 5, 5),
            shadow: Color::rgba(0, 0, 0, 150),
            graph_lanes: [
                Color::rgb(64, 186, 205),
                Color::rgb(106, 130, 232),
                Color::rgb(150, 102, 240),
                Color::rgb(196, 90, 222),
                Color::rgb(235, 86, 148),
                Color::rgb(232, 92, 85),
                Color::rgb(240, 130, 54),
                Color::rgb(222, 180, 60),
                Color::rgb(150, 214, 80),
                Color::rgb(66, 214, 160),
            ],
        }
    }
}

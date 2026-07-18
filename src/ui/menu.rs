//! Declarative context-menu model shared by the drawn overlay renderer and
//! the native macOS presenter.
//!
//! State builds a [`MenuSpec`] for the active right-click target; the windowed
//! macOS path hands it to `app::native_menu`, while the drawn fallback renders
//! it through [`layout`], which resolves panel, row, and submenu geometry
//! against the viewport and pointer.

use crate::ui::{Rect, action::UiAction};

/// Vertical extent of one actionable menu row.
const ROW_HEIGHT: f32 = 30.0;
/// Vertical extent of a separator rule.
const SEPARATOR_HEIGHT: f32 = 9.0;
/// Inner padding above and below the entry list.
const PADDING: f32 = 8.0;
/// Extent of the dimmed title line naming the menu target.
const HEADER_HEIGHT: f32 = 28.0;
/// Minimum gap kept between a panel and the viewport edge.
const MARGIN: f32 = 8.0;

/// One context-menu entry: an actionable row, a one-level submenu, or a rule.
#[derive(Clone, Debug)]
pub(crate) enum MenuEntry {
    Item {
        label: String,
        action: UiAction,
        enabled: bool,
    },
    Submenu {
        label: String,
        entries: Vec<(String, UiAction)>,
    },
    Separator,
}

impl MenuEntry {
    /// An enabled actionable row.
    pub(crate) fn item(label: impl Into<String>, action: UiAction) -> Self {
        Self::Item {
            label: label.into(),
            action,
            enabled: true,
        }
    }

    fn height(&self) -> f32 {
        match self {
            Self::Item { .. } | Self::Submenu { .. } => ROW_HEIGHT,
            Self::Separator => SEPARATOR_HEIGHT,
        }
    }
}

/// A complete context menu: a dimmed title naming the target plus entries.
#[derive(Clone, Debug)]
pub(crate) struct MenuSpec {
    pub(crate) title: String,
    pub(crate) entries: Vec<MenuEntry>,
}

/// One resolved drawn row, aligned with the spec entry it came from.
pub(crate) enum MenuRow<'a> {
    Item {
        rect: Rect,
        label: &'a str,
        action: &'a UiAction,
        enabled: bool,
    },
    Parent {
        rect: Rect,
        label: &'a str,
        open: bool,
    },
    Separator {
        rect: Rect,
    },
}

/// Geometry for the expanded submenu of a hovered parent row.
pub(crate) struct SubmenuLayout<'a> {
    pub(crate) panel: Rect,
    pub(crate) rows: Vec<(Rect, &'a str, &'a UiAction)>,
}

/// Fully resolved geometry for one drawn context menu.
pub(crate) struct MenuLayout<'a> {
    pub(crate) panel: Rect,
    pub(crate) title: &'a str,
    pub(crate) rows: Vec<MenuRow<'a>>,
    pub(crate) submenu: Option<SubmenuLayout<'a>>,
}

impl MenuLayout<'_> {
    /// Smallest rectangle covering the panel and any open submenu; used for
    /// outside-click dismissal.
    pub(crate) fn bounds(&self) -> Rect {
        match &self.submenu {
            Some(submenu) => union(self.panel, submenu.panel),
            None => self.panel,
        }
    }
}

/// Resolves panel, row, and submenu geometry for a drawn context menu.
/// The submenu of a parent row expands while the pointer rests on the row or
/// inside the submenu panel itself.
pub(crate) fn layout(
    spec: &MenuSpec,
    anchor: [f32; 2],
    viewport: [f32; 2],
    mouse: [f32; 2],
) -> MenuLayout<'_> {
    let width = panel_width(spec.entries.iter().map(|entry| match entry {
        MenuEntry::Item { label, .. } | MenuEntry::Submenu { label, .. } => label.as_str(),
        MenuEntry::Separator => "",
    }));
    let height =
        HEADER_HEIGHT + 2.0 * PADDING + spec.entries.iter().map(MenuEntry::height).sum::<f32>();
    let panel = Rect::new(
        anchor[0].min(viewport[0] - width - MARGIN).max(MARGIN),
        anchor[1].min(viewport[1] - height - MARGIN).max(MARGIN),
        width,
        height,
    );

    let mut rows = Vec::with_capacity(spec.entries.len());
    let mut submenu = None;
    let mut y = panel.y + HEADER_HEIGHT + PADDING;
    for entry in &spec.entries {
        let rect = Rect::new(panel.x + 6.0, y, panel.width - 12.0, entry.height());
        y += entry.height();
        match entry {
            MenuEntry::Item {
                label,
                action,
                enabled,
            } => rows.push(MenuRow::Item {
                rect,
                label,
                action,
                enabled: *enabled,
            }),
            MenuEntry::Separator => rows.push(MenuRow::Separator { rect }),
            MenuEntry::Submenu { label, entries } => {
                let candidate = submenu_layout(entries, rect, viewport);
                let open =
                    submenu.is_none() && (rect.contains(mouse) || candidate.panel.contains(mouse));
                if open {
                    submenu = Some(candidate);
                }
                rows.push(MenuRow::Parent { rect, label, open });
            }
        }
    }
    MenuLayout {
        panel,
        title: &spec.title,
        rows,
        submenu,
    }
}

fn submenu_layout(
    entries: &[(String, UiAction)],
    parent: Rect,
    viewport: [f32; 2],
) -> SubmenuLayout<'_> {
    let width = panel_width(entries.iter().map(|(label, _)| label.as_str()));
    let height = 2.0 * PADDING + entries.len().to_f32() * ROW_HEIGHT;
    // Prefer opening to the right, overlapping the parent edge by a few pixels
    // so the pointer can travel into the submenu without a dead gap.
    let x = if parent.right() + width - 4.0 <= viewport[0] - MARGIN {
        parent.right() - 4.0
    } else {
        (parent.x - width + 4.0).max(MARGIN)
    };
    let panel = Rect::new(
        x,
        parent.y.min(viewport[1] - height - MARGIN).max(MARGIN) - PADDING,
        width,
        height,
    );
    let rows = entries
        .iter()
        .enumerate()
        .map(|(index, (label, action))| {
            (
                Rect::new(
                    panel.x + 6.0,
                    panel.y + PADDING + index.to_f32() * ROW_HEIGHT,
                    panel.width - 12.0,
                    ROW_HEIGHT,
                ),
                label.as_str(),
                action,
            )
        })
        .collect();
    SubmenuLayout { panel, rows }
}

fn panel_width<'a>(labels: impl Iterator<Item = &'a str>) -> f32 {
    let longest = labels.map(str::len).max().unwrap_or(0);
    (longest.to_f32().mul_add(6.6, 52.0)).clamp(200.0, 380.0)
}

fn union(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Rect::new(
        x,
        y,
        a.right().max(b.right()) - x,
        a.bottom().max(b.bottom()) - y,
    )
}

trait ToF32 {
    fn to_f32(self) -> f32;
}

impl ToF32 for usize {
    fn to_f32(self) -> f32 {
        num_traits::ToPrimitive::to_f32(&self).unwrap_or(0.0)
    }
}

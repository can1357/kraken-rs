//! Declarative context-menu model shared by the slab-drawn overlay and the
//! native macOS presenter.
//!
//! State builds a [`MenuSpec`] for the active right-click target; the windowed
//! macOS path hands it to `app::native_menu`, while the slab document renders
//! it through the authored overlay lists.

use crate::ui::action::UiAction;

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
}

/// A complete context menu: a dimmed title naming the target plus entries.
#[derive(Clone, Debug)]
pub(crate) struct MenuSpec {
    pub(crate) title: String,
    pub(crate) entries: Vec<MenuEntry>,
}

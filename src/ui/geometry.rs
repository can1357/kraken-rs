use num_traits::ToPrimitive;

/// Height of the unified top chrome strip (tabs + actions).
pub(crate) const CHROME_HEIGHT: f32 = 44.0;
/// Height of the bottom status strip.
pub(crate) const STATUS_BAR_HEIGHT: f32 = 22.0;
/// Vertical origin of the three-pane workspace.
pub(crate) const CONTENT_TOP: f32 = CHROME_HEIGHT;
/// Height of the commit table's column header.
pub(crate) const COMMIT_HEADER_HEIGHT: f32 = 28.0;
/// Height of one virtualized commit or WIP row.
pub(crate) const COMMIT_ROW_HEIGHT: f32 = 26.0;

/// Converts an integer-like value to logical pixels without unchecked casts.
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn px(value: impl ToPrimitive) -> f32 {
    value.to_f32().unwrap_or(f32::MAX)
}

/// A logical-pixel rectangle used by layout, clipping, and hit testing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Rect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl Rect {
    /// Creates a rectangle from its top-left corner and extent.
    pub(crate) const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Returns the right edge.
    pub(crate) fn right(self) -> f32 {
        self.x + self.width
    }

    /// Returns the bottom edge.
    pub(crate) fn bottom(self) -> f32 {
        self.y + self.height
    }
}

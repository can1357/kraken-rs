use num_traits::ToPrimitive;

/// Height of the repository tab strip.
pub(crate) const TAB_BAR_HEIGHT: f32 = 40.0;
/// Height of the repository action toolbar.
pub(crate) const TOOLBAR_HEIGHT: f32 = 58.0;
/// Height of the bottom status strip.
pub(crate) const STATUS_BAR_HEIGHT: f32 = 22.0;
/// Vertical origin of the three-pane workspace.
pub(crate) const CONTENT_TOP: f32 = TAB_BAR_HEIGHT + TOOLBAR_HEIGHT;
/// Height of the commit table's column header.
pub(crate) const COMMIT_HEADER_HEIGHT: f32 = 28.0;
/// Height of one virtualized commit or WIP row.
pub(crate) const COMMIT_ROW_HEIGHT: f32 = 26.0;

/// Converts an integer-like value to logical pixels without unchecked casts.
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn px(value: impl ToPrimitive) -> f32 {
    value.to_f32().unwrap_or(f32::MAX)
}

/// An RGBA color with sRGB components in the 0–1 range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Color(pub(crate) [f32; 4]);

impl Color {
    /// Creates an opaque color from 8-bit sRGB components.
    pub(crate) fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self::rgba(red, green, blue, 255)
    }

    /// Creates a color from 8-bit sRGB components.
    pub(crate) fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self([
            f32::from(red) / 255.0,
            f32::from(green) / 255.0,
            f32::from(blue) / 255.0,
            f32::from(alpha) / 255.0,
        ])
    }

    /// Returns the color with a different alpha value.
    pub(crate) const fn with_alpha(self, alpha: f32) -> Self {
        Self([self.0[0], self.0[1], self.0[2], alpha])
    }
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

    /// Tests a logical point against this rectangle.
    pub(crate) fn contains(self, point: [f32; 2]) -> bool {
        point[0] >= self.x
            && point[0] <= self.right()
            && point[1] >= self.y
            && point[1] <= self.bottom()
    }

    /// Shrinks all edges by a uniform inset.
    pub(crate) fn inset(self, amount: f32) -> Self {
        Self::new(
            self.x + amount,
            self.y + amount,
            (self.width - amount * 2.0).max(0.0),
            (self.height - amount * 2.0).max(0.0),
        )
    }

    /// Returns the overlap of two rectangles, if any.
    pub(crate) fn intersection(self, other: Self) -> Option<Self> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > left && bottom > top).then(|| Self::new(left, top, right - left, bottom - top))
    }

    /// Clamps this rectangle to a clipping rectangle.
    pub(crate) fn clipped(self, clip: Self) -> Option<Self> {
        self.intersection(clip)
    }
}

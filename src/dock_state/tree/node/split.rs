use egui::Rect;

/// Default for a skipped, serialized layout [`Rect`]: an un-laid-out sentinel
/// that the next layout pass overwrites. Layout rects are transient and not
/// persisted (see the `serde(skip)` on [`SplitNode::rect`]).
#[cfg(feature = "serde")]
fn rect_unset() -> Rect {
    Rect::NOTHING
}

/// Identifies which child of a split [`Node`](crate::Node) a constraint applies
/// to.
///
/// For a [`Horizontal`](crate::Node::Horizontal) split the first child is the
/// left one and the second is the right one; for a
/// [`Vertical`](crate::Node::Vertical) split the first child is the top one and
/// the second is the bottom one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum FixedChild {
    /// The left (horizontal) or top (vertical) child.
    First,
    /// The right (horizontal) or bottom (vertical) child.
    Second,
}

/// A fixed-size constraint on one child of a split [`Node`](crate::Node).
///
/// When a split carries one of these, the named child keeps a constant pixel
/// size along the split axis as the parent resizes (the other child absorbs the
/// slack) instead of keeping a constant fraction of it. See
/// [`SplitNode::fixed`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct FixedSize {
    /// Which child keeps its size.
    pub child: FixedChild,
    /// The child's size along the split axis, in points.
    pub points: f32,
}

impl FixedSize {
    /// The top/left [`fraction`](SplitNode::fraction) that gives the fixed child
    /// `self.points` points when the split axis spans `extent` points.
    ///
    /// Returns `0.5` for a degenerate (non-positive) extent, matching the
    /// fallback used elsewhere for empty splits.
    pub fn fraction_for(self, extent: f32) -> f32 {
        if extent <= 0.0 {
            return 0.5;
        }
        let child_fraction = (self.points / extent).clamp(0.0, 1.0);
        match self.child {
            FixedChild::First => child_fraction,
            FixedChild::Second => 1.0 - child_fraction,
        }
    }
}

///the inner data of a [``Node::Horizontal``](crate::Node)/[``Node::Vertical``](crate::Node), which splits into two further nodes.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct SplitNode {
    /// The rectangle in which all children of this node are drawn.
    ///
    /// Transient layout state, recomputed every layout pass; not serialized.
    #[cfg_attr(feature = "serde", serde(skip, default = "rect_unset"))]
    pub rect: Rect,

    /// The fraction taken by the top child of this node.
    pub fraction: f32,

    /// Whether all subnodes are collapsed.
    pub fully_collapsed: bool,

    /// The number of collapsed leaf subnodes.
    pub collapsed_leaf_count: i32,

    /// Optional fixed-size constraint on one child.
    ///
    /// When `Some`, [`fraction`](Self::fraction) is re-derived from this on every
    /// layout so the named child holds a constant pixel size as the split
    /// resizes, while the other child absorbs the slack. Dragging the separator
    /// updates the stored size, so a fixed split stays draggable. `None` gives
    /// the classic proportional behavior.
    #[cfg_attr(feature = "serde", serde(default))]
    pub fixed: Option<FixedSize>,
}

impl SplitNode {
    /// Create a new ``SplitNode``
    pub const fn new(
        rect: Rect,
        fraction: f32,
        fully_collapsed: bool,
        collapsed_leaf_count: i32,
    ) -> Self {
        Self {
            rect,
            fraction,
            fully_collapsed,
            collapsed_leaf_count,
            fixed: None,
        }
    }
    /// Set the Area which this ``SplitNode`` occupies.
    #[inline]
    pub fn set_rect(&mut self, new_rect: Rect) {
        self.rect = new_rect;
    }

    /// Get the Area which this ``SplitNode`` occupies.
    pub fn rect(&self) -> Rect {
        self.rect
    }
}

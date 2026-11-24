// Vendored from https://github.com/storytold/craft-libs, crates/ui/src/layout/geometry.rs at
// 390fe98b0dd3fb84db7c3af7cbd958db4e8cc585.
// Copyright (c) 2026 ArtCraft Team and the craft-libs contributors.
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Changes for pdigger-factory-viz: no serde derives; `SplitLayout::resized` is dropped.

//! Bounded pane geometry: one split of a rectangle into two panes and a divider.

use egui::Rect;

/// The direction in which the children of a split are placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitAxis {
    /// First child on the left, second on the right.
    Horizontal,
    /// First child above the second.
    Vertical,
}

/// A sizing request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SplitSize {
    /// Fraction of the space left after the divider, assigned to the first child.
    Ratio(f32),
    /// First child size in points.
    FixedFirst(f32),
    /// Second child size in points.
    FixedSecond(f32),
}

impl SplitSize {
    fn value(self) -> f32 {
        match self {
            Self::Ratio(v) | Self::FixedFirst(v) | Self::FixedSecond(v) => v,
        }
    }
}

/// Contained, non-overlapping rectangles of one split, its divider included.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitLayout {
    /// The first pane.
    pub first: Rect,
    /// The divider.
    pub divider: Rect,
    /// The second pane.
    pub second: Rect,
}

/// Divides `rect` into two panes and a divider of `gap` points; each pane gets at least
/// `minimum` points (half the span when both do not fit). Invalid input gives `None`.
pub fn split_rect(rect: Rect, axis: SplitAxis, size: SplitSize, gap: f32, minimum: f32) -> Option<SplitLayout> {
    if !valid_rect(rect) || !valid_nonnegative(gap) || !valid_nonnegative(minimum) || !size.value().is_finite() {
        return None;
    }
    let total = match axis {
        SplitAxis::Horizontal => rect.width(),
        SplitAxis::Vertical => rect.height(),
    };
    let gap = gap.min(total);
    let space = f64::from(total - gap);
    let wanted = match size {
        SplitSize::Ratio(ratio) => space * f64::from(ratio),
        SplitSize::FixedFirst(points) => f64::from(points),
        SplitSize::FixedSecond(points) => space - f64::from(points),
    };
    let low = f64::from(minimum).min(space * 0.5);
    let first_size = wanted.clamp(low, space - low) as f32;
    let (mut first, mut divider, mut second) = (rect, rect, rect);
    match axis {
        SplitAxis::Horizontal => {
            first.max.x = (rect.min.x + first_size).min(rect.max.x);
            divider.min.x = first.max.x;
            divider.max.x = (divider.min.x + gap).min(rect.max.x);
            second.min.x = divider.max.x;
        }
        SplitAxis::Vertical => {
            first.max.y = (rect.min.y + first_size).min(rect.max.y);
            divider.min.y = first.max.y;
            divider.max.y = (divider.min.y + gap).min(rect.max.y);
            second.min.y = divider.max.y;
        }
    }
    Some(SplitLayout { first, divider, second })
}

fn valid_rect(rect: Rect) -> bool {
    rect.min.is_finite() && rect.max.is_finite() && rect.min.x <= rect.max.x && rect.min.y <= rect.max.y
}

fn valid_nonnegative(value: f32) -> bool {
    value.is_finite() && value >= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn splits_clamp_and_reject_invalid_input() {
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(106.0, 50.0));
        let s = split_rect(rect, SplitAxis::Horizontal, SplitSize::Ratio(0.5), 6.0, 10.0).unwrap();
        assert_eq!((s.first.max.x, s.divider.max.x, s.second.min.x, s.second.max.x), (50.0, 56.0, 56.0, 106.0));
        let v = split_rect(rect, SplitAxis::Vertical, SplitSize::FixedFirst(1.0), 0.0, 10.0).unwrap();
        assert_eq!(v.first.height(), 10.0);
        let w = split_rect(rect, SplitAxis::Vertical, SplitSize::FixedSecond(20.0), 0.0, 0.0).unwrap();
        assert_eq!(w.second.height(), 20.0);
        assert!(split_rect(rect, SplitAxis::Vertical, SplitSize::Ratio(f32::NAN), 0.0, 0.0).is_none());
        assert!(split_rect(Rect::NOTHING, SplitAxis::Vertical, SplitSize::Ratio(0.5), 0.0, 0.0).is_none());
        assert!(split_rect(rect, SplitAxis::Vertical, SplitSize::Ratio(0.5), -1.0, 0.0).is_none());
    }
}

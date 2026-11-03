//! Pane layout tree and its geometry.
//!
//! A tab owns a binary split tree whose leaves are pane ids. Splitting replaces
//! a leaf with a split node, closing a leaf promotes its sibling.

use serde::{Deserialize, Serialize};

use crate::protocol::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dir {
    /// Side by side.
    Columns,
    /// Stacked.
    Rows,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    Leaf(u64),
    Split {
        dir: Dir,
        first: Box<Layout>,
        second: Box<Layout>,
        /// Cells given to the first child, gutter excluded.
        first_size: u16,
    },
}

impl Layout {
    pub fn leaf(id: u64) -> Self {
        Self::Leaf(id)
    }

    pub fn split(&mut self, target: u64, new_id: u64, dir: Dir, first_size: u16) -> bool {
        if let Layout::Leaf(id) = self {
            if *id == target {
                *self = Layout::Split {
                    dir,
                    first: Box::new(Layout::Leaf(target)),
                    second: Box::new(Layout::Leaf(new_id)),
                    first_size,
                };
                return true;
            }
            return false;
        }
        match self {
            Layout::Split { first, second, .. } => {
                first.split(target, new_id, dir, first_size)
                    || second.split(target, new_id, dir, first_size)
            }
            Layout::Leaf(_) => false,
        }
    }

    /// Set the first child's size on the split at `path`.
    pub fn set_first_size(&mut self, path: &[bool], value: u16) -> bool {
        let Some((side, rest)) = path.split_first() else {
            return match self {
                Layout::Split { first_size, .. } => {
                    *first_size = value;
                    true
                }
                Layout::Leaf(_) => false,
            };
        };
        match self {
            Layout::Split { first, second, .. } => {
                let child = if *side { second } else { first };
                child.set_first_size(rest, value)
            }
            Layout::Leaf(_) => false,
        }
    }

    pub fn remove(&mut self, id: u64) -> bool {
        remove_node(self, id)
    }

    pub fn rects(&self, area: Rect, out: &mut Vec<(u64, Rect)>) {
        match self {
            Layout::Leaf(id) => out.push((*id, area)),
            Layout::Split {
                dir,
                first,
                second,
                first_size,
            } => {
                let (first_rect, _, second_rect) = split_area(area, *dir, *first_size);
                first.rects(first_rect, out);
                if let Some(rect) = second_rect {
                    second.rects(rect, out);
                }
            }
        }
    }

    pub fn leaves(&self, out: &mut Vec<u64>) {
        match self {
            Layout::Leaf(id) => out.push(*id),
            Layout::Split { first, second, .. } => {
                first.leaves(out);
                second.leaves(out);
            }
        }
    }

    pub fn first_leaf(&self) -> Option<u64> {
        match self {
            Layout::Leaf(id) => Some(*id),
            Layout::Split { first, .. } => first.first_leaf(),
        }
    }
}

/// The two halves of a split and the one cell gutter between them.
///
/// The gutter only exists when the area is at least three cells long; a
/// smaller area is split one cell at a time without a border.
pub(super) fn split_area(
    area: Rect,
    dir: Dir,
    first_size: u16,
) -> (Rect, Option<Rect>, Option<Rect>) {
    let size = match dir {
        Dir::Columns => area.cols,
        Dir::Rows => area.rows,
    };
    let (first, second) = split_size(size, first_size);
    let gutter = second > 0 && size >= 3;
    let offset = first + u16::from(gutter);
    let gutter_rect = match dir {
        Dir::Columns => Rect {
            x: area.x + first,
            y: area.y,
            cols: 1,
            rows: area.rows,
        },
        Dir::Rows => Rect {
            x: area.x,
            y: area.y + first,
            cols: area.cols,
            rows: 1,
        },
    };
    let second_rect = match dir {
        Dir::Columns => Rect {
            x: area.x + offset,
            cols: second,
            ..area
        },
        Dir::Rows => Rect {
            y: area.y + offset,
            rows: second,
            ..area
        },
    };
    (
        match dir {
            Dir::Columns => Rect {
                cols: first,
                ..area
            },
            Dir::Rows => Rect {
                rows: first,
                ..area
            },
        },
        gutter.then_some(gutter_rect),
        (second > 0).then_some(second_rect),
    )
}

/// Sizes of the two halves of a split, in cells.
///
/// Below three cells there is no gutter; otherwise the stored first size is
/// clamped so both children keep at least one cell.
fn split_size(size: u16, first_size: u16) -> (u16, u16) {
    if size < 3 {
        let first = (size / 2).max(1).min(size);
        return (first, size.saturating_sub(first));
    }
    let available = size - 1;
    let first = first_size.clamp(1, available - 1);
    (first, available - first)
}

fn remove_node(node: &mut Layout, id: u64) -> bool {
    let mut replacement: Option<Layout> = None;
    let found = match node {
        Layout::Leaf(_) => false,
        Layout::Split { first, second, .. } => {
            if matches!(**first, Layout::Leaf(x) if x == id) {
                replacement = Some(std::mem::replace(&mut **second, Layout::Leaf(0)));
                true
            } else if matches!(**second, Layout::Leaf(x) if x == id) {
                replacement = Some(std::mem::replace(&mut **first, Layout::Leaf(0)));
                true
            } else {
                remove_node(first, id) || remove_node(second, id)
            }
        }
    };
    if let Some(new) = replacement {
        *node = new;
    }
    found
}

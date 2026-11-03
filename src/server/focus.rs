//! Directional pane focus and the "previous pane" shortcut.
//!
//! Focus follows the geometry in `Tab::rects`: the pane whose centre is the
//! closest one in the requested direction wins, and a pane that shares a row or
//! column band with the focused pane beats one that is merely nearby.

use crate::protocol::Rect;
use crate::server::session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Up,
    Down,
    Right,
}

/// The pane to focus when moving `side` away from `focus`.
pub(super) fn neighbour(rects: &[(u64, Rect)], focus: u64, side: Side) -> Option<u64> {
    let from = rects.iter().find(|(id, _)| *id == focus)?.1;
    let (from_x, from_y) = centre(from);
    let mut best: Option<((u8, i32, i32), u64)> = None;
    for (id, rect) in rects {
        if *id == focus {
            continue;
        }
        let (x, y) = centre(*rect);
        let aligned = match side {
            Side::Left | Side::Right => bands_touch(from.y, from.rows, rect.y, rect.rows),
            Side::Up | Side::Down => bands_touch(from.x, from.cols, rect.x, rect.cols),
        };
        let (distance, across) = match side {
            Side::Left => (from_x - x, (from_y - y).abs()),
            Side::Right => (x - from_x, (from_y - y).abs()),
            Side::Up => (from_y - y, (from_x - x).abs()),
            Side::Down => (y - from_y, (from_x - x).abs()),
        };
        if distance <= 0 {
            continue;
        }
        let score = (if aligned { 0 } else { 1 }, distance, across);
        if best.is_none_or(|(best_score, _)| score < best_score) {
            best = Some((score, *id));
        }
    }
    best.map(|(_, id)| id)
}

fn centre(rect: Rect) -> (i32, i32) {
    (
        i32::from(rect.x) + i32::from(rect.cols) / 2,
        i32::from(rect.y) + i32::from(rect.rows) / 2,
    )
}

fn bands_touch(first: u16, first_len: u16, second: u16, second_len: u16) -> bool {
    let first_end = first.saturating_add(first_len);
    let second_end = second.saturating_add(second_len);
    first < second_end && second < first_end
}

impl Session {
    /// Move the focus. False when the pane has no neighbour on that side.
    pub(super) fn focus_side(&mut self, side: Side) -> bool {
        let area = self.content_area();
        let Some(tab) = self.active_tab_mut() else {
            return false;
        };
        let rects = tab.rects(area);
        let Some(next) = neighbour(&rects, tab.focus, side) else {
            return false;
        };
        tab.set_focus(next);
        self.dirty = true;
        true
    }

    /// Back to the pane that had the focus before the current one.
    pub(super) fn focus_last(&mut self) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let Some(previous) = tab.previous_focus else {
            return;
        };
        if previous == tab.focus || tab.pane(previous).is_none() {
            return;
        }
        tab.set_focus(previous);
        self.dirty = true;
    }

    pub(super) fn focus_next(&mut self) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let mut leaves = Vec::new();
        tab.layout.leaves(&mut leaves);
        if leaves.len() < 2 {
            return;
        }
        let index = leaves.iter().position(|id| *id == tab.focus).unwrap_or(0);
        tab.set_focus(leaves[(index + 1) % leaves.len()]);
        self.dirty = true;
    }

    /// The pane wins. Only Left and Right fall through to the tabs, and they
    /// wrap the same way the leader's tab keys do.
    pub(super) fn focus_or_tab(&mut self, side: Side) {
        if self.focus_side(side) {
            return;
        }
        match side {
            Side::Left => self.switch_tab(-1),
            Side::Right => self.switch_tab(1),
            Side::Up | Side::Down => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u16, y: u16, cols: u16, rows: u16) -> Rect {
        Rect { x, y, cols, rows }
    }

    #[test]
    fn left_and_right_follow_the_columns() {
        let rects = vec![(1, rect(0, 0, 50, 23)), (2, rect(50, 0, 50, 23))];
        assert_eq!(neighbour(&rects, 1, Side::Right), Some(2));
        assert_eq!(neighbour(&rects, 2, Side::Left), Some(1));
        assert_eq!(neighbour(&rects, 1, Side::Left), None);
        assert_eq!(neighbour(&rects, 2, Side::Right), None);
        assert_eq!(neighbour(&rects, 1, Side::Down), None);
    }

    #[test]
    fn up_and_down_follow_the_rows() {
        let rects = vec![(1, rect(0, 0, 100, 11)), (2, rect(0, 11, 100, 12))];
        assert_eq!(neighbour(&rects, 1, Side::Down), Some(2));
        assert_eq!(neighbour(&rects, 2, Side::Up), Some(1));
        assert_eq!(neighbour(&rects, 2, Side::Down), None);
    }

    #[test]
    fn an_aligned_pane_beats_a_closer_centre() {
        // Feed, one tall pane on the right split into two.
        let rects = vec![
            (1, rect(0, 0, 50, 23)),
            (2, rect(50, 0, 50, 11)),
            (3, rect(50, 11, 50, 12)),
        ];
        assert_eq!(neighbour(&rects, 1, Side::Right), Some(2));
        assert_eq!(neighbour(&rects, 2, Side::Left), Some(1));
        assert_eq!(neighbour(&rects, 2, Side::Down), Some(3));
        assert_eq!(neighbour(&rects, 3, Side::Up), Some(2));
        assert_eq!(neighbour(&rects, 3, Side::Left), Some(1));
    }

    #[test]
    fn a_missing_pane_has_no_neighbour() {
        let rects = vec![(1, rect(0, 0, 50, 23))];
        assert_eq!(neighbour(&rects, 7, Side::Left), None);
        assert_eq!(neighbour(&rects, 1, Side::Left), None);
    }
}

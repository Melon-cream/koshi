//! Screen geometry of one tab: the frame, the pane rects, and the gutters.
//!
//! This is the only place that decides where a pane sits on the client screen.
//! The ptys are sized from `panes`, the border painter draws `frame_cells` and
//! `gutters`, and a mouse cell is resolved through the same methods, so the
//! three cannot drift apart.

use crate::protocol::Rect;
use crate::server::layout::{Dir, Layout, split_area};

/// A one cell gutter between the two children of a split.
#[derive(Clone, Debug)]
pub(super) struct Gutter {
    pub(super) rect: Rect,
    /// Where the split sits in the layout: `false` for the first child at each
    /// level, `true` for the second.
    pub(super) path: Vec<bool>,
    /// The area that split divides.
    pub(super) area: Rect,
    pub(super) dir: Dir,
}

/// The pane that floats over the tiled ones, drawn last.
#[derive(Clone, Copy, Debug)]
pub(super) struct Float {
    /// The pane in the box.
    pub(super) pane: u64,
    /// The box, its own frame included.
    pub(super) frame: Rect,
    /// What the pty gets: `frame` inside one cell of border.
    pub(super) inner: Rect,
}

impl Float {
    pub(super) fn of(pane: u64, area: Rect) -> Self {
        let frame = float_rect(area);
        Self {
            pane,
            frame,
            inner: inset(frame),
        }
    }
}

/// Two thirds of the content area, centred, clamped so it always leaves the
/// tiled screen around it and one cell of pty inside its own frame.
fn float_rect(area: Rect) -> Rect {
    let cols = fit((u32::from(area.cols) * 2 / 3).max(24) as u16, area.cols);
    let rows = fit((u32::from(area.rows) * 2 / 3).max(7) as u16, area.rows);
    Rect {
        x: area.x + (area.cols.saturating_sub(cols)) / 2,
        y: area.y + (area.rows.saturating_sub(rows)) / 2,
        cols,
        rows,
    }
}

/// `want` bounded to `size`, keeping two cells of the tiled screen when they fit
/// and always leaving room for the frame and one cell of pty.
fn fit(want: u16, size: u16) -> u16 {
    let limit = if size > 3 { size - 2 } else { size };
    want.clamp(3.min(limit), limit)
}

/// Everything a tab draws, sizes ptys by, and hit tests against.
pub(super) struct Geometry {
    /// The tab's whole content area, frame included.
    pub(super) area: Rect,
    /// What the panes and the gutters share: `area` inside the frame, or `area`
    /// itself when the tab has no frame.
    pub(super) inner: Rect,
    /// True when the perimeter of `area` is the frame, i.e. no pane may reach it.
    pub(super) framed: bool,
    pub(super) panes: Vec<(u64, Rect)>,
    pub(super) gutters: Vec<Gutter>,
    /// The pane floating over the tiled ones, if one is shown.
    pub(super) float: Option<Float>,
}

impl Geometry {
    /// `count` is how many panes the tab has or is about to have; a second pane
    /// is what brings the frame.
    pub(super) fn of(layout: &Layout, area: Rect, count: usize) -> Self {
        let framed = count > 1;
        let inner = if framed { inset(area) } else { area };
        let mut panes = Vec::new();
        let mut gutters = Vec::new();
        walk(layout, inner, &mut Vec::new(), &mut panes, &mut gutters);
        Self {
            area,
            inner,
            framed,
            panes,
            gutters,
            float: None,
        }
    }

    /// The single pane of a zoomed tab, which keeps the whole content area.
    pub(super) fn zoomed(id: u64, area: Rect) -> Self {
        Self {
            area,
            inner: area,
            framed: false,
            panes: vec![(id, area)],
            gutters: Vec::new(),
            float: None,
        }
    }

    /// Put `pane` over the tiled ones, in a box of its own.
    pub(super) fn with_float(mut self, pane: u64) -> Self {
        self.float = Some(Float::of(pane, self.area));
        self
    }

    /// Every visible pane with its pty rect, the floating one last so that it
    /// is drawn over the tiled ones.
    pub(super) fn visible(&self) -> Vec<(u64, Rect)> {
        let mut out = self.panes.clone();
        if let Some(float) = &self.float {
            out.push((float.pane, float.inner));
        }
        out
    }

    pub(super) fn pane_rect(&self, id: u64) -> Option<Rect> {
        if let Some(float) = &self.float
            && float.pane == id
        {
            return Some(float.inner);
        }
        self.panes
            .iter()
            .find(|(pane, _)| *pane == id)
            .map(|(_, rect)| *rect)
    }

    /// The pane holding a cell, as the ptys were told it. The floating pane
    /// covers the tiled ones, so it answers first.
    pub(super) fn pane_at(&self, col: u16, row: u16) -> Option<(u64, Rect)> {
        if let Some(float) = &self.float
            && contains(float.frame, col, row)
        {
            return Some((float.pane, float.inner));
        }
        self.panes
            .iter()
            .copied()
            .find(|(_, rect)| contains(*rect, col, row))
    }

    /// True for a cell of the floating pane's own frame.
    pub(super) fn on_float_frame(&self, col: u16, row: u16) -> bool {
        let Some(float) = &self.float else {
            return false;
        };
        contains(float.frame, col, row) && !contains(float.inner, col, row)
    }

    /// The floating pane and its pane-local cell for a press inside its box. Its
    /// frame counts, pinned to the pty edge, so a press on the border selects
    /// from there instead of resizing.
    pub(super) fn float_cell(&self, col: u16, row: u16) -> Option<(u64, u16, u16)> {
        let float = self.float?;
        if !contains(float.frame, col, row) {
            return None;
        }
        let inner = float.inner;
        let local_col = col
            .saturating_sub(inner.x)
            .min(inner.cols.saturating_sub(1));
        let local_row = row
            .saturating_sub(inner.y)
            .min(inner.rows.saturating_sub(1));
        Some((float.pane, local_col, local_row))
    }

    /// True for a cell the floating pane hides, frame included.
    pub(super) fn under_float(&self, col: u16, row: u16) -> bool {
        self.float
            .is_some_and(|float| contains(float.frame, col, row))
    }

    /// The perimeter of the floating pane, in cells.
    pub(super) fn float_frame(&self) -> Vec<(u16, u16)> {
        let Some(float) = &self.float else {
            return Vec::new();
        };
        perimeter(float.frame)
    }

    /// True for a cell of the frame. A press here selects from the pane edge.
    pub(super) fn on_frame(&self, col: u16, row: u16) -> bool {
        self.framed && self.on_perimeter(col, row)
    }

    /// The perimeter of the content area, in cells.
    pub(super) fn frame_cells(&self) -> Vec<(u16, u16)> {
        if !self.framed {
            return Vec::new();
        }
        perimeter(self.area)
    }

    /// The frame cells 4-adjacent to one pane, i.e. the ones that belong to it.
    pub(super) fn frame_around(&self, id: u64) -> Vec<(u16, u16)> {
        let Some(rect) = self.pane_rect(id) else {
            return Vec::new();
        };
        self.frame_cells()
            .into_iter()
            .filter(|&(x, y)| {
                neighbours(x, y)
                    .into_iter()
                    .any(|(nx, ny)| contains(rect, nx, ny))
            })
            .collect()
    }

    /// Panes with a cell 4-adjacent to `(col, row)`. Two where a gutter meets
    /// the frame; the focus decides between them.
    pub(super) fn panes_next_to(&self, col: u16, row: u16) -> Vec<(u64, Rect)> {
        self.panes
            .iter()
            .copied()
            .filter(|(_, rect)| {
                neighbours(col, row)
                    .into_iter()
                    .any(|(nx, ny)| contains(*rect, nx, ny))
            })
            .collect()
    }

    pub(super) fn gutter_at(&self, col: u16, row: u16) -> Option<&Gutter> {
        self.gutters
            .iter()
            .find(|gutter| contains(gutter.rect, col, row))
    }

    fn on_perimeter(&self, col: u16, row: u16) -> bool {
        if self.area.cols == 0 || self.area.rows == 0 {
            return false;
        }
        let inside = contains(self.area, col, row);
        let right = self.area.x + self.area.cols - 1;
        let bottom = self.area.y + self.area.rows - 1;
        inside && (col == self.area.x || col == right || row == self.area.y || row == bottom)
    }
}

/// The content box inside the frame; the frame spends one cell on each side.
fn inset(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        cols: area.cols.saturating_sub(2),
        rows: area.rows.saturating_sub(2),
    }
}

/// The one cell ring around a rect, in cells.
fn perimeter(rect: Rect) -> Vec<(u16, u16)> {
    if rect.cols == 0 || rect.rows == 0 {
        return Vec::new();
    }
    let right = rect.x + rect.cols - 1;
    let bottom = rect.y + rect.rows - 1;
    let mut out = Vec::new();
    for x in rect.x..=right {
        out.push((x, rect.y));
        out.push((x, bottom));
    }
    for y in rect.y + 1..bottom {
        out.push((rect.x, y));
        out.push((right, y));
    }
    out
}

fn walk(
    node: &Layout,
    area: Rect,
    path: &mut Vec<bool>,
    panes: &mut Vec<(u64, Rect)>,
    gutters: &mut Vec<Gutter>,
) {
    match node {
        Layout::Leaf(id) => panes.push((*id, area)),
        Layout::Split {
            dir,
            first,
            second,
            first_size,
        } => {
            let (first_rect, gutter, second_rect) = split_area(area, *dir, *first_size);
            path.push(false);
            walk(first, first_rect, path, panes, gutters);
            path.pop();
            if let Some(rect) = second_rect {
                path.push(true);
                walk(second, rect, path, panes, gutters);
                path.pop();
            }
            if let Some(rect) = gutter {
                gutters.push(Gutter {
                    rect,
                    path: path.clone(),
                    area,
                    dir: *dir,
                });
            }
        }
    }
}

pub(super) fn contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x
        && col < rect.x.saturating_add(rect.cols)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.rows)
}

pub(super) fn neighbours(col: u16, row: u16) -> Vec<(u16, u16)> {
    let mut out = Vec::with_capacity(4);
    if let Some(left) = col.checked_sub(1) {
        out.push((left, row));
    }
    if let Some(right) = col.checked_add(1) {
        out.push((right, row));
    }
    if let Some(up) = row.checked_sub(1) {
        out.push((col, up));
    }
    if let Some(down) = row.checked_add(1) {
        out.push((col, down));
    }
    out
}

#[cfg(test)]
mod tests;

//! The colour and the character of every non-pane cell.
//!
//! The cells themselves come from `geom::Geometry`, so the frame and the
//! gutters are drawn exactly where the panes are not. Gutters always stay plain;
//! the focus colour marks only the frame cells around the focused pane, and the
//! whole frame of the floating pane, which owns the keyboard while it is shown.

use std::collections::HashSet;

use crate::protocol::{BorderCell, Rect};
use crate::server::geom::Geometry;
use crate::server::layout::Dir;

/// Border cells for a tab, or none when there is nothing to separate.
pub(super) fn compute(geometry: &Geometry, focus: u64) -> Vec<BorderCell> {
    let frame: HashSet<(u16, u16)> = geometry.frame_cells().into_iter().collect();
    let mut cells = frame.clone();
    let mut vertical: HashSet<(u16, u16)> = HashSet::new();
    for gutter in &geometry.gutters {
        for cell in cells_of(gutter.rect) {
            cells.insert(cell);
            if gutter.dir == Dir::Columns {
                vertical.insert(cell);
            }
        }
    }
    let float: HashSet<(u16, u16)> = geometry.float_frame().into_iter().collect();
    cells.extend(float.iter().copied());
    let mut strong: HashSet<(u16, u16)> = geometry.frame_around(focus).into_iter().collect();
    strong.extend(float.iter().copied());
    // The float hides the tiled borders behind it, so those cells are neither
    // drawn nor allowed to bend the characters around the hole.
    let tiled: HashSet<(u16, u16)> = cells
        .iter()
        .copied()
        .filter(|cell| !float.contains(cell) && !geometry.under_float(cell.0, cell.1))
        .collect();

    let mut out: Vec<BorderCell> = tiled
        .iter()
        .map(|&(x, y)| BorderCell {
            x,
            y,
            ch: frame_side(geometry, x, y)
                .unwrap_or_else(|| line_char(&tiled, vertical.contains(&(x, y)), x, y)),
            strong: strong.contains(&(x, y)),
        })
        .chain(float.iter().map(|&(x, y)| BorderCell {
            x,
            y,
            ch: float_side(geometry, x, y).unwrap_or_else(|| line_char(&float, false, x, y)),
            strong: true,
        }))
        .collect();
    out.sort_by_key(|cell| (cell.y, cell.x));
    out
}

fn cells_of(rect: Rect) -> Vec<(u16, u16)> {
    let mut out = Vec::with_capacity((rect.cols as usize) * (rect.rows as usize));
    for y in rect.y..rect.y + rect.rows {
        for x in rect.x..rect.x + rect.cols {
            out.push((x, y));
        }
    }
    out
}

/// A frame cell on the left or right edge keeps its bar; only the top and
/// bottom edges take a junction where a gutter meets them.
fn frame_side(geometry: &Geometry, x: u16, y: u16) -> Option<char> {
    if !geometry.on_frame(x, y) {
        return None;
    }
    let area = geometry.area;
    let right = area.x + area.cols - 1;
    let bottom = area.y + area.rows - 1;
    ((x == area.x || x == right) && y > area.y && y < bottom).then_some('│')
}

/// The left and right edges of the floating pane are plain bars too.
fn float_side(geometry: &Geometry, x: u16, y: u16) -> Option<char> {
    if !geometry.on_float_frame(x, y) {
        return None;
    }
    let inner = geometry.float?.inner;
    let top = inner.y - 1;
    let bottom = inner.y + inner.rows;
    (y > top && y < bottom).then_some('│')
}

/// The character for one border cell, from which of its neighbours are border
/// cells too. `vertical` picks the bar for a cell with no neighbour at all.
fn line_char(cells: &HashSet<(u16, u16)>, vertical: bool, x: u16, y: u16) -> char {
    let up = y > 0 && cells.contains(&(x, y - 1));
    let down = cells.contains(&(x, y + 1));
    let left = x > 0 && cells.contains(&(x - 1, y));
    let right = cells.contains(&(x + 1, y));
    match (up, down, left, right) {
        (true, true, true, true) => '┼',
        (true, true, false, true) => '├',
        (true, true, true, false) => '┤',
        (false, true, true, true) => '┬',
        (true, false, true, true) => '┴',
        (true, true, false, false) => '│',
        (false, false, true, true) => '─',
        (false, true, false, true) => '┌',
        (false, true, true, false) => '┐',
        (true, false, false, true) => '└',
        (true, false, true, false) => '┘',
        _ if vertical => '│',
        _ => '─',
    }
}

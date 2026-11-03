//! Rendering a pane to cells and the cursor position for the frame.

use alacritty_terminal::term::point_to_viewport;
use alacritty_terminal::vte::ansi::CursorShape;

use crate::protocol::{CellData, CursorPos, FLAG_SELECTED};
use crate::server::cells::convert_cell;

use super::Pane;

impl Pane {
    pub fn cursor(&self) -> Option<CursorPos> {
        let content = self.term.renderable_content();
        if content.cursor.shape == CursorShape::Hidden {
            return None;
        }
        let point = point_to_viewport(content.display_offset, content.cursor.point)?;
        if point.line >= self.size.rows || point.column.0 >= self.size.cols {
            return None;
        }
        Some(CursorPos {
            row: point.line as u16,
            col: point.column.0 as u16,
        })
    }

    pub fn render_rows(&self) -> Vec<Vec<CellData>> {
        let mut rows = vec![vec![CellData::default(); self.size.cols]; self.size.rows];
        let content = self.term.renderable_content();
        let offset = content.display_offset;
        let selection = content.selection;
        let shape = content.cursor.shape;
        // `contains_cell` skips the block cursor's own cell at the selection
        // ends, so it must get the cursor's point, not the cell's.
        let cursor = content.cursor.point;
        for indexed in content.display_iter {
            let Some(point) = point_to_viewport(offset, indexed.point) else {
                continue;
            };
            if point.line >= self.size.rows || point.column.0 >= self.size.cols {
                continue;
            }
            let mut cell = convert_cell(indexed.cell);
            if selection
                .as_ref()
                .is_some_and(|range| range.contains_cell(&indexed, cursor, shape))
            {
                cell.flags |= FLAG_SELECTED;
            }
            rows[point.line][point.column.0] = cell;
        }
        rows
    }
}

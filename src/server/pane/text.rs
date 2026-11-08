//! Reading a pane's grid back as plain text: history for the state file and the
//! unfinished line the record viewer shows.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};

use super::Pane;

impl Pane {
    /// The newest `max` grid lines as text, oldest first, colours dropped.
    pub fn text_lines(&self, max: usize) -> Vec<String> {
        if self.alt_screen() || max == 0 {
            return Vec::new();
        }
        let grid = self.term.grid();
        let bottom = grid.bottommost_line();
        let mut line = grid.topmost_line();
        if (bottom.0 - line.0) as usize >= max {
            line = Line(bottom.0 - max as i32 + 1);
        }
        let mut out = Vec::new();
        while line <= bottom {
            let text: String = grid[line][..].iter().map(|cell| cell.c).collect();
            out.push(text.trim_end().to_string());
            line += 1;
        }
        out
    }

    /// The line the cursor still sits on, up to the cursor. A full-screen
    /// program's screen is not part of the recording, so it has no line here.
    pub fn current_line(&self) -> Option<String> {
        if self.alt_screen() {
            return None;
        }
        let grid = self.term.grid();
        let point = grid.cursor.point;
        let row = &grid[point.line];
        let end = (point.column.0 + 1).min(self.cols() as usize);
        let line: String = (0..end).map(|col| row[Column(col)].c).collect();
        let line = line.trim_end().to_string();
        (!line.is_empty()).then_some(line)
    }
}

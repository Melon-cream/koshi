//! Client-side screen mirror and ANSI rendering.
//!
//! The mirror only exists so that the client can redraw the rows the server
//! reports as changed; it is never treated as the source of truth.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::protocol::{BorderCell, CellData, Frame, Overlay, PaneFrame, Rect, Status};

use super::paint;

const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";
const CLEAR_SCREEN: &str = "\x1b[2J";

pub struct Mirror {
    content: Rect,
    panes: HashMap<u64, PaneMirror>,
    status: Option<Status>,
    overlay: Option<Overlay>,
    borders: Vec<BorderCell>,
    toast: Option<String>,
    /// Absolute screen cell the last frame left the cursor on.
    cursor: Option<(u16, u16)>,
    first: bool,
}

struct PaneMirror {
    rect: Rect,
    cells: Vec<Vec<CellData>>,
}

impl PaneMirror {
    fn new(rect: Rect, cols: u16, rows: u16) -> Self {
        Self {
            rect,
            cells: paint::blank(rows as usize, cols as usize),
        }
    }

    fn matches(&self, rect: Rect, cols: u16, rows: u16) -> bool {
        self.rect == rect
            && self.cells.len() == rows as usize
            && self.cells.first().map(Vec::len) == Some(cols as usize)
    }
}

impl Mirror {
    pub fn new() -> Self {
        Self {
            content: Rect {
                x: 0,
                y: 0,
                cols: 0,
                rows: 0,
            },
            panes: HashMap::new(),
            status: None,
            overlay: None,
            borders: Vec::new(),
            toast: None,
            cursor: None,
            first: true,
        }
    }

    /// True once a frame has told the mirror the screen geometry.
    pub fn sized(&self) -> bool {
        self.content.cols > 0 && self.content.rows > 0
    }

    /// Apply a frame and return the ANSI bytes that reproduce it on screen.
    pub fn apply(&mut self, frame: &Frame) -> String {
        let mut out = String::new();
        let overlay_changed = self.overlay != frame.overlay;
        let borders_changed = self.borders != frame.borders;
        let toast_changed = self.toast != frame.toast;
        let layout_changed = self.first
            || self.content != frame.content
            || frame.panes.iter().any(|pane| {
                self.panes
                    .get(&pane.id)
                    .is_none_or(|mirror| mirror.rect != pane.rect)
            });
        let redraw_all =
            self.first || layout_changed || overlay_changed || borders_changed || toast_changed;
        // Panes cover the content area, so redrawing every row is enough after a
        // layout change; clearing again would flash the whole screen on a drag.
        let content_resized = self.first || self.content != frame.content;
        if redraw_all {
            out.push_str("\x1b[0m");
            if content_resized {
                out.push_str(CLEAR_SCREEN);
            }
            if self.first || layout_changed {
                self.panes.clear();
            }
        }
        self.content = frame.content;

        // A pane can cover part of the one drawn before it, as the floating pane
        // does. Its rows are then always repainted, or tiled output behind it
        // would come through the next time only the tile changed.
        let covering = covering_panes(&frame.panes);
        for (index, pane_frame) in frame.panes.iter().enumerate() {
            let needs_new = self.panes.get(&pane_frame.id).is_none_or(|mirror| {
                !mirror.matches(pane_frame.rect, pane_frame.cols, pane_frame.rows)
            });
            if needs_new {
                self.panes.insert(
                    pane_frame.id,
                    PaneMirror::new(pane_frame.rect, pane_frame.cols, pane_frame.rows),
                );
            }
            let mirror = self
                .panes
                .get_mut(&pane_frame.id)
                .expect("pane mirror just inserted");
            for update in &pane_frame.rows_changed {
                let row = update.row as usize;
                if row >= mirror.cells.len() {
                    continue;
                }
                let width = mirror.cells[row].len();
                let cells = &mut mirror.cells[row];
                cells.clear();
                cells.extend(update.cells.iter().copied().take(width));
                cells.resize(width, CellData::default());
            }
            let rows: Vec<usize> = if redraw_all || needs_new || covering.contains(&index) {
                let height = (mirror.rect.rows as usize).min(mirror.cells.len());
                (0..height).collect()
            } else {
                pane_frame
                    .rows_changed
                    .iter()
                    .map(|update| update.row as usize)
                    .collect()
            };
            for row in rows {
                paint::pane_row(&mut out, &mirror.cells[row], mirror.rect, row, self.content);
            }
        }
        self.panes
            .retain(|id, _| frame.panes.iter().any(|pane| pane.id == *id));

        paint::borders(&mut out, &frame.borders);

        let status_changed = self.status.as_ref() != Some(&frame.status);
        if redraw_all || status_changed {
            paint::status(&mut out, self.content, &frame.status);
            self.status = Some(frame.status.clone());
        }
        // Redraw the overlay after pane updates so streaming output cannot
        // punch holes through it.
        if let Some(overlay) = &frame.overlay {
            paint::overlay(&mut out, self.content, overlay);
        }
        self.overlay = frame.overlay.clone();
        if let Some(toast) = &frame.toast {
            paint::toast(&mut out, self.content, toast);
        }
        self.borders.clone_from(&frame.borders);
        self.toast.clone_from(&frame.toast);

        let cursor = if frame.overlay.is_some() {
            None
        } else {
            frame
                .panes
                .iter()
                .find(|pane| pane.focused)
                .and_then(|pane| pane.cursor.map(|cursor| (pane.rect, cursor)))
        };
        match cursor {
            Some((rect, cursor)) => {
                let row = rect.y as usize + cursor.row as usize;
                let col = rect.x as usize + cursor.col as usize;
                self.cursor = Some((row as u16, col as u16));
                let _ = write!(out, "\x1b[{};{}H{}", row + 1, col + 1, SHOW_CURSOR);
            }
            None => {
                self.cursor = None;
                out.push_str(HIDE_CURSOR);
            }
        }

        self.first = false;
        out
    }

    /// Draw a transient one line message on the status row.
    pub fn notice(&mut self, message: &str) -> String {
        let row = self.content.y as usize + self.content.rows as usize;
        let mut line = message
            .chars()
            .take(self.content.cols as usize)
            .collect::<String>();
        while line.chars().count() < self.content.cols as usize {
            line.push(' ');
        }
        let mut out = format!("\x1b[{};1H\x1b[0;7m{}\x1b[0m", row + 1, line);
        out.push_str(&self.cursor_placement());
        out
    }

    /// Repaint the stored status bar, used to take a transient notice back.
    pub fn status_refresh(&self) -> Option<String> {
        let status = self.status.clone()?;
        if !self.sized() {
            return None;
        }
        let mut out = String::new();
        paint::status(&mut out, self.content, &status);
        if let Some(overlay) = &self.overlay {
            paint::overlay(&mut out, self.content, overlay);
        }
        out.push_str(&self.cursor_placement());
        Some(out)
    }

    /// Put the terminal cursor back where the frame left it. Drawing the status
    /// row moves the cursor with it, and the user should never see it parked
    /// there.
    fn cursor_placement(&self) -> String {
        match self.cursor {
            Some((row, col)) => {
                format!("\x1b[{};{}H{}", row + 1, col + 1, SHOW_CURSOR)
            }
            None => HIDE_CURSOR.to_string(),
        }
    }
}

/// Indices of the panes that overlap a pane drawn before them. The floating
/// pane is the only such pane today.
fn covering_panes(panes: &[PaneFrame]) -> Vec<usize> {
    (0..panes.len())
        .filter(|index| {
            panes[..*index]
                .iter()
                .any(|earlier| overlaps(earlier.rect, panes[*index].rect))
        })
        .collect()
}

fn overlaps(first: Rect, second: Rect) -> bool {
    first.x < second.x.saturating_add(second.cols)
        && second.x < first.x.saturating_add(first.cols)
        && first.y < second.y.saturating_add(second.rows)
        && second.y < first.y.saturating_add(first.rows)
}

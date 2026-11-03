//! Drag selection, click fallback, the OSC 52 copy, and the cell arithmetic
//! that the mouse routing and the gutter drag share.

use alacritty_terminal::index::Point;

use crate::protocol::{MAX_FRAME_LEN, MouseAction, MouseInput, Rect, ServerMsg, base64};
use crate::server::mouse_encode::encode_mouse;
use crate::server::session::Session;

/// One in-progress drag. Points are grid coordinates so scrolling keeps them.
pub(super) struct Drag {
    pub(super) pane: u64,
    pub(super) anchor: Point,
    pub(super) started: bool,
    /// -1 while held above the pane, 1 while held below it, 0 inside.
    pub(super) edge: i8,
    pub(super) col: u16,
}

/// A scroll step produced by `tick_mouse` for one held-outside drag.
pub(super) struct Pending {
    pub(super) client: u64,
    pub(super) pane: u64,
    pub(super) anchor: Point,
    pub(super) started: bool,
    pub(super) edge: i8,
    pub(super) col: u16,
}

impl Session {
    pub(super) fn start_drag(&mut self, id: u64, pane_id: u64, local_col: u16, local_row: u16) {
        let Some(pane) = self.pane_by_id_mut(pane_id) else {
            return;
        };
        let anchor = pane.grid_point(local_row, local_col);
        pane.clear_selection();
        if let Some(client) = self.client_mut(id) {
            client.drag = Some(Drag {
                pane: pane_id,
                anchor,
                started: false,
                edge: 0,
                col: local_col,
            });
        }
        self.dirty = true;
    }

    pub(super) fn drag_motion(&mut self, id: u64, mut drag: Drag, event: MouseInput) {
        let Some(rect) = self.pane_rect(drag.pane) else {
            return;
        };
        let edge = edge_of(rect, event);
        let (local_col, local_row) = edge_cell(rect, event, edge);
        let Some(pane) = self.pane_by_id_mut(drag.pane) else {
            return;
        };
        let point = pane.grid_point(local_row, local_col);
        pane.select(drag.anchor, point);
        drag.started = true;
        // The alternate screen has no history, so never auto-scroll it.
        drag.edge = if pane.alt_screen() { 0 } else { edge };
        drag.col = local_col;
        if let Some(client) = self.client_mut(id) {
            client.drag = Some(drag);
        }
        self.dirty = true;
    }

    pub(super) fn drag_release(&mut self, id: u64, drag: Drag, event: MouseInput) {
        let Some(rect) = self.pane_rect(drag.pane) else {
            return;
        };
        let edge = edge_of(rect, event);
        let (local_col, local_row) = edge_cell(rect, event, edge);
        if !drag.started {
            self.click(drag.pane, local_col, local_row, event);
            self.dirty = true;
            return;
        }
        let text = match self.pane_by_id_mut(drag.pane) {
            Some(pane) => {
                let point = pane.grid_point(local_row, local_col);
                pane.select(drag.anchor, point);
                pane.selection_text()
            }
            None => None,
        };
        self.send_clipboard(id, text);
        self.dirty = true;
    }

    pub(super) fn click(
        &mut self,
        pane_id: u64,
        local_col: u16,
        local_row: u16,
        event: MouseInput,
    ) {
        let Some(pane) = self.pane_by_id_mut(pane_id) else {
            return;
        };
        if !pane.mouse_reporting() {
            return;
        }
        let press = MouseInput {
            action: MouseAction::Press,
            ..event
        };
        if let Some(bytes) = encode_mouse(pane.mode(), press, local_col, local_row) {
            let _ = pane.write(&bytes);
        }
        if let Some(bytes) = encode_mouse(pane.mode(), event, local_col, local_row) {
            let _ = pane.write(&bytes);
        }
    }

    pub(super) fn send_clipboard(&mut self, id: u64, text: Option<String>) {
        let Some(text) = text else { return };
        if text.is_empty() {
            return;
        }
        let payload = base64(text.as_bytes());
        if payload.len() + 64 > MAX_FRAME_LEN {
            self.notice(
                id,
                format!(
                    "koshi: selection is too large for OSC 52 ({} bytes); not copied",
                    text.len()
                ),
            );
            return;
        }
        self.broadcast(ServerMsg::Clipboard(payload));
        self.show_toast(format!("copied {} chars", text.chars().count()));
    }
}

pub(super) fn axis(start: u16, len: u16, at: u16) -> u16 {
    let last = len.saturating_sub(1);
    if at <= start {
        0
    } else if at >= start.saturating_add(last) {
        last
    } else {
        at - start
    }
}

pub(super) fn edge_of(rect: Rect, event: MouseInput) -> i8 {
    if event.row < rect.y {
        -1
    } else if event.row >= rect.y.saturating_add(rect.rows) {
        1
    } else {
        0
    }
}

pub(super) fn edge_cell(rect: Rect, event: MouseInput, edge: i8) -> (u16, u16) {
    let col = event
        .col
        .saturating_sub(rect.x)
        .min(rect.cols.saturating_sub(1));
    let row = match edge {
        -1 => 0,
        1 => rect.rows.saturating_sub(1),
        _ => event
            .row
            .saturating_sub(rect.y)
            .min(rect.rows.saturating_sub(1)),
    };
    (col, row)
}

pub(super) fn local_cell(rect: Rect, event: MouseInput) -> (u16, u16) {
    edge_cell(rect, event, edge_of(rect, event))
}

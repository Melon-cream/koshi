//! Mouse routing: pane focus, app mouse reports, scrollback, and selection copy.
//!
//! The client only reports coordinates. The server owns the VT state, so it
//! decides which pane receives the event and how it is encoded.

use anyhow::Result;

use crate::protocol::{MouseAction, MouseButton, MouseInput, Rect};
use crate::server::input::InputState;
use crate::server::layout::Dir;
use crate::server::mouse_encode::encode_mouse;
use crate::server::resize::Resize;
use crate::server::select::{Pending, axis, local_cell};
use crate::server::session::Session;

impl Session {
    pub fn handle_mouse(&mut self, id: u64, event: MouseInput) -> Result<()> {
        if !mouse_allowed(self, id) {
            if let Some(client) = self.client_mut(id) {
                client.drag = None;
                client.resize = None;
                client.report = None;
            }
            return Ok(());
        }
        // A click is a complete action, so it ends a pending leader.
        let mut left_leader = false;
        if let Some(client) = self.client_mut(id)
            && matches!(client.input, InputState::Leader(_))
        {
            client.input = InputState::Normal;
            left_leader = true;
        }
        if left_leader {
            self.dirty = true;
        }
        if self.in_overlay(id) {
            if event.action == MouseAction::Press
                && event.button == MouseButton::Left
                && let Some(index) = self.overlay_item_at(id, event.col, event.row)
            {
                self.overlay_click(id, index);
            } else if let Some(client) = self.client_mut(id) {
                client.drag = None;
                client.resize = None;
                client.report = None;
            }
            return Ok(());
        }
        // While shown, the floating pane covers the tiled ones and answers
        // first. A press on its box, frame included, starts a selection there;
        // a press beside it takes the float down and belongs to the pane below.
        // A drag that a press already started keeps its pane either way.
        if self.continue_drag(id, event) {
            return Ok(());
        }
        if event.action == MouseAction::Press {
            if let Some((pane, local_col, local_row)) = self.float_cell(event.col, event.row) {
                match event.button {
                    MouseButton::Left => self.start_drag(id, pane, local_col, local_row),
                    MouseButton::WheelUp | MouseButton::WheelDown => {
                        if let Some(rect) = self.pane_rect(pane) {
                            self.wheel(pane, rect, event);
                        }
                    }
                    _ => {}
                }
                return Ok(());
            }
            if event.button == MouseButton::Left {
                self.hide_float_if_shown();
            }
        }
        if event.action == MouseAction::Press
            && event.button == MouseButton::Left
            && let Some(index) = self.status_tab_at(id, event.col, event.row)
        {
            self.select_tab(index);
            return Ok(());
        }
        let held_resize = self.client_mut(id).and_then(|client| client.resize.take());
        if let Some(resize) = held_resize {
            match event.action {
                MouseAction::Motion => {
                    self.resize_motion(id, resize, event);
                    return Ok(());
                }
                MouseAction::Release => {
                    self.resize_release(id, resize, event);
                    return Ok(());
                }
                MouseAction::Press => {}
            }
        }
        // The gutter takes the press: no focus change, no report, no selection.
        if event.action == MouseAction::Press
            && event.button == MouseButton::Left
            && let Some((path, area, dir)) = self.gutter_at(event.col, event.row)
        {
            if let Some(client) = self.client_mut(id) {
                client.drag = None;
                client.report = None;
                client.resize_value = None;
                client.resize = Some(Resize { path, area, dir });
            }
            return Ok(());
        }
        // A frame cell has no pane of its own, so it starts the selection at the
        // edge cell of the pane it touches instead. A frame press never resizes
        // and never moves the focus.
        if event.action == MouseAction::Press
            && event.button == MouseButton::Left
            && let Some((pane_id, local_col, local_row)) = self.frame_at(event.col, event.row)
        {
            self.start_drag(id, pane_id, local_col, local_row);
            return Ok(());
        }
        if self.continue_drag(id, event) {
            return Ok(());
        }

        // While a button is down, an app keeps receiving its drag even when
        // the pointer leaves the pane.
        let reported = self.client_mut(id).and_then(|client| client.report);
        if let Some(pane_id) = reported
            && event.action != MouseAction::Press
        {
            self.report_continuation(id, pane_id, event);
            return Ok(());
        }
        if event.action == MouseAction::Press
            && !matches!(event.button, MouseButton::WheelUp | MouseButton::WheelDown)
        {
            if let Some(client) = self.client_mut(id) {
                client.report = None;
            }
        }

        let Some((pane, rect)) = self.pane_at(event.col, event.row) else {
            return Ok(());
        };
        match event.button {
            MouseButton::WheelUp | MouseButton::WheelDown => self.wheel(pane, rect, event),
            _ => self.button(id, pane, rect, event),
        }
        Ok(())
    }

    pub fn tick_mouse(&mut self) {
        let pending: Vec<Pending> = self
            .clients
            .iter()
            .filter_map(|client| {
                let drag = client.drag.as_ref()?;
                (drag.edge != 0).then_some(Pending {
                    client: client.id,
                    pane: drag.pane,
                    anchor: drag.anchor,
                    started: drag.started,
                    edge: drag.edge,
                    col: drag.col,
                })
            })
            .collect();
        for mut step in pending {
            let Some(rect) = self.pane_rect(step.pane) else {
                if let Some(client) = self.client_mut(step.client) {
                    client.drag = None;
                }
                continue;
            };
            let lines = self.scroll_lines.max(1) as i32;
            let Some(pane) = self.pane_by_id_mut(step.pane) else {
                continue;
            };
            // A positive delta moves the view into the history.
            let before = pane.display_offset();
            let delta = if step.edge < 0 { 1 } else { -1 };
            pane.scroll_display(delta * lines);
            let moved = pane.display_offset() != before;
            let was_started = step.started;
            if moved || !was_started {
                let row = if step.edge < 0 {
                    0
                } else {
                    rect.rows.saturating_sub(1)
                };
                let point = pane.grid_point(row, step.col);
                pane.select(step.anchor, point);
                step.started = true;
                if let Some(client) = self.client_mut(step.client)
                    && let Some(drag) = client.drag.as_mut()
                {
                    drag.started = true;
                }
                self.dirty = true;
            }
        }
        self.tick_resize();
    }

    /// Continue a drag this client already started. True when the event was used.
    fn continue_drag(&mut self, id: u64, event: MouseInput) -> bool {
        let Some(drag) = self.client_mut(id).and_then(|client| client.drag.take()) else {
            return false;
        };
        match event.action {
            MouseAction::Motion => self.drag_motion(id, drag, event),
            MouseAction::Release => self.drag_release(id, drag, event),
            MouseAction::Press => return false,
        }
        true
    }

    fn frame_at(&self, col: u16, row: u16) -> Option<(u64, u16, u16)> {
        let geometry = self.active_geometry()?;
        if !geometry.on_frame(col, row) {
            return None;
        }
        let focus = self.active_tab()?.focus;
        let candidates = geometry.panes_next_to(col, row);
        let (id, rect) = candidates
            .iter()
            .find(|(id, _)| *id == focus)
            .or_else(|| candidates.first())?;
        Some((
            *id,
            axis(rect.x, rect.cols, col),
            axis(rect.y, rect.rows, row),
        ))
    }

    fn gutter_at(&self, col: u16, row: u16) -> Option<(Vec<bool>, Rect, Dir)> {
        let geometry = self.active_geometry()?;
        let gutter = geometry.gutter_at(col, row)?;
        Some((gutter.path.clone(), gutter.area, gutter.dir))
    }

    fn report_continuation(&mut self, id: u64, pane_id: u64, event: MouseInput) {
        let Some(rect) = self.pane_rect(pane_id) else {
            if let Some(client) = self.client_mut(id) {
                client.report = None;
            }
            return;
        };
        let (local_col, local_row) = local_cell(rect, event);
        if let Some(pane) = self.pane_by_id_mut(pane_id)
            && pane.mouse_reporting()
            && let Some(bytes) = encode_mouse(pane.mode(), event, local_col, local_row)
        {
            let _ = pane.write(&bytes);
        }
        if event.action == MouseAction::Release {
            if let Some(client) = self.client_mut(id) {
                client.report = None;
            }
        }
    }

    fn button(&mut self, id: u64, pane_id: u64, rect: Rect, event: MouseInput) {
        if event.action == MouseAction::Press {
            self.focus_pane(pane_id);
        }
        let (local_col, local_row) = local_cell(rect, event);
        if event.button == MouseButton::Left {
            // A plain drag always selects, even over an app that tracks the
            // mouse. Only a click without motion is handed back to the app.
            if event.action == MouseAction::Press {
                self.start_drag(id, pane_id, local_col, local_row);
            }
            return;
        }
        let Some(pane) = self.pane_by_id_mut(pane_id) else {
            return;
        };
        if !pane.mouse_reporting() {
            return;
        }
        if let Some(bytes) = encode_mouse(pane.mode(), event, local_col, local_row) {
            let _ = pane.write(&bytes);
        }
        if event.action == MouseAction::Press {
            if let Some(client) = self.client_mut(id) {
                client.report = Some(pane_id);
            }
        }
    }

    fn wheel(&mut self, pane_id: u64, rect: Rect, event: MouseInput) {
        let lines = self.scroll_lines;
        let upward = event.button == MouseButton::WheelUp;
        let (local_col, local_row) = local_cell(rect, event);
        let Some(pane) = self.pane_by_id_mut(pane_id) else {
            return;
        };
        if pane.mouse_reporting() {
            if let Some(bytes) = encode_mouse(pane.mode(), event, local_col, local_row) {
                let _ = pane.write(&bytes);
            }
            return;
        }
        if pane.alt_screen() {
            let key: &[u8] = match (upward, pane.app_cursor()) {
                (true, true) => b"\x1bOA",
                (false, true) => b"\x1bOB",
                (true, false) => b"\x1b[A",
                (false, false) => b"\x1b[B",
            };
            let mut bytes = Vec::with_capacity(key.len() * lines);
            for _ in 0..lines {
                bytes.extend_from_slice(key);
            }
            let _ = pane.write(&bytes);
            return;
        }
        let delta = if upward {
            lines as i32
        } else {
            -(lines as i32)
        };
        pane.scroll_display(delta);
        self.dirty = true;
    }

    fn focus_pane(&mut self, id: u64) {
        if let Some(tab) = self.active_tab_mut()
            && tab.focus != id
        {
            tab.set_focus(id);
            self.dirty = true;
        }
    }

    fn pane_at(&self, col: u16, row: u16) -> Option<(u64, Rect)> {
        self.active_geometry()?.pane_at(col, row)
    }

    fn float_cell(&self, col: u16, row: u16) -> Option<(u64, u16, u16)> {
        self.active_geometry()?.float_cell(col, row)
    }

    fn hide_float_if_shown(&mut self) {
        if self.float_shown() {
            self.hide_float();
        }
    }

    pub(super) fn pane_rect(&self, id: u64) -> Option<Rect> {
        self.active_geometry()?.pane_rect(id)
    }
}

fn mouse_allowed(session: &Session, id: u64) -> bool {
    session
        .clients
        .iter()
        .find(|client| client.id == id)
        .is_some_and(|client| {
            matches!(
                client.input,
                InputState::Normal | InputState::Leader(_) | InputState::Overlay
            )
        })
}

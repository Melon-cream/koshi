//! Hit testing for status-bar tabs and workspace-overlay rows.
//!
//! The client draws from the same status text and overlay geometry, so a click
//! is resolved from the server's copy of that geometry instead of from cells
//! echoed back by the terminal.

use crate::overlay_geom;
use crate::server::input::InputState;
use crate::server::session::Session;

impl Session {
    /// True while the workspace list is open for this client.
    pub(super) fn in_overlay(&self, id: u64) -> bool {
        self.clients
            .iter()
            .find(|client| client.id == id)
            .is_some_and(|client| client.input == InputState::Overlay)
    }

    /// Index of the tab whose status-bar label contains `col`, if any.
    pub(super) fn status_tab_at(&self, id: u64, col: u16, row: u16) -> Option<usize> {
        if row != self.screen.rows.saturating_sub(1) || col >= self.screen.cols {
            return None;
        }
        let client = self.clients.iter().find(|client| client.id == id)?;
        if !matches!(client.input, InputState::Normal | InputState::Leader(_)) {
            return None;
        }
        let workspace = self.workspaces.get(self.active)?;
        let (base, _) = self.base_status();
        let line = super::frames::status_line(&base, &client.input, self.screen.cols, None).line;
        let prefix = line.find(&base)?;
        let start =
            line[..prefix].chars().count() + workspace.name.chars().count() + " | ".chars().count();
        let (_, ranges) = super::frames::tab_labels(workspace);
        ranges
            .iter()
            .position(|(from, to)| (start + from..start + to).contains(&(col as usize)))
    }

    /// Item index for a click on a drawn workspace row.
    pub(super) fn overlay_item_at(&self, id: u64, col: u16, row: u16) -> Option<usize> {
        let client = self.clients.iter().find(|client| client.id == id)?;
        if client.input != InputState::Overlay {
            return None;
        }
        let entries = self.workspace_entries();
        let overlay = super::frames::build_overlay(&entries, self.active, client)?;
        overlay_geom::geometry(self.content_area(), &overlay)?.item_at(col, row)
    }

    pub(super) fn overlay_click(&mut self, id: u64, index: usize) {
        if let Some(client) = self.client_mut(id) {
            client.overlay_selected = index;
        }
        self.overlay_switch(id);
    }

    pub(super) fn select_tab(&mut self, index: usize) {
        let Some(workspace) = self.workspaces.get_mut(self.active) else {
            return;
        };
        if index >= workspace.tabs.len() || workspace.active == index {
            return;
        }
        workspace.active = index;
        self.resize_active();
        self.invalidate();
    }
}

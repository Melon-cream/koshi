//! Workspace list and the inline name input.

use crate::server::input::{InputState, TextTarget};
use crate::server::session::Session;

impl Session {
    pub(super) fn overlay_move(&mut self, id: u64, delta: isize) {
        let count = self.workspaces.len();
        if count == 0 {
            return;
        }
        if let Some(client) = self.client_mut(id) {
            let current = client.overlay_selected as isize;
            client.overlay_selected = (current + delta).rem_euclid(count as isize) as usize;
        }
        self.dirty = true;
    }

    pub(super) fn overlay_switch(&mut self, id: u64) {
        let selected = self
            .client_mut(id)
            .map(|client| client.overlay_selected)
            .unwrap_or(0);
        if selected < self.workspaces.len() {
            self.active = selected;
            self.resize_active();
            self.invalidate();
        }
        if let Some(client) = self.client_mut(id) {
            client.input = InputState::Normal;
        }
        self.dirty = true;
    }

    pub(super) fn overlay_delete(&mut self, id: u64) {
        if self.workspaces.len() <= 1 {
            return;
        }
        let selected = self
            .client_mut(id)
            .map(|client| client.overlay_selected)
            .unwrap_or(0);
        if selected >= self.workspaces.len() {
            return;
        }
        let mut workspace = self.workspaces.remove(selected);
        for tab in &mut workspace.tabs {
            for pane in &mut tab.panes {
                pane.kill();
            }
        }
        let count = self.workspaces.len();
        if self.active > selected && self.active > 0 {
            self.active -= 1;
        } else if self.active >= count {
            self.active = count - 1;
        }
        let next = selected.min(count - 1);
        if let Some(client) = self.client_mut(id) {
            client.overlay_selected = next;
        }
        self.resize_active();
        self.invalidate();
    }

    pub(super) fn commit_text(&mut self, id: u64, target: TextTarget, name: String) {
        match target {
            TextTarget::Tab => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.title = name;
                }
                if let Some(client) = self.client_mut(id) {
                    client.input = InputState::Normal;
                }
            }
            TextTarget::Workspace(index) => {
                if let Some(workspace) = self.workspaces.get_mut(index) {
                    workspace.name = name;
                }
                if let Some(client) = self.client_mut(id) {
                    client.input = InputState::Overlay;
                }
            }
        }
        self.dirty = true;
    }

    pub(super) fn cancel_text(&mut self, id: u64, target: TextTarget) {
        let next = match target {
            TextTarget::Tab => InputState::Normal,
            TextTarget::Workspace(_) => InputState::Overlay,
        };
        if let Some(client) = self.client_mut(id) {
            client.input = next;
        }
        self.dirty = true;
    }
}

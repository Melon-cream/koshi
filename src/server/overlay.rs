//! Workspace list, the inline name input, and the workspace settings screen.

use std::process::{Command, Stdio};

use crate::server::input::{AliasCheck, InputState, TextTarget};
use crate::server::session::Session;
use crate::server::tab::Workspace;

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

    /// Open the settings screen of the workspace selected in the list.
    pub(super) fn open_settings(&mut self, id: u64) {
        let selected = self
            .client_mut(id)
            .map(|client| client.overlay_selected)
            .unwrap_or(0);
        let Some(workspace) = self.workspaces.get(selected) else {
            return;
        };
        let workspace_id = workspace.id;
        if let Some(client) = self.client_mut(id) {
            client.input = InputState::Settings {
                workspace: workspace_id,
                selected: 0,
                check: None,
            };
        }
        self.dirty = true;
    }

    /// Start editing the one item of the settings screen.
    pub(super) fn settings_edit(&mut self, id: u64) {
        let Some(workspace) = self.client_mut(id).and_then(|client| match client.input {
            InputState::Settings { workspace, .. } => Some(workspace),
            _ => None,
        }) else {
            return;
        };
        let buffer = self
            .workspace_by_id(workspace)
            .and_then(|workspace| workspace.ssh_alias.clone())
            .unwrap_or_default();
        if let Some(client) = self.client_mut(id) {
            client.input = InputState::Text {
                target: TextTarget::SshAlias(workspace),
                buffer,
            };
        }
        self.dirty = true;
    }

    pub(super) fn close_settings(&mut self, id: u64) {
        if let Some(client) = self.client_mut(id) {
            client.input = InputState::Overlay;
        }
        self.dirty = true;
    }

    /// The workspace the settings screen belongs to.
    pub(super) fn workspace_by_id(&self, id: u64) -> Option<&Workspace> {
        self.workspaces.iter().find(|workspace| workspace.id == id)
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
            TextTarget::SshAlias(workspace_id) => {
                let alias = (!name.is_empty()).then_some(name);
                if let Some(workspace) = self
                    .workspaces
                    .iter_mut()
                    .find(|workspace| workspace.id == workspace_id)
                {
                    workspace.ssh_alias.clone_from(&alias);
                }
                // A failed check is only a warning; the alias stays saved.
                let check = alias.map(|alias| AliasCheck {
                    ok: alias_reaches_ssh(&alias),
                    alias,
                });
                if let Some(client) = self.client_mut(id) {
                    client.input = InputState::Settings {
                        workspace: workspace_id,
                        selected: 0,
                        check,
                    };
                }
            }
        }
        self.dirty = true;
    }

    pub(super) fn cancel_text(&mut self, id: u64, target: TextTarget) {
        let next = match target {
            TextTarget::Tab => InputState::Normal,
            TextTarget::Workspace(_) => InputState::Overlay,
            TextTarget::SshAlias(workspace) => InputState::Settings {
                workspace,
                selected: 0,
                check: None,
            },
        };
        if let Some(client) = self.client_mut(id) {
            client.input = next;
        }
        self.dirty = true;
    }
}

/// Ask `ssh` whether it can resolve the alias. Output is dropped: it can hold
/// the expanded configuration and is not stored or logged.
fn alias_reaches_ssh(alias: &str) -> bool {
    Command::new("ssh")
        .arg("-G")
        .arg(alias)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

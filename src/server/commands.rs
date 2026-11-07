//! Input entry points and the command dispatch.

use anyhow::Result;

use crate::protocol::{KeyCode, ServerMsg};
use crate::server::input::{Command, InputState, TextTarget};
use crate::server::keymap::key_bytes;
use crate::server::session::Session;

impl Session {
    pub fn handle_input(&mut self, id: u64, bytes: &[u8]) -> Result<()> {
        // A lone Esc dismisses the floating pane unless the program inside it
        // keeps Esc for itself (vim, less and friends run on the alternate
        // screen).
        if bytes == [0x1b] && self.dismiss_float_on_escape(id) {
            return Ok(());
        }
        let leader = self.leader;
        let float = self.float_shown();
        let mut pane_bytes = Vec::new();
        let mut commands = Vec::new();
        let mut state_changed = false;
        if let Some(client) = self.client_mut(id) {
            let before = client.input.clone();
            client
                .input
                .feed(bytes, leader, float, &mut pane_bytes, &mut commands);
            state_changed = client.input != before;
        }
        if !pane_bytes.is_empty() {
            self.write_focused(&pane_bytes);
            self.dirty = true;
            self.pending_save = true;
        }
        self.finish_input(id, state_changed, commands)
    }

    pub fn handle_key(&mut self, id: u64, key: KeyCode) -> Result<()> {
        let float = self.float_shown();
        let mut keys = Vec::new();
        let mut commands = Vec::new();
        let mut state_changed = false;
        if let Some(client) = self.client_mut(id) {
            let before = client.input.clone();
            client.input.feed_key(key, float, &mut keys, &mut commands);
            state_changed = client.input != before;
        }
        for key in keys {
            self.write_key(key);
        }
        self.finish_input(id, state_changed, commands)
    }

    pub(super) fn finish_input(
        &mut self,
        id: u64,
        state_changed: bool,
        commands: Vec<Command>,
    ) -> Result<()> {
        if state_changed || !commands.is_empty() {
            self.pending_save = true;
        }
        let mut first_error = None;
        for command in commands {
            if let Err(err) = self.apply(id, command) {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
        if state_changed {
            self.dirty = true;
        }
        match first_error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    pub(super) fn dismiss_float_on_escape(&mut self, id: u64) -> bool {
        let normal = self
            .clients
            .iter()
            .find(|client| client.id == id)
            .is_some_and(|client| matches!(client.input, InputState::Normal));
        if !normal {
            return false;
        }
        let Some(tab) = self.active_tab() else {
            return false;
        };
        if !tab.float_shown {
            return false;
        }
        let Some(pane) = tab.floating.as_ref() else {
            return false;
        };
        if pane.alt_screen() {
            return false;
        }
        self.hide_float();
        self.dirty = true;
        self.pending_save = true;
        true
    }

    pub(super) fn write_focused(&mut self, bytes: &[u8]) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let typing = tab.typing_pane();
        if let Some(pane) = tab.pane_mut(typing) {
            let _ = pane.write(bytes);
            // Typing always returns the view to the live end.
            pane.focus_live();
        }
    }

    pub(super) fn write_key(&mut self, key: KeyCode) {
        let mut wrote = false;
        if let Some(tab) = self.active_tab_mut() {
            let typing = tab.typing_pane();
            if let Some(pane) = tab.pane_mut(typing) {
                let bytes = key_bytes(key, pane.app_cursor());
                wrote = pane.write(bytes).is_ok();
                pane.focus_live();
            }
        }
        if wrote {
            self.dirty = true;
            self.pending_save = true;
        }
    }

    pub(super) fn apply(&mut self, id: u64, command: Command) -> Result<()> {
        match command {
            Command::Detach => {
                if let Some(client) = self.client_mut(id) {
                    let _ = client.tx.send(ServerMsg::Detached);
                }
                self.remove_client(id);
            }
            Command::Split(dir) => self.split(dir)?,
            Command::Focus(side) => {
                self.focus_side(side);
            }
            Command::FocusOrTab(side) => self.focus_or_tab(side),
            Command::FocusNext => self.focus_next(),
            Command::FocusLast => self.focus_last(),
            Command::ClosePane => {
                if self.float_shown() {
                    self.close_float();
                } else if let Some(focus) = self.active_tab().map(|tab| tab.focus) {
                    self.close_pane(focus);
                }
            }
            Command::ToggleZoom => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.zoomed = !tab.zoomed;
                }
                self.resize_active();
                self.invalidate();
            }
            Command::ToggleFloat => self.toggle_float()?,
            Command::NewTab => self.new_tab()?,
            Command::CloseTab => self.close_tab(),
            Command::NextTab => self.switch_tab(1),
            Command::PrevTab => self.switch_tab(-1),
            Command::GotoTab(index) => self.goto_tab(index),
            Command::LastTab => self.last_tab(),
            Command::BeginTabRename => {
                if let Some(client) = self.client_mut(id) {
                    client.input = InputState::Text {
                        target: TextTarget::Tab,
                        buffer: String::new(),
                    };
                }
                self.dirty = true;
            }
            Command::RenameWorkspace => {
                let selected = self.active;
                if let Some(client) = self.client_mut(id) {
                    client.overlay_selected = selected;
                    client.input = InputState::Text {
                        target: TextTarget::Workspace(selected),
                        buffer: String::new(),
                    };
                }
                self.dirty = true;
            }
            Command::OpenWorkspaces => {
                let selected = self.active;
                if let Some(client) = self.client_mut(id) {
                    client.input = InputState::Overlay;
                    client.overlay_selected = selected;
                }
                self.dirty = true;
            }
            Command::ScrollLines(delta) => self.scroll_lines(delta),
            Command::ScrollPages(pages) => self.scroll_pages(pages, false),
            Command::ScrollHalfPages(pages) => self.scroll_pages(pages, true),
            Command::ScrollBottom => self.scroll_bottom(),
            Command::OverlayUp => self.overlay_move(id, -1),
            Command::OverlayDown => self.overlay_move(id, 1),
            Command::OverlaySwitch => self.overlay_switch(id),
            Command::OverlayCreate => {
                let index = self.create_workspace()?;
                if let Some(client) = self.client_mut(id) {
                    client.overlay_selected = index;
                }
            }
            Command::OverlayBeginRename => {
                let index = self
                    .client_mut(id)
                    .map(|client| client.overlay_selected)
                    .unwrap_or(0);
                if let Some(client) = self.client_mut(id) {
                    client.input = InputState::Text {
                        target: TextTarget::Workspace(index),
                        buffer: String::new(),
                    };
                }
                self.dirty = true;
            }
            Command::OverlayDelete => self.overlay_delete(id),
            Command::OverlaySettings => self.open_settings(id),
            Command::SettingsUp => self.settings_move(id, -1),
            Command::SettingsDown => self.settings_move(id, 1),
            Command::SettingsEdit => self.settings_edit(id),
            Command::CloseSettings => self.close_settings(id),
            Command::CloseOverlay => {
                if let Some(client) = self.client_mut(id) {
                    client.input = InputState::Normal;
                }
                self.dirty = true;
            }
            Command::CommitText(target, name) => self.commit_text(id, target, name),
            Command::CancelText(target) => self.cancel_text(id, target),
        }
        Ok(())
    }
}

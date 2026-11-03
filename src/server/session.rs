//! Session state: clients, workspaces, and the operations that change them.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::sync::mpsc::UnboundedSender;

use crate::config::Leader;
use crate::protocol::{BorderCell, CellData, CursorPos, Overlay, Rect, ServerMsg, Status};
use crate::server::geom::Geometry;
use crate::server::input::InputState;
use crate::server::pane::{Pane, PaneSignal};
use crate::server::resize::Resize;
use crate::server::select::Drag;
use crate::server::state;
use crate::server::tab::{Size, Tab, Toast, Workspace, resize_tab};

/// How long a toast stays on screen.
const TOAST_HOLD: Duration = Duration::from_secs(2);

pub(super) struct Client {
    pub(super) id: u64,
    pub(super) tx: UnboundedSender<ServerMsg>,
    pub(super) ready: bool,
    pub(super) fresh: bool,
    pub(super) input: InputState,
    pub(super) overlay_selected: usize,
    pub(super) panes: HashMap<u64, Vec<Vec<CellData>>>,
    pub(super) layout: Vec<(u64, Rect)>,
    pub(super) status: Option<Status>,
    pub(super) overlay: Option<Overlay>,
    /// Last border cells and toast sent, so a change alone is a reason to send.
    pub(super) borders: Vec<BorderCell>,
    pub(super) toast: Option<String>,
    pub(super) cursor: Option<(u64, Option<CursorPos>)>,
    pub(super) drag: Option<Drag>,
    /// Split gutter held by this client's left button.
    pub(super) resize: Option<Resize>,
    /// Size the held drag asks for but the tick has not applied yet.
    pub(super) resize_value: Option<u16>,
    /// Pane that received the button press while it was reporting mouse events.
    pub(super) report: Option<u64>,
}

pub(super) struct Session {
    pub(super) workspaces: Vec<Workspace>,
    pub(super) active: usize,
    pub(super) clients: Vec<Client>,
    pub(super) dirty: bool,
    /// Something worth persisting changed since the last save.
    pub(super) pending_save: bool,
    /// One line for the first client, e.g. an unreadable state file.
    pub(super) startup_notice: Option<String>,
    pub(super) toast: Option<Toast>,
    pub(super) history: usize,
    pub(super) scroll_lines: usize,
    pub(super) signals: UnboundedSender<PaneSignal>,
    pub(super) screen: Size,
    pub(super) next_pane: u64,
    pub(super) leader: Leader,
}

impl Session {
    pub fn new(
        history: usize,
        scroll_lines: usize,
        signals: UnboundedSender<PaneSignal>,
        leader: Leader,
        cols: u16,
        rows: u16,
    ) -> Result<Self> {
        let loaded = state::load();
        let mut session = Self {
            workspaces: Vec::new(),
            active: 0,
            clients: Vec::new(),
            dirty: true,
            pending_save: false,
            startup_notice: loaded.warning,
            toast: None,
            history,
            scroll_lines: scroll_lines.max(1),
            signals,
            screen: Size {
                cols: cols.max(1),
                rows: rows.max(2),
            },
            next_pane: 1,
            leader,
        };
        let restored = match loaded.state {
            Some(saved) => session.restore(saved),
            None => false,
        };
        if !restored {
            let area = session.content_area();
            let tab = session.build_tab("shell", area)?;
            session.workspaces.push(Workspace {
                name: "main".to_string(),
                tabs: vec![tab],
                active: 0,
                previous_tab: None,
            });
        }
        Ok(session)
    }

    pub(super) fn content_area(&self) -> Rect {
        Rect {
            x: 0,
            y: 0,
            cols: self.screen.cols.max(1),
            rows: self.screen.rows.saturating_sub(1).max(1),
        }
    }

    pub(super) fn active_tab(&self) -> Option<&Tab> {
        let workspace = self.workspaces.get(self.active)?;
        workspace.tabs.get(workspace.active)
    }

    pub(super) fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        let workspace = self.workspaces.get_mut(self.active)?;
        let active = workspace.active;
        workspace.tabs.get_mut(active)
    }

    pub(super) fn active_view(&self) -> Option<(Vec<(u64, Rect)>, u64)> {
        let tab = self.active_tab()?;
        Some((tab.rects(self.content_area()), tab.typing_pane()))
    }

    pub(super) fn active_geometry(&self) -> Option<Geometry> {
        let tab = self.active_tab()?;
        Some(tab.geometry(self.content_area()))
    }

    pub fn add_client(&mut self, id: u64, tx: UnboundedSender<ServerMsg>) {
        if let Some(text) = self.startup_notice.take() {
            let _ = tx.send(ServerMsg::Notice(text));
        }
        self.clients.push(Client {
            id,
            tx,
            ready: false,
            fresh: true,
            input: InputState::new(),
            overlay_selected: self.active,
            panes: HashMap::new(),
            layout: Vec::new(),
            status: None,
            overlay: None,
            borders: Vec::new(),
            toast: None,
            cursor: None,
            drag: None,
            resize: None,
            resize_value: None,
            report: None,
        });
    }

    pub fn remove_client(&mut self, id: u64) {
        // A drag that ends with its client must still leave the ptys at the size
        // the border shows, or the layout stays ahead of every shell.
        let pending = self
            .clients
            .iter()
            .find(|client| client.id == id)
            .is_some_and(|client| client.resize.is_some() && client.resize_value.is_some());
        self.clients.retain(|client| client.id != id);
        if pending {
            self.resize_active();
            self.invalidate();
        }
    }

    pub(super) fn client_mut(&mut self, id: u64) -> Option<&mut Client> {
        self.clients.iter_mut().find(|client| client.id == id)
    }

    pub fn notice(&mut self, id: u64, text: String) {
        if let Some(client) = self.client_mut(id) {
            let _ = client.tx.send(ServerMsg::Notice(text));
        }
    }

    /// Returns false when the client protocol version is incompatible.
    pub fn attach(&mut self, id: u64, version: u16, cols: u16, rows: u16) -> bool {
        if version != crate::protocol::PROTOCOL_VERSION {
            if let Some(client) = self.client_mut(id) {
                let _ = client.tx.send(ServerMsg::Notice(format!(
                    "the running server is protocol {}; this client is protocol {version}. \
                     Run `koshi kill` and attach again.",
                    crate::protocol::PROTOCOL_VERSION
                )));
            }
            self.remove_client(id);
            return false;
        }
        self.resize(cols, rows);
        for workspace in &mut self.workspaces {
            for tab in &mut workspace.tabs {
                for pane in tab.panes.iter_mut().chain(tab.floating.as_mut()) {
                    pane.arm_clear_if_restored();
                }
            }
        }
        if let Some(client) = self.client_mut(id) {
            client.ready = true;
            client.fresh = true;
            client.panes.clear();
            client.layout.clear();
            client.status = None;
            client.overlay = None;
            client.borders.clear();
            client.toast = None;
            client.cursor = None;
            client.drag = None;
            client.resize = None;
            client.resize_value = None;
            client.report = None;
        }
        self.dirty = true;
        true
    }

    pub(super) fn resize_active(&mut self) {
        let area = self.content_area();
        if let Some(tab) = self.active_tab_mut() {
            resize_tab(tab, area);
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        if cols == 0 || rows == 0 {
            return;
        }
        self.screen = Size {
            cols,
            rows: rows.max(2),
        };
        self.resize_active();
        self.invalidate();
    }

    pub fn output(&mut self, pane_id: u64, bytes: &[u8]) {
        if let Some(pane) = self.pane_by_id_mut(pane_id) {
            pane.process(bytes);
            self.dirty = true;
            self.pending_save = true;
        }
    }

    pub fn pane_by_id_mut(&mut self, id: u64) -> Option<&mut Pane> {
        for workspace in &mut self.workspaces {
            for tab in &mut workspace.tabs {
                if let Some(pane) = tab.pane_mut(id) {
                    return Some(pane);
                }
            }
        }
        None
    }

    pub fn is_empty(&self) -> bool {
        self.workspaces
            .iter()
            .flat_map(|workspace| &workspace.tabs)
            .all(|tab| tab.panes.is_empty())
    }

    pub(super) fn invalidate(&mut self) {
        self.dirty = true;
        self.pending_save = true;
        for client in &mut self.clients {
            client.panes.clear();
            client.layout.clear();
        }
    }

    pub fn shutdown(&mut self) {
        self.broadcast(ServerMsg::Exited);
        self.clients.clear();
    }

    pub fn broadcast(&self, msg: ServerMsg) {
        for client in &self.clients {
            let _ = client.tx.send(msg.clone());
        }
    }

    pub(super) fn show_toast(&mut self, text: String) {
        self.toast = Some(Toast {
            text,
            until: Instant::now() + TOAST_HOLD,
        });
        self.dirty = true;
    }

    /// Drop an expired toast. Called from the server tick.
    pub fn poll_toast(&mut self) {
        if self
            .toast
            .as_ref()
            .is_some_and(|toast| Instant::now() >= toast.until)
        {
            self.toast = None;
            self.dirty = true;
        }
    }

    /// Send the one Ctrl-L each restored pane owes after the first attach.
    /// Called from the server tick so the shell's startup output comes first.
    pub fn poll_restored(&mut self) {
        for workspace in &mut self.workspaces {
            for tab in &mut workspace.tabs {
                for pane in tab.panes.iter_mut().chain(tab.floating.as_mut()) {
                    pane.send_pending_clear();
                }
            }
        }
    }

    pub(super) fn toast_text(&self) -> Option<String> {
        self.toast.as_ref().map(|toast| toast.text.clone())
    }
}

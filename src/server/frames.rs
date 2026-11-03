//! Frame production: per-client diffs, the status bar, and the workspace overlay.

use crate::protocol::{
    BorderCell, CellData, CursorPos, Frame, Overlay, PaneFrame, Rect, RowUpdate, ServerMsg, Status,
};
use crate::server::borders;
use crate::server::input::{InputState, TextTarget};
use crate::server::session::{Client, Session};
use crate::server::tab::Workspace;

struct PaneSnapshot {
    id: u64,
    rect: Rect,
    cols: u16,
    rows: u16,
    cells: Vec<Vec<CellData>>,
    cursor: Option<CursorPos>,
}

impl Session {
    pub fn flush(&mut self) {
        if !self.dirty || self.clients.is_empty() {
            return;
        }
        let Some((rects, focus)) = self.active_view() else {
            self.dirty = false;
            return;
        };
        let snapshots = self.snapshots(&rects, focus);
        let area = self.content_area();
        let (base, active_tab) = self.base_status();
        let entries = self.workspace_entries();
        let borders = self.border_cells();
        let toast = self.toast_text();

        for client in &mut self.clients {
            if !client.ready {
                continue;
            }
            let initial = client.fresh;
            let layout_changed = client.layout != rects;
            if layout_changed {
                client.layout = rects.clone();
                client.panes.clear();
            }
            let mut send = initial || layout_changed;
            let mut panes = Vec::with_capacity(snapshots.len());
            let mut cursor_slot = None;

            for snapshot in &snapshots {
                let cache = client
                    .panes
                    .entry(snapshot.id)
                    .or_insert_with(|| blank(snapshot.cols, snapshot.rows));
                let size_changed = cache.len() != snapshot.rows as usize
                    || cache.first().map(Vec::len) != Some(snapshot.cols as usize);
                if size_changed {
                    *cache = blank(snapshot.cols, snapshot.rows);
                }
                let full = initial || layout_changed || size_changed;
                let mut updates = Vec::new();
                for (row, cells) in snapshot.cells.iter().enumerate() {
                    if full || cache[row] != *cells {
                        cache[row].clone_from(cells);
                        updates.push(RowUpdate {
                            row: row as u16,
                            cells: cells.clone(),
                        });
                    }
                }
                if !updates.is_empty() {
                    send = true;
                }
                let focused = snapshot.id == focus;
                let cursor = if focused { snapshot.cursor } else { None };
                if focused {
                    cursor_slot = Some((focus, cursor));
                }
                panes.push(PaneFrame {
                    id: snapshot.id,
                    rect: snapshot.rect,
                    cols: snapshot.cols,
                    rows: snapshot.rows,
                    rows_changed: updates,
                    cursor,
                    focused,
                });
            }
            client
                .panes
                .retain(|id, _| rects.iter().any(|(visible, _)| visible == id));

            let status = status_line(&base, &client.input, area.cols, active_tab);
            if client.status.as_ref() != Some(&status) {
                client.status = Some(status.clone());
                send = true;
            }
            let overlay = build_overlay(&entries, self.active, client);
            if client.overlay != overlay {
                client.overlay = overlay.clone();
                send = true;
            }
            if client.borders != borders {
                client.borders.clone_from(&borders);
                send = true;
            }
            if client.toast != toast {
                client.toast.clone_from(&toast);
                send = true;
            }
            if client.cursor != cursor_slot {
                client.cursor = cursor_slot;
                send = true;
            }
            if !send {
                continue;
            }
            client.fresh = false;
            let frame = Frame {
                content: area,
                panes,
                borders: borders.clone(),
                status,
                overlay,
                toast: toast.clone(),
            };
            let _ = client.tx.send(ServerMsg::Frame(Box::new(frame)));
        }
        self.dirty = false;
    }

    fn snapshots(&self, rects: &[(u64, Rect)], focus: u64) -> Vec<PaneSnapshot> {
        let Some(tab) = self.active_tab() else {
            return Vec::new();
        };
        rects
            .iter()
            .filter_map(|(id, rect)| {
                tab.pane(*id).map(|pane| PaneSnapshot {
                    id: *id,
                    rect: *rect,
                    cols: pane.cols(),
                    rows: pane.rows(),
                    cells: pane.render_rows(),
                    // The border can move ahead of a pty that has not been resized
                    // yet, so a cursor outside the rect is dropped instead of drawn
                    // inside the neighbouring pane.
                    cursor: if *id == focus {
                        pane.cursor()
                            .filter(|cursor| cursor.col < rect.cols && cursor.row < rect.rows)
                    } else {
                        None
                    },
                })
            })
            .collect()
    }

    /// Border cells of the active tab; empty for a single pane and when zoomed.
    pub(super) fn border_cells(&self) -> Vec<BorderCell> {
        let Some(tab) = self.active_tab() else {
            return Vec::new();
        };
        borders::compute(&tab.geometry(self.content_area()), tab.focus)
    }

    /// The status text and the char range the active tab's label takes in it.
    pub(super) fn base_status(&self) -> (String, Option<(u16, u16)>) {
        let Some(workspace) = self.workspaces.get(self.active) else {
            return (String::new(), None);
        };
        let Some(tab) = workspace.tabs.get(workspace.active) else {
            return (workspace.name.clone(), None);
        };
        let (labels, ranges) = tab_labels(workspace);
        let mut line = format!("{} | {labels}", workspace.name);
        let offset = line.chars().count() - labels.chars().count();
        let active = ranges
            .get(workspace.active)
            .map(|(from, to)| ((from + offset) as u16, (to + offset) as u16));
        if tab.zoomed {
            line.push_str(" [ZOOM]");
        }
        if tab.float_shown {
            line.push_str(" [FLOAT]");
        }
        (line, active)
    }

    pub(super) fn workspace_entries(&self) -> Vec<(String, usize, usize)> {
        self.workspaces
            .iter()
            .map(|workspace| {
                let panes = workspace.tabs.iter().map(|tab| tab.panes.len()).sum();
                (workspace.name.clone(), workspace.tabs.len(), panes)
            })
            .collect()
    }
}

/// Status-bar labels in draw order, with the char range each occupies.
pub(super) fn tab_labels(workspace: &Workspace) -> (String, Vec<(usize, usize)>) {
    let mut labels = String::new();
    let mut ranges = Vec::with_capacity(workspace.tabs.len());
    let mut cursor = 0;
    for (number, tab) in workspace.tabs.iter().enumerate() {
        if number > 0 {
            labels.push(' ');
            cursor += 1;
        }
        let label = format!(
            "{}:{}{}",
            number + 1,
            tab.title,
            if number == workspace.active { "*" } else { "" }
        );
        let end = cursor + label.chars().count();
        ranges.push((cursor, end));
        cursor = end;
        labels.push_str(&label);
    }
    (labels, ranges)
}

pub(super) fn status_line(
    base: &str,
    input: &InputState,
    cols: u16,
    active: Option<(u16, u16)>,
) -> Status {
    let (label, hint) = match input {
        InputState::Normal => ("NORMAL", ""),
        InputState::Leader(kind) => (kind.label(), kind.hint()),
        InputState::Overlay => ("NORMAL", "workspace list"),
        InputState::Text {
            target: TextTarget::Tab,
            buffer,
        } => {
            return Status {
                line: format!("[RENAME] tab name: {buffer}_"),
                hint: None,
                active: None,
            };
        }
        InputState::Text {
            target: TextTarget::Workspace(_),
            buffer,
        } => {
            return Status {
                line: format!("[RENAME] workspace name: {buffer}_"),
                hint: None,
                active: None,
            };
        }
    };
    let head = format!("[{label}] {base}");
    let offset = head.chars().count() - base.chars().count();
    let active = active.map(|(from, to)| {
        (
            (from as usize + offset) as u16,
            (to as usize + offset) as u16,
        )
    });
    with_hint(head, hint, cols, active)
}

/// Append the group hints, trimming them so the base line always survives.
fn with_hint(head: String, hint: &str, cols: u16, active: Option<(u16, u16)>) -> Status {
    if hint.is_empty() {
        return Status {
            line: head,
            hint: None,
            active,
        };
    }
    let used = head.chars().count() + " | ".chars().count();
    let room = (cols as usize).saturating_sub(used);
    if room == 0 {
        return Status {
            line: head,
            hint: None,
            active,
        };
    }
    let hint: String = hint.chars().take(room).collect();
    Status {
        line: format!("{head} | {hint}"),
        hint: Some(hint),
        active,
    }
}

/// `None` unless the client is in the workspace overlay or renaming in it.
pub(super) fn build_overlay(
    entries: &[(String, usize, usize)],
    active: usize,
    client: &Client,
) -> Option<Overlay> {
    let open = matches!(
        client.input,
        InputState::Overlay
            | InputState::Text {
                target: TextTarget::Workspace(_),
                ..
            }
    );
    if !open {
        return None;
    }
    let items = entries
        .iter()
        .enumerate()
        .map(|(index, (name, tabs, panes))| {
            format!(
                "{} {:<16} {} tabs, {} panes{}",
                if index == client.overlay_selected {
                    ">"
                } else {
                    " "
                },
                name,
                tabs,
                panes,
                if index == active { " (current)" } else { "" }
            )
        })
        .collect();
    let prompt = match &client.input {
        InputState::Text {
            target: TextTarget::Workspace(_),
            buffer,
        } => Some(format!("rename: {buffer}_")),
        _ => None,
    };
    Some(Overlay {
        title: "Workspaces".to_string(),
        items,
        selected: client.overlay_selected,
        prompt,
        footer: "n new  r rename  d delete  Enter switch  Esc close".to_string(),
    })
}

fn blank(cols: u16, rows: u16) -> Vec<Vec<CellData>> {
    vec![vec![CellData::default(); cols as usize]; rows as usize]
}

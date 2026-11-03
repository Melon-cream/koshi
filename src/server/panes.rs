//! Pane split and close.

use anyhow::Result;

use crate::protocol::Rect;
use crate::server::layout::{Dir, Layout};
use crate::server::session::Session;

impl Session {
    pub(super) fn split(&mut self, dir: Dir) -> Result<()> {
        let area = self.content_area();
        let id = self.next_pane;
        self.next_pane += 1;
        let pane = self.spawn_pane(id, area)?;

        let active = self.active;
        let workspace = &mut self.workspaces[active];
        let tab_index = workspace.active;
        let tab = &mut workspace.tabs[tab_index];
        let focus = tab.focus;
        // The frame only appears once the second pane is there, so the size the
        // new split starts with belongs to the geometry after the split.
        let inner = tab.geometry_with(area, tab.panes.len() + 1).inner;
        let first_size = default_first_size(&tab.layout, inner, focus, dir);
        if !tab.layout.split(focus, id, dir, first_size) {
            return Ok(());
        }
        tab.panes.push(pane);
        tab.set_focus(id);
        tab.zoomed = false;
        self.resize_active();
        self.invalidate();
        Ok(())
    }

    pub(super) fn close_pane(&mut self, pane_id: u64) -> bool {
        let mut removed_workspace = None;
        'outer: for workspace_index in 0..self.workspaces.len() {
            let tab_count = self.workspaces[workspace_index].tabs.len();
            for tab_index in 0..tab_count {
                let tab = &mut self.workspaces[workspace_index].tabs[tab_index];
                // The floating pane is not part of the layout, so ending it only
                // takes the float down.
                if tab
                    .floating
                    .as_ref()
                    .is_some_and(|pane| pane.id() == pane_id)
                {
                    if let Some(mut pane) = tab.floating.take() {
                        pane.kill();
                    }
                    tab.float_shown = false;
                    break 'outer;
                }
                if !tab.panes.iter().any(|pane| pane.id() == pane_id) {
                    continue;
                }
                if let Some(pane) = tab.pane_mut(pane_id) {
                    pane.kill();
                }
                tab.panes.retain(|pane| pane.id() != pane_id);
                let empty = tab.panes.is_empty();
                if !empty {
                    tab.layout.remove(pane_id);
                    if tab.focus == pane_id {
                        let next = tab
                            .layout
                            .first_leaf()
                            .or_else(|| tab.panes.first().map(|pane| pane.id()))
                            .unwrap_or(pane_id);
                        tab.focus = next;
                        tab.previous_focus = None;
                    }
                    tab.zoomed = false;
                }
                if empty {
                    let workspace = &mut self.workspaces[workspace_index];
                    workspace.tabs.remove(tab_index);
                    if workspace.active > tab_index {
                        workspace.active -= 1;
                    } else if workspace.active >= workspace.tabs.len() && !workspace.tabs.is_empty()
                    {
                        workspace.active = workspace.tabs.len() - 1;
                    }
                    if workspace.tabs.is_empty() {
                        removed_workspace = Some(workspace_index);
                    }
                }
                break 'outer;
            }
        }
        if let Some(index) = removed_workspace {
            self.workspaces.remove(index);
            if self.workspaces.is_empty() {
                self.active = 0;
            } else if self.active >= self.workspaces.len() {
                self.active = self.workspaces.len() - 1;
            } else if self.active > index {
                self.active -= 1;
            }
        }
        self.resize_active();
        self.invalidate();
        self.is_empty()
    }

    pub(super) fn poll_panes(&mut self) -> bool {
        let mut exited = Vec::new();
        for workspace in &mut self.workspaces {
            for tab in &mut workspace.tabs {
                for pane in tab.panes.iter_mut().chain(tab.floating.as_mut()) {
                    if pane.try_exit() {
                        exited.push(pane.id());
                    }
                }
            }
        }
        for id in exited {
            self.close_pane(id);
        }
        self.is_empty()
    }
}

/// Half of the area the new split will occupy, so splitting changes nothing
/// else until the gutter is dragged.
fn default_first_size(layout: &Layout, area: Rect, focus: u64, dir: Dir) -> u16 {
    let mut rects = Vec::new();
    layout.rects(area, &mut rects);
    let size = rects
        .into_iter()
        .find(|(id, _)| *id == focus)
        .map(|(_, rect)| match dir {
            Dir::Columns => rect.cols,
            Dir::Rows => rect.rows,
        })
        .unwrap_or(0);
    (size / 2).max(1)
}

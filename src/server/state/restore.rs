//! Turning a saved state into live panes.
//!
//! A restored pane always starts a local shell: the SSH alias of a workspace is
//! put back, but its connections are not resumed.

use crate::server::launch::Launch;
use crate::server::pane::{Pane, PaneInit};
use crate::server::session::Session;
use crate::server::tab::{Tab, Workspace};

use super::{SavedState, SavedTab};

impl Session {
    /// Replace the session with the saved one. False when nothing was usable.
    pub(in crate::server) fn restore(&mut self, saved: SavedState) -> bool {
        let mut workspaces = Vec::new();
        for saved_workspace in saved.workspaces {
            let mut tabs = Vec::new();
            for saved_tab in saved_workspace.tabs {
                if let Some(tab) = self.restore_tab(saved_tab) {
                    tabs.push(tab);
                }
            }
            if tabs.is_empty() {
                if saved_workspace.ssh_alias.is_none() {
                    continue;
                }
                let area = self.content_area();
                let Ok(tab) = self.build_tab("shell", area, Launch::Shell { cwd: None }) else {
                    continue;
                };
                tabs.push(tab);
            }
            let active = saved_workspace.active.min(tabs.len() - 1);
            workspaces.push(Workspace {
                id: saved_workspace.id,
                name: saved_workspace.name,
                ssh_alias: saved_workspace.ssh_alias,
                tabs,
                active,
                previous_tab: None,
            });
        }
        if workspaces.is_empty() {
            return false;
        }
        self.active = saved.active.min(workspaces.len() - 1);
        self.next_workspace = workspaces
            .iter()
            .map(|workspace| workspace.id + 1)
            .max()
            .unwrap_or(2)
            .max(2);
        self.workspaces = workspaces;
        true
    }

    fn restore_tab(&mut self, saved: SavedTab) -> Option<Tab> {
        let mut leaves = Vec::new();
        saved.layout.leaves(&mut leaves);
        let mut panes = Vec::new();
        for saved_pane in &saved.panes {
            if !leaves.contains(&saved_pane.id) {
                continue;
            }
            let init = PaneInit {
                cols: saved_pane.cols,
                rows: saved_pane.rows,
                launch: Launch::Shell {
                    cwd: saved_pane.cwd.as_deref(),
                },
                lines: &saved_pane.lines,
            };
            match Pane::new(saved_pane.id, self.history, self.signals.clone(), init) {
                Ok(mut pane) => {
                    pane.mark_restored();
                    panes.push(pane);
                }
                Err(err) => {
                    eprintln!("koshi: cannot restore pane {}: {err:#}", saved_pane.id)
                }
            }
        }
        if panes.is_empty() {
            return None;
        }
        let mut layout = saved.layout;
        for leaf in leaves {
            if !panes.iter().any(|pane| pane.id() == leaf) {
                layout.remove(leaf);
            }
        }
        let ids: Vec<u64> = panes.iter().map(Pane::id).collect();
        for id in &ids {
            self.next_pane = self.next_pane.max(id + 1);
        }
        let focus = if ids.contains(&saved.focus) {
            saved.focus
        } else {
            ids[0]
        };
        let zoomed = saved.zoomed && focus == saved.focus;
        Some(Tab {
            title: saved.title,
            layout,
            focus,
            previous_focus: None,
            zoomed,
            panes,
            floating: None,
            float_shown: false,
        })
    }
}

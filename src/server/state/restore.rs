//! Turning a saved state into live panes.
//!
//! A restored pane always starts a local shell: the SSH alias of a workspace is
//! put back, but its connections are not resumed.

use crate::server::launch::Launch;
use crate::server::pane::{Pane, PaneInit};
use crate::server::record::RecordTarget;
use crate::server::session::Session;
use crate::server::tab::{Tab, Workspace};

use super::{SavedState, SavedTab};

impl Session {
    /// Replace the session with the saved one. False when nothing was usable.
    pub(in crate::server) fn restore(&mut self, saved: SavedState) -> bool {
        let mut workspaces = Vec::new();
        for saved_workspace in saved.workspaces {
            let recorded = saved_workspace.record;
            let workspace_name = saved_workspace.name.clone();
            let workspace_id = saved_workspace.id;
            let record = recorded.then_some((workspace_name.as_str(), workspace_id));
            let mut tabs = Vec::new();
            for saved_tab in saved_workspace.tabs {
                if let Some(tab) = self.restore_tab(saved_tab, record) {
                    tabs.push(tab);
                }
            }
            if tabs.is_empty() {
                if saved_workspace.ssh_alias.is_none() {
                    continue;
                }
                // An SSH workspace keeps its definition without a pane.
                workspaces.push(Workspace {
                    id: workspace_id,
                    name: workspace_name,
                    ssh_alias: saved_workspace.ssh_alias,
                    record: recorded,
                    tabs: Vec::new(),
                    active: 0,
                    previous_tab: None,
                });
                continue;
            }
            let active = saved_workspace.active.min(tabs.len() - 1);
            workspaces.push(Workspace {
                id: workspace_id,
                name: workspace_name,
                ssh_alias: saved_workspace.ssh_alias,
                record: recorded,
                tabs,
                active,
                previous_tab: None,
            });
        }
        if workspaces.is_empty() {
            return false;
        }
        // A session with no pane anywhere gets a fresh local workspace, so an
        // empty SSH workspace never leaves the screen without input.
        let has_panes = workspaces
            .iter()
            .any(|workspace| workspace.tabs.iter().any(|tab| !tab.panes.is_empty()));
        let next_id = workspaces
            .iter()
            .map(|workspace| workspace.id + 1)
            .max()
            .unwrap_or(2)
            .max(2);
        if !has_panes {
            let area = self.content_area();
            if let Ok(tab) = self.build_tab("shell", area, Launch::Shell { cwd: None }, None) {
                workspaces.push(Workspace {
                    id: next_id,
                    name: "main".to_string(),
                    ssh_alias: None,
                    record: false,
                    tabs: vec![tab],
                    active: 0,
                    previous_tab: None,
                });
                // Start on the workspace that has a pane, not on an empty one.
                self.active = workspaces.len() - 1;
            }
        }
        if has_panes {
            self.active = saved.active.min(workspaces.len() - 1);
        } else {
            self.active = self.active.min(workspaces.len() - 1);
        }
        self.next_workspace = workspaces
            .iter()
            .map(|workspace| workspace.id + 1)
            .max()
            .unwrap_or(2)
            .max(2);
        self.workspaces = workspaces;
        true
    }

    fn restore_tab(&mut self, saved: SavedTab, record: Option<(&str, u64)>) -> Option<Tab> {
        let mut leaves = Vec::new();
        saved.layout.leaves(&mut leaves);
        let mut panes = Vec::new();
        for saved_pane in &saved.panes {
            if !leaves.contains(&saved_pane.id) {
                continue;
            }
            let record = record.as_ref().map(|(name, workspace)| {
                RecordTarget::new(&self.record_root, name, *workspace, saved_pane.id)
            });
            let init = PaneInit {
                cols: saved_pane.cols,
                rows: saved_pane.rows,
                launch: Launch::Shell {
                    cwd: saved_pane.cwd.as_deref(),
                },
                lines: &saved_pane.lines,
                record,
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

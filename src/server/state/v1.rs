//! The format-1 layout, kept only to read files written before SSH aliases.
//!
//! A format-1 file is decoded with these types and converted to the current
//! ones. The field order is the order the format-1 writer used, so a change to
//! the current types does not reach this copy.

use serde::{Deserialize, Serialize};

use crate::server::layout::Layout;

use super::{SavedPane, SavedState, SavedTab, SavedWorkspace};

/// Format 1 had no workspace `id`, no `ssh_alias`, and no `record`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct StateV1 {
    pub active: usize,
    pub workspaces: Vec<WorkspaceV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct WorkspaceV1 {
    pub name: String,
    pub active: usize,
    pub tabs: Vec<TabV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct TabV1 {
    pub title: String,
    pub layout: Layout,
    pub focus: u64,
    pub zoomed: bool,
    pub panes: Vec<PaneV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PaneV1 {
    pub id: u64,
    pub cwd: Option<String>,
    pub cols: u16,
    pub rows: u16,
    pub lines: Vec<String>,
}

impl StateV1 {
    /// Ids come from the file order, and a format-1 workspace has no alias.
    pub(super) fn into_current(self) -> SavedState {
        SavedState {
            active: self.active,
            workspaces: self
                .workspaces
                .into_iter()
                .enumerate()
                .map(|(index, workspace)| SavedWorkspace {
                    id: index as u64 + 1,
                    name: workspace.name,
                    ssh_alias: None,
                    record: false,
                    active: workspace.active,
                    tabs: workspace
                        .tabs
                        .into_iter()
                        .map(TabV1::into_current)
                        .collect(),
                })
                .collect(),
        }
    }
}

impl TabV1 {
    fn into_current(self) -> SavedTab {
        SavedTab {
            title: self.title,
            layout: self.layout,
            focus: self.focus,
            zoomed: self.zoomed,
            panes: self.panes.into_iter().map(PaneV1::into_current).collect(),
        }
    }
}

impl PaneV1 {
    fn into_current(self) -> SavedPane {
        SavedPane {
            id: self.id,
            cwd: self.cwd,
            cols: self.cols,
            rows: self.rows,
            lines: self.lines,
        }
    }
}

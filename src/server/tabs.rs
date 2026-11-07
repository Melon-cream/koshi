//! Tab commands: create, close, switch, jump, and the previous tab.

use anyhow::Result;

use crate::protocol::Rect;
use crate::server::geom::Float;
use crate::server::launch::Launch;
use crate::server::layout::Layout;
use crate::server::pane::{Pane, PaneInit};
use crate::server::record::RecordTarget;
use crate::server::session::Session;
use crate::server::tab::{Tab, Workspace};

impl Session {
    /// The SSH alias of the active workspace, cloned so that the `Launch` built
    /// from it does not borrow the session across `&mut self`.
    pub(super) fn active_alias(&self) -> Option<String> {
        self.workspaces
            .get(self.active)
            .and_then(|workspace| workspace.ssh_alias.clone())
    }

    pub(super) fn spawn_pane(
        &mut self,
        id: u64,
        area: Rect,
        launch: Launch<'_>,
        record: Option<RecordTarget>,
    ) -> Result<Pane> {
        let init = PaneInit {
            cols: area.cols.max(1),
            rows: area.rows.max(1),
            launch,
            lines: &[],
            record,
        };
        Pane::new(id, self.history, self.signals.clone(), init)
    }

    /// The recording a new pane of the active workspace needs, if it records.
    pub(super) fn new_record_target(&self, pane: u64) -> Option<RecordTarget> {
        let workspace = self.workspaces.get(self.active)?;
        workspace.record.then(|| {
            RecordTarget::new(&self.record_root, &workspace.name, workspace.id, pane)
        })
    }

    pub(super) fn build_tab(
        &mut self,
        title: &str,
        area: Rect,
        launch: Launch<'_>,
        record: Option<RecordTarget>,
    ) -> Result<Tab> {
        let id = self.next_pane;
        self.next_pane += 1;
        let pane = self.spawn_pane(id, area, launch, record)?;
        Ok(Tab {
            title: title.to_string(),
            layout: Layout::leaf(id),
            panes: vec![pane],
            floating: None,
            float_shown: false,
            focus: id,
            previous_focus: None,
            zoomed: false,
        })
    }

    pub(super) fn new_tab(&mut self) -> Result<()> {
        let area = self.content_area();
        let alias = self.active_alias();
        let record = self.new_record_target(self.next_pane);
        let tab = self.build_tab(
            "shell",
            area,
            Launch::with_alias(alias.as_deref()),
            record,
        )?;
        let active = self.active;
        let workspace = &mut self.workspaces[active];
        workspace.previous_tab = Some(workspace.active);
        workspace.tabs.push(tab);
        workspace.active = workspace.tabs.len() - 1;
        self.invalidate();
        Ok(())
    }

    /// Show, hide, or first create the tab's one floating pane. Zoom and the
    /// float never share the screen, so showing it leaves zoom.
    pub(super) fn toggle_float(&mut self) -> Result<()> {
        let Some(tab) = self.active_tab() else {
            return Ok(());
        };
        if tab.float_shown {
            self.hide_float();
            return Ok(());
        }
        let area = self.content_area();
        if tab.floating.is_none() {
            let id = self.next_pane;
            let inner = Float::of(id, area).inner;
            let launch = self.active_alias();
            let record = self.new_record_target(id);
            let pane = self.spawn_pane(id, inner, Launch::with_alias(launch.as_deref()), record)?;
            self.next_pane += 1;
            if let Some(tab) = self.active_tab_mut() {
                tab.floating = Some(pane);
            }
        }
        if let Some(tab) = self.active_tab_mut() {
            tab.zoomed = false;
            tab.float_shown = true;
        }
        self.resize_active();
        self.invalidate();
        Ok(())
    }

    pub(super) fn hide_float(&mut self) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        if !tab.float_shown {
            return;
        }
        tab.float_shown = false;
        self.resize_active();
        self.invalidate();
    }

    /// Close the floating pane of the active tab; its shell is killed.
    pub(super) fn close_float(&mut self) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let Some(mut pane) = tab.floating.take() else {
            return;
        };
        tab.float_shown = false;
        pane.kill();
        self.resize_active();
        self.invalidate();
    }

    /// True while the active tab shows its floating pane.
    pub(super) fn float_shown(&self) -> bool {
        self.active_tab().is_some_and(|tab| tab.float().is_some())
    }

    /// Close the active tab and its panes; the last tab of a workspace stays.
    pub(super) fn close_tab(&mut self) {
        let Some(workspace) = self.workspaces.get_mut(self.active) else {
            return;
        };
        if workspace.tabs.len() < 2 {
            return;
        }
        let index = workspace.active;
        let mut tab = workspace.tabs.remove(index);
        for pane in tab.panes.iter_mut().chain(tab.floating.as_mut()) {
            pane.kill();
        }
        let remaining = workspace.tabs.len();
        workspace.active = index.min(remaining - 1);
        workspace.previous_tab = None;
        self.resize_active();
        self.invalidate();
    }

    pub(super) fn goto_tab(&mut self, index: usize) {
        let Some(workspace) = self.workspaces.get_mut(self.active) else {
            return;
        };
        if index >= workspace.tabs.len() {
            return;
        }
        self.activate_tab(index);
    }

    /// Back to the tab that was visible before this one.
    pub(super) fn last_tab(&mut self) {
        let Some(workspace) = self.workspaces.get(self.active) else {
            return;
        };
        let Some(previous) = workspace.previous_tab else {
            return;
        };
        if previous >= workspace.tabs.len() || previous == workspace.active {
            return;
        }
        self.activate_tab(previous);
    }

    pub(super) fn switch_tab(&mut self, delta: isize) {
        let Some(workspace) = self.workspaces.get(self.active) else {
            return;
        };
        let count = workspace.tabs.len();
        if count == 0 {
            return;
        }
        let current = workspace.active as isize;
        let next = (current + delta).rem_euclid(count as isize) as usize;
        self.activate_tab(next);
    }

    fn activate_tab(&mut self, index: usize) {
        let Some(workspace) = self.workspaces.get_mut(self.active) else {
            return;
        };
        if index >= workspace.tabs.len() || workspace.active == index {
            return;
        }
        workspace.previous_tab = Some(workspace.active);
        workspace.active = index;
        self.resize_active();
        self.invalidate();
        self.dirty = true;
    }

    /// Create a workspace with one tab and make it the active one.
    pub(super) fn create_workspace(&mut self) -> Result<usize> {
        let mut number = self.workspaces.len() + 1;
        let mut name = format!("ws{number}");
        while self
            .workspaces
            .iter()
            .any(|workspace| workspace.name == name)
        {
            number += 1;
            name = format!("ws{number}");
        }
        let area = self.content_area();
        let tab = self.build_tab("shell", area, Launch::Shell { cwd: None }, None)?;
        let index = self.workspaces.len();
        let id = self.next_workspace;
        self.next_workspace += 1;
        self.workspaces.push(Workspace {
            id,
            name,
            ssh_alias: None,
            record: false,
            tabs: vec![tab],
            active: 0,
            previous_tab: None,
        });
        self.active = index;
        self.resize_active();
        self.invalidate();
        Ok(index)
    }
}

//! Tab and workspace shapes: the pane layout, its focus, and its geometry.

use std::time::Instant;

use crate::protocol::Rect;
use crate::server::geom::Geometry;
use crate::server::layout::Layout;
use crate::server::pane::Pane;

#[derive(Debug, Clone, Copy)]
pub(super) struct Size {
    pub(super) cols: u16,
    pub(super) rows: u16,
}

pub(super) struct Workspace {
    pub(super) id: u64,
    pub(super) name: String,
    /// Host alias of `~/.ssh/config`; `Some` makes this an SSH workspace.
    pub(super) ssh_alias: Option<String>,
    /// Record the output of this workspace's panes.
    pub(super) record: bool,
    pub(super) tabs: Vec<Tab>,
    pub(super) active: usize,
    /// Tab the client looked at before the current one.
    pub(super) previous_tab: Option<usize>,
}

pub(super) struct Tab {
    pub(super) title: String,
    pub(super) layout: Layout,
    pub(super) panes: Vec<Pane>,
    /// The one shell that can float over the tiled panes, kept across hides.
    pub(super) floating: Option<Pane>,
    /// True while `floating` is drawn over the tiled panes.
    pub(super) float_shown: bool,
    pub(super) focus: u64,
    /// Pane that had the focus before the current one.
    pub(super) previous_focus: Option<u64>,
    pub(super) zoomed: bool,
}

impl Tab {
    pub(super) fn set_focus(&mut self, id: u64) {
        if self.focus == id {
            return;
        }
        self.previous_focus = Some(self.focus);
        self.focus = id;
    }

    pub(super) fn rects(&self, area: Rect) -> Vec<(u64, Rect)> {
        self.geometry(area).visible()
    }

    /// The one description of where this tab's panes, gutters, and frame sit.
    /// Panes are sized and hit tested from it, and the borders are drawn from it.
    pub(super) fn geometry(&self, area: Rect) -> Geometry {
        if self.zoomed {
            return Geometry::zoomed(self.focus, area);
        }
        let geometry = Geometry::of(&self.layout, area, self.panes.len());
        match self.float() {
            Some(pane) => geometry.with_float(pane),
            None => geometry,
        }
    }

    /// The pane id of the shown floating pane.
    pub(super) fn float(&self) -> Option<u64> {
        let pane = self.floating.as_ref()?;
        self.float_shown.then(|| pane.id())
    }

    /// The geometry this tab will have once it has `count` panes.
    pub(super) fn geometry_with(&self, area: Rect, count: usize) -> Geometry {
        Geometry::of(&self.layout, area, count)
    }

    pub(super) fn pane(&self, id: u64) -> Option<&Pane> {
        if let Some(pane) = self.floating.as_ref().filter(|pane| pane.id() == id) {
            return Some(pane);
        }
        self.panes.iter().find(|pane| pane.id() == id)
    }

    pub(super) fn pane_mut(&mut self, id: u64) -> Option<&mut Pane> {
        if let Some(pane) = self.floating.as_mut().filter(|pane| pane.id() == id) {
            return Some(pane);
        }
        self.panes.iter_mut().find(|pane| pane.id() == id)
    }

    /// The pane that takes the keyboard: the float while it is shown.
    pub(super) fn typing_pane(&self) -> u64 {
        self.float().unwrap_or(self.focus)
    }
}

pub(super) struct Toast {
    pub(super) text: String,
    pub(super) until: Instant,
}

pub(super) fn resize_tab(tab: &mut Tab, area: Rect) {
    let rects = tab.rects(area);
    for (id, rect) in rects {
        if let Some(pane) = tab.pane_mut(id) {
            pane.resize(rect.cols.max(1), rect.rows.max(1));
        }
    }
}

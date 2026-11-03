//! Keyboard scrolling of the focused pane.
//!
//! Deltas use the same sign as `Pane::scroll_display`: a positive delta moves
//! the view into the history, a negative one back towards the live end.

use crate::server::pane::Pane;
use crate::server::session::Session;

impl Session {
    pub(super) fn scroll_lines(&mut self, delta: i32) {
        if let Some(pane) = self.focused_pane_mut() {
            pane.scroll_display(delta);
            self.dirty = true;
        }
    }

    /// Whole pages or half pages, measured in rows of the focused pane.
    pub(super) fn scroll_pages(&mut self, pages: i32, half: bool) {
        let Some(pane) = self.focused_pane_mut() else {
            return;
        };
        let rows = i32::from(pane.rows()).max(1);
        let step = if half { (rows / 2).max(1) } else { rows };
        pane.scroll_display(pages.saturating_mul(step));
        self.dirty = true;
    }

    pub(super) fn scroll_bottom(&mut self) {
        if let Some(pane) = self.focused_pane_mut() {
            pane.focus_live();
            self.dirty = true;
        }
    }

    fn focused_pane_mut(&mut self) -> Option<&mut Pane> {
        let tab = self.active_tab_mut()?;
        let typing = tab.typing_pane();
        tab.pane_mut(typing)
    }
}

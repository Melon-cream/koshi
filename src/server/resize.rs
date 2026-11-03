//! Split gutter drags: the border follows the pointer, the ptys follow the release.

use crate::protocol::{MouseInput, Rect};
use crate::server::layout::Dir;
use crate::server::session::Session;

/// A split gutter held by one client's left button.
#[derive(Clone)]
pub(super) struct Resize {
    /// `false` for the first child at each level, `true` for the second.
    pub(super) path: Vec<bool>,
    pub(super) area: Rect,
    pub(super) dir: Dir,
}

impl Session {
    pub(super) fn resize_motion(&mut self, id: u64, resize: Resize, event: MouseInput) {
        let value = size_at(&resize, event.col, event.row);
        if let Some(client) = self.client_mut(id) {
            client.resize = Some(resize);
            client.resize_value = Some(value);
        }
    }

    pub(super) fn resize_release(&mut self, id: u64, resize: Resize, event: MouseInput) {
        let value = size_at(&resize, event.col, event.row);
        self.apply_resize(&resize, value);
        if let Some(client) = self.client_mut(id) {
            client.resize_value = None;
        }
        self.resize_active();
        self.invalidate();
    }

    pub(super) fn tick_resize(&mut self) {
        let requests: Vec<(u64, Resize, u16)> = self
            .clients
            .iter()
            .filter_map(|client| Some((client.id, client.resize.clone()?, client.resize_value?)))
            .collect();
        for (id, request, value) in requests {
            self.apply_resize(&request, value);
            if let Some(client) = self.client_mut(id) {
                client.resize_value = None;
            }
            self.dirty = true;
        }
    }

    fn apply_resize(&mut self, resize: &Resize, value: u16) -> bool {
        self.active_tab_mut()
            .is_some_and(|tab| tab.layout.set_first_size(&resize.path, value))
    }
}

fn size_at(resize: &Resize, col: u16, row: u16) -> u16 {
    let (value, max) = match resize.dir {
        Dir::Columns => (
            col.saturating_sub(resize.area.x),
            resize.area.cols.saturating_sub(2),
        ),
        Dir::Rows => (
            row.saturating_sub(resize.area.y),
            resize.area.rows.saturating_sub(2),
        ),
    };
    value.clamp(1, max.max(1))
}

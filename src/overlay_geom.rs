//! Overlay box geometry shared by the client renderer and server hit testing.

use crate::protocol::{Overlay, Rect};

pub struct Geometry {
    /// Drawn into the top border.
    pub title: String,
    /// Item, prompt, and footer lines, already truncated to fit.
    pub lines: Vec<String>,
    /// Absolute screen column of the left border.
    pub left: u16,
    /// Absolute screen row of the top border.
    pub top: u16,
    /// Width inside the borders.
    pub inner_width: u16,
    visible_items: usize,
}

pub fn geometry(content: Rect, overlay: &Overlay) -> Option<Geometry> {
    if content.rows < 4 || content.cols < 8 {
        return None;
    }
    let mut lines = Vec::with_capacity(overlay.items.len() + 2);
    lines.extend(overlay.items.iter().cloned());
    if let Some(prompt) = &overlay.prompt {
        lines.push(prompt.clone());
    }
    lines.push(overlay.footer.clone());
    lines.truncate(content.rows as usize - 2);

    let widest = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    let title_width = overlay.title.chars().count();
    // One space of padding on each side, and a dash after the title.
    let box_cols = (widest + 4)
        .max(title_width + 4)
        .min(content.cols as usize)
        .max(4) as u16;
    Some(Geometry {
        title: overlay.title.clone(),
        left: content.x + (content.cols - box_cols) / 2,
        top: content.y,
        inner_width: box_cols - 2,
        visible_items: overlay.items.len().min(lines.len()),
        lines,
    })
}

impl Geometry {
    /// Item index for a click inside the box, ignoring the borders, the prompt,
    /// and the footer.
    pub fn item_at(&self, col: u16, row: u16) -> Option<usize> {
        let inner_left = self.left.saturating_add(1);
        if col < inner_left || col >= inner_left.saturating_add(self.inner_width) {
            return None;
        }
        let line = (row as usize).checked_sub(self.top as usize + 1)?;
        (line < self.visible_items).then_some(line)
    }
}

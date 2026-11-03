//! Drawing one screen element at a time.
//!
//! Everything here writes absolute cursor addresses and its own SGR state, so
//! the caller can draw the parts of a frame in any order it needs.

use std::fmt::Write as _;

use unicode_width::UnicodeWidthChar;

use crate::overlay_geom;
use crate::protocol::{
    BorderCell, CellData, Color, FLAG_BOLD, FLAG_DIM, FLAG_HIDDEN, FLAG_ITALIC,
    FLAG_LEADING_WIDE_CHAR_SPACER, FLAG_REVERSE, FLAG_SELECTED, FLAG_STRIKEOUT, FLAG_UNDERLINE,
    FLAG_WIDE_CHAR_SPACER, Overlay, Rect, Status,
};

/// Border of a pane split that is not owned by the focused pane.
const BORDER: Color = Color::Indexed(240);
/// Border owned by the focused pane: the same hue as the pane mode label.
const BORDER_FOCUSED: Color = Color::Indexed(110);
const BAR_BG: Color = Color::Indexed(236);
const BAR_DIM: Color = Color::Indexed(245);
const MODE_FG: Color = Color::Indexed(232);
/// The active tab is filled with the NORMAL label's own grey.
const TAB_BG: Color = Color::Indexed(245);
const KEY_BG: Color = Color::Indexed(238);
const KEY_FG: Color = Color::Indexed(252);

fn plain(fg: Color, flags: u16) -> CellData {
    CellData {
        ch: ' ',
        fg,
        bg: Color::Default,
        flags,
    }
}

fn text(ch: char, fg: Color, bg: Color, flags: u16) -> CellData {
    CellData { ch, fg, bg, flags }
}

pub(super) fn blank(rows: usize, cols: usize) -> Vec<Vec<CellData>> {
    vec![vec![CellData::default(); cols]; rows]
}

/// One row of a pane, drawn inside `rect` at `row`.
pub(super) fn pane_row(
    out: &mut String,
    cells: &[CellData],
    rect: Rect,
    row: usize,
    content: Rect,
) {
    if row >= rect.rows as usize {
        return;
    }
    let y = rect.y as usize + row;
    if y >= content.y as usize + content.rows as usize {
        return;
    }
    let x0 = rect.x as usize;
    let width = rect.cols as usize;
    let _ = write!(out, "\x1b[{};{}H", y + 1, x0 + 1);
    let mut col = 0usize;
    let mut previous: Option<(Color, Color, u16)> = None;
    for index in 0..width {
        let cell = cells.get(index).copied().unwrap_or_default();
        if cell.flags & (FLAG_WIDE_CHAR_SPACER | FLAG_LEADING_WIDE_CHAR_SPACER) != 0 {
            continue;
        }
        if index != col {
            let _ = write!(out, "\x1b[{};{}H", y + 1, x0 + index + 1);
            col = index;
        }
        let key = (cell.fg, cell.bg, cell.flags);
        if previous != Some(key) {
            out.push_str(&sgr(&cell));
            previous = Some(key);
        }
        out.push(cell.ch);
        col += cell.ch.width().unwrap_or(1);
    }
    out.push_str("\x1b[0m");
}

/// The status bar: one background row, the mode label and the active tab
/// inverted, the separators dimmed, and every hint key on its own pill.
pub(super) fn status(out: &mut String, content: Rect, status: &Status) {
    let row = content.y as usize + content.rows as usize;
    let width = content.cols as usize;
    let mut line: Vec<char> = status.line.chars().take(width).collect();
    // The hint starts in the text, not in the padding that fills the row.
    let used = line.len();
    line.resize(width, ' ');

    let background = text(' ', Color::Default, BAR_BG, 0);
    put(out, row, 0, &background, &line.iter().collect::<String>());

    let (label_len, background_index) = mode_label(&line);
    if label_len > 0 {
        let label = &line[..label_len];
        let cell = text(' ', MODE_FG, Color::Indexed(background_index), FLAG_BOLD);
        put(out, row, 0, &cell, &label.iter().collect::<String>());
    }

    let dim = text(' ', BAR_DIM, BAR_BG, 0);
    let base = text(' ', Color::Default, BAR_BG, 0);
    let hint_len = status
        .hint
        .as_ref()
        .map(|hint| hint.chars().count())
        .unwrap_or(0);
    let dim_from = used.saturating_sub(hint_len);
    let pills = hint_pills(&line, dim_from);
    let mut index = label_len;
    while index < line.len() {
        if separator_at(&line, index) {
            put(out, row, index, &dim, " | ");
            index += 3;
            continue;
        }
        if let Some(&(from, to)) = pills.iter().find(|(from, _)| *from == index) {
            let pill = text(' ', KEY_FG, KEY_BG, FLAG_BOLD);
            put(
                out,
                row,
                from,
                &pill,
                &line[from..to].iter().collect::<String>(),
            );
            index = to;
            continue;
        }
        let start = index;
        let dimmed = index >= dim_from;
        while index < line.len()
            && (index >= dim_from) == dimmed
            && !separator_at(&line, index)
            && !pills.iter().any(|(from, _)| *from == index)
        {
            index += 1;
        }
        let run: String = line[start..index].iter().collect();
        put(out, row, start, if dimmed { &dim } else { &base }, &run);
    }

    if let Some((from, to)) = status.active {
        let from = (from as usize).min(line.len());
        let to = (to as usize).min(line.len());
        if from < to {
            let cell = text(' ', MODE_FG, TAB_BG, FLAG_BOLD);
            put(
                out,
                row,
                from,
                &cell,
                &line[from..to].iter().collect::<String>(),
            );
        }
    }
}

/// The key of every hint pair, as char ranges of `line`. Pairs are separated by
/// two spaces and the key is the first word of the pair, so `Arrows focus` keys
/// on `Arrows` alone.
fn hint_pills(line: &[char], from: usize) -> Vec<(usize, usize)> {
    let mut pills = Vec::new();
    let mut start = from;
    while start < line.len() {
        if line[start] == ' ' {
            start += 1;
            continue;
        }
        let mut end = start;
        while end < line.len() && !separator_at(line, end) && !pair_gap(line, end) {
            end += 1;
        }
        let mut key_end = start;
        while key_end < end && line[key_end] != ' ' {
            key_end += 1;
        }
        if key_end > start {
            pills.push((start, key_end));
        }
        start = end;
    }
    pills
}

/// True at the first of the two spaces that separate one hint pair from the next.
fn pair_gap(line: &[char], index: usize) -> bool {
    index + 1 < line.len() && line[index] == ' ' && line[index + 1] == ' '
}

/// Length of the leading `[MODE]` label and the colour its background takes.
fn mode_label(line: &[char]) -> (usize, u8) {
    if line.first() != Some(&'[') {
        return (0, 245);
    }
    let Some(end) = line.iter().position(|ch| *ch == ']') else {
        return (0, 245);
    };
    let label: String = line[..=end].iter().collect();
    let background = match label.as_str() {
        "[PANE]" => 110,
        "[TAB]" => 176,
        "[SESSION]" => 108,
        "[SCROLL]" => 180,
        _ => 245,
    };
    (end + 1, background)
}

fn separator_at(line: &[char], index: usize) -> bool {
    line.len() >= index + 3 && line[index..index + 3] == [' ', '|', ' ']
}

/// The workspace list box.
pub(super) fn overlay(out: &mut String, content: Rect, overlay: &Overlay) {
    let Some(geometry) = overlay_geom::geometry(content, overlay) else {
        return;
    };
    let inner = geometry.inner_width as usize;
    let top = geometry.top as usize;
    let left = geometry.left as usize;
    let border = plain(BAR_DIM, 0);
    let footer = geometry.lines.len().saturating_sub(1);

    let title_width = geometry.title.chars().count();
    let dashes = inner.saturating_sub(title_width + 2);
    let top_edge = format!("┌ {} {}┐", geometry.title, "─".repeat(dashes));
    put(out, top, left, &border, &top_edge);
    for (index, line) in geometry.lines.iter().enumerate() {
        let selected = index == overlay.selected && index < overlay.items.len();
        let style = if selected {
            plain(Color::Default, FLAG_REVERSE)
        } else if index == footer {
            plain(BAR_DIM, 0)
        } else {
            plain(Color::Default, 0)
        };
        let mut body = format!(" {line}");
        let used = body.chars().count();
        body.push_str(&" ".repeat(inner.saturating_sub(used)));
        put(out, top + 1 + index, left, &border, "│");
        put(out, top + 1 + index, left + 1, &style, &body);
        put(out, top + 1 + index, left + inner + 1, &border, "│");
    }
    let bottom = top + 1 + geometry.lines.len();
    put(
        out,
        bottom,
        left,
        &border,
        &format!("└{}┘", "─".repeat(inner)),
    );
}

/// Split gutters, in the focus colour on the side that owns them.
pub(super) fn borders(out: &mut String, cells: &[BorderCell]) {
    for cell in cells {
        let style = if cell.strong {
            plain(BORDER_FOCUSED, 0)
        } else {
            plain(BORDER, 0)
        };
        put(
            out,
            cell.y as usize,
            cell.x as usize,
            &style,
            &cell.ch.to_string(),
        );
    }
}

/// A small box at the bottom right, one row above the status bar.
pub(super) fn toast(out: &mut String, content: Rect, message: &str) {
    let width = message.chars().count() + 4;
    if content.rows < 3 || (content.cols as usize) < width {
        return;
    }
    let left = content.x as usize + content.cols as usize - width;
    let top = content.y as usize + content.rows as usize - 3;
    let inner = width - 2;
    let border = plain(BAR_DIM, 0);
    put(out, top, left, &border, &format!("┌{}┐", "─".repeat(inner)));
    put(out, top + 1, left, &border, &format!("│ {message} │"));
    put(
        out,
        top + 2,
        left,
        &border,
        &format!("└{}┘", "─".repeat(inner)),
    );
}

fn put(out: &mut String, row: usize, col: usize, cell: &CellData, body: &str) {
    let _ = write!(out, "\x1b[{};{}H{}{}", row + 1, col + 1, sgr(cell), body);
}

pub(super) fn sgr(cell: &CellData) -> String {
    let mut out = String::from("\x1b[0");
    if cell.flags & FLAG_BOLD != 0 {
        out.push_str(";1");
    }
    if cell.flags & FLAG_DIM != 0 {
        out.push_str(";2");
    }
    if cell.flags & FLAG_ITALIC != 0 {
        out.push_str(";3");
    }
    if cell.flags & FLAG_UNDERLINE != 0 {
        out.push_str(";4");
    }
    let reverse = (cell.flags & (FLAG_REVERSE | FLAG_SELECTED)).count_ones() % 2 == 1;
    if reverse {
        out.push_str(";7");
    }
    if cell.flags & FLAG_HIDDEN != 0 {
        out.push_str(";8");
    }
    if cell.flags & FLAG_STRIKEOUT != 0 {
        out.push_str(";9");
    }
    push_color(&mut out, cell.fg, true);
    push_color(&mut out, cell.bg, false);
    out.push('m');
    out
}

fn push_color(out: &mut String, color: Color, foreground: bool) {
    match color {
        Color::Default => out.push_str(if foreground { ";39" } else { ";49" }),
        Color::Indexed(index) => {
            let _ = write!(out, ";{};5;{index}", if foreground { 38 } else { 48 });
        }
        Color::Rgb(r, g, b) => {
            let _ = write!(out, ";{};2;{r};{g};{b}", if foreground { 38 } else { 48 });
        }
    }
}

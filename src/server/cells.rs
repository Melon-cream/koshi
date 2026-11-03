//! Conversion from alacritty cells to the wire representation.

use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::vte::ansi::{Color as VteColor, NamedColor};

use crate::protocol::{
    CellData, Color, FLAG_BOLD, FLAG_DIM, FLAG_HIDDEN, FLAG_ITALIC, FLAG_LEADING_WIDE_CHAR_SPACER,
    FLAG_REVERSE, FLAG_STRIKEOUT, FLAG_UNDERLINE, FLAG_WIDE_CHAR, FLAG_WIDE_CHAR_SPACER,
};

pub(super) fn convert_cell(cell: &Cell) -> CellData {
    CellData {
        ch: cell.c,
        fg: convert_color(cell.fg),
        bg: convert_color(cell.bg),
        flags: convert_flags(cell.flags),
    }
}

fn convert_color(color: VteColor) -> Color {
    match color {
        VteColor::Named(named) => {
            let index = named as usize;
            if index <= NamedColor::BrightWhite as usize {
                Color::Indexed(index as u8)
            } else {
                Color::Default
            }
        }
        VteColor::Indexed(index) => Color::Indexed(index),
        VteColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

fn convert_flags(flags: Flags) -> u16 {
    let mut out = 0;
    if flags.contains(Flags::BOLD) {
        out |= FLAG_BOLD;
    }
    if flags.contains(Flags::DIM) {
        out |= FLAG_DIM;
    }
    if flags.contains(Flags::ITALIC) {
        out |= FLAG_ITALIC;
    }
    if flags.intersects(Flags::ALL_UNDERLINES) {
        out |= FLAG_UNDERLINE;
    }
    if flags.contains(Flags::INVERSE) {
        out |= FLAG_REVERSE;
    }
    if flags.contains(Flags::HIDDEN) {
        out |= FLAG_HIDDEN;
    }
    if flags.contains(Flags::STRIKEOUT) {
        out |= FLAG_STRIKEOUT;
    }
    if flags.contains(Flags::WIDE_CHAR) {
        out |= FLAG_WIDE_CHAR;
    }
    if flags.contains(Flags::WIDE_CHAR_SPACER) {
        out |= FLAG_WIDE_CHAR_SPACER;
    }
    if flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
        out |= FLAG_LEADING_WIDE_CHAR_SPACER;
    }
    out
}

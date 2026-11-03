//! xterm mouse encodings written to a pane's PTY.

use alacritty_terminal::term::TermMode;

use crate::protocol::{MouseAction, MouseButton, MouseInput};

/// Encode one event the way the pane's application asked for it.
pub(super) fn encode_mouse(
    mode: TermMode,
    event: MouseInput,
    col: u16,
    row: u16,
) -> Option<Vec<u8>> {
    let motion = event.action == MouseAction::Motion;
    if motion && !mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) {
        return None;
    }
    let release = event.action == MouseAction::Release;
    let mods = (event.mods as u32) << 2 & 0x1c;
    let mut code = button_code(event.button) | mods;
    if motion {
        code |= 32;
    }

    if mode.contains(TermMode::SGR_MOUSE) {
        let suffix = if release { 'm' } else { 'M' };
        let x = col as u32 + 1;
        let y = row as u32 + 1;
        return Some(format!("\x1b[<{code};{x};{y}{suffix}").into_bytes());
    }

    // Legacy and UTF-8 encodings cannot express every coordinate.
    let utf8 = mode.contains(TermMode::UTF8_MOUSE);
    let limit = if utf8 { 2015 } else { 223 };
    if col as usize >= limit || row as usize >= limit {
        return None;
    }
    let code = if release { 3 | mods } else { code };
    let mut out = b"\x1b[M".to_vec();
    out.push(32 + code as u8);
    push_position(&mut out, col as usize, utf8);
    push_position(&mut out, row as usize, utf8);
    Some(out)
}

fn button_code(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::None => 3,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
    }
}

/// xterm 1005 (UTF-8) position encoding; plain bytes below 95.
fn push_position(out: &mut Vec<u8>, pos: usize, utf8: bool) {
    if utf8 && pos >= 95 {
        let value = 33 + pos;
        out.push((0xC0 + value / 64) as u8);
        out.push((0x80 + (value & 63)) as u8);
    } else {
        out.push(33 + pos as u8);
    }
}

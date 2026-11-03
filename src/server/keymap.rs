//! The key table: which press opens a leader group, what a group does, and the
//! bytes a pane receives for a decoded key.

use crate::protocol::KeyCode;
use crate::server::focus::Side;
use crate::server::input::{Command, LeaderKind};
use crate::server::layout::Dir;

/// `p`, `t`, `o`, and `s` select the four groups.
pub(super) fn group_of(letter: u8) -> Option<LeaderKind> {
    match letter {
        b'p' => Some(LeaderKind::Pane),
        b't' => Some(LeaderKind::Tab),
        b'o' => Some(LeaderKind::Session),
        b's' => Some(LeaderKind::Scroll),
        _ => None,
    }
}

/// The command one byte runs inside a leader group.
///
/// While the floating pane is shown it owns the keyboard, so the pane group
/// keeps only the keys that act on it: `f` hides it, `x` closes it, `d`
/// detaches. The other groups cannot move the focus, so they are unchanged.
pub(super) fn leader_command(kind: LeaderKind, byte: u8, float: bool) -> Option<Command> {
    if float && kind == LeaderKind::Pane {
        return match byte {
            b'f' => Some(Command::ToggleFloat),
            b'x' => Some(Command::ClosePane),
            b'd' => Some(Command::Detach),
            _ => None,
        };
    }
    match kind {
        LeaderKind::Pane => match byte {
            b'p' => Some(Command::FocusNext),
            b';' => Some(Command::FocusLast),
            b'n' | b'r' => Some(Command::Split(Dir::Columns)),
            b'd' => Some(Command::Split(Dir::Rows)),
            b'x' => Some(Command::ClosePane),
            b'z' => Some(Command::ToggleZoom),
            b'f' => Some(Command::ToggleFloat),
            _ => None,
        },
        LeaderKind::Tab => match byte {
            b'n' => Some(Command::NewTab),
            b'x' => Some(Command::CloseTab),
            b'r' => Some(Command::BeginTabRename),
            b'\t' => Some(Command::LastTab),
            b'1'..=b'9' => Some(Command::GotoTab((byte - b'1') as usize)),
            _ => None,
        },
        LeaderKind::Session => match byte {
            b'w' => Some(Command::OpenWorkspaces),
            b'r' => Some(Command::RenameWorkspace),
            b'd' => Some(Command::Detach),
            _ => None,
        },
        LeaderKind::Scroll => match byte {
            b'j' => Some(Command::ScrollLines(-1)),
            b'k' => Some(Command::ScrollLines(1)),
            0x06 => Some(Command::ScrollPages(-1)),
            0x02 => Some(Command::ScrollPages(1)),
            b'd' => Some(Command::ScrollHalfPages(-1)),
            b'u' => Some(Command::ScrollHalfPages(1)),
            0x03 => Some(Command::ScrollBottom),
            _ => None,
        },
    }
}

/// The command one decoded key runs inside a leader group.
///
/// Directional focus is arrow keys only. The scroll group keeps the row keys
/// as well, so the arrows sit next to `j` and `k`.
pub(super) fn leader_key_command(kind: LeaderKind, key: KeyCode, float: bool) -> Option<Command> {
    // The pane group's arrows move the focus, which the float owns.
    if float && kind == LeaderKind::Pane {
        return None;
    }
    match kind {
        LeaderKind::Pane => match key {
            KeyCode::Up => Some(Command::Focus(Side::Up)),
            KeyCode::Down => Some(Command::Focus(Side::Down)),
            KeyCode::Left => Some(Command::Focus(Side::Left)),
            KeyCode::Right => Some(Command::Focus(Side::Right)),
            _ => None,
        },
        LeaderKind::Tab => match key {
            KeyCode::Left => Some(Command::PrevTab),
            KeyCode::Right => Some(Command::NextTab),
            _ => None,
        },
        LeaderKind::Scroll => match key {
            KeyCode::Up => Some(Command::ScrollLines(1)),
            KeyCode::Down => Some(Command::ScrollLines(-1)),
            _ => None,
        },
        LeaderKind::Session => None,
    }
}

/// Bytes a pane receives for a decoded key.
///
/// `app_cursor` is the pane's DECCKM state: applications such as less and vim
/// switch the arrows and Home/End to the SS3 form, and read the wrong form as
/// literal escape input.
pub(super) fn key_bytes(key: KeyCode, app_cursor: bool) -> &'static [u8] {
    match key {
        KeyCode::Up => {
            if app_cursor {
                b"\x1bOA"
            } else {
                b"\x1b[A"
            }
        }
        KeyCode::Down => {
            if app_cursor {
                b"\x1bOB"
            } else {
                b"\x1b[B"
            }
        }
        KeyCode::Right => {
            if app_cursor {
                b"\x1bOC"
            } else {
                b"\x1b[C"
            }
        }
        KeyCode::Left => {
            if app_cursor {
                b"\x1bOD"
            } else {
                b"\x1b[D"
            }
        }
        KeyCode::Home => {
            if app_cursor {
                b"\x1bOH"
            } else {
                b"\x1b[H"
            }
        }
        KeyCode::End => {
            if app_cursor {
                b"\x1bOF"
            } else {
                b"\x1b[F"
            }
        }
        KeyCode::PageUp => b"\x1b[5~",
        KeyCode::PageDown => b"\x1b[6~",
        // The Alt arrows are commands, not pane input; the doubled ESC is what a
        // terminal would send if one reached a pane.
        KeyCode::AltUp if app_cursor => b"\x1b\x1bOA",
        KeyCode::AltDown if app_cursor => b"\x1b\x1bOB",
        KeyCode::AltRight if app_cursor => b"\x1b\x1bOC",
        KeyCode::AltLeft if app_cursor => b"\x1b\x1bOD",
        KeyCode::AltUp => b"\x1b\x1b[A",
        KeyCode::AltDown => b"\x1b\x1b[B",
        KeyCode::AltRight => b"\x1b\x1b[C",
        KeyCode::AltLeft => b"\x1b\x1b[D",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_float_keeps_only_the_keys_that_act_on_it() {
        let cases: &[(u8, Option<Command>)] = &[
            (b'f', Some(Command::ToggleFloat)),
            (b'x', Some(Command::ClosePane)),
            (b'd', Some(Command::Detach)),
            (b'p', None),
            (b';', None),
            (b'n', None),
            (b'r', None),
            (b'z', None),
        ];
        for (byte, expected) in cases {
            assert_eq!(
                leader_command(LeaderKind::Pane, *byte, true).as_ref(),
                expected.as_ref(),
                "byte {:?}",
                *byte as char
            );
        }
        assert_eq!(
            leader_key_command(LeaderKind::Pane, KeyCode::Left, true),
            None
        );
    }

    #[test]
    fn z_zooms_and_f_floats() {
        assert_eq!(
            leader_command(LeaderKind::Pane, b'z', false),
            Some(Command::ToggleZoom)
        );
        assert_eq!(
            leader_command(LeaderKind::Pane, b'f', false),
            Some(Command::ToggleFloat)
        );
    }

    #[test]
    fn app_cursor_selects_the_ss3_form() {
        assert_eq!(key_bytes(KeyCode::Up, false), b"\x1b[A");
        assert_eq!(key_bytes(KeyCode::Up, true), b"\x1bOA");
        assert_eq!(key_bytes(KeyCode::Left, true), b"\x1bOD");
        assert_eq!(key_bytes(KeyCode::End, false), b"\x1b[F");
        assert_eq!(key_bytes(KeyCode::End, true), b"\x1bOF");
        assert_eq!(key_bytes(KeyCode::PageUp, true), b"\x1b[5~");
        assert_eq!(key_bytes(KeyCode::PageDown, false), b"\x1b[6~");
    }
}

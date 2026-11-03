//! Per-client input state machine.
//!
//! The client forwards raw bytes for typing and decoded keys for the sequences
//! it understands. The server owns the leader state so that the status bar
//! (which the server renders) is always the truth about the mode.
//!
//! A leader is one shot: the leader key, one command key, then back to normal.
//! There is no persistent mode, so a key that is not part of the group is lost
//! on purpose instead of reaching the pane.

use crate::config::Leader;
use crate::protocol::KeyCode;
use crate::server::focus::Side;
use crate::server::keymap::{group_of, leader_command, leader_key_command};
use crate::server::layout::Dir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaderKind {
    Pane,
    Tab,
    Session,
    Scroll,
}

impl LeaderKind {
    /// Status bar label for a pending leader.
    pub fn label(self) -> &'static str {
        match self {
            Self::Pane => "PANE",
            Self::Tab => "TAB",
            Self::Session => "SESSION",
            Self::Scroll => "SCROLL",
        }
    }

    /// Status bar hint listing the keys of this group.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Pane => {
                "Arrows focus  p next  ; last  n/r right  d down  x close  z zoom  f float"
            }
            Self::Tab => "n new  x close  r rename  Left/Right prev/next  Tab last  1-9",
            Self::Session => "w list  r rename  d detach",
            Self::Scroll => "j/k line  C-f/b page  d/u half  C-c bottom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTarget {
    Tab,
    Workspace(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Detach,
    Split(Dir),
    Focus(Side),
    /// Focus the pane on that side, or move the tab when there is none.
    FocusOrTab(Side),
    FocusNext,
    FocusLast,
    ClosePane,
    ToggleZoom,
    ToggleFloat,
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    GotoTab(usize),
    LastTab,
    BeginTabRename,
    RenameWorkspace,
    OpenWorkspaces,
    ScrollLines(i32),
    ScrollPages(i32),
    ScrollHalfPages(i32),
    ScrollBottom,
    OverlayUp,
    OverlayDown,
    OverlaySwitch,
    OverlayCreate,
    OverlayBeginRename,
    OverlayDelete,
    CloseOverlay,
    CommitText(TextTarget, String),
    CancelText(TextTarget),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputState {
    Normal,
    /// Waiting for the second key of a leader group.
    Leader(LeaderKind),
    Overlay,
    Text {
        target: TextTarget,
        buffer: String,
    },
}

impl InputState {
    pub fn new() -> Self {
        Self::Normal
    }

    /// Split raw bytes into probe input for the pane and commands for the session.
    ///
    /// `float` is true while the floating pane is shown: it owns the keyboard,
    /// so only the keys that act on it reach the session.
    pub fn feed(
        &mut self,
        bytes: &[u8],
        leader: Leader,
        float: bool,
        pane: &mut Vec<u8>,
        out: &mut Vec<Command>,
    ) {
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            let mut used = 1;
            match self {
                Self::Normal => {
                    if let Some((kind, len)) = leader_press(leader, &bytes[index..]) {
                        *self = Self::Leader(kind);
                        used = len;
                    } else {
                        pane.push(byte);
                    }
                }
                Self::Leader(kind) => {
                    let kind = *kind;
                    *self = Self::Normal;
                    if let Some((_, len)) = leader_press(leader, &bytes[index..]) {
                        // Pressing a leader again cancels instead of chaining.
                        used = len;
                    } else if let Some(command) = leader_command(kind, byte, float) {
                        out.push(command);
                        // Scrolling is a run of keys, so that group stays open.
                        if kind == LeaderKind::Scroll {
                            *self = Self::Leader(kind);
                        }
                    }
                }
                Self::Overlay => {
                    let command = match byte {
                        b'j' => Some(Command::OverlayDown),
                        b'k' => Some(Command::OverlayUp),
                        b'\r' | b'\n' => Some(Command::OverlaySwitch),
                        b'n' => Some(Command::OverlayCreate),
                        b'r' => Some(Command::OverlayBeginRename),
                        b'd' => Some(Command::OverlayDelete),
                        b'q' | 0x1b => Some(Command::CloseOverlay),
                        _ => None,
                    };
                    if let Some(command) = command {
                        out.push(command);
                    }
                }
                Self::Text { target, buffer } => match byte {
                    0x1b | 0x03 => out.push(Command::CancelText(*target)),
                    b'\r' | b'\n' => {
                        let name = buffer.trim().to_string();
                        if name.is_empty() {
                            out.push(Command::CancelText(*target));
                        } else {
                            out.push(Command::CommitText(*target, name));
                        }
                    }
                    0x7f | 0x08 => {
                        buffer.pop();
                    }
                    0x20..=0x7e => buffer.push(byte as char),
                    _ => {}
                },
            }
            index += used;
        }
    }

    /// Feed one key the client decoded. A pending leader turns it into a
    /// command; otherwise it is added to `keys` for the focused pane.
    pub fn feed_key(
        &mut self,
        key: KeyCode,
        float: bool,
        keys: &mut Vec<KeyCode>,
        out: &mut Vec<Command>,
    ) {
        match self {
            Self::Normal => match alt_side(key) {
                // The float owns the keyboard, so nothing moves behind it.
                Some(_) if float => {}
                Some(side) => out.push(Command::FocusOrTab(side)),
                None => keys.push(key),
            },
            Self::Leader(kind) => {
                let kind = *kind;
                *self = Self::Normal;
                if let Some(command) = leader_key_command(kind, key, float) {
                    out.push(command);
                    if kind == LeaderKind::Scroll {
                        *self = Self::Leader(kind);
                    }
                }
            }
            // The list keeps its own byte bindings; the arrows are decoded.
            Self::Overlay => match key {
                KeyCode::Up => out.push(Command::OverlayUp),
                KeyCode::Down => out.push(Command::OverlayDown),
                _ => {}
            },
            Self::Text { .. } => {}
        }
    }
}

/// The direction an Alt plus an arrow moves in. Those keys never reach a pane.
fn alt_side(key: KeyCode) -> Option<Side> {
    match key {
        KeyCode::AltLeft => Some(Side::Left),
        KeyCode::AltUp => Some(Side::Up),
        KeyCode::AltDown => Some(Side::Down),
        KeyCode::AltRight => Some(Side::Right),
        _ => None,
    }
}

/// The leader group a press opens, and how many bytes the press used.
///
/// Alt leaders are two bytes, so the letter has to be in the same chunk; the
/// client holds a trailing Esc for a moment before forwarding it, which keeps
/// the pair together. A press that cannot be completed stays pane input.
fn leader_press(leader: Leader, bytes: &[u8]) -> Option<(LeaderKind, usize)> {
    match leader {
        Leader::Ctrl => {
            let byte = *bytes.first()?;
            if !(0x01..=0x1a).contains(&byte) {
                return None;
            }
            group_of(byte | 0x60).map(|kind| (kind, 1))
        }
        Leader::Alt => {
            if bytes.first() != Some(&0x1b) {
                return None;
            }
            group_of(*bytes.get(1)?).map(|kind| (kind, 2))
        }
    }
}

#[cfg(test)]
mod tests;

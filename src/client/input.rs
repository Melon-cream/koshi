//! Split the user terminal's input stream into keys, mouse reports, and bytes.
//!
//! The client asks for button-event tracking (1002) and SGR encoding (1006).
//! X10 encoding is still parsed because a terminal may ignore 1006.
//!
//! Arrows and the paging keys arrive as several bytes and a read may end in the
//! middle of one. They are decoded here into a `KeyCode` so that the server
//! decides what they mean and a pane never receives half a sequence.

use crate::protocol::{KeyCode, MouseAction, MouseButton, MouseInput};

/// Upper bound on a pending CSI parameter buffer. Anything longer is treated
/// as ordinary input instead of being buffered without limit.
const MAX_PENDING: usize = 64;

pub enum ClientInput {
    Bytes(Vec<u8>),
    Key(KeyCode),
    Mouse(MouseInput),
}

#[derive(Default)]
pub struct InputParser {
    state: State,
    /// The terminal sent Alt as a second ESC, so the next key is its Alt form.
    alt: bool,
}

#[derive(Default)]
enum State {
    #[default]
    Ground,
    Escape,
    /// `ESC O`: the SS3 form of arrows and Home/End.
    Ss3,
    Csi(Vec<u8>),
    X10 {
        bytes: [u8; 3],
        len: usize,
    },
}

impl InputParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<ClientInput>) {
        let mut text = Vec::new();
        for &byte in bytes {
            self.feed_byte(byte, &mut text, out);
        }
        flush(&mut text, out);
    }

    /// Flush a lone trailing ESC (or SS3 prefix) after the input has been idle.
    ///
    /// A read may end in the middle of a sequence, so ESC is held until the next
    /// read arrives or this timeout expires. Holding it without a timeout
    /// would swallow the Esc key, which cancels prefix and closes overlays.
    pub fn flush_idle(&mut self, out: &mut Vec<ClientInput>) {
        let bytes: &[u8] = match self.state {
            State::Escape if self.alt => b"\x1b\x1b",
            State::Escape => b"\x1b",
            State::Ss3 if self.alt => b"\x1b\x1bO",
            State::Ss3 => b"\x1bO",
            _ => return,
        };
        self.alt = false;
        self.state = State::Ground;
        out.push(ClientInput::Bytes(bytes.to_vec()));
    }

    fn feed_byte(&mut self, byte: u8, text: &mut Vec<u8>, out: &mut Vec<ClientInput>) {
        let state = std::mem::replace(&mut self.state, State::Ground);
        self.state = match state {
            State::Ground => {
                if byte == 0x1b {
                    flush(text, out);
                    State::Escape
                } else {
                    text.push(byte);
                    State::Ground
                }
            }
            State::Escape => match byte {
                0x1b => {
                    self.alt = true;
                    State::Escape
                }
                b'[' => State::Csi(Vec::new()),
                b'O' => State::Ss3,
                _ => {
                    push_raw(text, std::mem::take(&mut self.alt), b"\x1b", &[byte]);
                    State::Ground
                }
            },
            State::Ss3 => {
                let alt = std::mem::take(&mut self.alt);
                match ss3_key(byte).and_then(|key| alt_key(key, alt)) {
                    Some(key) => out.push(ClientInput::Key(key)),
                    None => push_raw(text, alt, b"\x1bO", &[byte]),
                }
                State::Ground
            }
            State::Csi(mut params) => {
                if (0x40..=0x7e).contains(&byte) {
                    let alt = std::mem::take(&mut self.alt);
                    match csi_key(&params, byte).and_then(|key| alt_key(key, alt)) {
                        Some(key) => {
                            out.push(ClientInput::Key(key));
                            State::Ground
                        }
                        None if byte == b'M' && params.is_empty() => State::X10 {
                            bytes: [0; 3],
                            len: 0,
                        },
                        None if (byte == b'M' || byte == b'm') && params.first() == Some(&b'<') => {
                            if let Some(event) = parse_sgr(&params[1..], byte) {
                                out.push(ClientInput::Mouse(event));
                            }
                            State::Ground
                        }
                        None => {
                            params.push(byte);
                            push_raw(text, alt, b"\x1b[", &params);
                            State::Ground
                        }
                    }
                } else if (0x20..=0x3f).contains(&byte) && params.len() < MAX_PENDING {
                    params.push(byte);
                    State::Csi(params)
                } else {
                    // A control byte aborts the sequence; keep it as input.
                    params.push(byte);
                    push_raw(text, std::mem::take(&mut self.alt), b"\x1b[", &params);
                    State::Ground
                }
            }
            State::X10 { mut bytes, mut len } => {
                bytes[len] = byte;
                len += 1;
                if len < 3 {
                    State::X10 { bytes, len }
                } else {
                    if bytes.iter().all(|value| *value >= 32) {
                        if let Some(event) = parse_x10(bytes) {
                            out.push(ClientInput::Mouse(event));
                        }
                    } else {
                        text.extend_from_slice(b"\x1b[M");
                        text.extend_from_slice(&bytes);
                    }
                    State::Ground
                }
            }
        };
    }
}

/// Re-emit a sequence the client does not interpret. Modifier arrows such as
/// `ESC [ 1 ; 5 A` pass through this way. A pending Alt puts its own ESC back in
/// front, so the pane receives the bytes the terminal sent.
fn push_raw(text: &mut Vec<u8>, alt: bool, intro: &[u8], body: &[u8]) {
    if alt {
        text.push(0x1b);
    }
    text.extend_from_slice(intro);
    text.extend_from_slice(body);
}

/// The Alt form of a decoded key, if it has one. Only the arrows do; the other
/// keys keep reaching the pane as the raw sequence the terminal sent.
fn alt_key(key: KeyCode, alt: bool) -> Option<KeyCode> {
    if !alt {
        return Some(key);
    }
    match key {
        KeyCode::Up => Some(KeyCode::AltUp),
        KeyCode::Down => Some(KeyCode::AltDown),
        KeyCode::Left => Some(KeyCode::AltLeft),
        KeyCode::Right => Some(KeyCode::AltRight),
        _ => None,
    }
}

fn flush(text: &mut Vec<u8>, out: &mut Vec<ClientInput>) {
    if !text.is_empty() {
        out.push(ClientInput::Bytes(std::mem::take(text)));
    }
}

/// The keys of the `ESC [ ...` form. `params` excludes `ESC [` and the final
/// byte. Unmodified arrows, and the `1;3` xterm form of Alt plus an arrow, are
/// decoded; any other modifier stays raw input.
fn csi_key(params: &[u8], final_byte: u8) -> Option<KeyCode> {
    match (final_byte, params) {
        (b'A', []) => Some(KeyCode::Up),
        (b'B', []) => Some(KeyCode::Down),
        (b'C', []) => Some(KeyCode::Right),
        (b'D', []) => Some(KeyCode::Left),
        (b'A', [b'1', b';', b'3']) => Some(KeyCode::AltUp),
        (b'B', [b'1', b';', b'3']) => Some(KeyCode::AltDown),
        (b'C', [b'1', b';', b'3']) => Some(KeyCode::AltRight),
        (b'D', [b'1', b';', b'3']) => Some(KeyCode::AltLeft),
        (b'H', []) => Some(KeyCode::Home),
        (b'F', []) => Some(KeyCode::End),
        (b'~', [b'1']) | (b'~', [b'7']) => Some(KeyCode::Home),
        (b'~', [b'4']) | (b'~', [b'8']) => Some(KeyCode::End),
        (b'~', [b'5']) => Some(KeyCode::PageUp),
        (b'~', [b'6']) => Some(KeyCode::PageDown),
        _ => None,
    }
}

/// The keys of the `ESC O <byte>` form, which application cursor mode uses.
fn ss3_key(byte: u8) -> Option<KeyCode> {
    match byte {
        b'A' => Some(KeyCode::Up),
        b'B' => Some(KeyCode::Down),
        b'C' => Some(KeyCode::Right),
        b'D' => Some(KeyCode::Left),
        b'H' => Some(KeyCode::Home),
        b'F' => Some(KeyCode::End),
        _ => None,
    }
}

/// `body` is the parameters after `<`, without the final byte.
fn parse_sgr(body: &[u8], final_byte: u8) -> Option<MouseInput> {
    let text = std::str::from_utf8(body).ok()?;
    let mut fields = text.split(';');
    let code: u8 = fields.next()?.parse().ok()?;
    let col: u16 = fields.next()?.parse().ok()?;
    let row: u16 = fields.next()?.parse().ok()?;
    if fields.next().is_some() || col == 0 || row == 0 {
        return None;
    }
    build(code, col - 1, row - 1, final_byte == b'm')
}

fn parse_x10(bytes: [u8; 3]) -> Option<MouseInput> {
    let code = bytes[0].wrapping_sub(32);
    let col = bytes[1].wrapping_sub(33) as u16;
    let row = bytes[2].wrapping_sub(33) as u16;
    if col == 0 || row == 0 {
        return None;
    }
    let motion = code & 32 != 0;
    let release = !motion && (code & 0x03) == 0x03 && code & 0x40 == 0;
    build(code, col, row, release)
}

fn build(code: u8, col: u16, row: u16, release: bool) -> Option<MouseInput> {
    if code & 0x80 != 0 {
        return None;
    }
    let mods = (code >> 2) & 0x07;
    let action = if release {
        MouseAction::Release
    } else if code & 32 != 0 {
        MouseAction::Motion
    } else {
        MouseAction::Press
    };
    let button = if code & 64 != 0 {
        if release {
            return None;
        }
        match code & 0x03 {
            0 => MouseButton::WheelUp,
            1 => MouseButton::WheelDown,
            _ => return None,
        }
    } else {
        match code & 0x03 {
            0 => MouseButton::Left,
            1 => MouseButton::Middle,
            2 => MouseButton::Right,
            _ if release || code & 32 != 0 => MouseButton::None,
            _ => return None,
        }
    };
    Some(MouseInput {
        action,
        button,
        col,
        row,
        mods,
    })
}

#[cfg(test)]
mod tests;

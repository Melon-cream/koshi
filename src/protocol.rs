//! Internal wire format shared by the client and the server.
//!
//! A message is a 4 byte big-endian payload length followed by the postcard
//! encoded body.

use std::io;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// First released internal protocol. The client and the server are shipped
/// together, so any mismatch is a stale server, not a compatibility promise.
pub const PROTOCOL_VERSION: u16 = 1;
/// Also bounds one OSC 52 clipboard message. The client terminal's own limit is
/// not assumed here.
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

/// Flags carried in `CellData::flags`.
pub const FLAG_BOLD: u16 = 1;
pub const FLAG_DIM: u16 = 2;
pub const FLAG_ITALIC: u16 = 4;
pub const FLAG_UNDERLINE: u16 = 8;
pub const FLAG_REVERSE: u16 = 16;
pub const FLAG_HIDDEN: u16 = 32;
pub const FLAG_STRIKEOUT: u16 = 64;
pub const FLAG_WIDE_CHAR: u16 = 128;
pub const FLAG_WIDE_CHAR_SPACER: u16 = 256;
pub const FLAG_LEADING_WIDE_CHAR_SPACER: u16 = 512;
pub const FLAG_SELECTED: u16 = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientMsg {
    Hello {
        version: u16,
        cols: u16,
        rows: u16,
    },
    Input(Vec<u8>),
    /// A key the client decoded from a multi-byte escape sequence.
    Key(KeyCode),
    Mouse(MouseInput),
    Resize {
        cols: u16,
        rows: u16,
    },
    Detach,
}

/// A key that leaves the client as a typed value instead of raw bytes.
///
/// Arrows and paging keys are several bytes, and a read may end in the middle
/// of one. The client decodes them so that the server decides what they mean
/// and a pane never receives a fragment. A lone Esc is not a key here: it keeps
/// travelling as the plain byte the idle flush sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyCode {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    /// Alt plus an arrow. The client decodes both terminal encodings into these
    /// so the server can tell a plain arrow from an Alt one.
    AltUp,
    AltDown,
    AltLeft,
    AltRight,
}

/// One mouse event from the user's terminal, in cell coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseInput {
    pub action: MouseAction,
    pub button: MouseButton,
    /// 0-based cell inside the client screen (not the pane).
    pub col: u16,
    pub row: u16,
    /// xterm modifiers: 1 = shift, 2 = alt, 4 = ctrl.
    pub mods: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    /// No button (legacy release or buttonless motion).
    None,
    WheelUp,
    WheelDown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerMsg {
    Welcome {
        version: u16,
    },
    Frame(Box<Frame>),
    Bell,
    Title(String),
    /// One line of text for the user, e.g. a version mismatch.
    Notice(String),
    /// base64 text for OSC 52. The client writes it to its own terminal.
    Clipboard(String),
    /// The server accepted a detach command; the client should exit cleanly.
    Detached,
    /// Every pane is gone.
    Exited,
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    /// Screen area above the status bar.
    pub content: Rect,
    /// The visible panes, in draw order.
    pub panes: Vec<PaneFrame>,
    /// The split gutters, already resolved to the character to draw.
    pub borders: Vec<BorderCell>,
    pub status: Status,
    pub overlay: Option<Overlay>,
    /// Short lived message drawn over the content, e.g. a copy report.
    pub toast: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BorderCell {
    pub x: u16,
    pub y: u16,
    pub ch: char,
    /// The gutter belongs to the focused pane's side of its split.
    pub strong: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneFrame {
    pub id: u64,
    /// Absolute position inside `Frame::content`.
    pub rect: Rect,
    pub cols: u16,
    pub rows: u16,
    /// Only the rows that differ from the last frame sent to this client.
    pub rows_changed: Vec<RowUpdate>,
    /// Pane-relative cursor. Only the focused pane carries one.
    pub cursor: Option<CursorPos>,
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowUpdate {
    pub row: u16,
    pub cells: Vec<CellData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellData {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: u16,
}

impl Default for CellData {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            flags: 0,
        }
    }
}

/// Standard base64 with padding, the alphabet OSC 52 expects.
pub fn base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let bits = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(bits >> 18) as usize & 63] as char);
        out.push(TABLE[(bits >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(bits >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[bits as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Color {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorPos {
    pub row: u16,
    pub col: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// The complete status bar line, including the mode label.
    pub line: String,
    /// Group hints at the end of `line`, for their own colour.
    pub hint: Option<String>,
    /// Char range of the active tab's label inside `line`.
    pub active: Option<(u16, u16)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overlay {
    pub title: String,
    /// Pre-formatted rows; the selected one is already marked.
    pub items: Vec<String>,
    pub selected: usize,
    pub prompt: Option<String>,
    pub footer: String,
}

pub async fn write_msg<W, T>(writer: &mut W, msg: &T) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let body = postcard::to_stdvec(msg).map_err(invalid_data)?;
    if body.len() > MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message too large",
        ));
    }
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    writer.write_all(&frame).await?;
    writer.flush().await
}

/// Read one message. `Ok(None)` means the peer closed between frames.
pub async fn read_msg<R, T>(reader: &mut R) -> io::Result<Option<T>>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut len_bytes = [0u8; 4];
    match reader.read_exact(&mut len_bytes).await {
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(err) => return Err(err),
    }
    let len = u32::from_be_bytes(len_bytes) as usize;
    if len > MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    postcard::from_bytes(&body).map(Some).map_err(invalid_data)
}

fn invalid_data(err: postcard::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

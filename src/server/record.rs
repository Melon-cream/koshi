//! Recording the settled lines of one pane to files.
//!
//! The pane feeds the same bytes it gives the terminal emulator to a recorder.
//! Escape sequences, colours, and cursor control are dropped by the ANSI parser
//! that already ships with the terminal library, so only characters that reach
//! the screen are kept. A line becomes settled at the newline that ends it;
//! carriage returns and backspaces rewrite it in place, as on screen.
//!
//! Each settled line goes to a plain text file, and one ISO 8601 local timestamp
//! per line goes to a sidecar file whose line numbers match the body. The
//! alternate screen is not recorded: whatever a full-screen program draws stays
//! out of the log.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use alacritty_terminal::vte::ansi::{Handler, NamedPrivateMode, PrivateMode, Processor};

use crate::record::{Local, iso8601, local_offset_seconds, workspace_dir};

/// A tab stop every eight columns, like the terminal.
const TAB_WIDTH: usize = 8;

/// Where one pane's recording goes: which workspace it belongs to and which
/// pane of it is speaking.
pub(super) struct RecordTarget {
    pub(super) root: PathBuf,
    pub(super) workspace: String,
    pub(super) workspace_id: u64,
    pub(super) pane: u64,
}

pub(super) struct Recorder {
    log: File,
    times: File,
    /// Seconds the local time is ahead of UTC, taken once when the pane starts.
    offset: i64,
    parser: Processor,
    lines: Lines,
}

impl RecordTarget {
    pub(super) fn new(root: &Path, workspace: &str, workspace_id: u64, pane: u64) -> Self {
        Self {
            root: root.to_path_buf(),
            workspace: workspace.to_string(),
            workspace_id,
            pane,
        }
    }
}

impl Recorder {
    /// Create today's directory and the pane's two files.
    pub(super) fn start(target: &RecordTarget) -> io::Result<Self> {
        let offset = local_offset_seconds();
        let now = Local::now(offset);
        let day = format!("{:04}{:02}{:02}", now.year, now.month, now.day);
        let stem = format!(
            "{}-{:02}{:02}{:02}",
            target.pane, now.hour, now.minute, now.second
        );
        let date_dir = target.root.join(&day);
        let directory = date_dir.join(workspace_dir(&target.workspace, target.workspace_id));
        for path in [&target.root, &date_dir, &directory] {
            make_private_dir(path)?;
        }
        Ok(Self {
            log: open_private(&directory.join(format!("{stem}.log")))?,
            times: open_private(&directory.join(format!("{stem}.times")))?,
            offset,
            parser: Processor::new(),
            lines: Lines::default(),
        })
    }

    /// Record the lines this chunk of output settled.
    pub(super) fn feed(&mut self, bytes: &[u8]) {
        let Self {
            parser,
            lines,
            offset,
            log,
            times,
        } = self;
        parser.advance(lines, bytes);
        let mut body = String::new();
        let mut stamps = String::new();
        let stamp = iso8601(*offset, SystemTime::now());
        for line in lines.take() {
            body.push_str(&line);
            body.push('\n');
            stamps.push_str(&stamp);
            stamps.push('\n');
        }
        if body.is_empty() {
            return;
        }
        if log.write_all(body.as_bytes()).is_ok() {
            let _ = log.flush();
        }
        if times.write_all(stamps.as_bytes()).is_ok() {
            let _ = times.flush();
        }
    }

    /// Keep the line the cursor still sits on. The pane is going away, so that
    /// line can no longer change.
    pub(super) fn finish(&mut self) {
        let line = self.lines.current();
        if line.is_empty() {
            return;
        }
        let stamp = iso8601(self.offset, SystemTime::now());
        if self.log.write_all(format!("{line}\n").as_bytes()).is_ok() {
            let _ = self.log.flush();
        }
        if self.times.write_all(stamp.as_bytes()).is_ok() {
            let _ = self.times.write_all(b"\n");
            let _ = self.times.flush();
        }
    }

    /// Hand what is already written to the kernel, so the archive sees it.
    pub(super) fn flush(&mut self) -> io::Result<()> {
        self.log.flush()?;
        self.times.flush()
    }

    /// Start on the alternate screen when the pane is already there, so a
    /// recording that starts mid-session skips what a full-screen program drew.
    pub(super) fn enter_alt(&mut self) {
        self.lines.enter_alt();
    }
}

/// Move the uncompressed recordings of one workspace to its new name. Archives
/// keep the name they were written with.
pub(super) fn rename_workspace(
    root: &Path,
    old_name: &str,
    new_name: &str,
    workspace_id: u64,
) -> io::Result<()> {
    let old = workspace_dir(old_name, workspace_id);
    let new = workspace_dir(new_name, workspace_id);
    if old == new {
        return Ok(());
    }
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let from = entry.path().join(&old);
        if !from.is_dir() {
            continue;
        }
        let to = entry.path().join(&new);
        if to.exists() {
            continue;
        }
        fs::rename(&from, &to)?;
    }
    Ok(())
}

/// The line being written, and the lines this chunk settled.
#[derive(Default)]
struct Lines {
    body: Vec<char>,
    column: usize,
    settled: Vec<String>,
    /// True while the alternate screen is up; nothing on it is recorded.
    alt: bool,
}

impl Lines {
    fn take(&mut self) -> Vec<String> {
        std::mem::take(&mut self.settled)
    }

    /// The unfinished line, trailing blanks dropped.
    fn current(&self) -> String {
        self.body.iter().collect::<String>().trim_end().to_string()
    }

    fn clear(&mut self) {
        self.body.clear();
        self.column = 0;
        self.settled.clear();
    }

    fn settle(&mut self) {
        let line = self.current();
        self.settled.push(line);
        self.body.clear();
        self.column = 0;
    }

    /// Write one character at the cursor, overwriting what was there.
    fn put(&mut self, ch: char) {
        while self.body.len() < self.column {
            self.body.push(' ');
        }
        if self.column < self.body.len() {
            self.body[self.column] = ch;
        } else {
            self.body.push(ch);
        }
        self.column += 1;
    }

    fn enter_alt(&mut self) {
        if self.alt {
            return;
        }
        // The line still being typed belongs to the normal screen.
        self.body.clear();
        self.column = 0;
        self.alt = true;
    }

    fn leave_alt(&mut self) {
        if !self.alt {
            return;
        }
        // Whatever the alternate screen settled is dropped with it.
        self.clear();
        self.alt = false;
    }
}

impl Handler for Lines {
    fn input(&mut self, ch: char) {
        if self.alt {
            return;
        }
        self.put(ch);
    }

    fn linefeed(&mut self) {
        if self.alt {
            return;
        }
        self.settle();
    }

    fn carriage_return(&mut self) {
        if self.alt {
            return;
        }
        self.column = 0;
    }

    fn backspace(&mut self) {
        if self.alt {
            return;
        }
        self.column = self.column.saturating_sub(1);
    }

    fn put_tab(&mut self, count: u16) {
        if self.alt {
            return;
        }
        let stops = count as usize;
        self.column = (self.column / TAB_WIDTH + stops) * TAB_WIDTH;
    }

    fn set_private_mode(&mut self, mode: PrivateMode) {
        if is_alt(mode) {
            self.enter_alt();
        }
    }

    fn unset_private_mode(&mut self, mode: PrivateMode) {
        if is_alt(mode) {
            self.leave_alt();
        }
    }
}

fn is_alt(mode: PrivateMode) -> bool {
    matches!(
        mode,
        PrivateMode::Named(NamedPrivateMode::SwapScreenAndSetRestoreCursor)
    )
}

fn make_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

fn open_private(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collected(bytes: &[u8]) -> Vec<String> {
        let mut parser: Processor = Processor::new();
        let mut lines = Lines::default();
        parser.advance(&mut lines, bytes);
        lines.take()
    }

    #[test]
    fn newlines_settle_lines() {
        assert_eq!(collected(b"one\r\ntwo\nthree"), vec!["one", "two"]);
        assert_eq!(collected(b"\n\n"), vec!["", ""]);
    }

    #[test]
    fn escape_sequences_and_colours_are_dropped() {
        assert_eq!(
            collected(b"\x1b[31mred\x1b[0m plain\n"),
            vec!["red plain".to_string()]
        );
        assert_eq!(
            collected(b"\x1b]0;title\x07text\n"),
            vec!["text".to_string()]
        );
    }

    #[test]
    fn a_carriage_return_rewrites_the_line_in_place() {
        assert_eq!(collected(b"10%\r50%\r100%\n"), vec!["100%".to_string()]);
        assert_eq!(collected(b"abcdef\rXY\n"), vec!["XYcdef".to_string()]);
    }

    #[test]
    fn a_backspace_and_a_tab_move_the_cursor() {
        assert_eq!(collected(b"ab\x08c\n"), vec!["ac".to_string()]);
        assert_eq!(collected(b"a\tb\n"), vec!["a       b".to_string()]);
    }

    #[test]
    fn the_alternate_screen_is_skipped() {
        assert_eq!(
            collected(b"\x1b[?1049hHIDDEN\n\x1b[?1049lSHOWN\n"),
            vec!["SHOWN".to_string()]
        );
    }
}

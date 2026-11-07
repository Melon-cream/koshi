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
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use alacritty_terminal::vte::ansi::{Handler, Processor};

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
        let directory = date_dir.join(format!(
            "{}-{}",
            dir_name(&target.workspace),
            target.workspace_id
        ));
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
        let mut body = String::new();
        let line = self.lines.current();
        if !line.is_empty() {
            body.push_str(&line);
            body.push('\n');
        }
        if body.is_empty() {
            return;
        }
        let stamp = iso8601(self.offset, SystemTime::now());
        if self.log.write_all(body.as_bytes()).is_ok() {
            let _ = self.log.flush();
        }
        if self.times.write_all(stamp.as_bytes()).is_ok() {
            let _ = self.times.write_all(b"\n");
            let _ = self.times.flush();
        }
    }

    /// Drop the part of a line that was being written. Used when the pane
    /// switches to the alternate screen, which is not recorded.
    pub(super) fn reset(&mut self) {
        self.lines.clear();
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
    let old = format!("{}-{}", dir_name(old_name), workspace_id);
    let new = format!("{}-{}", dir_name(new_name), workspace_id);
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
}

impl Handler for Lines {
    fn input(&mut self, ch: char) {
        self.put(ch);
    }

    fn linefeed(&mut self) {
        self.settle();
    }

    fn carriage_return(&mut self) {
        self.column = 0;
    }

    fn backspace(&mut self) {
        self.column = self.column.saturating_sub(1);
    }

    fn put_tab(&mut self, count: u16) {
        let stops = count as usize;
        self.column = (self.column / TAB_WIDTH + stops) * TAB_WIDTH;
    }
}

/// The directory part of a workspace name. Separators and control characters
/// would move the recording out of its place, so they become `_`.
fn dir_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|ch| match ch {
            '/' | '\\' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return "_".to_string();
    }
    cleaned.to_string()
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

/// ISO 8601 local time with the UTC offset, e.g. `2026-11-08T22:31:05+09:00`.
fn iso8601(offset: i64, at: SystemTime) -> String {
    let seconds = at
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let local = Local::from_seconds(seconds + offset);
    let sign = if offset < 0 { '-' } else { '+' };
    let absolute = offset.abs();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}{:02}:{:02}",
        local.year,
        local.month,
        local.day,
        local.hour,
        local.minute,
        local.second,
        sign,
        absolute / 3600,
        (absolute % 3600) / 60
    )
}

struct Local {
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

impl Local {
    fn now(offset: i64) -> Self {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0);
        Self::from_seconds(seconds + offset)
    }

    fn from_seconds(seconds: i64) -> Self {
        let days = seconds.div_euclid(86_400);
        let rest = seconds.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        Self {
            year,
            month,
            day,
            hour: (rest / 3600) as u32,
            minute: ((rest % 3600) / 60) as u32,
            second: (rest % 60) as u32,
        }
    }
}

/// Civil date of a day count since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_part + 2) / 5 + 1) as u32;
    let month = if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// The local UTC offset, asked once of `date` so no timezone database is needed.
fn local_offset_seconds() -> i64 {
    let output = Command::new("date")
        .arg("+%z")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let Ok(output) = output else {
        return 0;
    };
    if !output.status.success() {
        return 0;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_offset(text.trim()).unwrap_or(0)
}

fn parse_offset(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != 5 {
        return None;
    }
    let sign = match bytes[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let hours: i64 = text[1..3].parse().ok()?;
    let minutes: i64 = text[3..5].parse().ok()?;
    if hours > 14 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 3600 + minutes * 60))
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
        assert_eq!(
            collected(b"10%\r50%\r100%\n"),
            vec!["100%".to_string()]
        );
        assert_eq!(collected(b"abcdef\rXY\n"), vec!["XYcdef".to_string()]);
    }

    #[test]
    fn a_backspace_and_a_tab_move_the_cursor() {
        assert_eq!(collected(b"ab\x08c\n"), vec!["ac".to_string()]);
        assert_eq!(collected(b"a\tb\n"), vec!["a       b".to_string()]);
    }

    #[test]
    fn a_known_day_becomes_its_date() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_675), (2026, 8, 10));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn timestamps_carry_the_local_offset() {
        let at = UNIX_EPOCH + std::time::Duration::from_secs(1_760_000_000);
        assert_eq!(iso8601(9 * 3600, at), "2025-10-09T17:53:20+09:00");
        assert_eq!(iso8601(-5 * 3600, at), "2025-10-09T03:53:20-05:00");
        assert_eq!(iso8601(0, at), "2025-10-09T08:53:20+00:00");
    }

    #[test]
    fn an_offset_is_five_characters_or_nothing() {
        assert_eq!(parse_offset("+0900"), Some(32_400));
        assert_eq!(parse_offset("-0530"), Some(-19_800));
        assert_eq!(parse_offset("+0000"), Some(0));
        assert_eq!(parse_offset("UTC"), None);
        assert_eq!(parse_offset("+900"), None);
        assert_eq!(parse_offset("+0960"), None);
    }

    #[test]
    fn a_name_that_would_leave_the_directory_is_flattened() {
        assert_eq!(dir_name("work"), "work");
        assert_eq!(dir_name("a/b\\c"), "a_b_c");
        assert_eq!(dir_name(".."), "_");
        assert_eq!(dir_name("  "), "_");
        assert_eq!(dir_name("日本語"), "日本語");
    }
}

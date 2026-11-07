//! One pane: the PTY, the terminal state, and the reader thread.

mod render;

use std::fs;
use std::io::{Read, Write};
use std::thread;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::Config as TermConfig;
use alacritty_terminal::term::{Term, TermMode, viewport_to_point};
use alacritty_terminal::vte::ansi::Processor;
use anyhow::{Context, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::mpsc::UnboundedSender;

use crate::server::launch::Launch;

/// Signals produced by a pane outside of its command channel.
pub enum PaneSignal {
    Output(u64, Vec<u8>),
    Event(u64, Event),
    Eof(u64),
}

#[derive(Clone)]
pub struct PaneEvents {
    id: u64,
    signals: UnboundedSender<PaneSignal>,
}

impl EventListener for PaneEvents {
    fn send_event(&self, event: Event) {
        let _ = self.signals.send(PaneSignal::Event(self.id, event));
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PaneSize {
    pub cols: usize,
    pub rows: usize,
    pub history: usize,
}

/// Everything a new pane needs beyond its identity.
pub struct PaneInit<'a> {
    pub cols: u16,
    pub rows: u16,
    pub launch: Launch<'a>,
    /// Restored history, fed before the reader thread starts.
    pub lines: &'a [String],
}

impl Dimensions for PaneSize {
    fn total_lines(&self) -> usize {
        self.rows + self.history
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

pub struct Pane {
    id: u64,
    term: Term<PaneEvents>,
    processor: Processor,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    size: PaneSize,
    exited: bool,
    /// Came back from the state file and still owes one redraw.
    restored: bool,
    /// Armed after the first attach; the next server tick sends the Ctrl-L.
    clear_pending: bool,
}

impl Pane {
    pub fn new(
        id: u64,
        history: usize,
        signals: UnboundedSender<PaneSignal>,
        init: PaneInit<'_>,
    ) -> Result<Self> {
        let cols = init.cols.max(1);
        let rows = init.rows.max(1);
        let size = PaneSize {
            cols: cols as usize,
            rows: rows as usize,
            history,
        };
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("cannot open a pty")?;

        let launch = init.launch.command();
        let mut command = CommandBuilder::new(&launch.program);
        for arg in &launch.args {
            command.arg(arg);
        }
        let term = std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".to_string());
        command.env("TERM", term);
        // A shell can tell it runs inside a pane, so a startup file can avoid
        // attaching a second time.
        command.env("KOSHI_PANE", id.to_string());
        if let Some(cwd) = launch.cwd.as_deref() {
            command.cwd(cwd);
        }
        let child = pair
            .slave
            .spawn_command(command)
            .with_context(|| format!("cannot start {}", launch.program))?;
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .context("cannot read from the pty")?;
        let writer = pair
            .master
            .take_writer()
            .context("cannot write to the pty")?;

        let mut term = Term::new(
            TermConfig {
                scrolling_history: history,
                ..TermConfig::default()
            },
            &size,
            PaneEvents {
                id,
                signals: signals.clone(),
            },
        );
        feed_history(&mut term, init.lines);
        let processor: Processor = Processor::new();

        thread::spawn(move || {
            let mut buf = [0u8; 65536];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if signals
                            .send(PaneSignal::Output(id, buf[..n].to_vec()))
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            let _ = signals.send(PaneSignal::Eof(id));
        });

        Ok(Self {
            id,
            term,
            processor,
            master: pair.master,
            writer,
            child,
            size,
            exited: false,
            restored: false,
            clear_pending: false,
        })
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    /// Working directory of the shell process, when the kernel still reports it.
    pub fn cwd(&self) -> Option<String> {
        if self.exited {
            return None;
        }
        let pid = self.child.process_id()?;
        let path = fs::read_link(format!("/proc/{pid}/cwd")).ok()?;
        path.to_str().map(str::to_string)
    }

    /// The newest `max` grid lines as text, oldest first, colours dropped.
    pub fn text_lines(&self, max: usize) -> Vec<String> {
        if self.alt_screen() || max == 0 {
            return Vec::new();
        }
        let grid = self.term.grid();
        let bottom = grid.bottommost_line();
        let mut line = grid.topmost_line();
        if (bottom.0 - line.0) as usize >= max {
            line = Line(bottom.0 - max as i32 + 1);
        }
        let mut out = Vec::new();
        while line <= bottom {
            let text: String = grid[line][..].iter().map(|cell| cell.c).collect();
            out.push(text.trim_end().to_string());
            line += 1;
        }
        out
    }

    /// Ask the pane to redraw itself once its first client has attached, so the
    /// prompt does not sit under the saved history.
    pub fn mark_restored(&mut self) {
        self.restored = true;
    }

    /// Arm the one redraw owed by a restored pane. Called after the first
    /// attach has resized the panes; the clear itself goes out on the next tick
    /// so the shell's own prompt lands first.
    pub fn arm_clear_if_restored(&mut self) {
        if self.restored {
            self.restored = false;
            self.clear_pending = true;
        }
    }

    /// Send the armed Ctrl-L. A pane on the alternate screen keeps its display.
    pub fn send_pending_clear(&mut self) {
        if !self.clear_pending {
            return;
        }
        self.clear_pending = false;
        if !self.alt_screen() {
            let _ = self.write(b"\x0c");
        }
    }

    pub fn cols(&self) -> u16 {
        self.size.cols as u16
    }

    pub fn rows(&self) -> u16 {
        self.size.rows as u16
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
    }

    pub fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> bool {
        let cols = cols.max(1);
        let rows = rows.max(1);
        if self.size.cols == cols as usize && self.size.rows == rows as usize {
            return false;
        }
        self.size.cols = cols as usize;
        self.size.rows = rows as usize;
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.term.resize(self.size);
        true
    }

    /// True when the child process has terminated.
    pub fn try_exit(&mut self) -> bool {
        if self.exited {
            return true;
        }
        match self.child.try_wait() {
            Ok(Some(_)) | Err(_) => self.exited = true,
            Ok(None) => {}
        }
        self.exited
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    pub fn mouse_reporting(&self) -> bool {
        self.term.mode().intersects(TermMode::MOUSE_MODE)
    }

    pub fn alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    pub fn app_cursor(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Move the visible window; a positive delta goes into the history.
    pub fn scroll_display(&mut self, delta: i32) {
        self.term.scroll_display(Scroll::Delta(delta));
    }

    /// Return to the live end and drop any stale selection.
    pub fn focus_live(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
        self.term.selection = None;
    }

    /// Grid point for a pane-local viewport cell.
    pub fn grid_point(&self, row: u16, col: u16) -> Point {
        let row = row.min(self.rows().saturating_sub(1)) as usize;
        let col = col.min(self.cols().saturating_sub(1)) as usize;
        viewport_to_point(self.display_offset(), Point::new(row, Column(col)))
    }

    /// Select from the press anchor to `point`. Both end cells are included, so
    /// the sides follow the direction: only a start on the left and an end on
    /// the right keep the outermost cells of a simple selection.
    pub fn select(&mut self, anchor: Point, point: Point) {
        let upward = point < anchor;
        let (start, end) = if upward {
            (Side::Right, Side::Left)
        } else {
            (Side::Left, Side::Right)
        };
        let mut selection = Selection::new(SelectionType::Simple, anchor, start);
        selection.update(point, end);
        self.term.selection = Some(selection);
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        // Dropping the master also hangs up the child, but kill it explicitly
        // so closed panes and workspaces do not leave processes behind.
        let _ = self.child.kill();
    }
}

fn feed_history(term: &mut Term<PaneEvents>, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let mut processor: Processor = Processor::new();
    for line in lines {
        let mut text = String::with_capacity(line.len() + 2);
        text.push_str(line);
        text.push_str("\r\n");
        processor.advance(term, text.as_bytes());
    }
}

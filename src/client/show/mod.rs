//! `koshi record show`: a read-only browser over the recorded output.
//!
//! Three screens in one terminal: the workspaces, the days of one workspace,
//! and the lines of one day. The lines come from the files; a running server is
//! asked for the unfinished line of its panes and nothing is ever written back.

mod tail;
mod view;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{execute, queue};

use crate::config::Config;
use crate::paths;
use crate::record::Date;
use crate::record::read::{self, DayLog, WorkspaceRecord};

use self::tail::{TAIL_REFRESH, Tail};

pub(crate) async fn run(config: Config, args: &[String]) -> Result<i32> {
    let root = config.record_root.clone().unwrap_or_else(paths::record_dir);
    let records = read::workspaces(&root);
    if records.is_empty() {
        eprintln!("koshi: no recordings under {}", root.display());
        return Ok(1);
    }
    let mut viewer = Viewer::new(root, records);
    if let Some(workspace) = args.first() {
        let record = viewer
            .find(workspace)
            .cloned()
            .ok_or_else(|| anyhow!("no recordings for {workspace}"))?;
        if let Some(date) = args.get(1) {
            let date = Date::parse(date)
                .ok_or_else(|| anyhow!("`{date}` is not a date (`YYYY-MM-DD`)"))?;
            viewer.open_workspace(&record);
            if !viewer.open_date(date) {
                eprintln!(
                    "koshi: no recordings for {} on {}",
                    record.name,
                    date.dashed()
                );
                return Ok(1);
            }
            viewer.screen = Screen::Logs;
        } else {
            viewer.open_workspace(&record);
        }
    }
    viewer.browse()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Workspaces,
    Dates,
    Logs,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Chronological,
    PerPane,
}

struct Viewer {
    root: PathBuf,
    records: Vec<WorkspaceRecord>,
    workspace: Option<WorkspaceRecord>,
    dates: Vec<Date>,
    date: Option<Date>,
    log: Option<DayLog>,
    /// Unfinished line of each running pane, refreshed while the log is open.
    live: Arc<Mutex<BTreeMap<u64, String>>>,
    mode: Mode,
    offset: usize,
    selected: usize,
    screen: Screen,
}

impl Viewer {
    fn new(root: PathBuf, records: Vec<WorkspaceRecord>) -> Self {
        Self {
            root,
            records,
            workspace: None,
            dates: Vec::new(),
            date: None,
            log: None,
            live: Arc::new(Mutex::new(BTreeMap::new())),
            mode: Mode::PerPane,
            offset: 0,
            selected: 0,
            screen: Screen::Workspaces,
        }
    }

    /// Resolve the `[workspace]` argument: an id first, then a name.
    fn find(&self, wanted: &str) -> Option<&WorkspaceRecord> {
        if let Ok(id) = wanted.parse::<u64>() {
            if let Some(record) = self.records.iter().find(|record| record.id == id) {
                return Some(record);
            }
        }
        self.records
            .iter()
            .find(|record| record.name == wanted)
            .or_else(|| {
                self.records
                    .iter()
                    .find(|record| format!("{}-{}", record.name, record.id) == wanted)
            })
    }

    fn open_workspace(&mut self, record: &WorkspaceRecord) {
        let dates = read::days(&self.root, record.id);
        if dates.is_empty() {
            return;
        }
        self.workspace = Some(WorkspaceRecord {
            id: record.id,
            name: record.name.clone(),
            days: record.days,
        });
        self.dates = dates;
        self.selected = 0;
        self.screen = Screen::Dates;
    }

    fn open_date(&mut self, date: Date) -> bool {
        let Some(workspace) = self.workspace.as_ref() else {
            return false;
        };
        match read::load(&self.root, workspace.id, date) {
            Ok(log) => {
                self.log = Some(log);
                self.date = Some(date);
                self.offset = 0;
                true
            }
            Err(err) => {
                eprintln!("koshi: cannot read {}: {err}", date.dashed());
                false
            }
        }
    }

    fn browse(&mut self) -> Result<i32> {
        let mut guard = ScreenGuard::enter()?;
        let tail = Tail::start(paths::socket_path(), self.workspace.as_ref().map(|w| w.id));
        let mut result: Result<i32> = Ok(0);
        loop {
            if let Ok(live) = tail.lines.lock() {
                if let Ok(mut held) = self.live.lock() {
                    *held = live.clone();
                }
            }
            if let Err(err) = self.draw() {
                result = Err(err.into());
                break;
            }
            if !event::poll(TAIL_REFRESH).unwrap_or(false) {
                continue;
            }
            let Ok(Event::Key(key)) = event::read() else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if self.key(key.code) == Action::Quit {
                break;
            }
            if let Some(workspace) = self.workspace.as_ref() {
                tail.follow(workspace.id);
            }
        }
        drop(tail);
        guard.restore();
        result
    }

    fn key(&mut self, code: KeyCode) -> Action {
        match self.screen {
            Screen::Workspaces => match code {
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    if self.selected + 1 < self.records.len() {
                        self.selected += 1;
                    }
                }
                KeyCode::Enter => {
                    if let Some(record) = self.records.get(self.selected).cloned() {
                        self.open_workspace(&record);
                    }
                }
                KeyCode::Char('q') => return Action::Quit,
                _ => {}
            },
            Screen::Dates => match code {
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    if self.selected + 1 < self.dates.len() {
                        self.selected += 1;
                    }
                }
                KeyCode::Enter => {
                    if let Some(date) = self.dates.get(self.selected).copied() {
                        if self.open_date(date) {
                            self.screen = Screen::Logs;
                        }
                    }
                }
                KeyCode::Esc => {
                    self.screen = Screen::Workspaces;
                    self.selected = 0;
                }
                KeyCode::Char('q') => return Action::Quit,
                _ => {}
            },
            Screen::Logs => match code {
                KeyCode::Tab => {
                    self.mode = match self.mode {
                        Mode::Chronological => Mode::PerPane,
                        Mode::PerPane => Mode::Chronological,
                    };
                    self.offset = 0;
                }
                KeyCode::Up => self.offset = self.offset.saturating_sub(1),
                KeyCode::Down => self.offset = self.offset.saturating_add(1),
                KeyCode::PageUp => self.offset = self.offset.saturating_sub(10),
                KeyCode::PageDown => self.offset = self.offset.saturating_add(10),
                KeyCode::Esc => {
                    self.screen = Screen::Dates;
                    self.offset = 0;
                }
                KeyCode::Char('q') => return Action::Quit,
                _ => {}
            },
        }
        Action::Stay
    }
}

#[derive(PartialEq, Eq)]
enum Action {
    Stay,
    Quit,
}

/// Raw mode and the alternate screen, restored on the way out.
struct ScreenGuard {
    restored: bool,
}

impl ScreenGuard {
    fn enter() -> std::io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        queue!(stdout, EnterAlternateScreen, Hide)?;
        std::io::Write::flush(&mut stdout)?;
        Ok(Self { restored: false })
    }

    fn restore(&mut self) {
        if self.restored {
            return;
        }
        self.restored = true;
        let mut stdout = std::io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen, Show);
        let _ = std::io::Write::flush(&mut stdout);
        let _ = disable_raw_mode();
    }
}

impl Drop for ScreenGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

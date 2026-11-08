//! Painting the viewer: one full frame per change, cut to the terminal width.

use std::io::{self, Write};

use super::{Mode, Screen, Viewer};
use crate::record::Local;

impl Viewer {
    pub(super) fn draw(&self) -> io::Result<()> {
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let width = cols as usize;
        let height = rows.saturating_sub(1) as usize;
        let mut lines = Vec::with_capacity(height);
        lines.push(self.header());
        lines.push(String::new());
        match self.screen {
            Screen::Workspaces => {
                for (index, record) in self.records.iter().enumerate() {
                    let marker = if index == self.selected { "> " } else { "  " };
                    lines.push(format!(
                        "{marker}{} (id {})  {} days",
                        record.name, record.id, record.days
                    ));
                }
            }
            Screen::Dates => {
                for (index, date) in self.dates.iter().enumerate() {
                    let marker = if index == self.selected { "> " } else { "  " };
                    lines.push(format!("{marker}{}", date.dashed()));
                }
            }
            Screen::Logs => {
                let visible = height.saturating_sub(2);
                lines.extend(self.log_lines().into_iter().skip(self.offset).take(visible));
            }
        }

        let mut out = String::from("\x1b[H");
        for row in 0..height {
            let text = lines.get(row).map(String::as_str).unwrap_or("");
            let text: String = text.chars().take(width).collect();
            out.push_str("\x1b[2K");
            out.push_str(&text);
            if row + 1 < height {
                out.push_str("\r\n");
            }
        }
        out.push_str("\x1b[2K");
        out.push_str(self.footer());
        let mut stdout = io::stdout();
        stdout.write_all(out.as_bytes())?;
        stdout.flush()
    }

    fn header(&self) -> String {
        match self.screen {
            Screen::Workspaces => format!("Records  {} workspaces", self.records.len()),
            Screen::Dates => match self.workspace.as_ref() {
                Some(workspace) => format!(
                    "{} (id {})  {} days",
                    workspace.name,
                    workspace.id,
                    self.dates.len()
                ),
                None => String::new(),
            },
            Screen::Logs => {
                let name = self
                    .workspace
                    .as_ref()
                    .map(|workspace| workspace.name.as_str())
                    .unwrap_or("");
                let date = self
                    .date
                    .map(|date| date.dashed())
                    .unwrap_or_else(|| "-".to_string());
                let mode = match self.mode {
                    Mode::Chronological => "CHRONO",
                    Mode::PerPane => "PER-PANE",
                };
                format!("{name} {date} [{mode}]")
            }
        }
    }

    fn footer(&self) -> &'static str {
        match self.screen {
            Screen::Workspaces => "Up/Down move  Enter open  q quit",
            Screen::Dates => "Up/Down move  Enter open  Esc back  q quit",
            Screen::Logs => "Tab mode  Up/Down scroll  PgUp/PgDn page  Esc back  q quit",
        }
    }

    /// The rows of the log screen, in the order the mode asks for.
    fn log_lines(&self) -> Vec<String> {
        let Some(log) = self.log.as_ref() else {
            return Vec::new();
        };
        let mut rows: Vec<(String, u64, String, bool)> = Vec::new();
        for line in &log.lines {
            let time = line
                .time
                .as_ref()
                .map(|stamp| short_time(stamp))
                .unwrap_or_else(|| "--:--:--".to_string());
            rows.push((time, line.pane, line.text.clone(), false));
        }
        if let Ok(live) = self.live.lock() {
            let now = now_clock();
            for (pane, text) in live.iter() {
                if !text.is_empty() {
                    rows.push((now.clone(), *pane, text.clone(), true));
                }
            }
        }

        let mut out = Vec::new();
        if log.warning {
            out.push(format!(
                "warning: {} lines shown; body and times differ in length",
                log.lines.len()
            ));
        }
        if self.mode == Mode::Chronological {
            rows.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
            for (time, pane, text, live) in rows {
                out.push(format!(
                    "{time}  p{pane}{}  {text}",
                    if live { "+" } else { "" }
                ));
            }
            return out;
        }

        rows.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
        let mut previous: Option<u64> = None;
        for (time, pane, text, live) in rows {
            if previous != Some(pane) {
                out.push(String::new());
                out.push(format!("pane {pane}"));
                previous = Some(pane);
            }
            out.push(format!("{time}{}  {text}", if live { "+" } else { "" }));
        }
        out
    }
}

/// `HH:MM:SS` out of an ISO 8601 stamp.
fn short_time(stamp: &str) -> String {
    stamp
        .get(11..19)
        .map(str::to_string)
        .unwrap_or_else(|| stamp.to_string())
}

/// The wall clock the live line is stamped with.
fn now_clock() -> String {
    let now = Local::now(crate::record::local_offset_seconds());
    format!("{:02}:{:02}:{:02}", now.hour, now.minute, now.second)
}

//! Raw mode and alternate screen handling. The guard always restores on drop.

use std::io::{self, Write};

use crossterm::cursor::{Hide, Show};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{execute, queue};

pub struct TermGuard {
    restored: bool,
}

impl TermGuard {
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        // 1002: report drags; 1006: SGR encoding with large coordinates.
        queue!(stdout, EnterAlternateScreen, Hide)?;
        queue!(stdout, crossterm::style::Print("\x1b[?1002h\x1b[?1006h"))?;
        stdout.flush()?;
        Ok(Self { restored: false })
    }

    pub fn restore(&mut self) {
        if self.restored {
            return;
        }
        self.restored = true;
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            crossterm::style::Print("\x1b[?1006l\x1b[?1002l"),
            LeaveAlternateScreen,
            Show
        );
        let _ = stdout.flush();
        let _ = disable_raw_mode();
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

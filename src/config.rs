//! Configuration file parsing. Every failure falls back to defaults.

use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::paths;

/// The modifier that turns p/t/o/s into a leader key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leader {
    /// Leaders are Ctrl+p, Ctrl+t, Ctrl+o, and Ctrl+s.
    Ctrl,
    /// Leaders are Alt+p, Alt+t, Alt+o, and Alt+s.
    Alt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub leader: Leader,
    pub scroll_lines: usize,
    pub history_lines: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            leader: Leader::Ctrl,
            scroll_lines: 3,
            history_lines: 10_000,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        match paths::config_path() {
            Some(path) => Self::from_file(&path),
            None => Self::default(),
        }
    }

    fn from_file(path: &Path) -> Self {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                eprintln!(
                    "koshi: cannot read {}: {err}; using defaults",
                    path.display()
                );
                return Self::default();
            }
        };
        match Self::parse(&text) {
            Ok(config) => config,
            Err(err) => {
                eprintln!("koshi: config {}: {err}; using defaults", path.display());
                Self::default()
            }
        }
    }

    fn parse(text: &str) -> Result<Self, String> {
        let value: toml::Value = toml::from_str(text).map_err(|err| err.to_string())?;
        let table = value.as_table().ok_or("expected a table of keys")?;
        for key in table.keys() {
            if !matches!(key.as_str(), "leader" | "scroll_lines" | "history_lines") {
                eprintln!("koshi: config: unknown key `{key}` ignored");
            }
        }
        let raw: RawConfig = toml::from_str(text).map_err(|err| err.to_string())?;
        let mut config = Self::default();
        if let Some(leader) = raw.leader {
            config.leader = parse_leader(&leader)?;
        }
        if let Some(lines) = raw.scroll_lines {
            if lines == 0 {
                return Err("scroll_lines must be at least 1".to_string());
            }
            config.scroll_lines = lines;
        }
        if let Some(lines) = raw.history_lines {
            if lines == 0 {
                return Err("history_lines must be at least 1".to_string());
            }
            config.history_lines = lines;
        }
        Ok(config)
    }
}

#[derive(Deserialize, Default)]
struct RawConfig {
    leader: Option<String>,
    scroll_lines: Option<usize>,
    history_lines: Option<usize>,
}

/// `"C"` for Ctrl leaders, `"A"` for Alt leaders.
fn parse_leader(value: &str) -> Result<Leader, String> {
    match value {
        "C" | "c" => Ok(Leader::Ctrl),
        "A" | "a" => Ok(Leader::Alt),
        other => Err(format!("leader must be \"C\" or \"A\", got {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_empty() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
        assert_eq!(Config::default().leader, Leader::Ctrl);
    }

    #[test]
    fn reads_values() {
        let config =
            Config::parse("leader = \"A\"\nscroll_lines = 5\nhistory_lines = 200\n").unwrap();
        assert_eq!(
            config,
            Config {
                leader: Leader::Alt,
                scroll_lines: 5,
                history_lines: 200
            }
        );
        assert_eq!(
            Config::parse("leader = \"C\"").unwrap().leader,
            Leader::Ctrl
        );
    }

    #[test]
    fn rejects_bad_values() {
        assert!(Config::parse("leader = \"b\"").is_err());
        assert!(Config::parse("leader = \"C-b\"").is_err());
        assert!(Config::parse("leader = \"CA\"").is_err());
        assert!(Config::parse("leader = \"\"").is_err());
        assert!(Config::parse("leader = ").is_err());
        assert!(Config::parse("scroll_lines = 0").is_err());
        assert!(Config::parse("history_lines = 0").is_err());
    }
}

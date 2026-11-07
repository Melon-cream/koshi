//! What a new pane starts: a local shell, or one ssh connection.
//!
//! The program and its arguments are decided here as plain data so that a pane
//! only builds its `CommandBuilder` from it, and so that the argument shape can
//! be tested without a pty.

use std::path::Path;

pub(super) enum Launch<'a> {
    Shell {
        cwd: Option<&'a str>,
    },
    /// The alias travels as the single argument of `ssh`; no shell sees it.
    Ssh {
        alias: &'a str,
    },
}

pub(super) struct Command {
    pub(super) program: String,
    pub(super) args: Vec<String>,
    pub(super) cwd: Option<String>,
}

impl<'a> Launch<'a> {
    /// What a workspace with this SSH alias starts: no alias means a shell.
    pub(super) fn with_alias(alias: Option<&'a str>) -> Self {
        match alias {
            Some(alias) => Self::Ssh { alias },
            None => Self::Shell { cwd: None },
        }
    }

    pub(super) fn command(self) -> Command {
        match self {
            Self::Shell { cwd } => Command {
                program: shell(),
                args: Vec::new(),
                cwd: cwd
                    .filter(|dir| Path::new(dir).is_dir())
                    .map(str::to_string),
            },
            Self::Ssh { alias } => Command {
                program: "ssh".to_string(),
                args: vec![alias.to_string()],
                cwd: None,
            },
        }
    }
}

fn shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_gets_the_alias_as_one_argument() {
        let command = Launch::Ssh { alias: "myhost" }.command();
        assert_eq!(command.program, "ssh");
        assert_eq!(command.args, vec!["myhost".to_string()]);
        assert_eq!(command.cwd, None);
    }

    #[test]
    fn an_alias_with_shell_syntax_stays_one_argument() {
        let alias = "my host; touch /tmp/x $HOME";
        let command = Launch::Ssh { alias }.command();
        assert_eq!(command.program, "ssh");
        assert_eq!(command.args, vec![alias.to_string()]);
        assert_eq!(command.cwd, None);
    }

    #[test]
    fn the_shell_keeps_its_directory() {
        let dir = std::env::temp_dir();
        let dir = dir.to_str().unwrap();
        let command = Launch::Shell { cwd: Some(dir) }.command();
        let expected = std::env::var("SHELL")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "/bin/sh".to_string());
        assert_eq!(command.program, expected);
        assert!(command.args.is_empty());
        assert_eq!(command.cwd.as_deref(), Some(dir));
    }

    #[test]
    fn a_missing_directory_is_dropped() {
        let command = Launch::Shell {
            cwd: Some("/nonexistent/koshi-launch"),
        }
        .command();
        assert_eq!(command.cwd, None);
    }
}

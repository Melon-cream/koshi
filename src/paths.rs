//! Filesystem locations used by the server and the client.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

pub const APP_NAME: &str = "koshi";
pub const SOCKET_FILE: &str = "koshi.sock";
pub const CONFIG_FILE: &str = "config.toml";
pub const STATE_FILE: &str = "state.bin";

unsafe extern "C" {
    fn geteuid() -> u32;
}

fn non_empty_env(key: &str) -> Option<OsString> {
    env::var_os(key).filter(|value| !value.is_empty())
}

pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = non_empty_env("KOSHI_RUNTIME_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(dir) = non_empty_env("XDG_RUNTIME_DIR") {
        let dir = PathBuf::from(dir);
        // WSL without systemd points XDG_RUNTIME_DIR at a directory that does
        // not exist, and creating it under /run needs root.
        if dir.is_dir() {
            return dir.join(APP_NAME);
        }
    }
    fallback_runtime_dir()
}

/// Where the socket lives when the session runtime directory is unusable.
fn fallback_runtime_dir() -> PathBuf {
    let uid = unsafe { geteuid() };
    std::env::temp_dir().join(format!("{APP_NAME}-{uid}"))
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join(SOCKET_FILE)
}

/// Create the runtime directory with owner-only permissions.
pub fn ensure_runtime_dir() -> io::Result<PathBuf> {
    let dir = runtime_dir();
    fs::create_dir_all(&dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

/// Directory that holds the saved session state.
///
/// `KOSHI_STATE_DIR` and an explicit `KOSHI_RUNTIME_DIR` pin the state to one
/// instance, which is what the end-to-end tests use for isolation.
pub fn state_dir() -> PathBuf {
    if let Some(dir) = non_empty_env("KOSHI_STATE_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(dir) = non_empty_env("KOSHI_RUNTIME_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(dir) = non_empty_env("XDG_STATE_HOME") {
        return PathBuf::from(dir).join(APP_NAME);
    }
    match non_empty_env("HOME") {
        Some(home) => PathBuf::from(home)
            .join(".local")
            .join("state")
            .join(APP_NAME),
        None => runtime_dir(),
    }
}

pub fn state_path() -> PathBuf {
    state_dir().join(STATE_FILE)
}

/// Create the state directory with owner-only permissions.
pub fn ensure_state_dir() -> io::Result<PathBuf> {
    let dir = state_dir();
    fs::create_dir_all(&dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

pub fn config_path() -> Option<PathBuf> {
    if let Some(dir) = non_empty_env("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(dir).join(APP_NAME).join(CONFIG_FILE));
    }
    let home = non_empty_env("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join(APP_NAME)
            .join(CONFIG_FILE),
    )
}

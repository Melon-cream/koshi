//! Saved session state: the on-disk format, its limits, and restore.
//!
//! The file is `state.bin` under the state directory. Layout is:
//! [`MAGIC`] (6) + format version (u16 LE) + payload length (u32 LE) +
//! postcard payload + FNV-1a 64 checksum (u64 LE) of the payload.
//! Anything that does not decode cleanly is dropped: the server then starts
//! empty and tells the first client.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use serde::{Deserialize, Serialize};

use crate::paths;
use crate::server::layout::Layout;
use crate::server::pane::{Pane, PaneInit};
use crate::server::session::Session;
use crate::server::tab::{Tab, Workspace};

const MAGIC: &[u8] = b"KOSHI\0";
const FORMAT: u16 = 1;
const HEADER_LEN: usize = MAGIC.len() + 2 + 4;
const CHECKSUM_LEN: usize = 8;

/// Newest history lines kept for one pane.
const MAX_PANE_LINES: usize = 2000;
/// History budget for the whole file.
const MAX_HISTORY_BYTES: usize = 400 * 1024;

const TMP_FILE: &str = ".state.bin.tmp";

pub const UNREADABLE: &str = "koshi: saved state was unreadable; starting empty";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedState {
    pub active: usize,
    pub workspaces: Vec<SavedWorkspace>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWorkspace {
    pub name: String,
    pub active: usize,
    pub tabs: Vec<SavedTab>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTab {
    pub title: String,
    pub layout: Layout,
    pub focus: u64,
    pub zoomed: bool,
    pub panes: Vec<SavedPane>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedPane {
    pub id: u64,
    pub cwd: Option<String>,
    pub cols: u16,
    pub rows: u16,
    /// History as plain text; colours and attributes are not restored.
    pub lines: Vec<String>,
}

/// What the state file had to offer at startup.
pub struct Loaded {
    pub state: Option<SavedState>,
    /// One line for the first client when the file existed but was unusable.
    pub warning: Option<String>,
}

pub fn load() -> Loaded {
    let path = paths::state_path();
    match fs::read(&path) {
        Ok(bytes) => match decode(&bytes) {
            Ok(state) => Loaded {
                state: Some(state),
                warning: None,
            },
            Err(err) => {
                eprintln!("koshi: {}: {err}", path.display());
                Loaded {
                    state: None,
                    warning: Some(UNREADABLE.to_string()),
                }
            }
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => Loaded {
            state: None,
            warning: None,
        },
        Err(err) => {
            eprintln!("koshi: cannot read {}: {err}", path.display());
            Loaded {
                state: None,
                warning: Some(UNREADABLE.to_string()),
            }
        }
    }
}

/// Write the state through a temporary file in the same directory.
pub fn write(state: &SavedState) -> io::Result<()> {
    let dir = paths::ensure_state_dir()?;
    let bytes = encode(state).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let tmp = dir.join(TMP_FILE);
    let _ = fs::remove_file(&tmp);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    fs::rename(&tmp, dir.join(paths::STATE_FILE))
}

fn encode(state: &SavedState) -> Result<Vec<u8>, String> {
    let payload = postcard::to_stdvec(state).map_err(|err| err.to_string())?;
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len() + CHECKSUM_LEN);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    out.extend_from_slice(&fnv1a(&payload).to_le_bytes());
    Ok(out)
}

fn decode(bytes: &[u8]) -> Result<SavedState, String> {
    if bytes.len() < HEADER_LEN + CHECKSUM_LEN {
        return Err("file is too short".to_string());
    }
    if &bytes[..MAGIC.len()] != MAGIC {
        return Err("not a koshi state file".to_string());
    }
    let version = u16::from_le_bytes([bytes[6], bytes[7]]);
    if version != FORMAT {
        return Err(format!("format version {version} is not supported"));
    }
    let len = u32::from_le_bytes(bytes[8..12].try_into().expect("4 bytes")) as usize;
    let end = HEADER_LEN
        .checked_add(len)
        .ok_or("payload length overflows")?;
    if bytes.len() != end + CHECKSUM_LEN {
        return Err("payload length does not match the file".to_string());
    }
    let payload = &bytes[HEADER_LEN..end];
    let stored = u64::from_le_bytes(bytes[end..].try_into().expect("8 bytes"));
    if fnv1a(payload) != stored {
        return Err("checksum mismatch".to_string());
    }
    postcard::from_bytes(payload).map_err(|err| format!("cannot decode the payload: {err}"))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

impl SavedState {
    /// Keep the newest lines within the per-pane and total limits.
    fn trim_history(&mut self) {
        let mut panes: Vec<&mut SavedPane> = self
            .workspaces
            .iter_mut()
            .flat_map(|workspace| &mut workspace.tabs)
            .flat_map(|tab| &mut tab.panes)
            .collect();
        for pane in &mut panes {
            if pane.lines.len() > MAX_PANE_LINES {
                let drop = pane.lines.len() - MAX_PANE_LINES;
                pane.lines.drain(..drop);
            }
        }
        let mut bytes: Vec<usize> = panes
            .iter()
            .map(|pane| pane.lines.iter().map(|line| line.len() + 1).sum())
            .collect();
        let mut total: usize = bytes.iter().sum();
        while total > MAX_HISTORY_BYTES {
            let Some(index) = (0..panes.len())
                .filter(|index| bytes[*index] > 0)
                .max_by_key(|index| bytes[*index])
            else {
                break;
            };
            let freed = panes[index].lines.remove(0).len() + 1;
            bytes[index] -= freed;
            total -= freed;
        }
    }
}

impl Session {
    /// Everything that belongs in the state file.
    pub fn snapshot(&self) -> SavedState {
        SavedState {
            active: self.active,
            workspaces: self
                .workspaces
                .iter()
                .map(|workspace| SavedWorkspace {
                    name: workspace.name.clone(),
                    active: workspace.active,
                    tabs: workspace
                        .tabs
                        .iter()
                        .map(|tab| SavedTab {
                            title: tab.title.clone(),
                            layout: tab.layout.clone(),
                            focus: tab.focus,
                            zoomed: tab.zoomed,
                            panes: tab
                                .panes
                                .iter()
                                .map(|pane| SavedPane {
                                    id: pane.id(),
                                    cwd: pane.cwd(),
                                    cols: pane.cols(),
                                    rows: pane.rows(),
                                    lines: pane.text_lines(MAX_PANE_LINES),
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// Persist the session. A failure is reported and otherwise ignored.
    pub fn save(&self) {
        let mut state = self.snapshot();
        state.trim_history();
        if let Err(err) = write(&state) {
            eprintln!("koshi: cannot save the session state: {err}");
        }
    }

    /// Replace the session with the saved one. False when nothing was usable.
    pub(super) fn restore(&mut self, saved: SavedState) -> bool {
        let mut workspaces = Vec::new();
        for saved_workspace in saved.workspaces {
            let mut tabs = Vec::new();
            for saved_tab in saved_workspace.tabs {
                if let Some(tab) = self.restore_tab(saved_tab) {
                    tabs.push(tab);
                }
            }
            if tabs.is_empty() {
                continue;
            }
            let active = saved_workspace.active.min(tabs.len() - 1);
            workspaces.push(Workspace {
                name: saved_workspace.name,
                tabs,
                active,
                previous_tab: None,
            });
        }
        if workspaces.is_empty() {
            return false;
        }
        self.active = saved.active.min(workspaces.len() - 1);
        self.workspaces = workspaces;
        true
    }

    fn restore_tab(&mut self, saved: SavedTab) -> Option<Tab> {
        let mut leaves = Vec::new();
        saved.layout.leaves(&mut leaves);
        let mut panes = Vec::new();
        for saved_pane in &saved.panes {
            if !leaves.contains(&saved_pane.id) {
                continue;
            }
            let init = PaneInit {
                cols: saved_pane.cols,
                rows: saved_pane.rows,
                cwd: saved_pane.cwd.as_deref(),
                lines: &saved_pane.lines,
            };
            match Pane::new(saved_pane.id, self.history, self.signals.clone(), init) {
                Ok(mut pane) => {
                    pane.mark_restored();
                    panes.push(pane);
                }
                Err(err) => {
                    eprintln!("koshi: cannot restore pane {}: {err:#}", saved_pane.id)
                }
            }
        }
        if panes.is_empty() {
            return None;
        }
        let mut layout = saved.layout;
        for leaf in leaves {
            if !panes.iter().any(|pane| pane.id() == leaf) {
                layout.remove(leaf);
            }
        }
        let ids: Vec<u64> = panes.iter().map(Pane::id).collect();
        for id in &ids {
            self.next_pane = self.next_pane.max(id + 1);
        }
        let focus = if ids.contains(&saved.focus) {
            saved.focus
        } else {
            ids[0]
        };
        let zoomed = saved.zoomed && focus == saved.focus;
        Some(Tab {
            title: saved.title,
            layout,
            focus,
            previous_focus: None,
            zoomed,
            panes,
            floating: None,
            float_shown: false,
        })
    }
}

#[cfg(test)]
mod tests;

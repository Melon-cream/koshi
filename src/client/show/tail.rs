//! Asking the server for the unfinished line of the running panes.
//!
//! The viewer sends `RecordTail` without a `Hello`, so the server never treats
//! it as a client that wants frames. The connection lives on a thread of its
//! own and refreshes once a second; a failed query leaves the file view as is.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::protocol::{self, ClientMsg, PaneTail, ServerMsg};

/// How often the running panes' unfinished lines are fetched again.
pub(super) const TAIL_REFRESH: Duration = Duration::from_secs(1);
/// How long the viewer waits for the server's answer.
const TAIL_TIMEOUT: Duration = Duration::from_millis(500);

pub(super) struct Tail {
    stop: Arc<AtomicBool>,
    pub(super) lines: Arc<Mutex<BTreeMap<u64, String>>>,
    workspace: Arc<Mutex<Option<u64>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Tail {
    pub(super) fn start(socket: PathBuf, workspace: Option<u64>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let lines = Arc::new(Mutex::new(BTreeMap::new()));
        let wanted = Arc::new(Mutex::new(workspace));
        let handle = {
            let stop = Arc::clone(&stop);
            let lines = Arc::clone(&lines);
            let wanted = Arc::clone(&wanted);
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let workspace = *wanted.lock().unwrap_or_else(|err| err.into_inner());
                    if let Some(workspace) = workspace {
                        let panes = query(&socket, workspace).unwrap_or_default();
                        let mut held = lines.lock().unwrap_or_else(|err| err.into_inner());
                        held.clear();
                        for PaneTail { pane, line } in panes {
                            held.insert(pane, line);
                        }
                    }
                    thread::sleep(TAIL_REFRESH);
                }
            })
        };
        Self {
            stop,
            lines,
            workspace: wanted,
            handle: Some(handle),
        }
    }

    pub(super) fn follow(&self, workspace: u64) {
        if let Ok(mut wanted) = self.workspace.lock() {
            *wanted = Some(workspace);
        }
    }
}

impl Drop for Tail {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn query(socket: &Path, workspace: u64) -> Option<Vec<PaneTail>> {
    let mut stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(TAIL_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TAIL_TIMEOUT)).ok()?;
    let body = postcard::to_stdvec(&ClientMsg::RecordTail { workspace }).ok()?;
    stream.write_all(&(body.len() as u32).to_be_bytes()).ok()?;
    stream.write_all(&body).ok()?;
    loop {
        let mut len = [0u8; 4];
        stream.read_exact(&mut len).ok()?;
        let len = u32::from_be_bytes(len) as usize;
        if len > protocol::MAX_FRAME_LEN {
            return None;
        }
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).ok()?;
        if let Ok(ServerMsg::RecordTail { panes }) = postcard::from_bytes::<ServerMsg>(&body) {
            return Some(panes);
        }
        // Anything else was not asked for; keep reading until the timeout.
    }
}

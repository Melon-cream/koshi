//! Unix socket listener and the single task that owns all session state.

mod borders;
mod cells;
mod commands;
mod focus;
mod frames;
mod geom;
mod input;
mod keymap;
mod launch;
mod layout;
mod mouse;
mod mouse_encode;
mod overlay;
mod pane;
mod panes;
mod pointer;
mod record;
mod resize;
mod scroll;
mod select;
mod session;
mod state;
mod tab;
mod tabs;

use std::fs;
use std::io;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use alacritty_terminal::event::Event;
use anyhow::{Context, Result};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::config::Config;
use crate::paths;
use crate::protocol::{self, ClientMsg, ServerMsg};

use self::pane::PaneSignal;
use self::session::{RecordSettings, Session};

const SIGHUP: i32 = 1;
const SIG_IGN: usize = 1;

unsafe extern "C" {
    fn setsid() -> i32;
    fn signal(signum: i32, handler: usize) -> usize;
}

enum CoreMsg {
    NewClient {
        id: u64,
        tx: UnboundedSender<ServerMsg>,
    },
    Client {
        id: u64,
        msg: ClientMsg,
    },
    ClientGone {
        id: u64,
    },
}

pub async fn run(daemon: bool) -> Result<i32> {
    if daemon {
        detach_process();
    }
    let config = Config::load();
    let runtime_dir =
        paths::ensure_runtime_dir().context("cannot prepare the runtime directory")?;
    let socket = runtime_dir.join(paths::SOCKET_FILE);

    let listener = match bind_socket(&socket).await {
        Ok(Some(listener)) => listener,
        Ok(None) => return Ok(0),
        Err(err) => return Err(err),
    };
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
        .context("cannot restrict the socket permissions")?;

    let (core_tx, core_rx) = mpsc::unbounded_channel();
    let (pane_tx, pane_rx) = mpsc::unbounded_channel();
    let record_root = config.record_root.clone().unwrap_or_else(paths::record_dir);
    let retain_days = config.record_retain_days;
    let mut session = match Session::new(
        config.history_lines,
        config.scroll_lines,
        pane_tx,
        config.leader,
        RecordSettings {
            root: record_root.clone(),
            retain_days,
        },
        80,
        24,
    ) {
        Ok(session) => session,
        Err(err) => {
            let _ = fs::remove_file(&socket);
            return Err(err);
        }
    };

    tokio::spawn(accept_loop(listener, core_tx));
    let archive = Archive {
        root: record_root,
        busy: Arc::new(AtomicBool::new(false)),
    };
    let result = core_loop(&mut session, core_rx, pane_rx, archive).await;

    session.shutdown();
    session.save();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = fs::remove_file(&socket);
    result.map(|()| 0)
}

fn detach_process() {
    // Ignore a hangup from the terminal the client was started on.
    unsafe {
        let _ = setsid();
        let _ = signal(SIGHUP, SIG_IGN);
    }
}

/// `Ok(None)` means another server already owns the socket.
async fn bind_socket(path: &Path) -> Result<Option<UnixListener>> {
    match UnixListener::bind(path) {
        Ok(listener) => Ok(Some(listener)),
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
            if UnixStream::connect(path).await.is_ok() {
                return Ok(None);
            }
            if let Ok(meta) = fs::symlink_metadata(path) {
                if meta.file_type().is_socket() {
                    let _ = fs::remove_file(path);
                }
            }
            UnixListener::bind(path).map(Some).map_err(Into::into)
        }
        Err(err) => Err(err.into()),
    }
}

async fn accept_loop(listener: UnixListener, core: UnboundedSender<CoreMsg>) {
    let mut next_id = 1u64;
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(_) => break,
        };
        spawn_client(stream, core.clone(), next_id);
        next_id += 1;
    }
}

fn spawn_client(stream: UnixStream, core: UnboundedSender<CoreMsg>, id: u64) {
    let (mut reader, mut writer) = stream.into_split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMsg>();

    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if protocol::write_msg(&mut writer, &msg).await.is_err() {
                break;
            }
        }
    });

    tokio::spawn(async move {
        if core.send(CoreMsg::NewClient { id, tx }).is_err() {
            return;
        }
        while let Ok(Some(msg)) = protocol::read_msg::<_, ClientMsg>(&mut reader).await {
            if core.send(CoreMsg::Client { id, msg }).is_err() {
                return;
            }
        }
        let _ = core.send(CoreMsg::ClientGone { id });
    });
}

/// The weekly archive step, run off the core loop on a thread of its own.
struct Archive {
    root: PathBuf,
    /// Set while a thread runs, so a slow pass is not started twice.
    busy: Arc<AtomicBool>,
}

impl Archive {
    fn start(&self, retain_days: u32) {
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let root = self.root.clone();
        let busy = Arc::clone(&self.busy);
        std::thread::spawn(move || {
            if let Err(err) = crate::record::archive::run(&root, retain_days) {
                eprintln!("koshi: cannot archive the recordings: {err}");
            }
            busy.store(false, Ordering::SeqCst);
        });
    }
}

async fn core_loop(
    session: &mut Session,
    mut core_rx: UnboundedReceiver<CoreMsg>,
    mut pane_rx: UnboundedReceiver<PaneSignal>,
    archive: Archive,
) -> Result<()> {
    let mut ticker = tokio::time::interval(Duration::from_millis(12));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut saver = tokio::time::interval(Duration::from_secs(3));
    saver.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut archive_tick = tokio::time::interval(Duration::from_secs(7 * 24 * 3600));
    archive_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("cannot watch for SIGTERM")?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .context("cannot watch for SIGINT")?;

    loop {
        tokio::select! {
            biased;
            _ = terminate.recv() => break,
            _ = interrupt.recv() => break,
            msg = core_rx.recv() => match msg {
                Some(msg) => {
                    if handle_core_msg(session, msg) {
                        break;
                    }
                }
                None => break,
            },
            signal = pane_rx.recv() => match signal {
                Some(signal) => {
                    if handle_pane_signal(session, signal) {
                        break;
                    }
                }
                None => break,
            },
            _ = saver.tick() => {
                if session.pending_save {
                    session.pending_save = false;
                    session.save();
                }
            },
            _ = archive_tick.tick() => {
                session.flush_recorders();
                archive.start(session.retain_days);
            },
            _ = ticker.tick() => {
                if session.poll_panes() {
                    break;
                }
                session.poll_restored();
                session.poll_toast();
                session.tick_mouse();
                session.flush();
            }
        }
        if session.is_empty() {
            break;
        }
    }
    Ok(())
}

/// True when the server should stop.
fn handle_core_msg(session: &mut Session, msg: CoreMsg) -> bool {
    match msg {
        CoreMsg::NewClient { id, tx } => session.add_client(id, tx),
        CoreMsg::ClientGone { id } => session.remove_client(id),
        CoreMsg::Client { id, msg } => match msg {
            ClientMsg::Hello {
                version,
                cols,
                rows,
            } => {
                session.attach(id, version, cols, rows);
            }
            ClientMsg::Mouse(event) => {
                if let Err(err) = session.handle_mouse(id, event) {
                    session.notice(id, format!("{err:#}"));
                }
            }
            ClientMsg::Input(bytes) => {
                if let Err(err) = session.handle_input(id, &bytes) {
                    session.notice(id, format!("{err:#}"));
                }
            }
            ClientMsg::Key(key) => {
                if let Err(err) = session.handle_key(id, key) {
                    session.notice(id, format!("{err:#}"));
                }
            }
            ClientMsg::Resize { cols, rows } => session.resize(cols, rows),
            ClientMsg::Detach => session.remove_client(id),
            ClientMsg::RecordTail { workspace } => {
                let panes = session.record_tail(workspace);
                if let Some(client) = session.client_mut(id) {
                    let _ = client.tx.send(ServerMsg::RecordTail { panes });
                }
            }
        },
    }
    session.is_empty()
}

/// True when the server should stop.
fn handle_pane_signal(session: &mut Session, signal: PaneSignal) -> bool {
    match signal {
        PaneSignal::Output(id, bytes) => {
            session.output(id, &bytes);
            false
        }
        PaneSignal::Eof(id) => session.close_pane(id),
        PaneSignal::Event(id, event) => match event {
            Event::PtyWrite(text) => {
                if let Some(pane) = session.pane_by_id_mut(id) {
                    let _ = pane.write(text.as_bytes());
                }
                false
            }
            Event::Title(title) => {
                session.broadcast(ServerMsg::Title(title));
                false
            }
            Event::Bell => {
                session.broadcast(ServerMsg::Bell);
                false
            }
            Event::Exit | Event::ChildExit(_) => session.close_pane(id),
            _ => false,
        },
    }
}

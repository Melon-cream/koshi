//! Attach mode: terminal setup, input forwarding, and rendering.

mod input;
mod paint;
mod render;
mod term;

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;

use crate::config::Config;
use crate::paths;
use crate::protocol::{self, ClientMsg, PROTOCOL_VERSION, ServerMsg};

use self::input::{ClientInput, InputParser};
use self::render::Mirror;
use self::term::TermGuard;

/// How long a trailing ESC waits for the rest of a possible escape sequence.
const ESC_IDLE: Duration = Duration::from_millis(50);
/// How long a server notice keeps the status row.
const NOTICE_HOLD: Duration = Duration::from_secs(3);

pub async fn attach(config: Config) -> Result<i32> {
    let socket = paths::socket_path();
    let stream = match UnixStream::connect(&socket).await {
        Ok(stream) => stream,
        Err(_) => {
            spawn_server()?;
            wait_for_server(&socket).await?
        }
    };
    run_client(stream, config).await
}

fn spawn_server() -> Result<()> {
    let exe = std::env::current_exe().context("cannot locate the koshi binary")?;
    Command::new(exe)
        .arg("server")
        .arg("--daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("cannot start the koshi server")?;
    Ok(())
}

async fn wait_for_server(socket: &Path) -> Result<UnixStream> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match UnixStream::connect(socket).await {
            Ok(stream) => return Ok(stream),
            Err(err) => {
                if Instant::now() >= deadline {
                    return Err(anyhow!("cannot connect to {}: {err}", socket.display()));
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }
}

async fn run_client(stream: UnixStream, _config: Config) -> Result<i32> {
    let (mut read_half, mut write_half) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<ClientMsg>();
    tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if protocol::write_msg(&mut write_half, &msg).await.is_err() {
                break;
            }
        }
        let _ = write_half.shutdown().await;
    });

    let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<ServerMsg>();
    tokio::spawn(async move {
        while let Ok(Some(msg)) = protocol::read_msg::<_, ServerMsg>(&mut read_half).await {
            if msg_tx.send(msg).is_err() {
                break;
            }
        }
    });

    let guard = TermGuard::enter().context("cannot switch the terminal to raw mode")?;
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let mut mirror = Mirror::new();
    let _ = out_tx.send(ClientMsg::Hello {
        version: PROTOCOL_VERSION,
        cols,
        rows,
    });

    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Option<ClientInput>>();
    std::thread::spawn(move || {
        let (raw_tx, raw_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut stdin = io::stdin();
            let mut buf = [0u8; 4096];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if raw_tx.send(buf[..n].to_vec()).is_err() {
                            return;
                        }
                    }
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });

        let mut parser = InputParser::new();
        loop {
            let mut events = Vec::new();
            let disconnected = match raw_rx.recv_timeout(ESC_IDLE) {
                Ok(chunk) => {
                    parser.feed(&chunk, &mut events);
                    false
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    parser.flush_idle(&mut events);
                    false
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => true,
            };
            for event in events {
                if input_tx.send(Some(event)).is_err() {
                    return;
                }
            }
            if disconnected {
                let _ = input_tx.send(None);
                break;
            }
        }
    });

    let mut winch = signal(SignalKind::window_change()).context("cannot watch for resizes")?;

    let mut stdout = io::stdout();
    let mut notice: Option<String> = None;
    // A notice that arrived before the first frame, so it had nowhere to go.
    let mut held_notice: Option<String> = None;
    // When the status bar takes a drawn notice back.
    let mut notice_until: Option<Instant> = None;
    let mut failure: Option<String> = None;
    let mut detached = false;

    'session: loop {
        let notice_left = notice_until
            .map(|at| at.saturating_duration_since(Instant::now()))
            .unwrap_or(NOTICE_HOLD);
        tokio::select! {
            msg = msg_rx.recv() => match msg {
                None => {
                    failure = Some(
                        notice.take().unwrap_or_else(|| "server closed the connection".to_string()),
                    );
                    break;
                }
                Some(ServerMsg::Welcome { version }) => {
                    if version != PROTOCOL_VERSION {
                        failure = Some(format!(
                            "server protocol {version} does not match client protocol {PROTOCOL_VERSION}"
                        ));
                        break;
                    }
                }
                Some(ServerMsg::Frame(frame)) => {
                    let ansi = mirror.apply(&frame);
                    emit(&mut stdout, ansi.as_bytes())?;
                    if let Some(text) = held_notice.take() {
                        let ansi = mirror.notice(&text);
                        emit(&mut stdout, ansi.as_bytes())?;
                        notice_until = Some(Instant::now() + NOTICE_HOLD);
                    }
                }
                Some(ServerMsg::Bell) => emit(&mut stdout, b"\x07")?,
                Some(ServerMsg::Title(title)) => {
                    emit(&mut stdout, format!("\x1b]0;{title}\x07").as_bytes())?;
                }
                Some(ServerMsg::Notice(text)) => {
                    notice = Some(text.clone());
                    if mirror.sized() {
                        let ansi = mirror.notice(&text);
                        emit(&mut stdout, ansi.as_bytes())?;
                        notice_until = Some(Instant::now() + NOTICE_HOLD);
                    } else {
                        held_notice = Some(text);
                    }
                }
                Some(ServerMsg::Clipboard(payload)) => {
                    emit(&mut stdout, format!("\x1b]52;c;{payload}\x07").as_bytes())?;
                }
                Some(ServerMsg::Detached) => {
                    detached = true;
                    break;
                }
                Some(ServerMsg::Exited) => break,
                Some(ServerMsg::Error(err)) => {
                    failure = Some(err);
                    break;
                }
            },
            input = input_rx.recv() => match input {
                None | Some(None) => {
                    detached = true;
                    break 'session;
                }
                Some(Some(ClientInput::Bytes(bytes))) => {
                    let _ = out_tx.send(ClientMsg::Input(bytes));
                }
                Some(Some(ClientInput::Key(key))) => {
                    let _ = out_tx.send(ClientMsg::Key(key));
                }
                Some(Some(ClientInput::Mouse(event))) => {
                    let _ = out_tx.send(ClientMsg::Mouse(event));
                }
            },
            _ = winch.recv() => {
                if let Ok((cols, rows)) = crossterm::terminal::size() {
                    let _ = out_tx.send(ClientMsg::Resize { cols, rows });
                }
            },
            _ = tokio::time::sleep(notice_left) => {
                if notice_until.take().is_some()
                    && let Some(ansi) = mirror.status_refresh()
                {
                    emit(&mut stdout, ansi.as_bytes())?;
                }
            }
        }
    }

    if detached {
        let _ = out_tx.send(ClientMsg::Detach);
    }
    drop(guard);
    if detached {
        // Give the writer task a chance to deliver the detach message.
        drop(out_tx);
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    if let Some(text) = failure {
        eprintln!("koshi: {text}");
        return Ok(1);
    }
    Ok(0)
}

fn emit(stdout: &mut io::Stdout, bytes: &[u8]) -> Result<()> {
    stdout.write_all(bytes)?;
    stdout.flush()?;
    Ok(())
}

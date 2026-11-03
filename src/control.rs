//! Stop a running server without speaking its protocol.
//!
//! A protocol mismatch leaves the user with a server no client can talk to, so
//! `koshi kill` finds the server through the socket itself: the kernel hands
//! over the peer pid of a Unix socket connection, and SIGTERM starts the normal
//! shutdown (state save, socket removal) on any version.

use std::io;
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use tokio::net::UnixStream;

use crate::paths;

const SOL_SOCKET: i32 = 1;
const SO_PEERCRED: i32 = 17;
const SIGTERM: i32 = 15;
const WAIT: Duration = Duration::from_secs(5);

#[repr(C)]
struct Credentials {
    pid: i32,
    uid: u32,
    gid: u32,
}

unsafe extern "C" {
    fn getsockopt(fd: i32, level: i32, name: i32, value: *mut u8, len: *mut u32) -> i32;
    #[link_name = "kill"]
    fn c_kill(pid: i32, signal: i32) -> i32;
}

/// The C `kill` and this module's entry point cannot share a name, so this
/// wrapper carries the public name.
pub async fn kill() -> Result<i32> {
    stop().await
}

/// Stopping an absent server is not an error worth a stack trace, but the exit
/// code still says that nothing was stopped.
async fn stop() -> Result<i32> {
    let socket = paths::socket_path();
    let stream = match UnixStream::connect(&socket).await {
        Ok(stream) => stream,
        Err(err) => {
            println!("koshi: no server is running ({})", socket.display());
            if socket.exists() {
                // A socket with no listener is left behind by a killed server.
                let _ = std::fs::remove_file(&socket);
                println!("koshi: removed the stale socket");
            }
            let _ = err;
            return Ok(1);
        }
    };
    let pid = peer_pid(&stream).context("cannot identify the server process")?;
    let sent = unsafe { c_kill(pid, SIGTERM) };
    if sent != 0 {
        return Err(anyhow!(
            "cannot stop the server (pid {pid}): {}",
            io::Error::last_os_error()
        ));
    }
    if wait_for_exit(&socket, pid).await {
        println!("koshi: stopped the server (pid {pid})");
        Ok(0)
    } else {
        Err(anyhow!(
            "the server (pid {pid}) did not stop within 5 seconds"
        ))
    }
}

fn peer_pid(stream: &UnixStream) -> Result<i32> {
    let mut credentials = Credentials {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<Credentials>() as u32;
    let result = unsafe {
        getsockopt(
            stream.as_raw_fd(),
            SOL_SOCKET,
            SO_PEERCRED,
            &mut credentials as *mut Credentials as *mut u8,
            &mut len,
        )
    };
    if result != 0 {
        return Err(anyhow!(
            "SO_PEERCRED failed: {}",
            io::Error::last_os_error()
        ));
    }
    if credentials.pid <= 0 {
        return Err(anyhow!("the socket peer reported no pid"));
    }
    Ok(credentials.pid)
}

/// True once the socket is gone and the process has either exited or become a
/// zombie. The server removes the socket last, so without the process check a
/// script could still see the dying process right after this returns. A zombie
/// counts as gone because nothing will reap it when its parent already exited.
async fn wait_for_exit(socket: &std::path::Path, pid: i32) -> bool {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if !socket.exists() && !alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// False when the pid is gone, unreadable, or a zombie.
fn alive(pid: i32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some(rest) = stat.rsplit_once(')').map(|(_, rest)| rest.trim_start()) else {
        return false;
    };
    !rest.starts_with('Z')
}

"""Shared plumbing for the end-to-end tests: run the real binary on a real PTY.

Every test process gets its own runtime directory, and every client and server
it started is torn down even when an assertion fails.
"""

import base64
import errno
import fcntl
import os
import pty
import select
import shutil
import signal
import struct
import sys
import tempfile
import termios
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ansi_screen import Screen  # noqa: E402

BIN = os.environ.get("KOSHI_BIN", os.path.join(os.getcwd(), "target/debug/koshi"))

# The pane starts $SHELL. Debian images have /bin/bash; NixOS does not, so an
# inherited SHELL is the fallback that keeps the suites runnable on the host.
DEFAULT_SHELL = "/bin/bash" if os.path.exists("/bin/bash") else os.environ.get("SHELL", "/bin/sh")
SHELL = os.environ.get("KOSHI_TEST_SHELL", DEFAULT_SHELL)

# A unique directory per run keeps parallel runs and stale servers apart.
_OWN_RUNTIME = "KOSHI_RUNTIME_DIR" not in os.environ
RUNTIME_DIR = os.environ.get("KOSHI_RUNTIME_DIR")
if not RUNTIME_DIR:
    RUNTIME_DIR = tempfile.mkdtemp(prefix="koshi-e2e-%d-" % os.getuid())

SOCKET_PATH = os.path.join(RUNTIME_DIR, "koshi.sock")
ROWS = 24
COLS = 100

_CLIENTS = []


class Client:
    """One attached client, running the real binary on a PTY of its own."""

    def __init__(self, args=(), rows=ROWS, cols=COLS, env=None):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            environ = dict(os.environ)
            environ["KOSHI_RUNTIME_DIR"] = RUNTIME_DIR
            environ["TERM"] = "xterm-256color"
            environ["SHELL"] = SHELL
            environ.pop("KOSHI_BIN", None)
            if env:
                environ.update(env)
            try:
                os.execve(BIN, [BIN] + list(args), environ)
            finally:
                os._exit(127)
        _CLIENTS.append(self)
        self.screen = Screen(rows, cols)
        self.resize(rows, cols)
        self.raw = b""
        self.exit_code = None

    def resize(self, rows, cols):
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

    def pump(self, timeout=0.2):
        """Read whatever is available for up to timeout seconds."""
        got = b""
        deadline = time.time() + timeout
        while True:
            remaining = deadline - time.time()
            if remaining <= 0:
                break
            r, _, _ = select.select([self.fd], [], [], remaining)
            if not r:
                break
            try:
                chunk = os.read(self.fd, 65536)
            except OSError as exc:
                if exc.errno == errno.EIO:
                    self._reap()
                    break
                raise
            if not chunk:
                self._reap()
                break
            got += chunk
            self.raw += chunk
            self.screen.feed(chunk)
        return got

    def _reap(self):
        if self.exit_code is not None:
            return
        try:
            pid, status = os.waitpid(self.pid, os.WNOHANG)
        except ChildProcessError:
            self.exit_code = -1
            return
        if pid == 0:
            return
        self.exit_code = os.waitstatus_to_exitcode(status)

    def wait_for(self, needle, timeout=10.0):
        """Wait until the screen shows needle. Returns the screen text or None."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            self.pump(0.2)
            if self.screen.contains(needle):
                return self.screen.text()
            if self.exit_code is not None:
                return self.screen.text() if self.screen.contains(needle) else None
        return None

    def wait_quiet(self, quiet=0.35, timeout=10.0):
        """Pump until the client has produced nothing for `quiet` seconds."""
        deadline = time.time() + timeout
        last_output = time.time()
        while time.time() < deadline:
            if self.pump(0.05):
                last_output = time.time()
            elif time.time() - last_output >= quiet:
                break
            if self.exit_code is not None and time.time() - last_output >= quiet:
                break
        return self.screen.text()

    def stable_screen(self, quiet=0.35, timeout=10.0):
        """The whole visible screen, normalized for comparison."""
        return normalize_screen(self.wait_quiet(quiet, timeout))

    def wait_exit(self, timeout=10.0):
        deadline = time.time() + timeout
        while time.time() < deadline:
            self.pump(0.2)
            if self.exit_code is not None:
                return self.exit_code
            try:
                pid, status = os.waitpid(self.pid, os.WNOHANG)
            except ChildProcessError:
                self.exit_code = -1
                return self.exit_code
            if pid:
                self.exit_code = os.waitstatus_to_exitcode(status)
                return self.exit_code
        return None

    def send(self, data):
        if isinstance(data, str):
            data = data.encode()
        os.write(self.fd, data)

    def detach_keys(self):
        """Ctrl+o d: the session leader group, then the detach command."""
        self.send(b"\x0fd")

    def close(self):
        try:
            os.close(self.fd)
        except OSError:
            pass
        if self.exit_code is None:
            try:
                os.kill(self.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            try:
                os.waitpid(self.pid, 0)
            except ChildProcessError:
                pass
            self.exit_code = -1
        if self in _CLIENTS:
            _CLIENTS.remove(self)


def send_mouse(client, code, col, row, release=False):
    """Send one SGR mouse report as if the user's terminal had produced it.

    `code` is the xterm button code, `col` and `row` are 0-based cells.
    """
    final = b"m" if release else b"M"
    client.send(b"\x1b[<%d;%d;%d%s" % (code, col + 1, row + 1, final))


def status_line(client):
    return client.screen.line(ROWS - 1)


def pump_until(client, predicate, timeout=10.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        client.pump(0.1)
        if predicate(client):
            return True
        if client.exit_code is not None:
            return predicate(client)
    return predicate(client)


def region(client, first, last):
    """Text of screen rows [first, last), trailing blanks removed."""
    return "\n".join(client.screen.line(row) for row in range(first, last))


# The two colours the client paints a split border with, and the characters it
# uses for one. A cell counts as a border cell only when both match, so a pane
# that prints a box character in its own colour does not split a region.
BORDER_CHARS = "│─┌┐└┘├┤┬┴┼"
BORDER_COLOURS = (110, 240)


def _border_cell(screen, col, row):
    ch = screen.buf[row][col]
    return ch in BORDER_CHARS and screen.fg_at(row, col) in BORDER_COLOURS


def pane_regions(client):
    """Bounding boxes of the pane areas, found by walking around the borders.

    A screen without borders (one pane, or a zoomed tab) is one region covering
    the whole content area.
    """
    screen = client.screen
    rows, cols = screen.rows - 1, screen.cols
    border = [[_border_cell(screen, col, row) for col in range(cols)] for row in range(rows)]
    seen = [[False] * cols for _ in range(rows)]
    out = []
    for row in range(rows):
        for col in range(cols):
            if border[row][col] or seen[row][col]:
                continue
            stack = [(col, row)]
            seen[row][col] = True
            x0 = x1 = col
            y0 = y1 = row
            while stack:
                x, y = stack.pop()
                x0, x1 = min(x0, x), max(x1, x)
                y0, y1 = min(y0, y), max(y1, y)
                for nx, ny in ((x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)):
                    if 0 <= nx < cols and 0 <= ny < rows and not seen[ny][nx] and not border[ny][nx]:
                        seen[ny][nx] = True
                        stack.append((nx, ny))
            out.append((x0, y0, x1, y1))
    out.sort(key=lambda box: (box[1], box[0]))
    return out


def focus_index(client):
    """Index into `pane_regions` of the region holding the cursor, or None."""
    row, col = client.screen.row, client.screen.col
    for index, (x0, y0, x1, y1) in enumerate(pane_regions(client)):
        if x0 <= col <= x1 and y0 <= row <= y1:
            return index
    return None


def wait_panes(client, count, timeout=10.0):
    return pump_until(client, lambda c: len(pane_regions(c)) == count, timeout)


def pane_state(client):
    """(1-based index of the focused pane, pane count), read from the screen.

    Regions are ordered top to bottom, left to right. That matches the order the
    server numbers its layout leaves for every layout this suite builds.
    """
    regions = pane_regions(client)
    index = focus_index(client)
    return (index + 1 if index is not None else 0, len(regions))


def wait_pane_state(client, focus, count, timeout=10.0):
    ok = pump_until(client, lambda c: pane_state(c) == (focus, count), timeout)
    if not ok:
        print("  [harness] pane_state wanted %r got %r regions=%r cursor=(%r,%r)"
              % ((focus, count), pane_state(client), pane_regions(client),
                 client.screen.col, client.screen.row))

    return ok


def wait_focus(client, index, timeout=10.0):
    return pump_until(client, lambda c: focus_index(c) == index, timeout)


def wait_focus_where(client, predicate, timeout=10.0):
    """Wait until the cursor, which the client draws in the focused pane, is
    somewhere the predicate accepts (col, row)."""
    return pump_until(client, lambda c: predicate(c.screen.col, c.screen.row), timeout)


def osc52(client):
    """Decoded OSC 52 clipboard payloads the client wrote to its terminal."""
    out = []
    for body in client.screen.osc:
        parts = body.split(";", 2)
        if parts[0] == "52" and len(parts) == 3 and parts[2] != "?":
            try:
                out.append(base64.b64decode(parts[2]).decode("utf-8", "replace"))
            except Exception:  # noqa: BLE001 - a malformed payload is a failure
                out.append("")
    return out


def normalize_screen(text):
    """Drop trailing padding and trailing blank lines, keep every content line.

    Cursor position is not part of the text at all. Nothing else is normalized:
    leading spaces inside the screen are significant (panes are laid out
    horizontally).
    """
    lines = [line.rstrip() for line in text.split("\n")]
    while lines and not lines[-1]:
        lines.pop()
    return "\n".join(lines)


def all_pids():
    for name in os.listdir("/proc"):
        if name.isdigit():
            yield int(name)


def _proc_bytes(path):
    try:
        with open(path, "rb") as handle:
            return handle.read()
    except OSError:
        return b""


def _session_id(pid):
    data = _proc_bytes("/proc/%d/stat" % pid)
    if not data:
        return None
    # The command name may contain spaces and parentheses; fields follow the
    # last ')' as: state ppid pgrp session ...
    rest = data[data.rfind(b")") + 2 :].split()
    if len(rest) < 4:
        return None
    try:
        return int(rest[3])
    except ValueError:
        return None


def server_pids():
    """PIDs of koshi servers bound to this run's runtime directory."""
    needle = ("KOSHI_RUNTIME_DIR=%s\0" % RUNTIME_DIR).encode()
    pids = []
    for pid in all_pids():
        if pid == os.getpid():
            continue
        if needle not in _proc_bytes("/proc/%d/environ" % pid):
            continue
        argv = _proc_bytes("/proc/%d/cmdline" % pid).split(b"\0")
        if len(argv) >= 2 and argv[1] == b"server":
            pids.append(pid)
    return pids


def _kill_session(session):
    for pid in all_pids():
        if pid == os.getpid():
            continue
        if _session_id(pid) == session:
            try:
                os.kill(pid, signal.SIGKILL)
            except OSError:
                pass


def clean_runtime():
    """Start from an empty runtime directory (the directory itself is unique)."""
    os.makedirs(RUNTIME_DIR, mode=0o700, exist_ok=True)
    if os.path.isdir(RUNTIME_DIR):
        for name in os.listdir(RUNTIME_DIR):
            path = os.path.join(RUNTIME_DIR, name)
            if os.path.isdir(path):
                shutil.rmtree(path, ignore_errors=True)
            else:
                try:
                    os.unlink(path)
                except OSError:
                    pass


def server_alive():
    """True while a koshi server process owns this run's socket."""
    return bool(server_pids()) and os.path.lexists(SOCKET_PATH)


def cleanup():
    """Kill every client, server, pane process, and the socket of this run."""
    for client in list(_CLIENTS):
        client.close()
    for pid in server_pids():
        session = _session_id(pid)
        if session is not None:
            _kill_session(session)
        try:
            os.kill(pid, signal.SIGKILL)
        except OSError:
            pass
    deadline = time.time() + 2.0
    while time.time() < deadline and server_pids():
        time.sleep(0.05)
    if _OWN_RUNTIME:
        shutil.rmtree(RUNTIME_DIR, ignore_errors=True)
    else:
        try:
            os.unlink(SOCKET_PATH)
        except OSError:
            pass


class Result:
    def __init__(self):
        self.checks = []
        self.failures = []

    def check(self, label, ok, detail=""):
        self.checks.append((label, bool(ok)))
        if not ok:
            self.failures.append("%s%s" % (label, (": " + detail) if detail else ""))
        print("  %s %s%s" % ("PASS" if ok else "FAIL", label, (" (" + detail + ")") if detail else ""))
        return ok

    def report(self, name):
        print()
        if self.failures:
            print("%s: FAIL (%d/%d checks)" % (name, len(self.checks) - len(self.failures), len(self.checks)))
            for f in self.failures:
                print("  - %s" % f)
            return 1
        print("%s: PASS (%d checks)" % (name, len(self.checks)))
        return 0

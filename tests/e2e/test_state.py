#!/usr/bin/env python3
"""M4 acceptance: FR-11 saved state.

Every scenario starts from an empty runtime directory, so the state file that
the server writes is the only source of the layout a later server restores.

Scenario 1 kills the server with SIGKILL after the periodic save, which is the
same path a machine restart takes: only what was written to the file survives.
Scenario 2 feeds a damaged file and expects an empty start plus one notice.
Scenario 3 checks where the file lives and who may read it.
Scenario 4 checks that an alternate-screen pane saves no history.
Scenario 5 checks that a clean shutdown (every pane exited) writes a valid file.
"""

import os
import signal
import stat
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
    pane_state,
    RUNTIME_DIR,
    Client,
    Result,
    clean_runtime,
    cleanup,
    normalize_screen,
    pump_until,
    send_mouse,
    server_pids,
    status_line,
)

STATE = os.path.join(RUNTIME_DIR, "state.bin")
MAGIC = b"KOSHI\x00"


def begin(name):
    cleanup()
    clean_runtime()
    print("scenario: %s" % name)


def state_bytes():
    try:
        with open(STATE, "rb") as handle:
            return handle.read()
    except OSError:
        return b""


def wait_for_state(since, timeout=15.0):
    """Wait until the server wrote the state file after `since`."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            info = os.stat(STATE)
            if info.st_size > 0 and info.st_mtime >= since:
                return True
        except OSError:
            pass
        time.sleep(0.1)
    return False


def kill_server():
    pids = server_pids()
    for pid in pids:
        try:
            os.kill(pid, signal.SIGKILL)
        except OSError:
            pass
    deadline = time.time() + 5.0
    while time.time() < deadline and server_pids():
        time.sleep(0.05)
    return bool(pids) and not server_pids()


def wheel_up(client, notches, col=10, row=5):
    for _ in range(notches):
        send_mouse(client, 64, col, row)
        client.pump(0.01)


def scenario_restore(res):
    begin("layout, working directory, and history come back after a restart")
    work = os.path.join(RUNTIME_DIR, "w")
    os.makedirs(work, exist_ok=True)
    client = Client()
    try:
        res.check("starts in normal mode", pump_until(
            client, lambda c: "[NORMAL] main" in status_line(c)), status_line(client))
        client.send("cd %s\n" % work)
        client.send("for i in $(seq -w 1 120); do echo HIST-$i; done\n")
        res.check("history output is visible", pump_until(
            client, lambda c: "HIST-120" in c.screen.text()))
        client.send("echo TOP-MARK\n")
        res.check("marker line is visible", pump_until(
            client, lambda c: "TOP-MARK" in c.screen.text()))

        client.send(b"\x10r")
        res.check("a split adds a second pane", pump_until(
            client, lambda c: pane_state(c) == (2, 2)), status_line(client))
        client.send(b"\x10p")
        res.check("focus returns to the first pane", pump_until(
            client, lambda c: pane_state(c) == (1, 2)), status_line(client))

        client.send(b"\x14n")
        res.check("a second tab is created", pump_until(
            client, lambda c: "2:shell*" in status_line(c)), status_line(client))
        client.send(b"\x14r")
        res.check("the rename prompt is shown", pump_until(
            client, lambda c: "[RENAME] tab name:" in status_line(c)), status_line(client))
        client.send("logs")
        res.check("the rename buffer echoes the name", pump_until(
            client, lambda c: "tab name: logs_" in status_line(c)), status_line(client))
        client.send(b"\r")
        res.check("the second tab is renamed", pump_until(
            client, lambda c: "2:logs*" in status_line(c)), status_line(client))

        last_change = time.time()
        client.detach_keys()
        res.check("the client detaches cleanly", client.wait_exit(10) == 0)
    finally:
        client.close()

    res.check("the periodic save wrote the state file", wait_for_state(last_change))
    saved = state_bytes()
    res.check("the saved file has the koshi magic", saved.startswith(MAGIC),
              "head=%r" % saved[:8])
    res.check("killing the server keeps the saved file", kill_server() and os.path.exists(STATE))

    second = Client()
    try:
        res.check("the restarted server restores the workspace", pump_until(
            second, lambda c: "[NORMAL] main" in status_line(c)), status_line(second))
        res.check("both tab names come back", pump_until(
            second, lambda c: "1:shell" in status_line(c) and "2:logs*" in status_line(c)),
            status_line(second))
        res.check("the active tab is restored", "2:logs*" in status_line(second),
                  status_line(second))

        second.send(b"\x14\x1b[D")
        res.check("the first tab keeps its split", pump_until(
            second, lambda c: "1:shell*" in status_line(c) and pane_state(c) == (1, 2)),
            status_line(second))
        res.check("the focused pane is restored", pane_state(second) == (1, 2),
                  status_line(second))

        second.send("pwd\n")
        res.check("the pane reopens in the saved working directory", pump_until(
            second, lambda c: work in c.screen.text()), "wanted=%s" % work)

        wheel_up(second, 45)
        res.check("older history is readable after the restart", pump_until(
            second, lambda c: "HIST-001" in c.screen.text(), 5), "top=%r" % second.screen.line(0))
        second.send("\r")
        res.check("a key returns the restored view to the live end", pump_until(
            second, lambda c: pane_state(c) == (1, 2)))
        second.send("exit\n")
        pump_until(second, lambda c: pane_state(c) == (1, 1))
        second.send("exit\n")
        second.wait_exit(10)
    finally:
        second.close()


def scenario_damaged_file(res):
    begin("a damaged state file starts empty and tells the client")
    os.makedirs(RUNTIME_DIR, mode=0o700, exist_ok=True)
    with open(STATE, "wb") as handle:
        handle.write(b"KOSHI\x00\x01\x00\x00\x00\x09not a payload")
    client = Client()
    try:
        res.check("the client is told the state was unreadable", pump_until(
            client, lambda c: b"saved state was unreadable" in c.raw))
        res.check("the message says it starts empty",
                  b"starting empty" in client.raw)
        res.check("the session starts with the default workspace", pump_until(
            client, lambda c: "[NORMAL] main" in status_line(c)), status_line(client))
        res.check("the default session has one tab and one pane",
                  "1:shell*" in status_line(client) and pane_state(client) == (1, 1),
                  status_line(client))
        client.send("echo AFTER-DAMAGE\n")
        res.check("the fresh session accepts input", pump_until(
            client, lambda c: "AFTER-DAMAGE" in c.screen.text()))

        res.check("the damaged file is replaced by a fresh save", pump_until(
            client, lambda c: state_bytes().startswith(MAGIC), 15),
            "head=%r" % state_bytes()[:8])
    finally:
        client.close()


def scenario_permissions(res):
    begin("the state file is owner-only under the runtime directory")
    client = Client()
    try:
        res.check("the session is up", pump_until(
            client, lambda c: "[NORMAL] main" in status_line(c)))
        client.send("echo FORMAT-CHECK\n")
        pump_until(client, lambda c: "FORMAT-CHECK" in c.screen.text())
        res.check("the state file is written", wait_for_state(time.time() - 1))
    finally:
        client.close()

    dir_mode = stat.S_IMODE(os.stat(RUNTIME_DIR).st_mode)
    file_mode = stat.S_IMODE(os.stat(STATE).st_mode)
    res.check("the state directory is 0700", dir_mode == 0o700, "mode=%o" % dir_mode)
    res.check("the state file is 0600", file_mode == 0o600, "mode=%o" % file_mode)
    leftovers = [name for name in os.listdir(RUNTIME_DIR) if name.startswith(".state")]
    res.check("no temporary state file is left behind", leftovers == [],
              "entries=%r" % leftovers)

    data = state_bytes()
    res.check("the header is the magic plus the current format version",
              data[:6] == MAGIC and data[6:7] == b"\x01", "head=%r" % data[:8])
    length = int.from_bytes(data[8:12], "little")
    res.check("the payload length matches the file size",
              len(data) == 12 + length + 8, "len=%d payload=%d" % (len(data), length))


def scenario_alternate_screen(res):
    begin("an alternate-screen pane saves no history")
    lines = os.path.join(RUNTIME_DIR, "pager.txt")
    with open(lines, "w", encoding="utf-8") as handle:
        handle.write("".join("PAGER-LINE-%03d\n" % index for index in range(1, 201)))
    client = Client()
    try:
        res.check("the shell is ready", pump_until(
            client, lambda c: "[NORMAL] main" in status_line(c)))
        client.send("less -R %s\n" % lines)
        res.check("the pager shows its first line", pump_until(
            client, lambda c: "PAGER-LINE-001" in c.screen.text()))
        client.wait_quiet(quiet=0.3)
        since = time.time()
        res.check("the pager state is saved", wait_for_state(since))
    finally:
        client.close()

    res.check("the server is killed while the pager owns the screen", kill_server())
    second = Client()
    try:
        res.check("the restarted server comes up", pump_until(
            second, lambda c: "[NORMAL] main" in status_line(c)), status_line(second))
        second.wait_quiet(quiet=0.4)
        screen = normalize_screen(second.screen.text())
        res.check("no pager lines were restored", "PAGER-LINE" not in screen,
                  screen.split("\n")[0] if screen else "")
        res.check("the shell prompt is at the top of the restored pane",
                  second.screen.line(0).strip() != "", "top=%r" % second.screen.line(0))
        second.send("exit\n")
        second.wait_exit(10)
    finally:
        second.close()


def scenario_clean_shutdown(res):
    begin("a clean shutdown writes a valid state file")
    client = Client()
    try:
        res.check("the shell is ready", pump_until(
            client, lambda c: "[NORMAL] main" in status_line(c)))
        client.send("cd %s\n" % RUNTIME_DIR)
        client.send("echo LAST-CMD\n")
        pump_until(client, lambda c: "LAST-CMD" in c.screen.text())
        client.send("exit\n")
        res.check("the client exits when the shell exits", client.wait_exit(10) is not None)
    finally:
        client.close()

    deadline = time.time() + 5.0
    while time.time() < deadline and server_pids():
        time.sleep(0.05)
    data = state_bytes()
    res.check("the state file is valid after the shutdown", data.startswith(MAGIC),
              "head=%r" % data[:8])

    third = Client()
    try:
        res.check("an empty saved session starts a fresh workspace", pump_until(
            third, lambda c: "[NORMAL] main" in status_line(c) and "1:shell*" in status_line(c)),
            status_line(third))
        res.check("no notice is shown for an empty session",
                  b"saved state was unreadable" not in third.raw)
        third.send("exit\n")
        third.wait_exit(10)
    finally:
        third.close()


def main():
    res = Result()
    try:
        scenario_restore(res)
        scenario_damaged_file(res)
        scenario_permissions(res)
        scenario_alternate_screen(res)
        scenario_clean_shutdown(res)
    finally:
        cleanup()
    return res.report("saved state")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)

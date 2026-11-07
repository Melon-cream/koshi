#!/usr/bin/env python3
"""Pane output recording acceptance: the flag, the files, and their shape.

Every scenario starts from an empty runtime directory, which the server also
uses as its state directory, so `$KOSHI_RUNTIME_DIR/record` is the only
recording root in play. The recordings are read from disk: the suite asserts
what was written, not what a message claims was written.

The scenarios cover the settled decisions of the logging design: recording is
off by default and per workspace, the flag turns new panes into recorded panes,
the sidecar carries one ISO 8601 stamp per line, the files are private, the
alternate screen stays out, and the flag survives a server restart.
"""

import datetime
import glob
import os
import re
import signal
import stat
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
    RUNTIME_DIR,
    Client,
    Result,
    clean_runtime,
    cleanup,
    pump_until,
    server_pids,
    status_line,
    wait_panes,
)

SESSION = b"\x0f"
PANE = b"\x10"
ENTER = b"\r"
ESC = b"\x1b"
DOWN = b"\x1b[B"

RECORD_ROOT = os.path.join(RUNTIME_DIR, "record")
STATE = os.path.join(RUNTIME_DIR, "state.bin")
STAMP = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[+-]\d{2}:\d{2}$")


def begin(name):
    cleanup()
    clean_runtime()
    print("scenario: %s" % name)


def wait_text(client, needle, timeout=10.0):
    return pump_until(client, lambda c: needle in c.screen.text(), timeout)


def wait_status(client, needle, timeout=10.0):
    return pump_until(client, lambda c: needle in status_line(c), timeout)


def logs():
    """Every recorded body file, oldest name first."""
    return sorted(glob.glob(os.path.join(RECORD_ROOT, "*", "main-1", "*.log")))


def wait_logs(count, timeout=10.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if len(logs()) >= count:
            return True
        time.sleep(0.05)
    return len(logs()) >= count


def read(path):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return handle.read()
    except OSError:
        return ""


def wait_content(path, needle, timeout=10.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if needle in read(path):
            return True
        time.sleep(0.05)
    return needle in read(path)


def open_settings(client):
    client.send(SESSION + b"w")
    if not wait_text(client, "Workspaces"):
        return False
    client.send(b"c")
    return wait_text(client, "Settings")


def leave_overlay(client):
    for _ in range(4):
        if "[NORMAL]" in status_line(client) and "workspace list" not in status_line(client):
            return True
        client.send(ESC)
        client.pump(0.2)
    return "[NORMAL]" in status_line(client)


def set_record(client, on):
    """Open the settings screen of the current workspace and Enter the record row."""
    if not open_settings(client):
        return False
    client.send(DOWN)
    client.pump(0.2)
    client.send(ENTER)
    wanted = "record: on" if on else "record: off"
    if not wait_text(client, wanted):
        return False
    return leave_overlay(client)


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


def wait_state_since(since, timeout=15.0):
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


def scenario_off_by_default(res):
    begin("recording is off by default")
    client = Client()
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status_line(client))
        client.send("echo OFF-MARK\n")
        res.check("the pane prints", wait_text(client, "OFF-MARK"))
        client.pump(0.5)
        res.check("no recording directory is created", not os.path.exists(RECORD_ROOT),
                  "root=%r" % RECORD_ROOT)
        res.check("the settings screen shows the record row as off", open_settings(client)
                  and wait_text(client, "record: off"))
        res.check("the alias row is still there", "ssh alias: (none)" in client.screen.text())
        res.check("the footer still lists the keys", "Enter edit" in client.screen.text())
        leave_overlay(client)
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_new_panes_record(res):
    begin("record on: new panes are recorded, the pane before it is not")
    client = Client()
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status_line(client))
        client.send("echo BEFORE-ON\n")
        res.check("the first pane prints", wait_text(client, "BEFORE-ON"))

        res.check("the record flag turns on", set_record(client, True))

        client.send(b"\x10n")
        res.check("a split adds a second pane", wait_panes(client, 2), status_line(client))
        client.send("printf 'first\\nsecond\\n'\n")
        res.check("the new pane prints", wait_text(client, "second"))
        res.check("a recording file appears for the new pane only", wait_logs(1),
                  "logs=%r" % logs())

        names = [os.path.basename(path) for path in logs()]
        res.check("the pane before the flag stays unrecorded", len(logs()) == 1,
                  "logs=%r" % names)
        res.check("the file is named after the recording pane",
                  names[:1] and names[0].startswith("2-") and names[0].endswith(".log"),
                  "names=%r" % names)

        if logs():
            log = logs()[0]
            times = log[: -len(".log")] + ".times"
            client.pump(0.3)
            body = read(log)
            stamps = read(times).splitlines()
            res.check("the body holds the settled lines",
                      "first" in body.splitlines() and "second" in body.splitlines(),
                      repr(body))
            res.check("the body has no escape sequences", "\x1b" not in body, repr(body))
            res.check("the sidecar exists", os.path.exists(times), times)
            res.check("one stamp per line", len(stamps) == len(body.splitlines()),
                      "lines=%d stamps=%d" % (len(body.splitlines()), len(stamps)))
            res.check("every stamp is local ISO 8601",
                      stamps and all(STAMP.match(stamp) for stamp in stamps),
                      repr(stamps[:3]))
            day = os.path.basename(os.path.dirname(os.path.dirname(log)))
            workspace = os.path.basename(os.path.dirname(log))
            res.check("the recording path carries the workspace and the date",
                      workspace == "main-1"
                      and day == datetime.datetime.now().strftime("%Y%m%d"),
                      "workspace=%r day=%r" % (workspace, day))
            res.check("the recording root is private",
                      stat.S_IMODE(os.stat(RECORD_ROOT).st_mode) == 0o700,
                      oct(stat.S_IMODE(os.stat(RECORD_ROOT).st_mode)))
            res.check("the body file is private",
                      stat.S_IMODE(os.stat(log).st_mode) == 0o600,
                      oct(stat.S_IMODE(os.stat(log).st_mode)))
            res.check("the sidecar file is private",
                      stat.S_IMODE(os.stat(times).st_mode) == 0o600,
                      oct(stat.S_IMODE(os.stat(times).st_mode)))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_alternate_screen(res):
    begin("the alternate screen is not recorded and recording resumes after it")
    client = Client()
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status_line(client))
        res.check("the record flag turns on", set_record(client, True))
        client.send(b"\x10n")
        client.pump(0.3)
        client.send("printf '\\033[?1049h'\n")
        client.pump(0.4)
        client.send("printf 'HIDDEN-LINE\\n'\n")
        client.pump(0.4)
        client.send("printf '\\033[?1049l'\n")
        client.pump(0.4)
        client.send("printf 'AFTER-ALT\\n'\n")
        res.check("the pane prints after leaving the alternate screen",
                  wait_text(client, "AFTER-ALT"))
        res.check("a recording file appears", wait_logs(1), "logs=%r" % logs())
        if logs():
            res.check("recording resumes after the alternate screen",
                      wait_content(logs()[0], "AFTER-ALT"), read(logs()[0]))
            res.check("the alternate screen text is not recorded",
                      "HIDDEN-LINE" not in read(logs()[0]), read(logs()[0]))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_off_stops_new_panes(res):
    begin("record off: panes made afterwards are not recorded")
    client = Client()
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status_line(client))
        res.check("the record flag turns on", set_record(client, True))
        client.send(b"\x10n")
        client.pump(0.2)
        client.send("echo FIRST-RECORD\n")
        res.check("the recorded pane prints", wait_text(client, "FIRST-RECORD"))
        res.check("one file exists", wait_logs(1), "logs=%r" % logs())
        before = len(logs())

        res.check("the record flag turns off", set_record(client, False))
        client.send(b"\x10n")
        client.pump(0.2)
        client.send("echo AFTER-OFF\n")
        res.check("the new pane prints", wait_text(client, "AFTER-OFF"))
        client.pump(0.6)
        res.check("no new file appears", len(logs()) == before,
                  "before=%d after=%r" % (before, logs()))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_flag_survives_restart(res):
    begin("the record flag comes back from the state file")
    client = Client()
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status_line(client))
        since = time.time()
        res.check("the record flag turns on", set_record(client, True))
        res.check("the state file is written", wait_state_since(since))
        res.check("the server stops", kill_server())
    finally:
        client.close()

    second = Client()
    try:
        res.check("the restored session starts", wait_status(second, "[NORMAL] main"),
                  status_line(second))
        res.check("the settings screen still shows record on", open_settings(second)
                  and wait_text(second, "record: on"))
        leave_overlay(second)
        second.send(b"\x10n")
        second.pump(0.3)
        second.send("echo RESTORED-RECORD\n")
        res.check("a pane after the restart prints", wait_text(second, "RESTORED-RECORD"))
        res.check("a pane after the restart is still recorded", wait_logs(1),
                  "logs=%r" % logs())
        second.send("exit\n")
        second.wait_exit(10)
    finally:
        second.close()


def main():
    res = Result()
    try:
        scenario_off_by_default(res)
        scenario_new_panes_record(res)
        scenario_alternate_screen(res)
        scenario_off_stops_new_panes(res)
        scenario_flag_survives_restart(res)
    finally:
        cleanup()
    return res.report("pane recording")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)

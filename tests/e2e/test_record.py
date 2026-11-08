#!/usr/bin/env python3
"""Pane output recording acceptance: the flag, the files, and their shape.

Every scenario starts from an empty runtime directory, which the server also
uses as its state directory, so `$KOSHI_RUNTIME_DIR/record` is the only
recording root in play. The recordings are read from disk: the suite asserts
what was written, not what a message claims was written.

The scenarios cover the settled decisions of the logging design: recording is
off by default and per workspace, flipping the flag starts and stops the panes
that are already running (tiled and floating), the sidecar carries one ISO 8601
stamp per line, the files are private, the alternate screen stays out, and the
flag survives a server restart.
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


def wait_no_logs(count, timeout=1.0):
    """Give a recording that should not happen time to appear."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        if len(logs()) != count:
            return False
        time.sleep(0.05)
    return len(logs()) == count


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


def any_log_has(needle, timeout=10.0):
    """True once some recorded body holds needle."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        if any(needle in read(path) for path in logs()):
            return True
        time.sleep(0.05)
    return any(needle in read(path) for path in logs())


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


def scenario_toggle_records_running_pane(res):
    begin("record on: the pane already running starts recording at the toggle")
    client = Client()
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status_line(client))
        client.send("echo BEFORE-ON\n")
        res.check("the pane prints before the toggle", wait_text(client, "BEFORE-ON"))
        client.pump(0.3)

        res.check("the record flag turns on", set_record(client, True))
        client.send("echo AFTER-ON\n")
        res.check("the same pane prints after the toggle", wait_text(client, "AFTER-ON"))
        res.check("the running pane records from the toggle", any_log_has("AFTER-ON"),
                  "logs=%r" % logs())
        res.check("only the running pane is recorded", len(logs()) == 1,
                  "logs=%r" % [os.path.basename(path) for path in logs()])

        names = [os.path.basename(path) for path in logs()]
        res.check("the file is named after the running pane",
                  names[:1] and names[0].startswith("1-") and names[0].endswith(".log"),
                  "names=%r" % names)

        if logs():
            log = logs()[0]
            times = log[: -len(".log")] + ".times"
            client.pump(0.3)
            body = read(log)
            stamps = read(times).splitlines()
            res.check("output printed before the toggle stays out",
                      "BEFORE-ON" not in body, repr(body))
            res.check("the body holds the settled lines after the toggle",
                      "AFTER-ON" in body.splitlines(), repr(body))
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


def scenario_new_pane_records(res):
    begin("record on: a pane split afterwards is recorded too")
    client = Client()
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status_line(client))
        res.check("the record flag turns on", set_record(client, True))
        client.send(b"\x10n")
        res.check("a split adds a second pane", wait_panes(client, 2), status_line(client))
        client.send("printf 'first\\nsecond\\n'\n")
        res.check("the new pane prints", wait_text(client, "second"))
        res.check("both panes have a recording", wait_logs(2), "logs=%r" % logs())

        names = sorted(os.path.basename(path) for path in logs())
        res.check("both panes are named in the files",
                  len(names) == 2 and names[0].startswith("1-") and names[1].startswith("2-"),
                  "names=%r" % names)
        named = [path for path in logs() if os.path.basename(path).startswith("2-")]
        if named:
            res.check("the split pane's own output is recorded",
                      wait_content(named[0], "second"), read(named[0]))
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
        client.send("printf '\\033[?1049h'\\n")
        client.pump(0.4)
        client.send("printf 'HIDDEN-LINE\\n'\\n")
        client.pump(0.4)
        client.send("printf '\\033[?1049l'\\n")
        client.pump(0.4)
        client.send("printf 'AFTER-ALT\\n'\\n")
        res.check("the pane prints after leaving the alternate screen",
                  wait_text(client, "AFTER-ALT"))
        res.check("the recording catches up after the alternate screen",
                  any_log_has("AFTER-ALT"), "logs=%r" % logs())
        res.check("the alternate screen text is not recorded",
                  not any_log_has("HIDDEN-LINE", timeout=0.5),
                  "\n".join(read(path) for path in logs()))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_off_stops_running_pane(res):
    begin("record off: the pane already running stops recording at the toggle")
    client = Client()
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status_line(client))
        res.check("the record flag turns on", set_record(client, True))
        client.send("echo FIRST-RECORD\n")
        res.check("the recorded pane prints", wait_text(client, "FIRST-RECORD"))
        res.check("the recording starts", wait_logs(1), "logs=%r" % logs())
        log = logs()[0]
        res.check("the first line reaches the file", wait_content(log, "FIRST-RECORD"), read(log))

        res.check("the record flag turns off", set_record(client, False))
        client.send("echo AFTER-OFF\n")
        res.check("the same pane prints after the toggle", wait_text(client, "AFTER-OFF"))
        client.pump(0.6)
        res.check("output after the toggle stays out of the file",
                  "AFTER-OFF" not in read(log), read(log))
        res.check("no second file appears for the pane", len(logs()) == 1,
                  "logs=%r" % [os.path.basename(path) for path in logs()])

        client.send(b"\x10n")
        client.pump(0.2)
        client.send("echo AFTER-OFF-NEW\n")
        res.check("the new pane prints", wait_text(client, "AFTER-OFF-NEW"))
        res.check("a pane made while off is not recorded", wait_no_logs(1, timeout=1.0),
                  "logs=%r" % [os.path.basename(path) for path in logs()])
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_floating_pane_toggle(res):
    begin("the toggle reaches a floating pane that already exists")
    client = Client()
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status_line(client))
        client.send(PANE + b"f")
        res.check("a floating pane appears", wait_status(client, "[FLOAT]"), status_line(client))
        client.send("echo FLOAT-BEFORE\n")
        res.check("the float prints", wait_text(client, "FLOAT-BEFORE"))
        client.send(PANE + b"f")
        res.check("the float hides", pump_until(
            client, lambda c: "[FLOAT]" not in status_line(c)), status_line(client))

        res.check("the record flag turns on", set_record(client, True))
        client.send(PANE + b"f")
        res.check("the float is back", wait_status(client, "[FLOAT]"), status_line(client))
        client.send("echo FLOAT-AFTER\n")
        res.check("the float prints again", wait_text(client, "FLOAT-AFTER"))
        res.check("the floating pane is recorded after the toggle",
                  any_log_has("FLOAT-AFTER"), "logs=%r" % logs())
        names = [os.path.basename(path) for path in logs()]
        res.check("the floating pane has its own file",
                  names and all(name.startswith("2-") for name in names), "names=%r" % names)
        res.check("the float's earlier output stays out",
                  not any_log_has("FLOAT-BEFORE", timeout=0.5),
                  "\n".join(read(path) for path in logs()))
        client.send(PANE + b"f")
        client.pump(0.2)
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
        second.send("echo RESTORED-RECORD\n")
        res.check("a restored pane prints", wait_text(second, "RESTORED-RECORD"))
        res.check("a restored pane is recorded", any_log_has("RESTORED-RECORD"),
                  "logs=%r" % logs())
        second.send("exit\n")
        second.wait_exit(10)
    finally:
        second.close()


def main():
    res = Result()
    try:
        scenario_off_by_default(res)
        scenario_toggle_records_running_pane(res)
        scenario_new_pane_records(res)
        scenario_alternate_screen(res)
        scenario_off_stops_running_pane(res)
        scenario_floating_pane_toggle(res)
        scenario_flag_survives_restart(res)
    finally:
        cleanup()
    return res.report("pane recording")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)

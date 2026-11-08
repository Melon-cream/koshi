#!/usr/bin/env python3
"""`koshi record show`: the workspace list, the dates, the log, and the live tail.

The viewer is read-only. It reads the uncompressed recordings from disk, asks a
running server for the unfinished line of a live pane, and draws both in the
same terminal. The suite drives the real binary on a PTY and reads the screen.
"""

import datetime
import glob
import os
import re
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
    status_line,
)

SESSION = b"\x0f"
ENTER = b"\r"
ESC = b"\x1b"
TAB = b"\t"
DOWN = b"\x1b[B"

RECORD_ROOT = os.path.join(RUNTIME_DIR, "record")
CLOCK = re.compile(r"\d\d:\d\d:\d\d")


def begin(name):
    cleanup()
    clean_runtime()
    print("scenario: %s" % name)


def wait_text(client, needle, timeout=10.0):
    return pump_until(client, lambda c: needle in c.screen.text(), timeout)


def wait_status(client, needle, timeout=10.0):
    return pump_until(client, lambda c: needle in status_line(c), timeout)


def per_pane_line(client, body, live=False):
    """The Per-pane row for `body`: a clock, then the text, with no pane id.

    A live row is the pane's current shell line, so `body` sits inside it
    rather than at the end; only the clock and the `+` marker are anchored.
    """
    if live:
        pattern = r"^\d\d:\d\d:\d\d\+  [^\n]*%s" % re.escape(body)
    else:
        pattern = r"^\d\d:\d\d:\d\d  %s\s*$" % re.escape(body)
    return re.compile(pattern, re.M).search(client.screen.text())


def logs():
    return sorted(glob.glob(os.path.join(RECORD_ROOT, "*", "main-1", "*.log")))


def read(path):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return handle.read()
    except OSError:
        return ""


def any_log_has(needle, timeout=10.0):
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
    if not open_settings(client):
        return False
    client.send(DOWN)
    client.pump(0.2)
    client.send(ENTER)
    wanted = "record: on" if on else "record: off"
    if not wait_text(client, wanted):
        return False
    return leave_overlay(client)


def scenario_browse_and_read(res):
    begin("record show walks workspaces, dates, and the log")
    today = datetime.date.today().strftime("%Y-%m-%d")
    client = Client()
    viewer = None
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status_line(client))
        res.check("the record flag turns on", set_record(client, True))
        client.send("printf 'SHOW-ONE\\nSHOW-TWO\\n'\n")
        res.check("the pane records", any_log_has("SHOW-TWO"), "logs=%r" % logs())
        client.pump(0.3)
        before = {path: os.stat(path).st_mtime_ns for path in logs()}

        viewer = Client(args=("record", "show"))
        res.check("the workspace list opens", wait_text(viewer, "Records"), viewer.screen.text())
        res.check("the workspace is listed by name and id", wait_text(viewer, "main (id 1)"),
                  viewer.screen.text())
        res.check("the list footer names the keys", wait_text(viewer, "Enter open"),
                  viewer.screen.text())

        viewer.send(ENTER)
        res.check("Enter opens the dates", wait_text(viewer, today), viewer.screen.text())
        res.check("the date list footer offers the way back", wait_text(viewer, "Esc back"),
                  viewer.screen.text())

        viewer.send(ENTER)
        res.check("Enter opens the log", wait_text(viewer, "SHOW-ONE"), viewer.screen.text())
        res.check("the log keeps the recorded order", wait_text(viewer, "SHOW-TWO"),
                  viewer.screen.text())
        res.check("the log opens in per pane", wait_text(viewer, "[PER-PANE]"),
                  viewer.screen.text())
        res.check("the pane block header stays", wait_text(viewer, "pane 1"), viewer.screen.text())
        res.check("a per pane line carries its time and text but no pane id",
                  pump_until(viewer, lambda c: per_pane_line(c, "SHOW-ONE") is not None),
                  viewer.screen.text())
        res.check("the second per pane line keeps the recorded order",
                  pump_until(viewer, lambda c: per_pane_line(c, "SHOW-TWO") is not None),
                  viewer.screen.text())

        viewer.send(TAB)
        res.check("Tab switches to chronological", wait_text(viewer, "[CHRONO]"),
                  viewer.screen.text())
        res.check("a line carries its time", pump_until(
            viewer, lambda c: CLOCK.search(c.screen.text()) is not None), viewer.screen.text())
        res.check("a line carries its pane", wait_text(viewer, "p1"), viewer.screen.text())
        viewer.send(TAB)
        res.check("Tab switches back to per pane", wait_text(viewer, "[PER-PANE]"),
                  viewer.screen.text())

        viewer.send(ESC)
        res.check("Esc returns to the dates", wait_text(viewer, today), viewer.screen.text())
        viewer.send(ESC)
        res.check("Esc returns to the workspaces", wait_text(viewer, "Records"), viewer.screen.text())
        viewer.send("q")
        res.check("an empty key quits cleanly", viewer.wait_exit(10) == 0, viewer.exit_code)

        after = {path: os.stat(path).st_mtime_ns for path in logs()}
        res.check("the viewer does not touch the recordings", before == after,
                  "before=%r after=%r" % (before, after))
    finally:
        if viewer is not None:
            viewer.close()
        client.close()


def scenario_direct_arguments(res):
    begin("record show takes a workspace and a date directly")
    today = datetime.date.today()
    client = Client()
    viewer = None
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status_line(client))
        res.check("the record flag turns on", set_record(client, True))
        client.send("printf 'DIRECT-ONE\\nDIRECT-TWO\\n'\n")
        res.check("the pane records", any_log_has("DIRECT-TWO"), "logs=%r" % logs())

        viewer = Client(args=("record", "show", "main"))
        res.check("a workspace name opens its dates",
                  wait_text(viewer, today.strftime("%Y-%m-%d")), viewer.screen.text())
        viewer.send("q")
        viewer.wait_exit(10)
        viewer.close()

        viewer = Client(args=("record", "show", "1", today.strftime("%Y%m%d")))
        res.check("an id and a compact date open the log directly",
                  wait_text(viewer, "DIRECT-ONE"), viewer.screen.text())
        res.check("the direct view shows the second line", wait_text(viewer, "DIRECT-TWO"),
                  viewer.screen.text())
        viewer.send("q")
        res.check("the direct view quits", viewer.wait_exit(10) == 0, viewer.exit_code)
    finally:
        if viewer is not None:
            viewer.close()
        client.close()


def scenario_live_tail(res):
    begin("the unfinished line of a running pane is shown but not stored")
    today = datetime.date.today().strftime("%Y-%m-%d")
    client = Client()
    viewer = None
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status_line(client))
        res.check("the record flag turns on", set_record(client, True))
        client.send("echo SETTLED-LINE\n")
        res.check("the settled line is recorded", any_log_has("SETTLED-LINE"), "logs=%r" % logs())
        client.send("printf 'LIVE-PARTIAL'")
        res.check("the pane holds an unfinished line", wait_text(client, "LIVE-PARTIAL"))
        client.pump(0.4)
        res.check("the unfinished line is not in the file",
                  not any_log_has("LIVE-PARTIAL", timeout=0.3),
                  "\n".join(read(path) for path in logs()))

        viewer = Client(args=("record", "show", "main", today))
        res.check("the viewer shows the settled line", wait_text(viewer, "SETTLED-LINE"),
                  viewer.screen.text())
        res.check("the viewer adds the running pane's current line",
                  wait_text(viewer, "LIVE-PARTIAL"), viewer.screen.text())
        res.check("the log opens in per pane", wait_text(viewer, "[PER-PANE]"),
                  viewer.screen.text())
        res.check("the live line keeps its marker and drops the pane id",
                  pump_until(viewer, lambda c: per_pane_line(c, "LIVE-PARTIAL", live=True) is not None),
                  viewer.screen.text())
        res.check("the recorded line beside it drops the pane id too",
                  pump_until(viewer, lambda c: per_pane_line(c, "SETTLED-LINE") is not None),
                  viewer.screen.text())
        viewer.send(TAB)
        res.check("Tab switches to chronological", wait_text(viewer, "[CHRONO]"),
                  viewer.screen.text())
        res.check("the current line is marked as live", wait_text(viewer, "p1+"),
                  viewer.screen.text())
        viewer.send("q")
        viewer.wait_exit(10)
    finally:
        if viewer is not None:
            viewer.close()
        client.close()


def scenario_unknown_workspace(res):
    begin("an unknown workspace is an error, not an empty screen")
    client = Client()
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status_line(client))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()

    viewer = Client(args=("record", "show", "no-such-workspace"))
    try:
        code = viewer.wait_exit(10)
        res.check("the viewer fails", code == 1, str(code))
        res.check("the viewer says what is missing",
                  pump_until(viewer, lambda c: "no recordings" in c.screen.text(), 3.0),
                  viewer.screen.text())
    finally:
        viewer.close()


def scenario_length_mismatch(res):
    begin("a body and sidecar of different lengths show the shorter run and warn")
    day = datetime.date.today()
    directory = os.path.join(RECORD_ROOT, day.strftime("%Y%m%d"), "main-1")
    os.makedirs(directory, mode=0o700)
    with open(os.path.join(directory, "7-120000.log"), "w", encoding="utf-8") as handle:
        handle.write("FIRST-LINE\nSECOND-LINE\nTHIRD-LINE\n")
    with open(os.path.join(directory, "7-120000.times"), "w", encoding="utf-8") as handle:
        handle.write("%sT12:00:00+09:00\n" % day.isoformat())

    viewer = Client(args=("record", "show", "main", day.isoformat()))
    try:
        res.check("the shorter run is shown", wait_text(viewer, "FIRST-LINE"),
                  viewer.screen.text())
        res.check("a line past the sidecar is left out",
                  not wait_text(viewer, "THIRD-LINE", timeout=0.5), viewer.screen.text())
        res.check("the mismatch is reported", wait_text(viewer, "warning:"),
                  viewer.screen.text())
        viewer.send("q")
        res.check("the viewer quits", viewer.wait_exit(10) == 0, str(viewer.exit_code))
    finally:
        viewer.close()


def main():
    res = Result()
    try:
        scenario_browse_and_read(res)
        scenario_direct_arguments(res)
        scenario_live_tail(res)
        scenario_length_mismatch(res)
        scenario_unknown_workspace(res)
    finally:
        cleanup()
    return res.report("record show")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)

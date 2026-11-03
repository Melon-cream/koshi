#!/usr/bin/env python3
"""M8B acceptance: dragging a split border resizes the panes.

Geometry of a 100 x 24 client with more than one pane: the tab keeps a one cell
frame, so the panes live in x=1..98, y=1..21. Side by side that is 49 + border
column 50 + 48; stacked, 10 + border row 11 + 11.
"""

import os
import signal
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
    pane_state,
    ROWS,
    RUNTIME_DIR,
    Client,
    Result,
    clean_runtime,
    cleanup,
    pump_until,
    send_mouse,
    server_pids,
    status_line,
    wait_pane_state,
)

PANE = b"\x10"
GUTTER_COL = 50
GUTTER_ROW = 11
FRAME_RIGHT = 99


def begin(name):
    cleanup()
    clean_runtime()
    print("scenario: %s" % name)


def status(client):
    return status_line(client)


def wait_status(client, needle, timeout=10.0):
    return pump_until(client, lambda c: needle in status_line(c), timeout)


def wait_text(client, needle, timeout=10.0):
    return pump_until(client, lambda c: needle in c.screen.text(), timeout)


def drag(client, start, end, steps=3):
    send_mouse(client, 0, start[0], start[1])
    for step in range(1, steps + 1):
        col = start[0] + (end[0] - start[0]) * step // steps
        row = start[1] + (end[1] - start[1]) * step // steps
        send_mouse(client, 32, col, row)
        client.pump(0.02)
    send_mouse(client, 32, end[0], end[1], release=True)


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


def state_ready(since, timeout=20.0):
    path = os.path.join(RUNTIME_DIR, "state.bin")
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            info = os.stat(path)
            if info.st_size > 0 and info.st_mtime >= since:
                return True
        except OSError:
            pass
        time.sleep(0.1)
    return False


def scenario_horizontal(res):
    begin("dragging the vertical border moves it and resizes the panes")
    client = Client()
    try:
        res.check("normal state", wait_status(client, "[NORMAL] main"), status(client))
        client.send("clear\n")
        client.wait_quiet(quiet=0.3)
        client.send(PANE + b"n")
        res.check("the tab is split", wait_pane_state(client, 2, 2), status(client))
        client.wait_quiet(quiet=0.3)
        res.check("the border starts in the middle",
                  client.screen.line(5)[GUTTER_COL] == "│", client.screen.line(5))

        drag(client, (GUTTER_COL, 10), (30, 10))
        res.check("the border followed the drag", pump_until(
            client, lambda c: c.screen.line(5)[30] == "│"), "row=%r" % client.screen.line(5))
        res.check("the old border column is gone", client.screen.line(5)[GUTTER_COL] != "│",
                  client.screen.line(5))
        res.check("the focus did not move", pane_state(client) == (2, 2), status(client))

        client.send(PANE + b"p")
        wait_pane_state(client, 1, 2)
        client.send("clear\n")
        client.wait_quiet(quiet=0.3)
        client.send("for i in $(seq 1 60); do printf L; done; echo\n")
        res.check("the left pane is 29 cells wide", pump_until(
            client, lambda c: any(c.screen.line(index)[1:30] == "L" * 29
                                  for index in range(ROWS - 1))), client.screen.text())
        rows = [index for index in range(ROWS - 1)
                if client.screen.line(index)[1:30] == "L" * 29]
        first = rows[0] if rows else None
        res.check("the 60 cells wrap as three rows",
                  first is not None
                  and client.screen.line(first + 2)[1:3] == "LL"
                  and client.screen.line(first + 2)[3:30].strip() == "",
                  "rows=%r" % [client.screen.line(index) for index in range(first or 0,
                                                                          (first or 0) + 3)])

        drag(client, (30, 10), (0, 10))
        res.check("dragging to the left edge clamps at one cell", pump_until(
            client, lambda c: c.screen.line(5)[2] == "│"), "row=%r" % client.screen.line(5))

        drag(client, (2, 10), (99, 10))
        res.check("dragging to the right edge leaves one cell at the right",
                  pump_until(client, lambda c: c.screen.line(5)[97] == "│"),
                  "row=%r" % client.screen.line(5))
        res.check("the frame is still drawn", client.screen.line(5)[FRAME_RIGHT] == "│",
                  client.screen.line(5))

        client.send(PANE + b"p")
        wait_pane_state(client, 2, 2)
        client.send(PANE + b"x")
        res.check("closing the narrow pane returns to one pane",
                  wait_pane_state(client, 1, 1), status(client))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_vertical(res):
    begin("dragging the horizontal border moves it")
    client = Client()
    try:
        client.send(PANE + b"d")
        res.check("the tab is split down", wait_pane_state(client, 2, 2), status(client))
        client.wait_quiet(quiet=0.3)
        res.check("the border starts on row 11", client.screen.line(GUTTER_ROW)[50] == "─",
                  "row=%r" % client.screen.line(GUTTER_ROW))

        drag(client, (50, GUTTER_ROW), (50, 5))
        res.check("the border followed the drag", pump_until(
            client, lambda c: c.screen.line(5)[60] == "─"), "row=%r" % client.screen.line(5))
        res.check("the old border row is content again",
                  client.screen.line(GUTTER_ROW)[60] != "─",
                  "row=%r" % client.screen.line(GUTTER_ROW))
        res.check("the vertical frame edges are still there",
                  client.screen.line(5)[0] == "│" and client.screen.line(5)[99] == "│",
                  client.screen.line(5))

        send_mouse(client, 0, 60, 5)
        send_mouse(client, 0, 60, 5, release=True)
        client.wait_quiet(quiet=0.25)
        res.check("a click on the border does not move it",
                  client.screen.line(5)[60] == "─", "row=%r" % client.screen.line(5))

        client.send("exit\n")
        res.check("closing the bottom pane leaves one pane",
                  wait_pane_state(client, 1, 1), status(client))
        client.detach_keys()
        client.wait_exit(10)
    finally:
        client.close()


def scenario_restore(res):
    begin("the dragged size survives a restart")
    client = Client()
    try:
        client.send(PANE + b"n")
        res.check("the tab is split", wait_pane_state(client, 2, 2), status(client))
        client.wait_quiet(quiet=0.3)
        drag(client, (GUTTER_COL, 10), (40, 10))
        res.check("the border followed the drag", pump_until(
            client, lambda c: c.screen.line(5)[40] == "│"), "row=%r" % client.screen.line(5))
        since = time.time()
        res.check("the state file was saved", state_ready(since))
        res.check("the server was killed", kill_server())
    finally:
        client.close()

    second = Client()
    try:
        res.check("the restored tab still has two panes", pump_until(
            second, lambda c: pane_state(c) == (2, 2)), status_line(second))
        res.check("the border came back where it was dragged to", pump_until(
            second, lambda c: c.screen.line(5)[40] == "│"), "row=%r" % second.screen.line(5))
        second.send(PANE + b"d")
        res.check("the restored layout still splits", pump_until(
            second, lambda c: pane_state(c) == (3, 3)), status_line(second))
        second.detach_keys()
        second.wait_exit(10)
    finally:
        second.close()


def main():
    res = Result()
    try:
        scenario_horizontal(res)
        scenario_vertical(res)
        scenario_restore(res)
    finally:
        cleanup()
    return res.report("dragging a split border")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)

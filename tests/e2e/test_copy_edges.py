#!/usr/bin/env python3
"""M8A acceptance: the copy edges, dragging over vim, and a clean restored screen.

1. A drag that runs right to left (or bottom to top) keeps the cell it started
   on, so selecting a line in `less` from its end no longer drops the last
   character.
2. A plain drag copies even when the pane reports mouse events. A click without
   a drag still reaches the application.
3. A restored pane receives Ctrl-L once, so the prompt does not sit at the
   bottom of a screen full of old output. The history stays in the scrollback.
"""

import os
import signal
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
    ROWS,
    RUNTIME_DIR,
    Client,
    Result,
    clean_runtime,
    cleanup,
    osc52,
    pump_until,
    send_mouse,
    server_pids,
    status_line,
)

SCROLL = b"\x13"
STATE_WAIT = 20.0


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


def write_file(name, text):
    path = os.path.join(RUNTIME_DIR, name)
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(text)
    return path


def last_row_of(client, needle):
    rows = [row for row in range(ROWS) if needle in client.screen.line(row)]
    return rows[-1] if rows else None


def drag(client, start, end):
    send_mouse(client, 0, start[0], start[1])
    send_mouse(client, 32, end[0], end[1])
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


def state_ready(since, timeout=STATE_WAIT):
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


def scenario_selection_both_ways(res):
    begin("a drag keeps both ends whichever way it runs")
    client = Client()
    try:
        res.check("normal state", wait_status(client, "[NORMAL] main"), status(client))
        client.send("clear\n")
        client.wait_quiet(quiet=0.3)
        client.send("echo EDGE-ABCDEFGHIJ\n")
        text = "EDGE-ABCDEFGHIJ"
        res.check("the marker line is printed", wait_text(client, text))
        client.wait_quiet()
        row = last_row_of(client, text)
        res.check("the marker line is on the screen", row is not None)
        col = client.screen.line(row).find(text)
        first = (col, row)
        last = (col + len(text) - 1, row)

        drag(client, last, first)
        res.check("the clipboard received a payload", pump_until(
            client, lambda c: len(osc52(c)) == 1), "count=%d" % len(osc52(client)))
        copied = osc52(client)[0] if osc52(client) else ""
        res.check("right to left keeps the character it started on", copied == text,
                  "copied=%r want=%r" % (copied, text))

        drag(client, first, last)
        res.check("the second drag is copied too", pump_until(
            client, lambda c: len(osc52(c)) == 2), "count=%d" % len(osc52(client)))
        copied = osc52(client)[-1] if osc52(client) else ""
        res.check("left to right copies the same text", copied == text,
                  "copied=%r want=%r" % (copied, text))

        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_less_edge(res):
    begin("selecting a line from its end in less keeps the last character")
    path = write_file("edge.txt", "".join("LESS-EDGE-%03d\n" % i for i in range(1, 101)))
    client = Client()
    try:
        client.send("less -R %s\n" % path)
        res.check("less shows the first line", pump_until(
            client, lambda c: c.screen.line(0).startswith("LESS-EDGE-001")))
        client.wait_quiet(quiet=0.25)
        line = client.screen.line(0)
        text = "LESS-EDGE-001"

        drag(client, (len(text) - 1, 0), (0, 0))
        res.check("the copy lands", pump_until(
            client, lambda c: len(osc52(c)) == 1), "count=%d" % len(osc52(client)))
        copied = osc52(client)[0] if osc52(client) else ""
        res.check("the whole line arrives, last character included", copied == text,
                  "copied=%r line=%r" % (copied, line))

        client.send("q")
        client.wait_quiet(quiet=0.25)
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_vim_drag(res):
    begin("a drag over vim copies and a click still reaches vim")
    path = write_file("vim.txt", "".join("VIM-LINE-%02d\n" % i for i in range(1, 21)))
    client = Client()
    try:
        client.send("vim -u NONE -N %s\n" % path)
        res.check("vim shows the first line", pump_until(
            client, lambda c: c.screen.line(0).startswith("VIM-LINE-01")))
        client.send(":set mouse=a\r")
        client.wait_quiet(quiet=0.35)
        res.check("vim still shows the first line", "VIM-LINE-01" in client.screen.line(0))

        drag(client, (0, 0), (9, 0))
        res.check("dragging over vim writes the clipboard", pump_until(
            client, lambda c: len(osc52(c)) == 1), "count=%d" % len(osc52(client)))
        copied = osc52(client)[0] if osc52(client) else ""
        res.check("the dragged text is copied", copied.strip() == "VIM-LINE-0",
                  "copied=%r" % copied)
        res.check("vim did not enter visual mode",
                  "VISUAL" not in client.screen.text(), client.screen.text()[-200:])

        send_mouse(client, 0, 4, 3)
        send_mouse(client, 0, 4, 3, release=True)
        res.check("a click without a drag reaches vim", pump_until(
            client, lambda c: c.screen.row == 3), "row=%r" % client.screen.row)

        client.send(b"\x1b")
        client.send(":q!\r")
        client.wait_quiet(quiet=0.3)
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_restore_prompt(res):
    begin("a restored pane is cleared so the prompt sits at the top")
    client = Client()
    baseline = None
    try:
        res.check("normal state", wait_status(client, "[NORMAL] main"), status(client))
        # A fresh shell is the reference for "the prompt is at the top": the host
        # and the container disagree about a prompt's leading blank line.
        client.wait_quiet(quiet=0.3)
        baseline = client.screen.row
        since = time.time()
        client.send("for i in $(seq -w 1 40); do echo RST-$i; done\n")
        res.check("history output is visible", wait_text(client, "RST-40", 20))
        res.check("the state file was saved", state_ready(since))
        res.check("the server was killed", kill_server())
    finally:
        client.close()

    second = Client()
    try:
        res.check("the new server starts", wait_status(second, "[NORMAL] main"))
        res.check("the prompt sits at the top like a fresh session", pump_until(
            second, lambda c: c.screen.row <= (baseline or 0) + 1 and c.screen.col > 0),
            "row=%r baseline=%r" % (second.screen.row, baseline))
        res.check("the old output is not filling the screen", "RST-40" not in second.screen.text(),
                  second.screen.text()[:200])
        second.send(SCROLL + b"k")
        for _ in range(9):
            second.send(b"k")
            second.pump(0.02)
        saved = ["RST-%02d" % index for index in range(1, 41)]
        res.check("the saved history is still in scrollback",
                  any(line in second.screen.text() for line in saved),
                  second.screen.text()[:200])
        second.detach_keys()
        second.wait_exit(10)
    finally:
        second.close()


def main():
    res = Result()
    try:
        scenario_selection_both_ways(res)
        scenario_less_edge(res)
        scenario_vim_drag(res)
        scenario_restore_prompt(res)
    finally:
        cleanup()
    return res.report("copy edges, vim drag, restored prompt")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)

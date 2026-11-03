#!/usr/bin/env python3
"""M3 acceptance: drag selection, auto-scroll, and OSC 52 copy.

The whole clipboard payload is decoded from the bytes the client wrote to its
terminal, so the checks cover multi-line, wrapped, wide-char, history, and
alternate-screen selections.
"""

import os
import re
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
    COLS,
    pane_state,
    ROWS,
    RUNTIME_DIR,
    Client,
    Result,
    clean_runtime,
    cleanup,
    osc52,
    pump_until,
    region,
    send_mouse,
    status_line,
)

TOP_PANE = (1, 11)
# The frame takes the outer row and col; row 11 is the split gutter.
BOTTOM_PANE = (12, ROWS - 2)
LAST_ROW = ROWS - 3
LAST_COL = 98


def begin(name):
    cleanup()
    clean_runtime()
    print("scenario: %s" % name)


def write_file(name, text):
    path = os.path.join(RUNTIME_DIR, name)
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(text)
    return path


def pane_line(client, row):
    """One screen row without the tab frame columns, trailing blanks dropped."""
    return client.screen.line(row)[1 : COLS - 1].rstrip()


def scenario_drag_autoscroll(res):
    begin("drag out of the top edge selects the whole history into OSC 52")
    client = Client()
    try:
        res.check("normal mode is shown", pump_until(
            client, lambda c: "[NORMAL] main" in status_line(c)), status_line(client))
        client.send(b'\x10d')
        res.check("the bottom pane is created", pump_until(
            client, lambda c: pane_state(c) == (2, 2)), status_line(client))

        client.send('for i in $(seq -w 1 100); do echo "SEL-$i"; done\n')
        res.check("history output is complete", pump_until(
            client, lambda c: "SEL-100" in c.screen.text()))
        client.send('echo "WRAP-' + "x" * 130 + '"\n')
        res.check("a wrapped line is printed", pump_until(
            client, lambda c: "WRAP-" in c.screen.text()))
        client.send('echo "WIDE-日本語-END"\n')
        res.check("a wide-char line is printed", pump_until(
            client, lambda c: "WIDE-" in c.screen.text()))
        client.wait_quiet()
        screen_before = region(client, *BOTTOM_PANE)
        anchor_row = pane_line(client, LAST_ROW)

        # Press on the last cell, then hold the pointer above the pane. The
        # server scrolls one line per tick and keeps extending the selection.
        send_mouse(client, 0, LAST_COL, LAST_ROW)
        send_mouse(client, 32, 0, 4)
        res.check("holding above the pane scrolls into the history", pump_until(
            client, lambda c: region(c, *BOTTOM_PANE) != screen_before, 10))
        res.check("auto-scroll reaches the oldest history line", pump_until(
            client, lambda c: "seq -w" in c.screen.line(BOTTOM_PANE[0]), 10),
            "top=%r" % client.screen.line(BOTTOM_PANE[0]))
        oldest = pane_line(client, BOTTOM_PANE[0])

        send_mouse(client, 32, 0, 2, release=True)
        res.check("mouse up writes one OSC 52 message", pump_until(
            client, lambda c: len(osc52(c)) == 1), "count=%d" % len(osc52(client)))
        payloads = osc52(client)
        res.check("exactly one clipboard payload was sent", len(payloads) == 1,
                  "count=%d" % len(payloads))
        text = payloads[0] if payloads else ""

        numbers = re.findall(r"SEL-(\d{3})", text)
        expected = ["%03d" % i for i in range(1, 101)]
        res.check("the copy contains all 100 history lines in order",
                  numbers == expected, "%d lines" % len(numbers))
        res.check("the copy includes the wrapped line without a break",
                  "WRAP-" + "x" * 120 in text)
        res.check("the copy keeps wide characters intact",
                  "WIDE-日本語-END" in text, repr(text[-80:]))
        res.check("the copy starts at the oldest history line",
                  text.split("\n", 1)[0] == oldest,
                  "copy=%r oldest=%r" % (text[:60], oldest))
        res.check("the copy ends at the pressed line",
                  text.split("\n")[-1] == anchor_row,
                  "copy=%r anchor=%r" % (text[-60:], anchor_row))
        res.check("no mode switch was needed to copy", "[NORMAL]" in status_line(client))

        client.send("echo AFTER-COPY\n")
        res.check("the pane still accepts input after copying", pump_until(
            client, lambda c: "AFTER-COPY" in region(c, *BOTTOM_PANE)))

        client.send("exit\n")
        res.check("closing the bottom pane returns to one pane", pump_until(
            client, lambda c: pane_state(c) == (1, 1)), status_line(client))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_alt_screen_selection(res):
    begin("alternate-screen selection stays inside the visible screen")
    path = write_file("alt.txt", "".join("LINE-%03d\n" % i for i in range(1, 201)))
    client = Client()
    try:
        client.send("less -R %s\n" % path)
        res.check("less shows the first line", pump_until(
            client, lambda c: c.screen.line(0).startswith("LINE-001")))
        client.wait_quiet(quiet=0.2)
        visible = [client.screen.line(row) for row in range(0, ROWS - 1)]
        while visible and not visible[-1]:
            visible.pop()

        send_mouse(client, 0, 0, 0)
        send_mouse(client, 32, LAST_COL, LAST_ROW + 1)
        time.sleep(1.0)
        client.pump(0.4)
        res.check("holding below the pager does not scroll it",
                  [client.screen.line(row) for row in range(len(visible))] == visible)

        send_mouse(client, 32, LAST_COL, LAST_ROW + 1, release=True)
        res.check("mouse up writes one OSC 52 message", pump_until(
            client, lambda c: len(osc52(c)) == 1), "count=%d" % len(osc52(client)))
        payloads = osc52(client)
        text = payloads[0] if payloads else ""
        copied = text.split("\n") if text else []
        res.check("the copy is exactly the visible screen",
                  copied == visible, "copied=%d visible=%d" % (len(copied), len(visible)))
        res.check("the copy has no history beyond the visible screen",
                  "LINE-200" not in text)

        client.send("q")
        res.check("less quits", pump_until(
            client, lambda c: not c.screen.line(0).startswith("LINE-")))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def main():
    res = Result()
    try:
        scenario_drag_autoscroll(res)
        scenario_alt_screen_selection(res)
    finally:
        cleanup()
    return res.report("drag selection, auto-scroll, OSC 52")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)

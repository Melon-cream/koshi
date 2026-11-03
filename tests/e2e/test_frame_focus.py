#!/usr/bin/env python3
"""M9 acceptance: frame-only focus, selecting from a pane's edge, and a calm drag.

1. A tab with more than one pane keeps a one cell frame; the frame segments that
   touch the focused pane are the only thing painted in the focus colour, and the
   split gutters stay plain.
2. A press on a frame cell starts a selection at the edge of the pane that frame
   touches, so the leftmost column of a pane can be copied from.
3. A press on a gutter resizes and does nothing else; no drag clears the screen.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
    COLS,
    ROWS,
    Client,
    Result,
    clean_runtime,
    cleanup,
    osc52,
    pane_regions,
    pane_state,
    pump_until,
    send_mouse,
    status_line,
)

PANE = b"\x10"
ARROW_LEFT = b"\x1b[D"
ARROW_UP = b"\x1b[A"
FOCUS_COLOUR = 110
BORDER_COLOUR = 240
RIGHT_FRAME = COLS - 1
BOTTOM_FRAME = ROWS - 2


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


def row_of(client, needle):
    rows = [row for row in range(ROWS) if needle in client.screen.line(row)]
    return rows[-1] if rows else None


def scenario_frame_only(res):
    begin("only the frame of the focused pane takes the focus colour")
    client = Client()
    try:
        res.check("normal state", wait_status(client, "[NORMAL] main"), status(client))
        client.send(PANE + b"n")
        res.check("the tab is split", pump_until(
            client, lambda c: pane_state(c) == (2, 2)), str(pane_state(client)))
        client.wait_quiet(quiet=0.3)
        res.check("the frame above the focused pane is lit",
                  client.screen.fg_at(0, 80) == FOCUS_COLOUR,
                  "top-right=%r" % client.screen.fg_at(0, 80))
        res.check("the frame above the other pane is plain",
                  client.screen.fg_at(0, 20) == BORDER_COLOUR,
                  "top-left=%r" % client.screen.fg_at(0, 20))
        res.check("the gutter stays plain on both halves",
                  client.screen.fg_at(5, 50) == BORDER_COLOUR
                  and client.screen.fg_at(15, 50) == BORDER_COLOUR,
                  "high=%r low=%r" % (client.screen.fg_at(5, 50),
                                      client.screen.fg_at(15, 50)))
        res.check("the right frame is lit",
                  client.screen.fg_at(5, RIGHT_FRAME) == FOCUS_COLOUR,
                  "right=%r" % client.screen.fg_at(5, RIGHT_FRAME))

        client.send(PANE + ARROW_LEFT)
        res.check("the left pane has the focus", pump_until(
            client, lambda c: pane_state(c)[0] == 1), str(pane_state(client)))
        res.check("the frame follows the focus", pump_until(
            client, lambda c: c.screen.fg_at(0, 20) == FOCUS_COLOUR
            and c.screen.fg_at(0, 80) == BORDER_COLOUR),
            "left=%r right=%r" % (client.screen.fg_at(0, 20),
                                  client.screen.fg_at(0, 80)))
        res.check("the left frame is lit now",
                  client.screen.fg_at(5, 0) == FOCUS_COLOUR,
                  "left=%r" % client.screen.fg_at(5, 0))

        client.send(b"\x0fd")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_left_edge(res):
    begin("a drag from the left frame starts at the pane's first column")
    client = Client()
    try:
        client.send(PANE + b"n")
        client.send(PANE + ARROW_LEFT)
        res.check("the left pane has the focus", pump_until(
            client, lambda c: pane_state(c)[0] == 1), str(pane_state(client)))
        client.send("clear\n")
        client.wait_quiet(quiet=0.3)
        client.send("echo LEFT-EDGE-ABCDEF\n")
        text = "LEFT-EDGE-ABCDEF"
        res.check("the marker line is printed", wait_text(client, text))
        client.wait_quiet()
        row = row_of(client, text)
        res.check("the marker line is on the screen", row is not None)
        col = client.screen.line(row).find(text)
        res.check("the marker starts at the pane's first column", col == 1,
                  "col=%r row=%r" % (col, client.screen.line(row)))

        drag(client, (0, row), (10, row))
        res.check("the drag copies", pump_until(
            client, lambda c: len(osc52(c)) == 1), "count=%d" % len(osc52(client)))
        copied = osc52(client)[0] if osc52(client) else ""
        res.check("the copy starts at the pane's first character",
                  copied == text[:10], "copied=%r" % copied)

        client.send(b"\x0fd")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_top_edge(res):
    begin("a drag from the top frame starts at the pane's first row")
    client = Client()
    try:
        client.send(PANE + b"d")
        res.check("the tab is split down", pump_until(
            client, lambda c: pane_state(c)[1] == 2), str(pane_state(client)))
        client.send(PANE + ARROW_UP)
        res.check("the top pane has the focus", pump_until(
            client, lambda c: pane_state(c)[0] == 1), str(pane_state(client)))
        client.send("clear\n")
        client.wait_quiet(quiet=0.3)
        client.send("echo TOP-EDGE-12345\n")
        res.check("the marker line is printed", wait_text(client, "TOP-EDGE-12345"))
        client.wait_quiet()

        drag(client, (5, 0), (5, 6))
        res.check("the drag copies", pump_until(
            client, lambda c: len(osc52(c)) == 1), "count=%d" % len(osc52(client)))
        copied = osc52(client)[0] if osc52(client) else ""
        res.check("the copy holds a column of the top pane",
                  "EDGE-12345" in copied and copied.count("\n") >= 3,
                  "copied=%r" % copied)

        client.send(b"\x0fd")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_gutter_and_clear(res):
    begin("a gutter press only resizes, and no drag clears the screen")
    client = Client()
    try:
        client.send(PANE + b"n")
        res.check("the tab is split", pump_until(
            client, lambda c: pane_state(c)[1] == 2), str(pane_state(client)))
        client.wait_quiet(quiet=0.3)

        send_mouse(client, 0, 50, 10)
        send_mouse(client, 0, 50, 10, release=True)
        client.wait_quiet(quiet=0.25)
        res.check("a press on the gutter copies nothing", osc52(client) == [],
                  "count=%d" % len(osc52(client)))
        res.check("a press on the gutter does not move it",
                  client.screen.line(5)[50] == "│", client.screen.line(5))

        before = client.raw.count(b"\x1b[2J")
        drag(client, (50, 10), (30, 10))
        client.wait_quiet(quiet=0.3)
        after = client.raw.count(b"\x1b[2J")
        res.check("resizing does not clear the screen", after == before,
                  "clears %d -> %d" % (before, after))
        res.check("the border followed the drag", client.screen.line(5)[30] == "│",
                  client.screen.line(5))

        client.send(PANE + ARROW_LEFT)
        res.check("the left pane has the focus for the selection", pump_until(
            client, lambda c: pane_state(c)[0] == 1), str(pane_state(client)))
        # A storm of motions used to resize the pty on every one of them, and the
        # shell's prompt redraws smeared into the prompt line.
        client.send(PANE + ARROW_LEFT)
        res.check("the left pane has the focus again", pump_until(
            client, lambda c: pane_state(c)[0] == 1), str(pane_state(client)))
        client.send("clear\n")
        client.wait_quiet(quiet=0.3)
        for step in range(20):
            target = 30 + (step % 2) * 8
            send_mouse(client, 0, 30 if step == 0 else 30 + ((step - 1) % 2) * 8, 8)
            send_mouse(client, 32, target, 8)
            send_mouse(client, 32, target, 8, release=True)
            client.pump(0.01)
        client.wait_quiet(quiet=0.6)
        def prompt_counts(screen_client):
            counts = []
            for (x0, y0, x1, y1) in pane_regions(screen_client):
                text = "\n".join(screen_client.screen.line(row)[x0:x1 + 1]
                                 for row in range(y0, y1 + 1))
                counts.append(text.count("$"))
            return counts

        counts = prompt_counts(client)
        res.check("dragging the border does not smear the prompt",
                  counts and all(count <= 1 for count in counts),
                  "per-pane counts=%r" % counts)
        res.check("the border sits where the drag ended", client.screen.line(5)[38] == "│",
                  client.screen.line(5))

        client.send("clear\n")
        client.wait_quiet(quiet=0.3)
        client.send("echo PICK-ME-1234\n")
        res.check("the marker line is printed", wait_text(client, "PICK-ME-1234"))
        client.wait_quiet()
        row = row_of(client, "PICK-ME-1234")
        before = client.raw.count(b"\x1b[2J")
        drag(client, (1, row), (12, row))
        client.wait_quiet(quiet=0.3)
        after = client.raw.count(b"\x1b[2J")
        res.check("selecting does not clear the screen", after == before,
                  "clears %d -> %d" % (before, after))
        res.check("the selection was copied", len(osc52(client)) == 1,
                  "count=%d" % len(osc52(client)))
        highlighted = "".join("R" if client.screen.reverse_at(row, col) else "."
                              for col in range(1, 13))
        res.check("both ends of the selection are highlighted",
                  highlighted.startswith("R") and highlighted.endswith("R")
                  and "." not in highlighted,
                  "mask=%r" % highlighted)

        client.send(b"\x0fd")
        client.wait_exit(10)
    finally:
        client.close()


def main():
    res = Result()
    try:
        scenario_frame_only(res)
        scenario_left_edge(res)
        scenario_top_edge(res)
        scenario_gutter_and_clear(res)
    finally:
        cleanup()
    return res.report("focus frame, edge selection, calm drags")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)

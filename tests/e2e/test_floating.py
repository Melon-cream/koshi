#!/usr/bin/env python3
"""M10 acceptance: the selection highlight's ends, zoom on z, and a floating pane.

Geometry of a 100 x 24 client: the content area is 100 x 23; a floating pane is
two thirds of that (66 x 15) and centred, so its frame runs from (17, 4) to
(82, 18) and its pty lives inside it.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
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
FLOAT_LEFT = 17
FLOAT_TOP = 4
FLOAT_RIGHT = 82
FLOAT_BOTTOM = 18


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


def wait_gone(client, needle, timeout=10.0):
    return pump_until(client, lambda c: needle not in c.screen.text(), timeout)


def row_of(client, needle):
    rows = [row for row in range(ROWS) if needle in client.screen.line(row)]
    return rows[-1] if rows else None


def float_shown(client):
    return (client.screen.line(FLOAT_TOP)[FLOAT_LEFT:FLOAT_LEFT + 1] == "┌"
            and client.screen.line(FLOAT_BOTTOM)[FLOAT_RIGHT:FLOAT_RIGHT + 1] == "┘")


def drag(client, start, end, steps=3):
    send_mouse(client, 0, start[0], start[1])
    for step in range(1, steps + 1):
        col = start[0] + (end[0] - start[0]) * step // steps
        row = start[1] + (end[1] - start[1]) * step // steps
        send_mouse(client, 32, col, row)
        client.pump(0.02)
    send_mouse(client, 32, end[0], end[1], release=True)


def scenario_keys(res):
    begin("z zooms and f floats")
    client = Client()
    try:
        res.check("normal state", wait_status(client, "[NORMAL] main"), status(client))
        client.send(PANE + b"z")
        res.check("z zooms", wait_status(client, "[ZOOM]"), status(client))
        client.send(PANE + b"z")
        res.check("z leaves zoom", pump_until(
            client, lambda c: "[ZOOM]" not in status_line(c)), status(client))

        client.send(PANE + b"f")
        res.check("f does not zoom", pump_until(
            client, lambda c: "[FLOAT]" in status_line(c)
            and "[ZOOM]" not in status_line(c)), status(client))
        res.check("the floating pane is drawn in the middle", pump_until(
            client, lambda c: float_shown(c)), client.screen.text()[:400])
        client.send(PANE + b"f")
        res.check("f hides it again", pump_until(
            client, lambda c: "[FLOAT]" not in status_line(c) and not float_shown(c)),
            status(client))
        client.send(b"\x0fd")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_typing_and_state(res):
    begin("the floating pane takes the keyboard and keeps its shell")
    client = Client()
    try:
        client.send("clear\n")
        client.wait_quiet(quiet=0.3)
        client.send("echo TILED-MARK\n")
        res.check("the tiled pane is marked", wait_text(client, "TILED-MARK"))

        client.send(PANE + b"f")
        res.check("the float appears", pump_until(client, lambda c: float_shown(c)),
                  client.screen.text()[:300])
        client.send("echo FLOAT-MARK\n")
        res.check("typing reaches the floating pane", wait_text(client, "FLOAT-MARK"))
        row = row_of(client, "FLOAT-MARK")
        res.check("the marker is inside the floating pane",
                  row is not None and FLOAT_TOP < row < FLOAT_BOTTOM,
                  "row=%r" % row)

        client.send(PANE + b"f")
        res.check("the float hides", pump_until(client, lambda c: not float_shown(c)),
                  client.screen.text()[:300])
        res.check("the tiled output is back", wait_text(client, "TILED-MARK"))
        res.check("the floating output is hidden", "FLOAT-MARK" not in client.screen.text())

        client.send(PANE + b"f")
        res.check("the float comes back with its shell", pump_until(
            client, lambda c: float_shown(c) and "FLOAT-MARK" in c.screen.text()),
            client.screen.text()[:300])

        client.send(PANE + b"x")
        res.check("x closes the floating pane", pump_until(
            client, lambda c: not float_shown(c) and "[FLOAT]" not in status_line(c)),
            status(client))
        res.check("the tiled pane still takes input", wait_text(client, "TILED-MARK"))
        client.send("echo AFTER-FLOAT\n")
        res.check("typing is back on the tiled pane", wait_text(client, "AFTER-FLOAT"))
        client.send(b"\x0fd")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_click_outside(res):
    begin("a click outside the floating pane hides it and focuses that pane")
    client = Client()
    try:
        client.send(PANE + b"n")
        client.send(PANE + ARROW_LEFT)
        res.check("the left pane has the focus", pump_until(
            client, lambda c: pane_state(c)[0] == 1), str(pane_state(client)))
        client.send(PANE + b"f")
        res.check("the float appears", pump_until(client, lambda c: float_shown(c)),
                  client.screen.text()[:300])

        send_mouse(client, 0, 70, 20)
        send_mouse(client, 0, 70, 20, release=True)
        res.check("the float is gone", pump_until(
            client, lambda c: not float_shown(c) and "[FLOAT]" not in status_line(c)),
            status(client))
        client.send("echo RIGHT-CLICK\n")
        res.check("the clicked pane takes the input", wait_text(client, "RIGHT-CLICK"))
        row = row_of(client, "RIGHT-CLICK")
        res.check("the marker landed on the right side", row is not None
                  and client.screen.line(row).find("RIGHT-CLICK") > 50,
                  "row=%r" % (client.screen.line(row) if row is not None else None))
        client.send(b"\x0fd")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_copy_inside(res):
    begin("a drag inside the floating pane copies")
    client = Client()
    try:
        client.send(PANE + b"f")
        res.check("the float appears", pump_until(client, lambda c: float_shown(c)),
                  client.screen.text()[:300])
        client.send("clear\n")
        client.wait_quiet(quiet=0.3)
        client.send("echo FLOAT-COPY-TEXT\n")
        text = "FLOAT-COPY-TEXT"
        res.check("the marker is printed", wait_text(client, text))
        client.wait_quiet()
        row = row_of(client, text)
        col = client.screen.line(row).find(text)
        res.check("the marker is inside the float", row is not None
                  and FLOAT_TOP < row < FLOAT_BOTTOM and FLOAT_LEFT < col < FLOAT_RIGHT,
                  "row=%r col=%r" % (row, col))
        drag(client, (col, row), (col + len(text) - 1, row))
        res.check("the drag copied", pump_until(client, lambda c: len(osc52(c)) == 1),
                  "count=%d" % len(osc52(client)))
        copied = osc52(client)[0] if osc52(client) else ""
        res.check("the copied text is the whole marker", copied == text, "copied=%r" % copied)
        client.send(PANE + b"f")
        client.send(b"\x0fd")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_escape(res):
    begin("Esc dismisses the floating pane unless its program owns Esc")
    client = Client()
    try:
        client.send(PANE + b"f")
        res.check("the float appears", pump_until(client, lambda c: float_shown(c)),
                  client.screen.text()[:300])
        client.send(b"\x1b")
        res.check("Esc hides the float", pump_until(
            client, lambda c: not float_shown(c) and "[FLOAT]" not in status_line(c)),
            status(client))
        res.check("only the tiled pane is left", pump_until(
            client, lambda c: len(pane_regions(c)) == 1), str(pane_regions(client)))

        client.send(PANE + b"f")
        res.check("the float comes back", pump_until(client, lambda c: float_shown(c)),
                  client.screen.text()[:300])
        client.send("vim -u NONE -N\n")
        res.check("vim runs inside the float", pump_until(
            client, lambda c: sum(1 for row in range(FLOAT_TOP + 1, FLOAT_BOTTOM)
                                  if c.screen.line(row)[FLOAT_LEFT + 1:FLOAT_LEFT + 2] == "~")
            >= 5), client.screen.text()[:300])
        client.wait_quiet(quiet=0.4)
        client.send(b"\x1b")
        client.wait_quiet(quiet=0.4)
        res.check("Esc belongs to vim, the float stays",
                  float_shown(client) and "[FLOAT]" in status_line(client), status(client))
        client.send(":q!\r")
        client.wait_quiet(quiet=0.4)
        client.send(b"\x1b")
        res.check("back at the shell, Esc hides the float", pump_until(
            client, lambda c: not float_shown(c)), status(client))
        client.send(b"\x0fd")
        client.wait_exit(10)
    finally:
        client.close()


def main():
    res = Result()
    try:
        scenario_keys(res)
        scenario_typing_and_state(res)
        scenario_click_outside(res)
        scenario_copy_inside(res)
        scenario_escape(res)
    finally:
        cleanup()
    return res.report("selection ends, zoom, floating pane")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)

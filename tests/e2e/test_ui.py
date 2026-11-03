#!/usr/bin/env python3
"""M6 acceptance: the UI round.

Copy popup, pane borders with a focus marker, right-hand splitting for
`Ctrl+p n`, arrow keys in the workspace list, and the framed look. Everything
is read back from the real client's screen, so the checks are about what a user
sees, not about server state.

Geometry the suite fixes (100 x 24 client, status bar on row 23):

- a split keeps one cell for its border: a 100 wide area becomes 50 + border
  column 50 + 49, a 23 row area becomes 11 + border row 11 + 11
- a gutter belongs to the second child of its split, and the client paints that
  side in the focus colour only while the focus lives there
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
    COLS,
    pane_state,
    ROWS,
    Client,
    Result,
    clean_runtime,
    cleanup,
    osc52,
    pump_until,
    send_mouse,
    status_line,
    wait_pane_state,
)

PANE = b"\x10"
TAB = b"\x14"
SESSION = b"\x0f"
ARROW_UP = b"\x1b[A"
ARROW_DOWN = b"\x1b[B"
ARROW_LEFT = b"\x1b[D"
ARROW_RIGHT = b"\x1b[C"

LAST_CONTENT_ROW = ROWS - 2
GUTTER_COL = 50
GUTTER_ROW = 11
SECOND_GUTTER_COL = 25
BORDER_CHARS = "│─┌┐└┘├┤┬┴┼"
# The two colours the client paints a gutter with.
FOCUS_COLOUR = 110
BORDER_COLOUR = 240
TAB_COLOUR = 245
KEY_COLOUR = 238
BAR_COLOUR = 236


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


def screen_after_quiet(client):
    client.wait_quiet()
    return client.screen.text()


def column(client, col, first=0, last=ROWS - 1):
    """The character at one column of every row in [first, last)."""
    return "".join(
        client.screen.line(row)[col] if len(client.screen.line(row)) > col else " "
        for row in range(first, last)
    )


def row_of(client, needle, last_only=True):
    rows = [row for row in range(ROWS) if needle in client.screen.line(row)]
    if not rows:
        return None
    return rows[-1] if last_only else rows[0]


def selected_name(client):
    """Name on the workspace list row that carries the selection marker."""
    for row in range(ROWS):
        index = client.screen.line(row).find("> ")
        if index >= 0:
            rest = client.screen.line(row)[index + 2:].strip()
            return rest.split()[0] if rest else ""
    return None


def scenario_copy_popup(res):
    begin("copying shows a popup, and the rows come back when it goes")
    client = Client()
    try:
        res.check("normal state", wait_status(client, "[NORMAL] main"), status(client))
        client.send("clear\n")
        client.wait_quiet(quiet=0.3)
        client.send("for i in $(seq -w 1 30); do echo ROW-$i; done\n")
        res.check("rows are printed", wait_text(client, "ROW-30", 20))
        client.send("echo ZYXWVUTSRQPONMLKJIHGFEDCBA\n")
        res.check("the target line is printed", wait_text(client, "ZYXWVUTSRQPONMLKJIHG"))
        client.wait_quiet()

        row = row_of(client, "ZYXWVUTSRQPONMLK")
        res.check("the target line is on the screen", row is not None)
        covered = [client.screen.line(index) for index in range(19, ROWS - 1)]

        send_mouse(client, 0, 0, row)
        send_mouse(client, 32, 10, row)
        send_mouse(client, 32, 10, row, release=True)
        res.check("the drag writes one clipboard message", pump_until(
            client, lambda c: len(osc52(c)) == 1), "count=%d" % len(osc52(client)))
        payloads = osc52(client)
        text = payloads[0] if payloads else ""
        res.check("the copied text starts at the anchor",
                  text and "ZYXWVUTSRQPONMLKJIHG".startswith(text), repr(text))

        res.check("a popup reports the copy", pump_until(
            client, lambda c: "copied %d chars" % len(text) in c.screen.text()),
            "payload=%r" % text)
        screen = screen_after_quiet(client)
        res.check("the popup is drawn as a box",
                  "┌" in screen and "└" in screen, "popup missing its frame")
        res.check("the popup sits at the bottom right",
                  any("copied" in line and line.rstrip().endswith("│")
                      for line in screen.splitlines()[-4:]), screen.splitlines()[-3])

        res.check("the popup disappears by itself", pump_until(
            client, lambda c: "copied" not in c.screen.text(), 6))
        client.wait_quiet()
        res.check("the rows the popup covered are restored",
                  [client.screen.line(index) for index in range(19, ROWS - 1)] == covered,
                  "covered rows changed")

        client.send("echo AFTER-POPUP\n")
        res.check("the pane still takes input", wait_text(client, "AFTER-POPUP"))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_borders(res):
    begin("splits draw a border and the focused side of it takes the focus colour")
    client = Client()
    try:
        res.check("normal state", wait_status(client, "[NORMAL] main"), status(client))
        client.send("clear\n")
        client.wait_quiet(quiet=0.3)
        res.check("one pane draws no border",
                  not any(char in client.screen.text() for char in BORDER_CHARS),
                  "a single pane has a frame")

        client.send(PANE + b"n")
        res.check("n splits to the right", wait_pane_state(client, 2, 2), status(client))
        client.wait_quiet(quiet=0.3)
        res.check("the tab gains a frame and the border column runs down it",
                  column(client, GUTTER_COL, 0, ROWS - 1)
                  == "┬" + "│" * (ROWS - 3) + "┴",
                  "column=%r" % column(client, GUTTER_COL, 0, ROWS - 1))
        res.check("only the frame of the focused pane takes the focus colour",
                  pump_until(client, lambda c: c.screen.fg_at(0, 80) == FOCUS_COLOUR
                             and c.screen.fg_at(0, 20) == BORDER_COLOUR
                             and c.screen.fg_at(5, GUTTER_COL) == BORDER_COLOUR
                             and c.screen.fg_at(15, GUTTER_COL) == BORDER_COLOUR),
                  "frame=%r/%r gutter=%r" % (client.screen.fg_at(0, 80),
                                             client.screen.fg_at(0, 20),
                                             client.screen.fg_at(5, GUTTER_COL)))

        client.send(PANE + ARROW_LEFT)
        res.check("Left moves to the left pane", wait_pane_state(client, 1, 2), status(client))
        res.check("the frame follows the focus to the other pane", pump_until(
            client, lambda c: c.screen.fg_at(0, 20) == FOCUS_COLOUR
            and c.screen.fg_at(0, 80) == BORDER_COLOUR
            and c.screen.fg_at(5, GUTTER_COL) == BORDER_COLOUR),
            "left=%r right=%r" % (client.screen.fg_at(0, 20),
                                  client.screen.fg_at(0, 80)))

        client.send("clear\n")
        client.wait_quiet(quiet=0.3)
        client.send("for i in $(seq 1 60); do printf L; done; echo\n")

        def long_row(screen_client):
            for index in range(ROWS - 1):
                if screen_client.screen.line(index)[1:GUTTER_COL] == "L" * 49:
                    return index
            return None

        res.check("the left pane wraps at its own width", pump_until(
            client, lambda c: long_row(c) is not None), client.screen.text())
        first = long_row(client)
        res.check("the border column is not pane content",
                  first is not None and client.screen.line(first)[GUTTER_COL] == "│",
                  "row=%r" % (client.screen.line(first) if first is not None else None))
        res.check("the wrapped remainder is 11 cells wide",
                  first is not None
                  and client.screen.line(first + 1)[1:12] == "L" * 11
                  and client.screen.line(first + 1)[12:GUTTER_COL].strip() == "",
                  "row=%r" % (client.screen.line(first + 1) if first is not None else None))

        client.send(PANE + ARROW_RIGHT)
        wait_pane_state(client, 2, 2)
        client.send(PANE + b"d")
        res.check("d splits the focused pane down", wait_pane_state(client, 3, 3),
                  status(client))
        client.wait_quiet(quiet=0.3)
        res.check("the horizontal border runs across the right side",
                  client.screen.line(GUTTER_ROW)[GUTTER_COL + 1:GUTTER_COL + 11]
                  == "─" * 10, "row=%r" % client.screen.line(GUTTER_ROW))
        res.check("the crossing is drawn as a junction",
                  client.screen.line(GUTTER_ROW)[GUTTER_COL] == "├",
                  "cell=%r" % client.screen.line(GUTTER_ROW)[GUTTER_COL])
        res.check("the frame around the focused pane lights up, the gutters do not",
                  pump_until(client, lambda c: c.screen.fg_at(22, 80) == FOCUS_COLOUR
                             and c.screen.fg_at(0, 80) == BORDER_COLOUR
                             and c.screen.fg_at(11, GUTTER_COL + 25) == BORDER_COLOUR
                             and c.screen.fg_at(15, GUTTER_COL) == BORDER_COLOUR),
                  "bottom=%r top=%r" % (client.screen.fg_at(22, 80),
                                        client.screen.fg_at(0, 80)))

        client.send(PANE + ARROW_UP)
        res.check("Up moves to the top right pane", wait_pane_state(client, 2, 3),
                  status(client))
        res.check("the frame follows the focus up", pump_until(
            client, lambda c: c.screen.fg_at(0, 80) == FOCUS_COLOUR
            and c.screen.fg_at(22, 80) == BORDER_COLOUR),
            "top=%r bottom=%r" % (client.screen.fg_at(0, 80),
                                  client.screen.fg_at(22, 80)))

        client.send(PANE + b"z")
        res.check("f zooms", wait_status(client, "[ZOOM]"), status(client))
        client.wait_quiet(quiet=0.3)
        res.check("a zoomed pane draws no border",
                  not any(char in client.screen.text() for char in BORDER_CHARS),
                  "zoom kept a frame")
        client.send(PANE + b"z")
        wait_pane_state(client, 2, 3)
        res.check("leaving zoom draws the borders again", pump_until(
            client, lambda c: c.screen.line(GUTTER_ROW)[GUTTER_COL] == "├"),
            "row=%r" % client.screen.line(GUTTER_ROW))

        client.send(SESSION + b"d")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_right_split(res):
    begin("Ctrl+p n adds panes on the right")
    client = Client()
    try:
        client.send("clear\n")
        client.wait_quiet(quiet=0.3)
        client.send("echo LEFT-ORIGINAL\n")
        res.check("the first pane takes input", wait_text(client, "LEFT-ORIGINAL"))
        client.send(PANE + b"n")
        res.check("n adds a pane", wait_pane_state(client, 2, 2), status(client))
        client.send("echo RIGHT-NEW\n")
        res.check("the new pane takes input", wait_text(client, "RIGHT-NEW"))
        row = row_of(client, "RIGHT-NEW")
        res.check("the new pane is to the right of the border",
                  row is not None and client.screen.line(row).find("RIGHT-NEW") > GUTTER_COL,
                  "row=%r" % (client.screen.line(row) if row is not None else None))
        res.check("the old pane keeps its output",
                  any("LEFT-ORIGINAL" in client.screen.line(index) and
                      client.screen.line(index).find("LEFT-ORIGINAL") < GUTTER_COL
                      for index in range(ROWS - 1)), screen_after_quiet(client))

        client.send(PANE + b"p")
        res.check("p returns to the first pane", wait_pane_state(client, 1, 2), status(client))
        client.send(PANE + b"n")
        res.check("n splits the left pane to the right as well",
                  wait_pane_state(client, 2, 3), status(client))
        client.wait_quiet(quiet=0.3)
        res.check("the first border is halfway through the left column",
                  client.screen.line(5)[SECOND_GUTTER_COL] == "│",
                  "row=%r" % client.screen.line(5))
        res.check("the second border is still there",
                  client.screen.line(5)[GUTTER_COL] == "│", "row=%r" % client.screen.line(5))
        res.check("only the frame of the focused pane carries the focus colour",
                  pump_until(client, lambda c: c.screen.fg_at(0, 35) == FOCUS_COLOUR
                             and c.screen.fg_at(0, 10) == BORDER_COLOUR
                             and c.screen.fg_at(0, 70) == BORDER_COLOUR
                             and c.screen.fg_at(GUTTER_ROW, GUTTER_COL) == BORDER_COLOUR),
                  "frame=%r/%r/%r" % (client.screen.fg_at(0, 35),
                                      client.screen.fg_at(0, 10),
                                      client.screen.fg_at(0, 70)))

        client.send(SESSION + b"d")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_overlay_arrows(res):
    begin("arrow keys move the workspace selection")
    client = Client()
    try:
        client.send("echo MAIN-MARK\n")
        res.check("main takes input", wait_text(client, "MAIN-MARK"))
        client.send(SESSION + b"w")
        res.check("the workspace list opens", wait_text(client, "Workspaces"))
        res.check("the current workspace is selected", pump_until(
            client, lambda c: selected_name(c) == "main"), screen_after_quiet(client))

        client.send(b"n")
        res.check("n creates a workspace", pump_until(
            client, lambda c: "> ws2" in c.screen.text()))
        res.check("the selection follows the new workspace", pump_until(
            client, lambda c: selected_name(c) == "ws2"), screen_after_quiet(client))

        client.send(ARROW_UP)
        res.check("Up selects the workspace above", pump_until(
            client, lambda c: selected_name(c) == "main"), screen_after_quiet(client))
        client.send(ARROW_DOWN)
        res.check("Down selects the workspace below", pump_until(
            client, lambda c: selected_name(c) == "ws2"), screen_after_quiet(client))
        client.send(ARROW_DOWN)
        res.check("Down at the end wraps to the first workspace", pump_until(
            client, lambda c: selected_name(c) == "main"), screen_after_quiet(client))

        client.send(b"\r")
        res.check("Enter switches to the selected workspace", wait_status(
            client, "[NORMAL] main"), status(client))
        res.check("the list closes", pump_until(
            client, lambda c: "Workspaces" not in c.screen.text()))
        res.check("switching back shows the old output", wait_text(client, "MAIN-MARK"))

        client.send(SESSION + b"w")
        wait_text(client, "Workspaces")
        client.send(ARROW_DOWN)
        client.send(b"\r")
        res.check("Enter with the arrows switches to the other workspace",
                  wait_status(client, "[NORMAL] ws2"), status(client))
        client.send("exit\n")
        res.check("closing the last pane returns to main",
                  wait_status(client, "[NORMAL] main"), status(client))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_look(res):
    begin("the status bar keeps its text and the boxes use box drawing")
    client = Client()
    try:
        res.check("the status bar is unchanged",
                  wait_status(client, "[NORMAL] main | 1:shell*"),
                  status(client))
        client.send(SESSION + b"w")
        res.check("the list opens", wait_text(client, "Workspaces"))
        screen = screen_after_quiet(client)
        res.check("the list is drawn with box drawing",
                  "┌" in screen and "└" in screen and "┐" in screen and "┘" in screen,
                  "list frame missing")
        res.check("the title sits in the top border",
                  any("┌ Workspaces" in line for line in screen.splitlines()),
                  screen.splitlines()[0])
        res.check("the footer lists the keys",
                  "Enter switch" in screen, screen)
        client.send(b"\x1b")
        res.check("Esc closes the list", wait_status(client, "[NORMAL] main"), status(client))

        # The active tab is filled; the other one keeps the bar's background.
        client.send(TAB + b"n")
        res.check("a second tab exists", pump_until(
            client, lambda c: "2:shell*" in status_line(c)), status(client))
        client.wait_quiet(quiet=0.3)
        line = status(client)
        active_col = line.find("2:shell*")
        idle_col = line.find("1:shell")
        res.check("the active tab is filled",
                  pump_until(client, lambda c: c.screen.bg_at(ROWS - 1, active_col) == TAB_COLOUR),
                  "bg=%r" % client.screen.bg_at(ROWS - 1, active_col))
        res.check("the other tab keeps the bar background",
                  client.screen.bg_at(ROWS - 1, idle_col) == BAR_COLOUR,
                  "bg=%r" % client.screen.bg_at(ROWS - 1, idle_col))

        # The mode hint puts each key on its own pill.
        client.send(PANE)
        res.check("the pane hint is shown", wait_status(client, "[PANE]"), status(client))
        client.wait_quiet(quiet=0.3)
        hint = status(client)
        key_col = hint.find("n/r right")
        res.check("the hint lists the key and its label", key_col > 0, hint)
        res.check("the key sits on a pill",
                  pump_until(client, lambda c: c.screen.bg_at(ROWS - 1, key_col) == KEY_COLOUR),
                  "bg=%r" % client.screen.bg_at(ROWS - 1, key_col))
        res.check("the label keeps the bar background",
                  client.screen.bg_at(ROWS - 1, key_col + len("n/r ")) == BAR_COLOUR,
                  "bg=%r" % client.screen.bg_at(ROWS - 1, key_col + len("n/r ")))
        client.send(b"\x1b")
        wait_status(client, "[NORMAL]")
        client.send(SESSION + b"d")
        client.wait_exit(10)
    finally:
        client.close()


def main():
    res = Result()
    try:
        scenario_copy_popup(res)
        scenario_borders(res)
        scenario_right_split(res)
        scenario_overlay_arrows(res)
        scenario_look(res)
    finally:
        cleanup()
    return res.report("UI: popup, borders, right split, list arrows, look")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)

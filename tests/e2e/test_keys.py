#!/usr/bin/env python3
"""M5 acceptance: the one-shot leader groups.

Nothing here inspects server state: every key is sent on the real client's PTY
and the result is read back from the rendered screen. Pane positions come from
the geometry the layout produces, the status bar is the last row, and the
markers typed into a pane say which pane received the input.
"""

import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
    COLS,
    ROWS,
    Client,
    Result,
    clean_runtime,
    cleanup,
    normalize_screen,
    pane_state,
    pump_until,
    server_alive,
    status_line,
    wait_focus_where,
    wait_pane_state,
    wait_panes,
)

PANE = b"\x10"
TAB = b"\x14"
SESSION = b"\x0f"
SCROLL = b"\x13"
# Arrows arrive as several bytes and are decoded by the client.
ARROW_UP = b"\x1b[A"
ARROW_DOWN = b"\x1b[B"
ARROW_RIGHT = b"\x1b[C"
ARROW_LEFT = b"\x1b[D"
# Ctrl+f and Ctrl+b inside the scroll group: page down and page up.
PAGE_DOWN = bytes([0x06])
PAGE_UP = bytes([0x02])

LAST_ROW = ROWS - 1
TOP = (0, 11)
BOTTOM = (11, LAST_ROW)
LEFT = (0, 50)
RIGHT = (50, COLS)


def focus_in(rows, cols):
    """Predicate for wait_focus_where: the cursor sits in that quadrant."""
    return lambda col, row: cols[0] <= col < cols[1] and rows[0] <= row < rows[1]


QUAD_TL = focus_in(TOP, LEFT)
QUAD_TR = focus_in(TOP, RIGHT)
QUAD_BL = focus_in(BOTTOM, LEFT)
QUAD_BR = focus_in(BOTTOM, RIGHT)


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


def wait_gone(timeout=10.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if not server_alive():
            return True
        time.sleep(0.1)
    return False


def cell_text(client, rows=TOP, cols=LEFT):
    """Text of one rectangle of the screen, rows/cols as (start, end)."""
    return "\n".join(client.screen.line(row)[cols[0]:cols[1]] for row in range(*rows))


def screen_after_quiet(client):
    client.wait_quiet()
    return normalize_screen(client.screen.text())


def viewport(client):
    return [client.screen.line(row) for row in range(LAST_ROW)]


def number(line):
    """The LINE-NNN number in `line`, or -1."""
    marker = line.find("LINE-")
    if marker < 0:
        return -1
    digits = line[marker + 5:marker + 8]
    return int(digits) if digits.isdigit() else -1


def top_number(client):
    return number(client.screen.line(0))


def scenario_status_labels(res):
    begin("the status bar names the pending leader group")
    client = Client()
    try:
        res.check("starts in the normal state", wait_status(client, "[NORMAL] main"),
                  "status=%r" % status(client))
        res.check("the normal state shows no hint", "focus" not in status(client),
                  "status=%r" % status(client))

        groups = [
            (PANE, "[PANE]", "z zoom"),
            (TAB, "[TAB]", "Tab last"),
            (SESSION, "[SESSION]", "d detach"),
            (SCROLL, "[SCROLL]", "C-c bottom"),
        ]
        for keys, label, hint in groups:
            client.send(keys)
            res.check("%s is shown at the left edge" % label,
                      wait_status(client, label) and status(client).startswith(label + " "),
                      "status=%r" % status(client))
            res.check("%s lists its keys" % label, hint in status(client),
                      "status=%r" % status(client))
            res.check("%s keeps the workspace and tab line" % label,
                      "main" in status(client) and "1:shell*" in status(client)
                      and pane_state(client) == (1, 1), "status=%r" % status(client))
            res.check("%s fits the screen width" % label,
                      len(status(client).rstrip()) <= COLS, "status=%r" % status(client))
            client.send(b"\x1b")
            res.check("%s is left by Esc" % label, wait_status(client, "[NORMAL]"),
                      "status=%r" % status(client))

        client.send(SESSION + b"d")
        res.check("detach exits cleanly", client.wait_exit(10) == 0)
    finally:
        client.close()


def scenario_narrow_status(res):
    begin("a narrow terminal truncates the hints and keeps the base line")
    client = Client(cols=60)
    try:
        res.check("the narrow client starts normally", wait_status(client, "[NORMAL] main"),
                  "status=%r" % status(client))
        client.send(PANE)
        res.check("the group label is shown", wait_status(client, "[PANE]"),
                  "status=%r" % status(client))
        line = status(client).rstrip()
        res.check("the line is cut to the terminal width", len(line) <= 60, "line=%r" % line)
        res.check("the workspace and tab line survives", "main | 1:shell*" in line,
                  "line=%r" % line)
        res.check("the start of the hint is kept", "Arrows focus" in line, "line=%r" % line)
        res.check("the end of the hint is dropped", "f zoom" not in line, "line=%r" % line)
        client.send(b"\x1b")
        wait_status(client, "[NORMAL]")
        client.send(SESSION + b"d")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_pane_group(res):
    begin("pane group: splits, directional focus, next, previous, zoom, close")
    client = Client()
    try:
        res.check("normal state", wait_status(client, "[NORMAL] main"), status(client))

        client.send(PANE + b"r")
        res.check("r splits to the right", wait_pane_state(client, 2, 2), status(client))
        client.send(PANE + b"d")
        res.check("d splits the focused pane down", wait_pane_state(client, 3, 3), status(client))
        client.send(PANE + ARROW_LEFT)
        res.check("Left moves to the pane on the left", wait_pane_state(client, 1, 3), status(client))

        client.send(PANE + b"d")
        res.check("splitting the left pane makes a grid",
                  wait_panes(client, 4) and wait_focus_where(client, QUAD_BL), status(client))
        client.send("echo GRID-BL\n")
        res.check("the bottom-left pane has the focus", wait_text(client, "GRID-BL"))
        res.check("the marker landed in the bottom-left quadrant",
                  "GRID-BL" in cell_text(client, BOTTOM, LEFT))
        res.check("no other quadrant shows the bottom-left marker",
                  all("GRID-BL" not in cell_text(client, rows, cols)
                      for rows, cols in [(TOP, LEFT), (TOP, RIGHT), (BOTTOM, RIGHT)]))

        client.send(PANE + ARROW_UP)
        res.check("Up moves up", wait_focus_where(client, QUAD_TL), status(client))
        client.send("echo GRID-TL\n")
        res.check("the top-left pane has the focus",
                  wait_text(client, "GRID-TL") and "GRID-TL" in cell_text(client, TOP, LEFT))
        res.check("no other quadrant shows the top-left marker",
                  all("GRID-TL" not in cell_text(client, rows, cols)
                      for rows, cols in [(BOTTOM, LEFT), (TOP, RIGHT), (BOTTOM, RIGHT)]))

        client.send(PANE + ARROW_RIGHT)
        res.check("Right moves right", wait_focus_where(client, QUAD_TR), status(client))
        client.send("echo GRID-TR\n")
        res.check("the top-right pane has the focus",
                  wait_text(client, "GRID-TR") and "GRID-TR" in cell_text(client, TOP, RIGHT))
        res.check("no other quadrant shows the top-right marker",
                  all("GRID-TR" not in cell_text(client, rows, cols)
                      for rows, cols in [(TOP, LEFT), (BOTTOM, LEFT), (BOTTOM, RIGHT)]))

        client.send(PANE + ARROW_DOWN)
        res.check("Down moves down", wait_focus_where(client, QUAD_BR), status(client))
        client.send("echo GRID-BR\n")
        res.check("the bottom-right pane has the focus",
                  wait_text(client, "GRID-BR") and "GRID-BR" in cell_text(client, BOTTOM, RIGHT))
        res.check("no other quadrant shows the bottom-right marker",
                  all("GRID-BR" not in cell_text(client, rows, cols)
                      for rows, cols in [(TOP, LEFT), (BOTTOM, LEFT), (TOP, RIGHT)]))

        client.send(PANE + ARROW_UP)
        wait_focus_where(client, QUAD_TR)
        client.send(PANE + ARROW_LEFT)
        res.check("Left from the right column returns to the left one",
                  wait_focus_where(client, QUAD_TL), status(client))
        client.send(PANE + ARROW_DOWN)
        res.check("Down returns to the pane below", wait_focus_where(client, QUAD_BL), status(client))

        client.send(PANE + b"p")
        res.check("p moves to the next pane", wait_focus_where(client, QUAD_TR), status(client))
        client.send(PANE + b"p")
        res.check("p moves on to the pane after that", wait_focus_where(client, QUAD_BR), status(client))
        client.send(PANE + b";")
        res.check("; returns to the pane focused before", wait_focus_where(client, QUAD_TR),
                  status(client))

        client.send(PANE + b"z")
        res.check("f shows the zoom marker", wait_status(client, "[ZOOM]"), status(client))
        screen = screen_after_quiet(client)
        res.check("zoomed, only the focused pane is drawn",
                  "GRID-TR" in screen and "GRID-TL" not in screen)
        client.send(PANE + b"z")
        res.check("f leaves zoom", wait_focus_where(client, QUAD_TR) and "[ZOOM]" not in status(client),
                  status(client))
        res.check("leaving zoom draws the other panes again", wait_text(client, "GRID-TL"))

        client.send(PANE + b"x")
        res.check("x closes the focused pane",
                  wait_panes(client, 3) and wait_focus_where(client, QUAD_TL), status(client))
        client.wait_quiet()
        res.check("the pane that was closed is gone", "GRID-TR" not in client.screen.text())
        res.check("the other panes survived", "GRID-TL" in client.screen.text()
                  and "GRID-BL" in client.screen.text())

        client.send("exit\n")
        client.wait_quiet()
        client.send(PANE + b"x")
        client.send(PANE + b"x")
        res.check("closing every pane ends the session", client.wait_exit(10) is not None)
    finally:
        client.close()
    res.check("the server exits after the pane scenario", wait_gone())


def scenario_tab_group(res):
    begin("tab group: create, jump, previous, cycle, rename, close")
    client = Client()
    try:
        client.send("echo TAB-ONE\n")
        res.check("the first tab accepts input", wait_text(client, "TAB-ONE"))

        client.send(TAB + b"n")
        res.check("n creates a second tab", wait_status(client, "2:shell*"), status(client))
        client.send("echo TAB-TWO\n")
        res.check("the second tab accepts input", wait_text(client, "TAB-TWO"))
        client.send(TAB + b"n")
        res.check("n creates a third tab", wait_status(client, "3:shell*"), status(client))

        client.send(TAB + b"1")
        res.check("1 jumps to the first tab", wait_status(client, "1:shell*"), status(client))
        res.check("jumping shows the first tab content", wait_text(client, "TAB-ONE"))
        res.check("jumping hides the other tabs", "TAB-TWO" not in client.screen.text())

        client.send(TAB + b"\t")
        res.check("Tab returns to the tab seen before", wait_status(client, "3:shell*"),
                  status(client))
        client.send(TAB + b"\t")
        res.check("Tab goes back again", wait_status(client, "1:shell*"), status(client))

        client.send(TAB + ARROW_RIGHT)
        res.check("Right moves to the next tab", wait_status(client, "2:shell*"), status(client))
        client.send(TAB + ARROW_RIGHT)
        res.check("Right moves on to the next tab", wait_status(client, "3:shell*"), status(client))
        client.send(TAB + ARROW_LEFT)
        res.check("Left moves to the previous tab", wait_status(client, "2:shell*"), status(client))
        client.send(TAB + ARROW_LEFT)
        res.check("Left goes back again", wait_status(client, "1:shell*"), status(client))

        client.send(TAB + b"5")
        client.wait_quiet(quiet=0.2)
        res.check("a number with no tab changes nothing", "1:shell*" in status(client),
                  status(client))

        client.send(TAB + b"r")
        res.check("r opens the tab rename prompt", wait_status(client, "[RENAME] tab name:"),
                  status(client))
        client.send("logs")
        res.check("the rename buffer echoes the name", wait_status(client, "tab name: logs_"),
                  status(client))
        client.send(b"\r")
        res.check("the tab is renamed", wait_status(client, "1:logs*"), status(client))

        client.send(TAB + b"x")
        res.check("x closes the renamed tab", wait_status(client, "1:shell*"), status(client))
        res.check("the closed tab is gone from the list", "logs" not in status(client),
                  status(client))
        client.send(TAB + b"x")
        res.check("x closes the next tab as well",
                  wait_status(client, "1:shell*") and status(client).count(":shell") == 1,
                  status(client))
        client.send(TAB + b"x")
        client.wait_quiet(quiet=0.2)
        res.check("x keeps the last tab of a workspace",
                  "1:shell*" in status(client) and status(client).count(":shell") == 1,
                  status(client))
        client.send("echo STILL-HERE\n")
        res.check("the last tab still accepts input", wait_text(client, "STILL-HERE"))

        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()
    res.check("the server exits after the tab scenario", wait_gone())


def scenario_session_group(res):
    begin("session group: workspace list, rename, detach")
    client = Client()
    try:
        res.check("normal state", wait_status(client, "[NORMAL] main"), status(client))

        client.send(SESSION + b"w")
        res.check("w opens the workspace list", wait_text(client, "Workspaces"))
        res.check("the list is named in the status bar", wait_status(client, "workspace list"),
                  status(client))
        client.send(b"\x1b")
        res.check("Esc closes the list", wait_status(client, "[NORMAL] main"), status(client))

        client.send(SESSION + b"r")
        res.check("r starts renaming the current workspace",
                  wait_status(client, "[RENAME] workspace name:"), status(client))
        client.send("proj")
        res.check("the rename buffer echoes the name", wait_status(client, "workspace name: proj_"),
                  status(client))
        client.send(b"\r")
        res.check("the workspace is renamed in the status bar",
                  wait_status(client, "[NORMAL] proj"), status(client))
        res.check("the renamed workspace is listed with the rename prompt gone",
                  wait_text(client, "proj") and "rename:" not in client.screen.text())
        client.send(b"\x1b")
        res.check("Esc closes the list after the rename",
                  wait_status(client, "[NORMAL] proj") and "Workspaces" not in client.screen.text(),
                  status(client))

        client.send(SESSION + b"d")
        res.check("d detaches the client", client.wait_exit(10) == 0)
        res.check("the server survives the detach", server_alive())

        second = Client()
        try:
            res.check("reattaching shows the renamed workspace",
                      wait_status(second, "[NORMAL] proj"), status(second))
            second.detach_keys()
            res.check("the harness detach keys work", second.wait_exit(10) == 0)
        finally:
            second.close()
    finally:
        client.close()


def scenario_scroll_group(res):
    begin("scroll group: lines, half pages, pages, and the live end")
    client = Client()
    try:
        res.check("normal state", wait_status(client, "[NORMAL] main"), status(client))
        client.send("for i in $(seq -w 1 100); do echo LINE-$i; done\n")
        res.check("the history is complete", wait_text(client, "LINE-100", 20))
        client.wait_quiet()
        bottom = viewport(client)
        res.check("the view starts at the live end", bottom[LAST_ROW - 1].strip() != ""
                  and top_number(client) > 0, "top=%r" % bottom[0])

        client.send(SCROLL + b"k")
        res.check("k scrolls one line up and the group stays open",
                  wait_status(client, "[SCROLL]")
                  and pump_until(client, lambda c: viewport(c)[1:] == bottom[:-1]),
                  "top=%r" % client.screen.line(0))
        res.check("the scroll moved exactly one line", top_number(client) == number(bottom[0]) - 1,
                  "top=%r want=%r" % (client.screen.line(0), bottom[0]))

        client.send(b"j")
        res.check("j scrolls one line down without the leader",
                  pump_until(client, lambda c: viewport(c) == bottom))

        client.send(ARROW_UP)
        res.check("Up scrolls one line up", pump_until(
            client, lambda c: viewport(c)[1:] == bottom[:-1]),
            "top=%r" % client.screen.line(0))
        client.send(ARROW_DOWN)
        res.check("Down scrolls one line down", pump_until(client, lambda c: viewport(c) == bottom))

        client.send(b"u")
        half = (ROWS - 1) // 2
        res.check("u scrolls half a page up", pump_until(
            client, lambda c: viewport(c)[half:] == bottom[:LAST_ROW - half]),
            "top=%r" % client.screen.line(0))
        client.send(b"d")
        res.check("d scrolls half a page down", pump_until(
            client, lambda c: viewport(c) == bottom))

        client.send(PAGE_UP)
        res.check("Ctrl+b scrolls a page up", pump_until(
            client, lambda c: viewport(c) != bottom and top_number(c) > 0),
            "top=%r" % client.screen.line(0))
        page_up = top_number(client)
        res.check("a page is the height of the pane", page_up == number(bottom[0]) - (ROWS - 1),
                  "top=%r want=%r" % (client.screen.line(0), bottom[0]))
        client.send(PAGE_DOWN)
        res.check("Ctrl+f scrolls a page down", pump_until(
            client, lambda c: viewport(c) == bottom))

        client.send(PAGE_DOWN)
        client.wait_quiet(quiet=0.2)
        res.check("scrolling past the live end stays put", viewport(client) == bottom)

        for _ in range(3):
            client.send(PAGE_UP)
            client.pump(0.05)
        client.send(b"\x03")
        res.check("Ctrl+c returns to the live end", pump_until(
            client, lambda c: viewport(c) == bottom))

        client.send(b"\x1b")
        res.check("Esc leaves the scroll group", wait_status(client, "[NORMAL]"))

        client.send("echo AFTER-SCROLL\n")
        res.check("the pane still accepts input after scrolling",
                  wait_text(client, "AFTER-SCROLL"))

        client.send(SESSION + b"d")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_cancellation(res):
    begin("leaders end on Esc, on a repeated leader, and on unknown keys")
    client = Client()
    try:
        res.check("normal state", wait_status(client, "[NORMAL] main"), status(client))

        client.send(PANE + b"q")
        res.check("an unknown key ends the leader", wait_status(client, "[NORMAL]"),
                  status(client))
        client.send("echo UNKNOWN-OK\n")
        res.check("the unknown key is not sent to the pane",
                  wait_text(client, "UNKNOWN-OK")
                  and "qecho" not in client.screen.text()
                  and "command not found" not in client.screen.text())

        client.send(PANE + PANE)
        res.check("pressing the leader again ends the leader", wait_status(client, "[NORMAL]"),
                  status(client))
        client.send(PANE + b"r")
        res.check("the pane group still works after a repeated leader",
                  wait_pane_state(client, 2, 2), status(client))
        client.send(PANE + b"x")
        wait_pane_state(client, 1, 1)

        client.send(TAB + b"w")
        client.wait_quiet(quiet=0.2)
        res.check("a session key inside the tab group does nothing",
                  "[NORMAL]" in status(client) and "Workspaces" not in client.screen.text(),
                  status(client))
        client.send(SESSION + b"p")
        client.wait_quiet(quiet=0.2)
        res.check("a pane key inside the session group does nothing",
                  "[NORMAL]" in status(client) and pane_state(client) == (1, 1),
                  status(client))

        client.send(TAB + b"\x1b")
        res.check("Esc ends the tab leader", wait_status(client, "[NORMAL]"), status(client))
        client.send(TAB + b"n")
        res.check("the tab group still works after Esc", wait_status(client, "2:shell*"),
                  status(client))

        client.send(SESSION + b"d")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_arrows(res):
    begin("arrows: split sequences, pane delivery, and unbound letters")
    client = Client()
    try:
        res.check("normal state", wait_status(client, "[NORMAL] main"), status(client))

        client.send(PANE + b"r")
        res.check("r splits to the right", wait_pane_state(client, 2, 2), status(client))
        client.send(PANE)
        wait_status(client, "[PANE]")
        client.send(b"\x1b")
        client.send(b"[D")
        res.check("an arrow split across two writes still moves the focus",
                  wait_pane_state(client, 1, 2), status(client))

        client.send(PANE + b"p")
        res.check("p moves to the right pane again", wait_pane_state(client, 2, 2),
                  status(client))
        client.send(PANE + b"h")
        client.wait_quiet(quiet=0.2)
        res.check("h is unbound in the pane group and ends the leader",
                  pane_state(client) == (2, 2) and "[NORMAL]" in status(client),
                  status(client))
        client.send("echo AFTER-H\n")
        res.check("the unbound h never reached the pane",
                  wait_text(client, "AFTER-H")
                  and "hecho AFTER-H" not in client.screen.text()
                  and "command not found" not in client.screen.text())

        client.send(PANE + b"x")
        res.check("closing the split leaves one pane", wait_pane_state(client, 1, 1),
                  status(client))
        client.send("cat -v\n")
        client.wait_quiet(quiet=0.3)
        client.send(ARROW_UP)
        res.check("an arrow in the normal state reaches the pane unchanged",
                  wait_text(client, "^[[A"))
        client.send(b"\x03")
        client.wait_quiet(quiet=0.3)
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()
    res.check("the server exits after the arrow scenario", wait_gone())


def main():
    res = Result()
    try:
        scenario_status_labels(res)
        scenario_narrow_status(res)
        scenario_pane_group(res)
        scenario_arrows(res)
        scenario_tab_group(res)
        scenario_session_group(res)
        scenario_scroll_group(res)
        scenario_cancellation(res)
    finally:
        cleanup()
    return res.report("one-shot leader groups")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)

#!/usr/bin/env python3
"""M3 completion: status-bar tab clicks, overlay row clicks, input framing.

Everything is observed through the real client on a PTY. The tab label and
overlay row positions are read from the rendered screen, so the tests do not
assume hard-coded geometry.
"""

import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
    pane_state,
    ROWS,
    Client,
    Result,
    clean_runtime,
    cleanup,
    pump_until,
    send_mouse,
    status_line,
)

PANE_LEADER = b"\x10"
TOP_PANE = (0, 11)
BOTTOM_PANE = (11, ROWS - 1)


def begin(name):
    cleanup()
    clean_runtime()
    print("scenario: %s" % name)


def click_status_label(client, label):
    line = status_line(client)
    col = line.find(label)
    assert col >= 0, "status label %r not found in %r" % (label, line)
    send_mouse(client, 0, col + 1, ROWS - 1)


def overlay_row(client, needle):
    for row in range(ROWS - 1):
        line = client.screen.line(row)
        col = line.find(needle)
        if col >= 0:
            return row, col + 1
    return None, None


def send_fragments(client, pieces, gap=0.003):
    for piece in pieces:
        client.send(piece)
        time.sleep(gap)


def scenario_status_tab_click(res):
    begin("clicking a status-bar tab switches the active tab")
    client = Client()
    try:
        res.check("normal mode is shown", pump_until(
            client, lambda c: "[NORMAL] main" in status_line(c)), status_line(client))
        client.send("echo TAB-ONE\n")
        res.check("first tab accepts input", pump_until(
            client, lambda c: "TAB-ONE" in c.screen.text()))

        client.send(b"\x14n")
        res.check("second tab is created", pump_until(
            client, lambda c: "2:shell*" in status_line(c)), status_line(client))
        client.send("echo TAB-TWO\n")
        res.check("second tab accepts input", pump_until(
            client, lambda c: "TAB-TWO" in c.screen.text()))
        res.check("both tab labels are shown", "1:shell" in status_line(client),
                  status_line(client))

        click_status_label(client, "1:shell")
        res.check("clicking the first tab activates it", pump_until(
            client, lambda c: "1:shell*" in status_line(c)), status_line(client))
        client.wait_quiet(quiet=0.25)
        res.check("the first tab content is shown", "TAB-ONE" in client.screen.text())
        res.check("the second tab content is hidden", "TAB-TWO" not in client.screen.text())

        client.send("echo CLICK-TAB-ONE\n")
        res.check("typing goes to the clicked tab", pump_until(
            client, lambda c: "CLICK-TAB-ONE" in c.screen.text()))

        click_status_label(client, "2:shell")
        res.check("clicking the second tab activates it", pump_until(
            client, lambda c: "2:shell*" in status_line(c)), status_line(client))
        client.wait_quiet(quiet=0.25)
        res.check("the second tab content is restored", "TAB-TWO" in client.screen.text())
        res.check("the first tab content is hidden", "CLICK-TAB-ONE" not in client.screen.text())

        send_mouse(client, 0, 99, ROWS - 1)
        client.wait_quiet(quiet=0.2)
        res.check("clicking blank status space does not switch tabs",
                  "2:shell*" in status_line(client), status_line(client))

        client.send("exit\n")
        res.check("closing the second tab returns to the first", pump_until(
            client, lambda c: "1:shell*" in status_line(c)), status_line(client))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_overlay_row_click(res):
    begin("clicking a workspace overlay row selects and switches")
    client = Client()
    try:
        client.send("echo MAIN-OUTPUT\n")
        res.check("main workspace accepts input", pump_until(
            client, lambda c: "MAIN-OUTPUT" in c.screen.text()))

        client.send(b"\x0fw")
        res.check("overlay opens", pump_until(
            client, lambda c: "Workspaces" in c.screen.text()))
        client.send("n")
        res.check("second workspace is created", pump_until(
            client, lambda c: "> ws2" in c.screen.text()), client.screen.text())
        client.send(b"\x1b")
        res.check("escape closes the overlay", pump_until(
            client, lambda c: "[NORMAL] ws2" in status_line(c)), status_line(client))
        client.send("echo WS2-OUTPUT\n")
        res.check("second workspace accepts input", pump_until(
            client, lambda c: "WS2-OUTPUT" in c.screen.text()))

        client.send(b"\x0fw")
        res.check("overlay reopens", pump_until(
            client, lambda c: "Workspaces" in c.screen.text()))
        row, col = overlay_row(client, "main")
        res.check("the overlay lists main", row is not None, client.screen.text())
        if row is not None:
            send_mouse(client, 0, col, row)
        res.check("clicking main switches to it", pump_until(
            client, lambda c: "[NORMAL] main" in status_line(c)), status_line(client))
        client.wait_quiet(quiet=0.25)
        res.check("main keeps its output", "MAIN-OUTPUT" in client.screen.text())
        res.check("the other workspace output is hidden", "WS2-OUTPUT" not in client.screen.text())
        res.check("the click closes the overlay", "Workspaces" not in client.screen.text())

        client.send(b"\x0fw")
        res.check("overlay reopens for ws2", pump_until(
            client, lambda c: "Workspaces" in c.screen.text()))
        row, col = overlay_row(client, "ws2")
        res.check("the overlay lists ws2", row is not None, client.screen.text())
        if row is not None:
            send_mouse(client, 0, col, row)
        res.check("clicking ws2 switches back", pump_until(
            client, lambda c: "[NORMAL] ws2" in status_line(c)), status_line(client))
        client.wait_quiet(quiet=0.25)
        res.check("ws2 keeps its output", "WS2-OUTPUT" in client.screen.text())
        res.check("main output is hidden in ws2", "MAIN-OUTPUT" not in client.screen.text())

        client.send("exit\n")
        res.check("closing ws2 returns to main", pump_until(
            client, lambda c: "[NORMAL] main" in status_line(c)), status_line(client))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_fragmented_reports(res):
    begin("fragmented and malformed SGR/X10 reports keep ordinary Esc working")
    client = Client()
    try:
        client.send(b"\x10d")
        res.check("split creates two panes", pump_until(
            client, lambda c: pane_state(c) == (2, 2)), status_line(client))
        client.send(b"\x10p")
        res.check("prefix focus moves to the top pane", pump_until(
            client, lambda c: pane_state(c) == (1, 2)), status_line(client))

        # Split after the ESC byte: the parser must wait for the rest of the
        # report instead of treating ESC as the Esc key.
        send_fragments(client, [b"\x1b", b"[<0;11;16M"])
        res.check("a report split after ESC still clicks the bottom pane", pump_until(
            client, lambda c: pane_state(c) == (2, 2)), status_line(client))

        # Split inside the SGR parameters.
        send_fragments(client, [b"\x1b[<0", b";11;", b"4M"])
        res.check("a report split inside the parameters clicks the top pane", pump_until(
            client, lambda c: pane_state(c) == (1, 2)), status_line(client))

        # X10 fallback split after the CSI M introducer and inside its bytes.
        send_fragments(client, [b"\x1b[M", b"\x20", b"\x2b\x30"])
        res.check("a fragmented X10 report clicks the bottom pane", pump_until(
            client, lambda c: pane_state(c) == (2, 2)), status_line(client))

        send_fragments(client, [b"\x1b", b"[M", b"\x20\x2b\x24"])
        res.check("an X10 report split after ESC clicks the top pane", pump_until(
            client, lambda c: pane_state(c) == (1, 2)), status_line(client))

        client.send(b"\x1b[<0;11;;16M")
        client.wait_quiet(quiet=0.15)
        res.check("a malformed SGR report is dropped", pane_state(client) == (1, 2),
                  status_line(client))
        client.send(b"\x1b[<0;0;0M")
        client.wait_quiet(quiet=0.15)
        res.check("a zero-coordinate report is dropped", pane_state(client) == (1, 2),
                  status_line(client))

        client.send(PANE_LEADER)
        res.check("the pane leader group is pending", pump_until(
            client, lambda c: "[PANE]" in status_line(c)), status_line(client))
        client.send(b"\x1b")
        res.check("a lone Esc still cancels the leader", pump_until(
            client, lambda c: "[NORMAL]" in status_line(c)), status_line(client))
        client.send("echo AFTER-REPORTS\n")
        res.check("the pane accepts input after malformed reports", pump_until(
            client, lambda c: "AFTER-REPORTS" in c.screen.text()))

        send_mouse(client, 0, 10, 15)
        res.check("a normal report still works after malformed input", pump_until(
            client, lambda c: pane_state(c) == (2, 2)), status_line(client))

        client.send(b"\x10x")
        res.check("closing the bottom pane leaves the top", pump_until(
            client, lambda c: pane_state(c) == (1, 1)), status_line(client))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def main():
    res = Result()
    try:
        scenario_status_tab_click(res)
        scenario_overlay_row_click(res)
        scenario_fragmented_reports(res)
    finally:
        cleanup()
    return res.report("tab click, overlay click, input framing")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)

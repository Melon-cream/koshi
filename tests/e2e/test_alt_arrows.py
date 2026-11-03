#!/usr/bin/env python3
"""M7 acceptance: Alt plus an arrow moves the pane, then the tab.

The pane wins: an arrow with a pane in that direction moves the focus there.
With no pane in that direction, Left and Right fall through to the previous and
next tab; Up and Down do nothing. Two encodings arrive from real terminals, the
doubled ESC and the modified CSI form, and both have to work.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
    Client,
    pane_state,
    Result,
    clean_runtime,
    cleanup,
    pump_until,
    status_line,
    wait_pane_state,
)

PANE = b"\x10"
TAB = b"\x14"
SESSION = b"\x0f"
# Alt plus an arrow: ESC ESC [ X (meta as escape) and ESC [ 1 ; 3 X (xterm).
ALT_UP = b"\x1b\x1b[A"
ALT_DOWN = b"\x1b\x1b[B"
ALT_RIGHT = b"\x1b\x1b[C"
ALT_LEFT = b"\x1b\x1b[D"
ALT_UP_CSI = b"\x1b[1;3A"
ALT_DOWN_CSI = b"\x1b[1;3B"
ALT_RIGHT_CSI = b"\x1b[1;3C"
ALT_LEFT_CSI = b"\x1b[1;3D"


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


def detach(client):
    client.send(SESSION + b"d")
    client.wait_exit(10)


def scenario_tabs(res):
    begin("with one pane the arrows walk the tabs")
    client = Client()
    try:
        res.check("normal state", wait_status(client, "[NORMAL] main"), status(client))
        client.send(TAB + b"n")
        res.check("a second tab exists", wait_status(client, "2:shell*"), status(client))
        client.send(TAB + b"n")
        res.check("a third tab exists", wait_status(client, "3:shell*"), status(client))

        client.send(ALT_RIGHT)
        res.check("Alt+Right moves to the next tab", wait_status(client, "1:shell*"),
                  status(client))
        client.send(ALT_RIGHT_CSI)
        res.check("the modified CSI form moves as well", wait_status(client, "2:shell*"),
                  status(client))
        client.send(ALT_LEFT)
        res.check("Alt+Left moves to the previous tab", wait_status(client, "1:shell*"),
                  status(client))
        client.send(ALT_LEFT_CSI)
        res.check("Alt+Left wraps to the last tab", wait_status(client, "3:shell*"),
                  status(client))

        client.send(ALT_UP)
        client.wait_quiet(quiet=0.25)
        res.check("Alt+Up does nothing with one pane",
                  "3:shell*" in status(client) and pane_state(client) == (1, 1),
                  status(client))
        client.send(ALT_DOWN_CSI)
        client.wait_quiet(quiet=0.25)
        res.check("Alt+Down does nothing with one pane",
                  "3:shell*" in status(client) and pane_state(client) == (1, 1),
                  status(client))

        detach(client)
    finally:
        client.close()


def scenario_pane_first(res):
    begin("the pane wins, and the tab only takes over at the edge")
    client = Client()
    try:
        client.send(TAB + b"n")
        res.check("a second tab exists", wait_status(client, "2:shell*"), status(client))
        client.send(TAB + b"1")
        res.check("back on the first tab", wait_status(client, "1:shell*"), status(client))
        client.send(PANE + b"n")
        res.check("the tab is split to the right", wait_pane_state(client, 2, 2),
                  status(client))
        client.send("echo RIGHT-PANE\n")
        res.check("the right pane takes input", wait_text(client, "RIGHT-PANE"))

        client.send(ALT_LEFT)
        res.check("Alt+Left moves to the pane on the left", wait_pane_state(client, 1, 2),
                  status(client))
        res.check("the tab did not move", "1:shell*" in status(client), status(client))
        client.send("echo LEFT-PANE\n")
        res.check("the left pane takes input", wait_text(client, "LEFT-PANE"))

        client.send(ALT_RIGHT_CSI)
        res.check("Alt+Right moves back to the right pane", wait_pane_state(client, 2, 2),
                  status(client))
        res.check("the tab still did not move", "1:shell*" in status(client), status(client))

        client.send(ALT_DOWN_CSI)
        client.wait_quiet(quiet=0.25)
        res.check("Alt+Down does nothing without a pane below",
                  pane_state(client) == (2, 2) and "1:shell*" in status(client),
                  status(client))
        client.send(ALT_UP)
        client.wait_quiet(quiet=0.25)
        res.check("Alt+Up does nothing without a pane above",
                  pane_state(client) == (2, 2) and "1:shell*" in status(client),
                  status(client))

        client.send(ALT_RIGHT)
        res.check("Alt+Right at the right edge moves to the next tab",
                  wait_status(client, "2:shell*"), status(client))
        res.check("the other tab has a single pane", pane_state(client) == (1, 1),
                  status(client))
        client.send(ALT_LEFT)
        res.check("Alt+Left wraps back to the split tab", wait_status(client, "1:shell*"),
                  status(client))
        res.check("the split came back with the focus it had",
                  pane_state(client) == (2, 2), status(client))

        client.send(ALT_LEFT)
        res.check("Alt+Left moves to the left pane first", wait_pane_state(client, 1, 2),
                  status(client))
        res.check("the tab did not move on that press", "1:shell*" in status(client),
                  status(client))
        client.send(ALT_LEFT)
        res.check("a second Alt+Left at the edge moves the tab",
                  wait_status(client, "2:shell*"), status(client))

        detach(client)
    finally:
        client.close()


def scenario_no_leak(res):
    begin("an Alt arrow never reaches the pane")
    client = Client()
    try:
        client.send("cat -v\n")
        client.wait_quiet(quiet=0.35)
        for keys in (ALT_RIGHT, ALT_LEFT_CSI, ALT_UP, ALT_DOWN_CSI):
            client.send(keys)
            client.pump(0.1)
        client.wait_quiet(quiet=0.35)
        screen = client.screen.text()
        res.check("no escape sequence was echoed by the pane",
                  "^[" not in screen and "[1;3" not in screen, repr(screen[-200:]))
        res.check("the pane is still running the shell",
                  pane_state(client) == (1, 1), status(client))

        client.send(b"\x04")
        client.wait_quiet(quiet=0.35)
        client.send("echo AFTER-ALT\n")
        res.check("the pane answers after the Alt keys", wait_text(client, "AFTER-ALT"))
        detach(client)
    finally:
        client.close()


def main():
    res = Result()
    try:
        scenario_tabs(res)
        scenario_pane_first(res)
        scenario_no_leak(res)
    finally:
        cleanup()
    return res.report("Alt plus arrow: pane first, then the tab")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)

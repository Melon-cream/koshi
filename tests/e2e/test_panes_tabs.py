#!/usr/bin/env python3
"""M2 acceptance: panes, tabs, leader groups, status bar, and workspaces.

Everything is checked through the real client's PTY output: the status bar is
the last screen row, panes are the rows above it, and the workspace overlay is
drawn on top of the pane area.
"""

import os
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
    normalize_screen,
    server_alive,
    wait_pane_state,
)


def status(client):
    return client.screen.line(ROWS - 1)


# The tab group moves with the arrows only, so these travel as ESC sequences.
PREV_TAB = b"\x1b[D"
NEXT_TAB = b"\x1b[C"


def pump_until(client, predicate, timeout=10.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        client.pump(0.1)
        if predicate(client):
            return True
        if client.exit_code is not None:
            return predicate(client)
    return predicate(client)


def wait_status(client, needle, timeout=10.0):
    return pump_until(client, lambda c: needle in status(c), timeout)


def wait_text(client, needle, timeout=10.0):
    return pump_until(client, lambda c: needle in c.screen.text(), timeout)


def wait_gone(timeout=10.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if not server_alive():
            return True
        time.sleep(0.1)
    return False


def begin(name):
    cleanup()
    clean_runtime()
    print("scenario: %s" % name)


def pane(client, key):
    """Ctrl+p and one key: the pane leader group."""
    client.send(b"\x10" + key)


def tab(client, key):
    """Ctrl+t and one key: the tab leader group."""
    client.send(b"\x14" + key)


def session(client, key):
    """Ctrl+o and one key: the session leader group."""
    client.send(b"\x0f" + key)


def pid_from(screen, marker):
    for line in screen.splitlines():
        if marker in line:
            tail = line.split(marker)[-1].strip()
            digits = "".join(ch for ch in tail if ch.isdigit())
            if digits:
                return int(digits)
    return 0


def pid_alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def screen_after_quiet(client):
    client.wait_quiet()
    return normalize_screen(client.screen.text())


def scenario_status_and_leader(res):
    begin("status bar and leader groups")
    client = Client()
    try:
        ok = wait_status(client, "[NORMAL] main")
        res.check("status bar shows normal mode and workspace", ok, "status=%r" % status(client))
        res.check("status bar lists the first tab", "1:shell*" in status(client), "status=%r" % status(client))
        res.check("status bar shows pane focus", pane_state(client) == (1, 1), "status=%r" % status(client))

        pane(client, b"")
        res.check("leader key shows the pane group", wait_status(client, "[PANE]"), "status=%r" % status(client))
        pane(client, b"\x1b")
        res.check("escape cancels the pending leader", wait_status(client, "[NORMAL]"), "status=%r" % status(client))

        session(client, b"d")
        res.check("detach from the leader exits cleanly", client.wait_exit(10) == 0)
    finally:
        client.close()


def scenario_custom_leader(res):
    begin("leader key from the configuration file")
    config_home = os.path.join(RUNTIME_DIR, "config-home")
    os.makedirs(os.path.join(config_home, "koshi"), exist_ok=True)
    with open(os.path.join(config_home, "koshi", "config.toml"), "w", encoding="utf-8") as handle:
        handle.write('leader = "A"\n')
    client = Client(env={"XDG_CONFIG_HOME": config_home})
    try:
        res.check("custom leader starts in normal mode", wait_status(client, "[NORMAL] main"),
                  "status=%r" % status(client))
        client.send(b"\x1bp")
        res.check("Alt+p opens the pane group", wait_status(client, "[PANE]"),
                  "status=%r" % status(client))
        client.send(b"\x1b")
        res.check("escape cancels the Alt leader", wait_status(client, "[NORMAL]"),
                  "status=%r" % status(client))
        client.send(b"\x10")
        res.check('Ctrl+p is not a leader under leader = "A"',
                  pump_until(client, lambda c: "[NORMAL]" in status(c)), "status=%r" % status(client))
        client.send(b"\x1bod")
        res.check("Alt+o then d detaches", client.wait_exit(10) == 0)
        res.check("server survives the Alt leader detach", server_alive())
    finally:
        client.close()


def scenario_panes(res):
    begin("pane split, focus, fullscreen, and close")
    client = Client()
    try:
        client.send("echo PANE-ONE\n")
        res.check("first pane accepts input", wait_text(client, "PANE-ONE"))

        pane(client, b"r")
        res.check("vertical split shows two panes", wait_pane_state(client, 2, 2), "status=%r" % status(client))

        client.send("echo RIGHT-SIDE\n")
        res.check("new pane accepts input", wait_text(client, "RIGHT-SIDE"))

        pane(client, b"p")
        res.check("focus moves to the first pane", wait_pane_state(client, 1, 2), "status=%r" % status(client))
        client.send("echo LEFT-SIDE\n")
        res.check("first pane accepts input again", wait_text(client, "LEFT-SIDE"))

        screen = screen_after_quiet(client)
        res.check("both panes are visible at the same time",
                  "LEFT-SIDE" in screen and "RIGHT-SIDE" in screen)

        pane(client, b"z")
        res.check("fullscreen is shown in the status bar", wait_status(client, "[ZOOM]"), "status=%r" % status(client))
        screen = screen_after_quiet(client)
        res.check("fullscreen hides the other pane", "RIGHT-SIDE" not in screen)
        res.check("fullscreen keeps the focused pane", "LEFT-SIDE" in screen)

        pane(client, b"z")
        res.check("leaving fullscreen restores the split", wait_pane_state(client, 1, 2)
                  and "[ZOOM]" not in status(client), "status=%r" % status(client))
        res.check("the other pane is visible again", wait_text(client, "RIGHT-SIDE"))

        pane(client, b"x")
        res.check("closing a pane updates the status bar", wait_pane_state(client, 1, 1), "status=%r" % status(client))
        screen = screen_after_quiet(client)
        res.check("the remaining pane keeps its output", "RIGHT-SIDE" in screen)

        pane(client, b"d")
        res.check("horizontal split shows two panes", wait_pane_state(client, 2, 2),
                  "status=%r" % status(client))
        client.send("echo BOTTOM-SIDE\n")
        res.check("bottom pane accepts input", wait_text(client, "BOTTOM-SIDE"))
        screen = screen_after_quiet(client)
        res.check("both rows of the horizontal split are visible",
                  "RIGHT-SIDE" in screen and "BOTTOM-SIDE" in screen)
        pane(client, b"x")
        res.check("closing the bottom pane restores one pane", wait_pane_state(client, 1, 1),
                  "status=%r" % status(client))

        client.send("exit\n")
        res.check("shell exit ends the client", client.wait_exit(10) is not None)
    finally:
        client.close()
    res.check("server exits after the pane scenario", wait_gone())


def scenario_tabs(res):
    begin("tab create, switch, and rename")
    client = Client()
    try:
        client.send("echo TAB-ONE\n")
        res.check("tab one accepts input", wait_text(client, "TAB-ONE"))

        tab(client, b"n")
        res.check("new tab is listed in the status bar", wait_status(client, "2:shell*"), "status=%r" % status(client))
        client.send("echo TAB-TWO\n")
        res.check("tab two accepts input", wait_text(client, "TAB-TWO"))

        tab(client, PREV_TAB)
        res.check("previous tab becomes active", wait_status(client, "1:shell*"), "status=%r" % status(client))
        screen = screen_after_quiet(client)
        res.check("switching tabs preserves the old tab", "TAB-ONE" in screen and "TAB-TWO" not in screen)

        tab(client, NEXT_TAB)
        res.check("next tab becomes active", wait_status(client, "2:shell*"), "status=%r" % status(client))
        screen = screen_after_quiet(client)
        res.check("switching back preserves the second tab", "TAB-TWO" in screen and "TAB-ONE" not in screen)

        tab(client, b"r")
        res.check("tab rename prompt is shown", wait_status(client, "[RENAME]"), "status=%r" % status(client))
        client.send("logs")
        res.check("rename input is echoed in the status bar", wait_status(client, "tab name: logs_"),
                  "status=%r" % status(client))
        client.send(b"\r")
        res.check("renamed tab appears in the status bar", wait_status(client, "2:logs*"),
                  "status=%r" % status(client))

        tab(client, PREV_TAB)
        wait_status(client, "1:shell*")
        tab(client, NEXT_TAB)
        res.check("renamed tab survives switching", wait_status(client, "2:logs*"), "status=%r" % status(client))

        client.send("exit\n")
        res.check("closing the renamed tab leaves the first tab",
                  wait_status(client, "1:shell*"), "status=%r" % status(client))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()
    res.check("server exits after the tab scenario", wait_gone())


def scenario_workspaces(res):
    begin("workspace overlay: create, switch, rename, delete")
    client = Client()
    try:
        client.send("echo MAIN-OUTPUT\n")
        res.check("main workspace accepts input", wait_text(client, "MAIN-OUTPUT"))
        client.send("sleep 300 & echo MAIN-SLEEPER=$!\n")
        screen = wait_text(client, "MAIN-SLEEPER=") and client.screen.text()
        sleeper = pid_from(screen or "", "MAIN-SLEEPER=")
        res.check("main workspace process started", sleeper > 0, "pid=%s" % sleeper)

        session(client, b"w")
        res.check("overlay opens", wait_text(client, "Workspaces"))
        res.check("overlay lists the current workspace", wait_text(client, "main"))
        res.check("overlay mode is shown", wait_status(client, "workspace list"), "status=%r" % status(client))

        client.send("n")
        res.check("overlay creates a workspace", wait_text(client, "ws2") and wait_status(client, "ws2"),
                  "status=%r" % status(client))
        res.check("new workspace is selected", wait_text(client, "> ws2"))
        client.send(b"\x1b")
        res.check("escape closes the overlay", wait_status(client, "[NORMAL] ws2"), "status=%r" % status(client))

        client.send("echo WS2-OUTPUT\n")
        res.check("new workspace accepts input", wait_text(client, "WS2-OUTPUT"))

        session(client, b"w")
        wait_text(client, "Workspaces")
        client.send("k")
        res.check("selection moves up to main", wait_text(client, "> main"))
        client.send(b"\r")
        res.check("enter switches to main", wait_status(client, "[NORMAL] main"), "status=%r" % status(client))
        screen = screen_after_quiet(client)
        res.check("main workspace keeps its output", "MAIN-OUTPUT" in screen)
        res.check("other workspace output is not shown", "WS2-OUTPUT" not in screen)
        res.check("main workspace process survived the switch", pid_alive(sleeper),
                  "pid=%s" % sleeper)

        session(client, b"w")
        wait_text(client, "Workspaces")
        client.send("j")
        wait_text(client, "> ws2")
        client.send(b"\r")
        res.check("switching back restores ws2", wait_status(client, "[NORMAL] ws2"), "status=%r" % status(client))
        screen = screen_after_quiet(client)
        res.check("ws2 output survived the round trip", "WS2-OUTPUT" in screen)
        res.check("main output is not shown in ws2", "MAIN-OUTPUT" not in screen)

        session(client, b"w")
        wait_text(client, "Workspaces")
        client.send("r")
        res.check("workspace rename prompt is shown", wait_status(client, "[RENAME]"), "status=%r" % status(client))
        client.send("proj")
        res.check("rename input is echoed", wait_text(client, "rename: proj_"))
        client.send(b"\r")
        res.check("renamed workspace is shown in the status bar", wait_status(client, "proj"),
                  "status=%r" % status(client))
        res.check("renamed workspace is shown in the overlay", wait_text(client, "proj"))
        client.send(b"\x1b")
        res.check("overlay closes after rename", wait_status(client, "[NORMAL] proj"), "status=%r" % status(client))

        session(client, b"w")
        wait_text(client, "Workspaces")
        client.send("d")
        res.check("delete removes the workspace", wait_status(client, "main"), "status=%r" % status(client))
        screen = screen_after_quiet(client)
        res.check("deleted workspace disappears from the overlay", "proj" not in screen)
        res.check("the remaining workspace is shown", "MAIN-OUTPUT" in screen)
        client.send(b"\x1b")
        wait_status(client, "[NORMAL] main")

        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()
    res.check("server exits after the workspace scenario", wait_gone())


def scenario_last_workspace_and_tab(res):
    begin("reattach returns to the last workspace and tab")
    client = Client()
    try:
        client.send("echo KEEP-MAIN\n")
        wait_text(client, "KEEP-MAIN")
        session(client, b"w")
        wait_text(client, "Workspaces")
        client.send("n")
        wait_text(client, "ws2")
        client.send(b"\x1b")
        wait_status(client, "[NORMAL] ws2")

        client.send("echo KEEP-WS2\n")
        wait_text(client, "KEEP-WS2")
        tab(client, b"n")
        wait_status(client, "2:shell*")
        client.send("echo KEEP-TAB2\n")
        wait_text(client, "KEEP-TAB2")

        session(client, b"d")
        res.check("detach from the second workspace", client.wait_exit(10) == 0)
        res.check("server stays alive", server_alive())

        second = Client()
        try:
            res.check("reattach returns to the last workspace", wait_status(second, "[NORMAL] ws2"),
                      "status=%r" % status(second))
            res.check("reattach returns to the last tab", wait_status(second, "2:shell*"),
                      "status=%r" % status(second))
            screen = screen_after_quiet(second)
            res.check("last tab content is rebuilt", "KEEP-TAB2" in screen)
            res.check("the previous tab is not shown", "KEEP-WS2" not in screen)

            tab(second, PREV_TAB)
            res.check("switching to the first tab works", wait_status(second, "1:shell*"),
                      "status=%r" % status(second))
            res.check("first tab content survived", wait_text(second, "KEEP-WS2"))

            session(second, b"w")
            wait_text(second, "Workspaces")
            second.send("k")
            wait_text(second, "> main")
            second.send(b"\r")
            res.check("switching to main after reattach", wait_status(second, "[NORMAL] main"),
                      "status=%r" % status(second))
            res.check("main content survived", wait_text(second, "KEEP-MAIN"))

            second.detach_keys()
            res.check("detach from the restored session", second.wait_exit(10) == 0)
        finally:
            second.close()
    finally:
        client.close()


def scenario_concurrent_panes(res):
    begin("two clients share split panes")
    alice = Client()
    bob = None
    try:
        alice.send("echo SHARED-BASE\n")
        res.check("first client shows base output", wait_text(alice, "SHARED-BASE"))
        bob = Client()
        res.check("second client sees the same base output", wait_text(bob, "SHARED-BASE"))

        pane(alice, b"r")
        res.check("first client sees the split", wait_pane_state(alice, 2, 2),
                  "status=%r" % status(alice))
        res.check("second client sees the split", wait_pane_state(bob, 2, 2),
                  "status=%r" % status(bob))

        alice.send("echo SHARED-SPLIT\n")
        res.check("split output reaches the first client", wait_text(alice, "SHARED-SPLIT"))
        res.check("split output reaches the second client", wait_text(bob, "SHARED-SPLIT"))

        bob.send("echo FROM-BOB-PANE\n")
        res.check("second client input reaches the first", wait_text(alice, "FROM-BOB-PANE"))
        res.check("second client input reaches itself", wait_text(bob, "FROM-BOB-PANE"))

        alice.wait_quiet()
        bob.wait_quiet()
        res.check("both clients show the same split screen",
                  normalize_screen(alice.screen.text()) == normalize_screen(bob.screen.text()))

        bob.detach_keys()
        res.check("second client detaches", bob.wait_exit(10) == 0)
        res.check("server survives the second client", server_alive())
        alice.send("echo ALICE-AFTER-DETACH\n")
        res.check("first client stays usable", wait_text(alice, "ALICE-AFTER-DETACH"))

        pane(alice, b"x")
        res.check("closing one pane works", wait_pane_state(alice, 1, 1),
                  "status=%r" % status(alice))
        alice.send("exit\n")
        alice.wait_exit(10)
    finally:
        alice.close()
        if bob is not None:
            bob.close()
    res.check("server exits after the concurrent pane scenario", wait_gone())


def main():
    res = Result()
    try:
        scenario_status_and_leader(res)
        scenario_custom_leader(res)
        scenario_panes(res)
        scenario_tabs(res)
        scenario_workspaces(res)
        scenario_last_workspace_and_tab(res)
        scenario_concurrent_panes(res)
    finally:
        cleanup()
    return res.report("panes, tabs, leader groups, status bar, workspaces")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
